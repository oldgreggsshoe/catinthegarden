#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-/home/dad/catingard-target}"
binary="${target_dir}/release/catinthegarden-app"

cd "${repo_root}"
if pgrep -f "^${binary}( |$)" >/dev/null; then
    echo "catinthegarden-app is already running from ${target_dir}; stop it before measuring parity" >&2
    exit 1
fi

CARGO_TARGET_DIR="${target_dir}" cargo build --release -p catinthegarden-app

for render_path in raster ray; do
    for debug_mode in final albedo lighting aerial; do
        echo "=== ${render_path} / ${debug_mode} ==="
        CATINGARDEN_PRESENT_MODE=immediate \
        CATINGARDEN_RENDER_PATH="${render_path}" \
        CATINGARDEN_DEBUG_MODE="${debug_mode}" \
            "${binary}" --scenario render_path_parity
    done
done

echo "=== ray / ray_hit ==="
CATINGARDEN_PRESENT_MODE=immediate \
CATINGARDEN_RENDER_PATH=ray \
CATINGARDEN_DEBUG_MODE=ray_hit \
    "${binary}" --scenario render_path_parity
