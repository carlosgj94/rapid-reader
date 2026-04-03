#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  bash scripts/wipe_sd_card.sh [--yes] <mounted-sd-path>

Examples:
  bash scripts/wipe_sd_card.sh /run/media/$USER/MOTIF_SD
  bash scripts/wipe_sd_card.sh --yes /media/$USER/MOTIF_SD

This deletes all files and directories under the mounted SD card root.
EOF
}

confirm=false

while (($# > 0)); do
  case "$1" in
    --yes)
      confirm=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      break
      ;;
  esac
done

if (($# != 1)); then
  usage
  exit 1
fi

mount_path="$1"

if [[ ! -d "$mount_path" ]]; then
  echo "error: '$mount_path' is not a directory" >&2
  exit 1
fi

resolved_mount_path="$(cd "$mount_path" && pwd -P)"

case "$resolved_mount_path" in
  /|/boot|/home|/tmp|/var|/usr|/opt)
    echo "error: refusing to wipe protected path '$resolved_mount_path'" >&2
    exit 1
    ;;
esac

if command -v mountpoint >/dev/null 2>&1; then
  if ! mountpoint -q "$resolved_mount_path"; then
    echo "error: '$resolved_mount_path' is not a mount point" >&2
    exit 1
  fi
fi

echo "About to wipe all contents under: $resolved_mount_path"

if [[ "$confirm" != true ]]; then
  read -r -p "Type WIPE to continue: " reply
  if [[ "$reply" != "WIPE" ]]; then
    echo "aborted"
    exit 1
  fi
fi

find "$resolved_mount_path" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
sync

echo "SD card contents removed from $resolved_mount_path"
