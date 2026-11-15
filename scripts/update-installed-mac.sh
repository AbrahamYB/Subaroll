#!/bin/zsh
set -euo pipefail

project_dir="$(cd "$(dirname "$0")/.." && pwd)"
built_app="$project_dir/src-tauri/target/release/bundle/macos/Subtitle Burner.app"
installed_app="/Applications/Subtitle Burner.app"
installed_executable="$installed_app/Contents/MacOS/Subtitle Burner"

cd "$project_dir"
npm test
npm run build:mac

# Stop only the installed copy so its files can be safely refreshed in place.
/usr/bin/pkill -f "^${installed_executable}$" 2>/dev/null || true
/bin/rm -rf "$installed_app"
/usr/bin/ditto "$built_app" "$installed_app"
/usr/bin/codesign --force --deep --sign - "$installed_app"
/usr/bin/codesign --verify --deep --strict "$installed_app"
/usr/bin/touch "$project_dir/src-tauri/target/.metadata_never_index"
/usr/bin/open "$installed_app"

echo "Updated and opened: $installed_app"

