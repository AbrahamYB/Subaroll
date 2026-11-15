#!/bin/zsh
set -euo pipefail

project_dir="$(cd "$(dirname "$0")/.." && pwd)"
built_app="$project_dir/src-tauri/target/release/bundle/macos/Subaroll.app"
installed_app="/Applications/Subaroll.app"
legacy_app="/Applications/Subtitle Burner.app"
installed_executable="$installed_app/Contents/MacOS/Subaroll"
legacy_executable="$legacy_app/Contents/MacOS/Subtitle Burner"

cd "$project_dir"
npm test
npm run build:mac

# Stop either installed name so the new bundle can replace it cleanly.
/usr/bin/pkill -f "^${installed_executable}$" 2>/dev/null || true
/usr/bin/pkill -f "^${legacy_executable}$" 2>/dev/null || true
/bin/mkdir -p "$installed_app"
/bin/rm -rf "$installed_app/Contents"
/usr/bin/ditto "$built_app/Contents" "$installed_app/Contents"
/usr/bin/codesign --force --deep --sign - "$installed_app"
/usr/bin/codesign --verify --deep --strict "$installed_app"
# Remove the old branded copy after the new bundle is installed and verified.
if [[ -d "$legacy_app/Contents" ]]; then
  /bin/rm -rf "$legacy_app/Contents"
fi
if [[ -d "$legacy_app" ]]; then
  /bin/rmdir "$legacy_app" 2>/dev/null || true
fi
/usr/bin/touch "$project_dir/src-tauri/target/.metadata_never_index"
if /usr/bin/open "$installed_app"; then
  echo "Updated and opened: $installed_app"
else
  echo "Updated: $installed_app (Launch Services could not start it from this session)"
fi
