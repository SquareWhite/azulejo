#!/usr/bin/env bash
# Runs inside a herdr pane: logs every SIGWINCH (with the new size) to $2 and
# redraws a coloured, size-revealing screen in ONE write (like a real TUI).
name=$1; out=$2; n=0
: > "$out"
draw() {
  local rows cols r buf pad line
  read -r rows cols < <(stty size)
  printf -v pad '%*s' $((cols > 8 ? cols - 8 : 0)) ''
  pad=${pad// /=}
  buf=$'\e[H\e[2J'
  for ((r = 1; r < rows; r++)); do
    printf -v line '\e[38;5;%dm%s %02d %s\e[0m\r\n' $(( (r * 7) % 200 + 20 )) "$name" "$r" "$pad"
    buf+=$line
  done
  printf -v line '\e[1;33m%s winch=%d size=%sx%s\e[0m' "$name" "$n" "$cols" "$rows"
  buf+=$line
  printf '%s' "$buf"
}
trap 'n=$((n+1)); read -r r c < <(stty size); echo "$n ${c}x${r} $(date +%s.%N)" >> "$out"; draw' WINCH
draw
while true; do sleep 1 & wait $!; done
