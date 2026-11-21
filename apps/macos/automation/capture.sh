# Window captures for automation runs. Source it (e2e/lib.sh does).
#
# watch_captures PATTERN…: takes the window captures a running automation
# asks for, until killed. Each PATTERN is an automation --out folder, or a
# glob of them ("$out/*"). The app (`capture NAME`) drops a request in
# <folder>/.capture/NAME; this removes it, then writes <folder>/NAME.png, a
# `screencapture -l` of that run's own window (<folder>/window-id) and of
# nothing else, or <folder>/.capture/.failed-NAME when screencapture got
# nothing (a locked screen). `captured` in a script waits for one or the
# other, so scripts need no fixed wait for the PNG. Run it in the
# background and kill it when the app is done:
#
#   watch_captures "$out/run" & watcher=$!
#   …
#   kill "$watcher" 2>/dev/null || true
watch_captures() {
  local pattern dir req name wid tmp IFS=$'\n'  # globs expand; paths keep their spaces
  shopt -s nullglob
  while true; do
    for pattern in "$@"; do
      for dir in $pattern; do
        [[ -d "$dir/.capture" ]] || continue
        for req in "$dir/.capture"/*; do
          [[ -f "$req" ]] || continue
          name="$(basename "$req")"; rm -f "$req"
          wid="$(cat "$dir/window-id" 2>/dev/null || true)"
          tmp="$dir/.capture/.taking-$name.png"
          if [[ -n "$wid" ]] && /usr/sbin/screencapture -l "$wid" -x -o "$tmp" 2>/dev/null && [[ -s "$tmp" ]]; then
            mv -f "$tmp" "$dir/$name.png"
          else
            rm -f "$tmp"; : >"$dir/.capture/.failed-$name"
          fi
        done
      done
    done
    sleep 0.01
  done
}
