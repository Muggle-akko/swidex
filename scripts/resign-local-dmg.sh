#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script only works on macOS." >&2
  exit 1
fi

input_dmg="${1:-src-tauri/target/release/bundle/dmg/Swidex_0.1.2_aarch64.dmg}"
output_dmg="${2:-${input_dmg%.dmg}-local-ad-hoc.dmg}"

if [[ ! -f "$input_dmg" ]]; then
  echo "Input DMG not found: $input_dmg" >&2
  exit 1
fi

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/swidex-resign.XXXXXX")"
mount_dir="$work_dir/mount"
stage_dir="$work_dir/stage"
mkdir -p "$mount_dir" "$stage_dir"

cleanup() {
  hdiutil detach "$mount_dir" >/dev/null 2>&1 || true
  rm -rf "$work_dir"
}
trap cleanup EXIT

echo "Mounting $input_dmg"
hdiutil attach -nobrowse -readonly -mountpoint "$mount_dir" "$input_dmg" >/dev/null

app_path="$(find "$mount_dir" -maxdepth 1 -name '*.app' -type d | head -n 1)"
if [[ -z "$app_path" ]]; then
  echo "No .app bundle found in $input_dmg" >&2
  exit 1
fi

app_name="$(basename "$app_path")"
cp -R "$app_path" "$stage_dir/$app_name"
ln -s /Applications "$stage_dir/Applications"

echo "Cleaning extended attributes"
xattr -cr "$stage_dir/$app_name"

echo "Ad-hoc signing $app_name"
codesign --force --deep --sign - "$stage_dir/$app_name"
codesign --verify --deep --strict --verbose=2 "$stage_dir/$app_name"

rm -f "$output_dmg"
echo "Creating $output_dmg"
hdiutil create \
  -volname "${app_name%.app}" \
  -srcfolder "$stage_dir" \
  -ov \
  -format UDZO \
  "$output_dmg" >/dev/null

echo "Done: $output_dmg"
echo
echo "Friend-side Gatekeeper bypass if macOS blocks it:"
echo "  xattr -dr com.apple.quarantine /Applications/$app_name"
