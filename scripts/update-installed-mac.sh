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
npm run build:mac -- --config '{"bundle":{"createUpdaterArtifacts":false}}'

# Never replace the executable while an old process is still using the bundle.
for executable in "$installed_executable" "$legacy_executable"; do
  /usr/bin/pkill -TERM -f "^${executable}$" 2>/dev/null || true
  for attempt in {1..20}; do
    if /usr/bin/pgrep -f "^${executable}$" >/dev/null; then
      /bin/sleep 0.25
    else
      result=$?
      if (( result != 1 )); then
        echo "Could not check whether Subaroll is running; leaving the installed app untouched." >&2
        exit 1
      fi
      break
    fi
  done
  if /usr/bin/pgrep -f "^${executable}$" >/dev/null; then
    echo "Subaroll is still running; leaving the installed app untouched." >&2
    exit 1
  elif (( $? != 1 )); then
    echo "Could not check whether Subaroll is running; leaving the installed app untouched." >&2
    exit 1
  fi
done
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
if /usr/bin/open -a "$installed_app"; then
  echo "Updated and opened: $installed_app"
else
  echo "Updated: $installed_app (Launch Services could not start it from this session)"
fi
