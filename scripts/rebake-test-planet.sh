#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

output="assets/outmaps/test-planet"
staging="assets/outmaps/test-planet.rebake"
backup="assets/outmaps/test-planet.ocean-backup-$(date +%Y%m%d-%H%M%S)"

if [[ -e "$staging" ]]; then
    echo "Refusing to overwrite existing staging directory: $staging" >&2
    echo "Move or remove that directory, then run this script again." >&2
    exit 1
fi

cargo run --release -p catinthegarden-baker -- --output "$staging"
cargo run --release -p catinthegarden-baker -- --validate "$staging"

if [[ -e "$output" ]]; then
    mv "$output" "$backup"
    echo "Previous outmap preserved at $backup"
fi
mv "$staging" "$output"

echo "Installed rebaked coastal outmap at $output"
echo "Run: cargo run -p catinthegarden-app"
