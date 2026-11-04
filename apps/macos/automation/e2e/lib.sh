# Shared helpers for the end-to-end pass (docs/testing/e2e-plan.md). Source it.
#
# Every suite runs on its own dev instance (AZJ_DEV_NAME=e2e-…, herdr session
# azj-dev-e2e-…). Nothing here touches herdr's default session, the user's
# Azulejo state or config, the clipboard or ~/Downloads, and no window is
# raised: the app runs with --bg or in-process automation only.
#
#   E2E_SUITE   short name of the suite (used for the output folder)
#   E2E_KEEP=1  leave the dev instance and fixtures behind for a look

e2e_here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$e2e_here/../../../.." && pwd)"
: "${AZJ_DEV_NAME:?set AZJ_DEV_NAME=e2e-…}"
case "$AZJ_DEV_NAME" in e2e*) ;; *) echo "e2e: AZJ_DEV_NAME must start with e2e" >&2; exit 2 ;; esac
export AZJ_DEV_NAME
dev="$root/.dev/$AZJ_DEV_NAME"
session="azj-dev-$AZJ_DEV_NAME"
config="$dev/config/azulejo/config.toml"
state="$dev/state/state.json"
sock="$dev/run/azulejo.sock"
applog="$dev/logs/app.log"
suite="${E2E_SUITE:-$AZJ_DEV_NAME}"
out="$root/.dev/e2e.out/$suite"   # outside .dev/<name>, which --reset wipes
dev_sh="$root/apps/macos/dev.sh"
azj_bin="$root/target/debug/azj"
mkdir -p "$out"
checks="$out/checks.txt"
: >"$checks"

clean_env=(env -u HERDR_SOCKET_PATH -u HERDR_PANE_ID -u HERDR_TAB_ID -u HERDR_WORKSPACE_ID -u HERDR_ENV
  -u AZULEJO_TILE_ID -u AZULEJO_WORKSPACE_ID -u AZULEJO_SOCKET)
hd() { "$e2e_here/hd" "$session" "$@"; }
azj() { "${clean_env[@]}" AZULEJO_SOCKET="$sock" "$azj_bin" --no-caller "$@"; }
say() { printf '\n== %s\n' "$*"; }
now_ms() { python3 -c 'import time; print(int(time.time()*1000))'; }

# check NAME COMMAND…: run a check, record PASS/FAIL with the name.
check() {
  local name="$1"; shift
  if "$@" >>"$out/check-output.txt" 2>&1; then
    echo "PASS $name" | tee -a "$checks"
  else
    echo "FAIL $name" | tee -a "$checks"
  fi
}
note() { echo "NOTE $*" | tee -a "$checks"; }

write_config() { # extra TOML lines on stdin
  mkdir -p "$(dirname "$config")"
  { printf '# e2e dev config\n[herdr]\nsession = "%s"\n' "$session"; cat; } >"$config"
}

# A fresh herdr server for this instance, started with a clean environment.
herdr_fresh() {
  hd server stop >/dev/null 2>&1 || true
  for _ in $(seq 50); do hd workspace list >/dev/null 2>&1 || break; sleep 0.1; done
  hd session delete "$session" >/dev/null 2>&1 || true
  nohup "$e2e_here/hd" "$session" server >/dev/null 2>&1 &
  for _ in $(seq 100); do hd workspace list >/dev/null 2>&1 && return 0; sleep 0.1; done
  echo "e2e: herdr server for $session didn't start" >&2; return 1
}

launch_bg() { "$dev_sh" --bg; }
stop_app() { "$dev_sh" --stop; }
reset_instance() { "$dev_sh" --reset >/dev/null 2>&1 || true; }

# run_azs FILE NAME: run an automation script, output in $out/NAME.
run_azs() {
  local file="$1" name="$2"
  rm -rf "$out/$name"; mkdir -p "$out/$name"
  if "$dev_sh" --automation "$file" --out "$out/$name" >"$out/$name.run.txt" 2>&1; then
    echo "PASS azs:$name" | tee -a "$checks"
  else
    echo "FAIL azs:$name ($(cat "$out/$name/result" 2>/dev/null | cut -c1-400))" | tee -a "$checks"
  fi
  cp "$applog" "$out/$name/app.log" 2>/dev/null || true
}

wait_ping() { for _ in $(seq 200); do azj core ping >/dev/null 2>&1 && return 0; sleep 0.05; done; return 1; }

# Fixtures (plan section 1), in a temp folder removed on exit.
make_fixtures() {
  fx="$(mktemp -d "${TMPDIR:-/tmp}/azj-e2e.XXXXXX")"
  local r
  for r in azulejo:main sample-service:development sample-project:audit/critical-path-review; do
    local name="${r%%:*}" branch="${r#*:}"
    mkdir -p "$fx/$name"
    git -C "$fx/$name" init -q -b "$branch"
    printf '# %s\n' "$name" >"$fx/$name/README.md"
    git -C "$fx/$name" add . && git -C "$fx/$name" -c user.name=e2e -c user.email=e2e@example.invalid commit -qm init
  done
  mkdir -p "$fx/not-a-repo"
  printf '# Plan\n\n- [ ] step one\n\n```rust\nfn main() {}\n```\n' >"$fx/plan.md"
  printf 'fn main() {\n    println!("hi");\n}\n' >"$fx/main.rs"
  sips -s format png /System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericDocumentIcon.icns \
    --out "$fx/shot.png" >/dev/null
  printf 'Azulejo e2e PDF\n' | cupsfilter -i text/plain -m application/pdf - >"$fx/doc.pdf" 2>/dev/null
  avconvert --source /System/Library/CoreServices/ControlCenter.app/Contents/Resources/BentoGalleryIntroduction.mov \
    --preset PresetLowQuality --output "$fx/clip.mp4" --replace >/dev/null 2>&1 || true
}

serve_site() { # PORT → serves automation/browser-site on 127.0.0.1:PORT
  (cd "$root/apps/macos/automation/browser-site" && exec python3 -m http.server "$1" --bind 127.0.0.1 >/dev/null 2>&1) &
  site_pid=$!
}

e2e_cleanup() {
  [[ -n "${site_pid:-}" ]] && kill "$site_pid" 2>/dev/null || true
  [[ "${E2E_KEEP:-}" == 1 ]] && return 0
  reset_instance
  [[ -n "${fx:-}" && -d "$fx" ]] && rm -rf "$fx"
  return 0
}
