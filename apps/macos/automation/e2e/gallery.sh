#!/usr/bin/env bash
# C3 / C4 / C5: state gallery and edge layouts (gallery.azs.in), dev instance
# e2e-gal. E2E_LIGHT=1 runs it with the app (only the app) in light
# appearance: `-AppleInterfaceStyle Light` in its argument domain, never the
# system setting.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-gal}"
export E2E_SUITE="C-gallery${E2E_LIGHT:+-light}"
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
make_fixtures
serve_site 47843
write_config </dev/null
sed -e "s|@FX@|$fx|g" -e "s|@SITE@|http://127.0.0.1:47843/a.html|g" \
  -e "s|@FOCUS@|$root/apps/macos/automation/focus|g" "$e2e_here/gallery.azs.in" >"$out/gallery.azs"
if [[ -z "${E2E_LIGHT:-}" ]]; then
  run_azs "$out/gallery.azs" gallery
else
  # dev.sh's launch, plus an argument for the app alone.
  o="$out/gallery"; rm -rf "$o"; mkdir -p "$o" "$dev/run" "$dev/logs" "$(dirname "$state")"; chmod 700 "$dev/run"
  pidfile="$dev/run/app.pid"; rm -f "$pidfile"
  options="$(printf '{"config_path":"%s","state_path":"%s","socket_path":"%s","herdr_session":"%s","api_required":true}' \
    "$config" "$state" "$sock" "$session")"
  : >"$applog"
  "${clean_env[@]}" open -n -g --env AZULEJO_PROFILE=dev --env "AZULEJO_CORE_OPTIONS=$options" \
    --env "AZULEJO_PIDFILE=$pidfile" --env AZULEJO_BACKGROUND=1 --env "AZULEJO_AUTOMATION=$out/gallery.azs" \
    --env "AZULEJO_AUTOMATION_OUT=$o" --stdout "$applog" --stderr "$applog" \
    "$root/target/macos/Azulejo.app" --args -AppleInterfaceStyle Light
  for _ in $(seq 100); do [[ -s "$pidfile" ]] && break; sleep 0.1; done
  pid="$(cat "$pidfile")"
  while kill -0 "$pid" 2>/dev/null; do sleep 0.3; done
  r="$(cat "$o/result" 2>/dev/null || echo none)"
  echo "$( [[ $r == ok ]] && echo PASS || echo FAIL ) azs:gallery-light ($r)" | tee -a "$checks"
  cp "$applog" "$o/app.log"
fi
! grep -q FAIL "$checks"
