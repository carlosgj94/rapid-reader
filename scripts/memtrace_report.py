#!/usr/bin/env python3

from __future__ import annotations

import argparse
import csv
import re
import subprocess
from collections import defaultdict
from pathlib import Path


INT_RE = re.compile(r"^-?\d+$")
ANSI_ESCAPE_RE = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
MEMTRACE_PREFIX = "MEMTRACE "


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Parse motif MEMTRACE serial logs into CSV and Markdown reports."
    )
    parser.add_argument("logfile", type=Path, help="Captured serial log file")
    parser.add_argument(
        "--out-dir",
        type=Path,
        default=Path("memtrace-report"),
        help="Directory to write reports into",
    )
    parser.add_argument(
        "--elf",
        type=Path,
        default=None,
        help="Optional built firmware ELF for llvm-size / llvm-nm reports",
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path(__file__).resolve().parents[1],
        help="Repository root; used for TLS config inspection",
    )
    return parser.parse_args()


def coerce(value: str):
    if INT_RE.match(value):
        return int(value)
    return value


def backfill_heap_fields(fields: dict[str, object]) -> dict[str, object]:
    region_rows = []
    for index in range(3):
        kind = str(fields.get(f"region{index}_kind", "none"))
        size = fields.get(f"region{index}_size")
        used = fields.get(f"region{index}_used")
        free = fields.get(f"region{index}_free")
        if isinstance(used, int) and f"region{index}_peak_used" not in fields:
            fields[f"region{index}_peak_used"] = used
        if isinstance(free, int) and f"region{index}_min_free" not in fields:
            fields[f"region{index}_min_free"] = free
        region_rows.append(
            {
                "kind": kind,
                "size": size if isinstance(size, int) else 0,
                "used": used if isinstance(used, int) else 0,
                "free": free if isinstance(free, int) else 0,
                "peak_used": fields.get(f"region{index}_peak_used", 0),
                "min_free": fields.get(f"region{index}_min_free", 0),
            }
        )

    def apply_capability(prefix: str, kinds: tuple[str, ...]) -> None:
        matching = [row for row in region_rows if row["kind"] in kinds]
        if not matching:
            return
        if f"{prefix}_heap_regions" not in fields:
            fields[f"{prefix}_heap_regions"] = len(matching)
        if f"{prefix}_heap_size" not in fields:
            fields[f"{prefix}_heap_size"] = sum(int(row["size"]) for row in matching)
        if f"{prefix}_heap_used" not in fields:
            fields[f"{prefix}_heap_used"] = sum(int(row["used"]) for row in matching)
        if f"{prefix}_heap_free" not in fields:
            fields[f"{prefix}_heap_free"] = sum(int(row["free"]) for row in matching)
        if f"{prefix}_heap_peak_used" not in fields:
            fields[f"{prefix}_heap_peak_used"] = sum(
                int(row["peak_used"]) for row in matching
            )
        if f"{prefix}_heap_min_free" not in fields:
            fields[f"{prefix}_heap_min_free"] = sum(
                int(row["min_free"]) for row in matching
            )

    apply_capability("internal", ("internal", "mixed"))
    apply_capability("external", ("external", "mixed"))
    return fields


def parse_memtrace_line(line: str) -> dict[str, object] | None:
    line = ANSI_ESCAPE_RE.sub("", line)
    if MEMTRACE_PREFIX not in line:
        return None
    payload = line.split(MEMTRACE_PREFIX, 1)[1].strip()
    fields: dict[str, object] = {}
    for token in payload.split():
        if "=" not in token:
            continue
        key, value = token.split("=", 1)
        fields[key] = coerce(value)
    if not fields:
        return None
    return backfill_heap_fields(fields)


def ensure_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)


def write_csv(path: Path, rows: list[dict[str, object]]) -> None:
    keys = sorted({key for row in rows for key in row})
    with path.open("w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=keys)
        writer.writeheader()
        for row in rows:
            writer.writerow(row)


def max_int(rows: list[dict[str, object]], key: str) -> int:
    values = [int(row[key]) for row in rows if isinstance(row.get(key), int)]
    return max(values, default=0)


def min_int(rows: list[dict[str, object]], key: str) -> int | None:
    values = [int(row[key]) for row in rows if isinstance(row.get(key), int)]
    return min(values) if values else None


def summarize_requests(requests: list[dict[str, object]]) -> list[str]:
    if not requests:
        return ["No `request_complete` events found."]

    slowest = max(requests, key=lambda row: int(row.get("total_ms", 0)))
    largest = max(requests, key=lambda row: int(row.get("body_bytes", 0)))
    buffered = [
        row for row in requests if int(row.get("response_buffer_capacity", 0) or 0) > 0
    ]
    tightest = (
        min(
            buffered,
            key=lambda row: int(row.get("response_buffer_headroom", 1 << 60)),
        )
        if buffered
        else None
    )
    tightest_internal = min(
        requests,
        key=lambda row: int(row.get("internal_heap_free", 1 << 60)),
    )
    highest_external = max(
        requests,
        key=lambda row: int(row.get("external_heap_used", 0)),
    )

    lines = [
        f"Count: {len(requests)}",
        f"Slowest: `{slowest.get('path', '-')}` in {slowest.get('total_ms', 0)} ms",
        f"Largest body: `{largest.get('path', '-')}` at {largest.get('body_bytes', 0)} bytes",
        f"Tightest request-complete internal headroom: `{tightest_internal.get('path', '-')}` with {tightest_internal.get('internal_heap_free', 0)} bytes left",
    ]
    if tightest is not None:
        lines.append(
            f"Tightest buffered headroom: `{tightest.get('path', '-')}` with {tightest.get('response_buffer_headroom', 0)} bytes left"
        )
    if int(highest_external.get("external_heap_used", 0)) > 0:
        lines.append(
            f"Highest request-complete external heap usage: `{highest_external.get('path', '-')}` at {highest_external.get('external_heap_used', 0)} bytes"
        )
    else:
        lines.append("Highest request-complete external heap usage: 0 bytes")
    return lines


def build_article_rows(events: list[dict[str, object]]) -> list[dict[str, object]]:
    articles: dict[tuple[object, object], dict[str, object]] = {}

    for event in events:
        kind = event.get("kind")
        content_id = event.get("content_id")
        sync_id = event.get("sync_id")
        if not content_id or sync_id is None:
            continue
        if kind not in {"storage_stage", "reader_package", "reader_open", "reader_window"}:
            continue

        key = (sync_id, content_id)
        row = articles.setdefault(
            key,
            {
                "sync_id": sync_id,
                "content_id": content_id,
            },
        )
        action = event.get("action")
        if kind == "storage_stage" and action == "begin":
            row["collection"] = event.get("collection", row.get("collection", ""))
            row["slot_id"] = event.get("slot_id", row.get("slot_id", 0))
            row["stage_started_at_ms"] = event.get("at_ms", 0)
            row["stage_result"] = "started"
        elif kind == "storage_stage" and action == "commit":
            row["collection"] = event.get("collection", row.get("collection", ""))
            row["slot_id"] = event.get("slot_id", row.get("slot_id", 0))
            row["remote_item_id"] = event.get("remote_item_id", "")
            row["stage_bytes_written"] = event.get("bytes_written", 0)
            row["copied_bytes"] = event.get("copied_bytes", 0)
            row["stage_elapsed_ms"] = event.get("elapsed_ms", 0)
            row["stage_result"] = "committed"
            row["total_units"] = event.get("total_units", 0)
            row["paragraph_count"] = event.get("paragraph_count", 0)
            row["sd_free_bytes_after"] = event.get("sd_free_bytes", 0)
            row["motif_total_bytes_after"] = event.get("motif_total_bytes", 0)
        elif kind == "storage_stage" and action == "abort":
            row["slot_id"] = event.get("slot_id", row.get("slot_id", 0))
            row["stage_bytes_written"] = event.get("bytes_written", 0)
            row["stage_elapsed_ms"] = event.get("elapsed_ms", 0)
            row["stage_result"] = "aborted"
        elif kind == "reader_package":
            row["package_open_total_units"] = event.get("total_units", 0)
            row["package_open_paragraphs"] = event.get("paragraph_count", 0)
            row["package_size_bytes"] = event.get("size_bytes", 0)
            row["initial_window_units"] = event.get("initial_window_units", 0)
        elif kind == "reader_open":
            row["reader_action"] = action
            row["reader_bytes_read"] = event.get("bytes_read", 0)
            row["reader_parse_ms"] = event.get("parse_ms", 0)
            row["reader_total_ms"] = event.get("total_ms", 0)
            row["reader_units"] = event.get("unit_count", 0)
            row["reader_paragraphs"] = event.get("paragraph_count", 0)
            row["reader_truncated"] = event.get("truncated", 0)
        elif kind == "reader_window":
            row["last_window_start"] = event.get("loaded_start", 0)
            row["last_window_units"] = event.get("unit_count", 0)

    return sorted(articles.values(), key=lambda row: (row["sync_id"], row["content_id"]))


def summarize_articles(article_rows: list[dict[str, object]]) -> list[str]:
    if not article_rows:
        return ["No article-level events found."]

    committed = [row for row in article_rows if row.get("stage_result") == "committed"]
    if not committed:
        return [f"Tracked article operations: {len(article_rows)}"]

    largest = max(committed, key=lambda row: int(row.get("stage_bytes_written", 0)))
    slowest = max(committed, key=lambda row: int(row.get("stage_elapsed_ms", 0)))
    return [
        f"Tracked article operations: {len(article_rows)}",
        f"Largest committed article: `{largest.get('content_id', '-')}` at {largest.get('stage_bytes_written', 0)} bytes",
        f"Slowest commit: `{slowest.get('content_id', '-')}` in {slowest.get('stage_elapsed_ms', 0)} ms",
    ]


def build_storage_rows(events: list[dict[str, object]]) -> list[dict[str, object]]:
    kinds = {"storage_stage", "storage_evict", "storage_snapshot", "storage_queue", "boot_state"}
    rows = [event for event in events if event.get("kind") in kinds]
    return sorted(rows, key=lambda row: int(row.get("event_id", 0)))


def summarize_storage(storage_rows: list[dict[str, object]]) -> list[str]:
    if not storage_rows:
        return ["No storage events found."]

    queue_peak = max_int(storage_rows, "queue_depth_peak")
    payload_peak = max_int(storage_rows, "queue_payload_peak")
    min_sd_free = min_int(
        [row for row in storage_rows if int(row.get("sd_free_known", 0)) == 1],
        "sd_free_bytes",
    )
    lines = [
        f"Queue depth peak: {queue_peak}",
        f"Queued payload peak: {payload_peak} bytes",
    ]
    if min_sd_free is not None:
        lines.append(f"Lowest observed SD free space: {min_sd_free} bytes")
    return lines


def describe_event(row: dict[str, object]) -> str:
    parts: list[str] = []
    for key in ("kind", "component", "action", "phase", "path", "content_id"):
        value = row.get(key)
        if value in (None, "", 0):
            continue
        parts.append(f"{key}={value}")
    if not parts and "event_id" in row:
        parts.append(f"event_id={row['event_id']}")
    return " ".join(parts)


def summarize_regions(events: list[dict[str, object]]) -> list[str]:
    lines: list[str] = []
    for index in range(3):
        size_key = f"region{index}_size"
        kind_key = f"region{index}_kind"
        peak_key = f"region{index}_peak_used"
        min_key = f"region{index}_min_free"
        free_key = f"region{index}_free"
        rows = [
            row
            for row in events
            if isinstance(row.get(size_key), int) and int(row.get(size_key, 0)) > 0
        ]
        if not rows:
            continue
        kind = next(
            (
                str(row.get(kind_key, "unknown"))
                for row in rows
                if str(row.get(kind_key, "unknown")) != "none"
            ),
            "unknown",
        )
        tightest = min(rows, key=lambda row: int(row.get(free_key, 1 << 60)))
        lines.append(
            f"Region {index} ({kind}): peak used {max_int(rows, peak_key)} bytes, lowest free {min_int(rows, min_key) or 0} bytes, tightest current free {tightest.get(free_key, 0)} bytes at `{describe_event(tightest)}`"
        )
    return lines


def summarize_heap(events: list[dict[str, object]]) -> list[str]:
    lines = [
        f"Peak allocator usage: {max_int(events, 'heap_peak')} bytes",
        f"Peak live heap: {max_int(events, 'heap_used')} bytes",
        f"Lowest free heap: {min_int(events, 'heap_free') or 0} bytes",
        f"Total allocated counter max: {max_int(events, 'heap_total_allocated')} bytes",
        f"Total freed counter max: {max_int(events, 'heap_total_freed')} bytes",
    ]
    internal_rows = [
        row for row in events if isinstance(row.get("internal_heap_free"), int)
    ]
    if internal_rows:
        tightest_internal = min(
            internal_rows,
            key=lambda row: int(row.get("internal_heap_free", 1 << 60)),
        )
        lines.extend(
            [
                f"Peak internal live heap: {max_int(internal_rows, 'internal_heap_used')} bytes",
                f"Peak internal used watermark: {max_int(internal_rows, 'internal_heap_peak_used')} bytes",
                f"Lowest internal free heap: {min_int(internal_rows, 'internal_heap_free') or 0} bytes",
                f"Tightest internal headroom event: `{describe_event(tightest_internal)}` at {tightest_internal.get('internal_heap_free', 0)} bytes free",
            ]
        )
    external_rows = [
        row
        for row in events
        if isinstance(row.get("external_heap_size"), int)
        and int(row.get("external_heap_size", 0)) > 0
    ]
    if external_rows:
        tightest_external = min(
            external_rows,
            key=lambda row: int(row.get("external_heap_free", 1 << 60)),
        )
        lines.extend(
            [
                f"Peak external live heap: {max_int(external_rows, 'external_heap_used')} bytes",
                f"Peak external used watermark: {max_int(external_rows, 'external_heap_peak_used')} bytes",
                f"Lowest external free heap: {min_int(external_rows, 'external_heap_free') or 0} bytes",
                f"Tightest external headroom event: `{describe_event(tightest_external)}` at {tightest_external.get('external_heap_free', 0)} bytes free",
            ]
        )
    lines.extend(summarize_regions(events))
    return lines


def read_runtime_static_inventory(events: list[dict[str, object]]) -> dict[str, dict[str, object]]:
    inventory: dict[str, dict[str, object]] = {}
    for event in events:
        if event.get("kind") == "static_inventory":
            component = str(event.get("component", "unknown"))
            inventory[component] = event
    return inventory


def build_heap_rows(events: list[dict[str, object]]) -> list[dict[str, object]]:
    keys = [
        "event_id",
        "at_ms",
        "kind",
        "component",
        "action",
        "phase",
        "path",
        "sync_id",
        "req_id",
        "content_id",
        "heap_size",
        "heap_used",
        "heap_free",
        "heap_peak",
        "heap_total_allocated",
        "heap_total_freed",
        "internal_heap_regions",
        "internal_heap_size",
        "internal_heap_used",
        "internal_heap_free",
        "internal_heap_peak_used",
        "internal_heap_min_free",
        "external_heap_regions",
        "external_heap_size",
        "external_heap_used",
        "external_heap_free",
        "external_heap_peak_used",
        "external_heap_min_free",
        "region0_kind",
        "region0_size",
        "region0_used",
        "region0_free",
        "region0_peak_used",
        "region0_min_free",
        "region1_kind",
        "region1_size",
        "region1_used",
        "region1_free",
        "region1_peak_used",
        "region1_min_free",
        "region2_kind",
        "region2_size",
        "region2_used",
        "region2_free",
        "region2_peak_used",
        "region2_min_free",
    ]
    rows = []
    for event in events:
        row = {key: event[key] for key in keys if key in event}
        rows.append(row)
    return rows


def parse_mbedtls_config(config_path: Path) -> dict[str, bool]:
    interesting = {
        "MBEDTLS_PLATFORM_MEMORY",
        "MBEDTLS_PSA_CRYPTO_C",
        "MBEDTLS_SSL_KEEP_PEER_CERTIFICATE",
        "MBEDTLS_SSL_PROTO_TLS1_3",
        "MBEDTLS_SSL_VARIABLE_BUFFER_LENGTH",
    }
    if not config_path.exists():
        return {}

    enabled = set()
    for line in config_path.read_text().splitlines():
        line = line.strip()
        if not line.startswith("#define "):
            continue
        parts = line.split()
        if len(parts) >= 2 and parts[1] in interesting:
            enabled.add(parts[1])

    return {name: name in enabled for name in sorted(interesting)}


def run_command(command: list[str]) -> str:
    try:
        completed = subprocess.run(
            command,
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError) as exc:
        return f"Command failed: {' '.join(command)}\n{exc}"
    return completed.stdout.strip()


def build_static_report(
    out_path: Path,
    events: list[dict[str, object]],
    repo_root: Path,
    elf: Path | None,
) -> None:
    inventory = read_runtime_static_inventory(events)
    config = parse_mbedtls_config(
        repo_root / "third_party/mbedtls-rs-sys/gen/include/config.h"
    )

    lines = ["# Static Report", ""]
    lines.append("## Runtime Inventory")
    if inventory:
        for component in sorted(inventory):
            lines.append(f"### {component}")
            for key in sorted(inventory[component]):
                if key in {"kind", "component"}:
                    continue
                lines.append(f"- `{key}`: {inventory[component][key]}")
    else:
        lines.append("- No `static_inventory` events found.")

    lines.append("")
    lines.append("## TLS Config")
    if config:
        for key, enabled in config.items():
            lines.append(f"- `{key}`: {'enabled' if enabled else 'disabled'}")
    else:
        lines.append("- No MbedTLS config file found.")

    if elf is not None:
        lines.append("")
        lines.append("## ELF Sections")
        lines.append("```text")
        lines.append(run_command(["llvm-size", "-A", str(elf)]))
        lines.append("```")
        lines.append("")
        lines.append("## Largest Symbols")
        lines.append("```text")
        lines.append(run_command(["llvm-nm", "--print-size", "--size-sort", str(elf)]))
        lines.append("```")

    out_path.write_text("\n".join(lines) + "\n")


def build_summary(
    out_path: Path,
    events: list[dict[str, object]],
    heap_rows: list[dict[str, object]],
    requests: list[dict[str, object]],
    articles: list[dict[str, object]],
    storage_rows: list[dict[str, object]],
) -> None:
    failures = [
        event
        for event in events
        if str(event.get("action", "")).endswith("failed")
        or event.get("status", 0) not in (0, 200)
        and event.get("kind") == "request_complete"
    ]
    lines = ["# Memtrace Summary", ""]
    lines.append("## Overview")
    lines.append(f"- Total MEMTRACE events: {len(events)}")
    lines.append(f"- Total heap rows: {len(heap_rows)}")
    lines.append(f"- Total requests: {len(requests)}")
    lines.append(f"- Total article rows: {len(articles)}")
    lines.append(f"- Failure-like events: {len(failures)}")
    lines.append("")
    lines.append("## Heap")
    for line in summarize_heap(events):
        lines.append(f"- {line}")
    lines.append("")
    lines.append("## Requests")
    for line in summarize_requests(requests):
        lines.append(f"- {line}")
    lines.append("")
    lines.append("## Articles")
    for line in summarize_articles(articles):
        lines.append(f"- {line}")
    lines.append("")
    lines.append("## Storage")
    for line in summarize_storage(storage_rows):
        lines.append(f"- {line}")
    out_path.write_text("\n".join(lines) + "\n")


def main() -> None:
    args = parse_args()
    ensure_dir(args.out_dir)

    events = []
    for line in args.logfile.read_text(errors="replace").splitlines():
        parsed = parse_memtrace_line(line)
        if parsed is not None:
            events.append(parsed)
    events.sort(key=lambda row: int(row.get("event_id", 0)))

    heap_rows = build_heap_rows(events)
    requests = [row for row in events if row.get("kind") == "request_complete"]
    articles = build_article_rows(events)
    storage_rows = build_storage_rows(events)

    write_csv(args.out_dir / "heap.csv", heap_rows)
    write_csv(args.out_dir / "requests.csv", requests)
    write_csv(args.out_dir / "articles.csv", articles)
    write_csv(args.out_dir / "storage.csv", storage_rows)
    build_summary(
        args.out_dir / "summary.md",
        events,
        heap_rows,
        requests,
        articles,
        storage_rows,
    )
    build_static_report(
        args.out_dir / "static-report.md",
        events,
        args.repo_root,
        args.elf,
    )


if __name__ == "__main__":
    main()
