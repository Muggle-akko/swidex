#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
binary="${1:-$project_dir/src-tauri/target/release/swidex}"
[[ -x "$binary" ]] || { echo "Build first: npm run tauri -- build --no-bundle" >&2; exit 1; }

install -Dm755 "$binary" "$HOME/.local/lib/swidex/swidex"
install -Dm755 "$project_dir/scripts/swidex-linux" "$HOME/.local/bin/swidex"
install -Dm644 "$project_dir/src-tauri/icons/128x128.png" "$HOME/.local/share/icons/hicolor/128x128/apps/swidex.png"
install -Dm644 "$project_dir/scripts/dev.awu.swidex.desktop" "$HOME/.local/share/applications/dev.awu.swidex.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$HOME/.local/share/applications"
echo "Installed Swidex. Open it from the application menu or run swidex."
