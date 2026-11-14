#!/usr/bin/env bash
# Notarise and staple the release .dmg built by scripts/release/build-mac.sh.
#
# This uploads the build to Apple's notary service, so it refuses to run
# unless asked explicitly:
#
#   AZJ_NOTARIZE=1 AZJ_NOTARY_PROFILE=<keychain-profile> scripts/release/notarize.sh
#
# The keychain profile comes from a one-off
#   xcrun notarytool store-credentials <keychain-profile> --apple-id … --team-id …
# (docs/release.md). The build must be signed with a Developer ID; ad-hoc
# builds are rejected before anything is uploaded.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
version="$(tr -d '[:space:]' < "$root/VERSION")"
out="$root/target/release-mac"
app="$out/Azulejo.app"
dmg="$out/Azulejo-$version.dmg"

if [[ "${AZJ_NOTARIZE:-}" != 1 ]]; then
  echo "refusing: notarisation uploads the build to Apple. Set AZJ_NOTARIZE=1 to confirm." >&2
  exit 2
fi
if [[ -z "${AZJ_NOTARY_PROFILE:-}" ]]; then
  echo "refusing: set AZJ_NOTARY_PROFILE to a notarytool keychain profile (docs/release.md)." >&2
  exit 2
fi
for f in "$app" "$dmg"; do
  [[ -e "$f" ]] || { echo "missing $f: run just release-mac first" >&2; exit 1; }
done
sig="$(codesign -dv --verbose=2 "$app" 2>&1 || true)"
if [[ "$sig" == *"Signature=adhoc"* ]]; then
  echo "refusing: $app is ad-hoc signed; rebuild with a Developer ID (DEVELOPER_ID=…)." >&2
  exit 1
fi
if [[ "$sig" != *"Authority=Developer ID Application"* ]]; then
  echo "refusing: $app isn't signed with a Developer ID Application certificate." >&2
  exit 1
fi

echo "==> submitting $dmg (profile $AZJ_NOTARY_PROFILE)"
xcrun notarytool submit "$dmg" --keychain-profile "$AZJ_NOTARY_PROFILE" --wait

echo "==> stapling"
xcrun stapler staple "$dmg"
xcrun stapler staple "$app"
# The .zip was made before stapling; rebuild it so it carries the ticket.
ditto -c -k --keepParent "$app" "$out/Azulejo-$version.zip"

echo "==> Gatekeeper check"
spctl --assess --type execute --verbose=2 "$app"
spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
echo "==> notarised: $dmg"
