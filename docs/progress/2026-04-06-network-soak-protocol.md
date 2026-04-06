# Network Soak Protocol

This is the lightweight manual soak loop for comparing firmware runs after the
April 6 reliability tickets.

## Goal

Produce repeatable log captures and comparable reliability summaries without
needing lab automation.

## Capture Command

Use a unique label per run:

```bash
CARGO_TERM_COLOR=never cargo run --release 2>&1 | tee /tmp/motif-<label>.log
```

Generate the report immediately after the session:

```bash
python3 scripts/memtrace_report.py /tmp/motif-<label>.log --out-dir /tmp/motif-<label>-report
```

Primary outputs:

- `summary.md`: human-readable run summary
- `sli.csv`: one-row reliability snapshot for spreadsheet-style comparison
- `request-class-summary.csv`: per-class attempts, success ratio, median, p95
- `log-events.csv`: matched retries, disconnects, fallback, timeout, and SD
  failure lines

## Manual Session Steps

Keep the session short enough to repeat but broad enough to exercise startup,
metadata, cached reads, and uncached package fetch:

1. Boot to the dashboard and wait for `backend status=Ready`.
2. Stay on the dashboard for at least `30s` so background sync has time to
   settle.
3. Open the `Saved` collection and open one cached article.
4. Return to `Saved` and open one uncached article so a package fetch, stage,
   commit, and open happen on the run.
5. In the reader, rotate enough to force at least one additional reader-window
   load.
6. Return to `Saved` and, if available, open one more article whose package
   state differs from the previous one.

If the firmware is specifically under sleep/wake review, add one suspend/resume
cycle after step 4 and note that in the run label.

## Comparison Rules

- Compare runs using `sli.csv` and `request-class-summary.csv`, not by feel.
- Treat `package_success_ratio`, `startup_retry_count`, timeout counts, and SD
  failure counts as the primary reliability gates.
- Treat request-class median and p95 as the primary latency view.
- Do not treat a higher SD clock or other tuning as a win unless the run stays
  clean and the latency summaries improve materially.

## Notes

- `package_success_ratio` is calculated as successful
  `streaming_package` completions divided by `streaming_package` open attempts.
- Request-class latency summaries are derived from successful
  `request_complete` events.
- DNS fallback visibility is expected in `summary.md` and `log-events.csv`
  whenever Ticket 11 is actually exercised on device.
