#!/bin/sh
# themes.azs.in's file edits (colours can't go in an .azs line: `#` starts a
# comment there). CONFIG is the dev profile's config.toml.
#   files.sh dusk CONFIG          install themes/dusk.toml beside it
#   files.sh dusk-edit CONFIG     change dusk's window colour in place
#   files.sh ghostty-set CONFIG   the profile's Ghostty config sets colours
#   files.sh ghostty-clear CONFIG
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
dir="$(dirname "$2")"
case "$1" in
  dusk) mkdir -p "$dir/themes" && cp "$here/dusk.toml" "$dir/themes/dusk.toml" ;;
  dusk-edit) sed -i '' 's/#3a1f4d/#1f3a4d/' "$dir/themes/dusk.toml" ;;
  ghostty-set) mkdir -p "$dir/../ghostty" && printf 'background = #102030\nforeground = #e0e0e0\n' >"$dir/../ghostty/config" ;;
  ghostty-clear) rm -f "$dir/../ghostty/config" ;;
  *) echo "files.sh: unknown $1" >&2; exit 2 ;;
esac
