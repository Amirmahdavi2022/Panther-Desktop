#!/usr/bin/env bash
# Puts the flat repository into the layout Tauri expects.
#
# Why the files sit flat at the root: this project is maintained from a phone,
# and GitHub's web uploader drops every file into the repository root — it
# cannot create folders. So the repo keeps a flat, uploadable shape and this
# script builds the real tree at build time. CI runs it first; run it yourself
# before `cargo tauri build` if you are building locally.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$root"

mkdir -p src src-tauri/src src-tauri/tests src-tauri/capabilities src-tauri/icons

place() {
  local from="$1" to="$2"
  if [ -f "$from" ]; then
    cp -f "$from" "$to"
    echo "  $from -> $to"
  elif [ -f "$to" ]; then
    echo "  $to already in place"
  else
    echo "MISSING: neither $from nor $to exists" >&2
    return 1
  fi
}

echo "frontend"
place app.index.html          src/index.html
place app.styles.css          src/styles.css
place app.main.js             src/main.js

echo "tauri"
place tauri.conf.json         src-tauri/tauri.conf.json
place Cargo.toml              src-tauri/Cargo.toml
place build.rs                src-tauri/build.rs
place capabilities.json       src-tauri/capabilities/default.json

echo "rust"
place engines.rs              src-tauri/src/engines.rs
place main.rs                 src-tauri/src/main.rs
place live.rs                 src-tauri/tests/live.rs

echo "icons"
# Windows runners expose "python"; most Linux ones expose "python3". Pick
# whichever is actually there rather than assuming.
py=""
for candidate in python3 python py; do
  if command -v "$candidate" >/dev/null 2>&1; then py="$candidate"; break; fi
done
if [ -z "$py" ]; then
  echo "No Python interpreter found; cannot build the icons." >&2
  exit 1
fi
echo "  using $py"
"$py" make-icons.py

echo "layout ready"
