#!/usr/bin/env bash
# Signing and checking Hive.app on macOS (TODO 17.11, D17.1).
#
#   macos-bundle.sh signing-mode
#       Prints "developer-id" when all six APPLE_* secrets are in the environment, "adhoc" when
#       none is; fails on a partial set.
#   macos-bundle.sh sign-hive <hive binary>
#       Signs the service binary before Tauri bundles it, with the identity Tauri then signs the app
#       with: $APPLE_SIGNING_IDENTITY (a Developer ID, with the hardened runtime and a secure
#       timestamp, as notarization requires), else ad-hoc ("-").
#   macos-bundle.sh check <Hive.app> [--developer-id]
#       The bundle validates (`codesign --verify --deep --strict`); the nested `hive` is signed
#       (not only linker-signed) with the app's identity; the app's entitlements allow the
#       WebView's JIT; the Info.plist names Hive in the folder prompts. With --developer-id also
#       Gatekeeper's verdict (`spctl -a`) and the stapled notarization ticket.
set -euo pipefail

die() { echo "macos-bundle: $*" >&2; exit 1; }

# The keys of the prompts terminals and watchers trigger (src-tauri/Info.plist).
USAGE_KEYS="NSDocumentsFolderUsageDescription NSDesktopFolderUsageDescription
NSDownloadsFolderUsageDescription NSRemovableVolumesUsageDescription
NSAppleEventsUsageDescription NSLocalNetworkUsageDescription"

sign_hive() {
  local bin=$1 identity=${APPLE_SIGNING_IDENTITY:--}
  [ -f "$bin" ] || die "no such binary: $bin"
  local args=(--force --sign "$identity" --identifier com.lucas.hive.service)
  if [ "$identity" != - ]; then args+=(--options runtime --timestamp); fi
  codesign "${args[@]}" "$bin"
}

# signer <path>: who signed it, one line: the certificate chain, else "adhoc", and the team.
signer() {
  local info
  info=$(codesign -dvv "$1" 2>&1) || die "not signed: $1"
  local chain team
  chain=$(sed -n 's/^Authority=//p' <<<"$info" | paste -sd '>' -)
  team=$(sed -n 's/^TeamIdentifier=//p' <<<"$info")
  echo "${chain:-adhoc} team=${team:-none}"
}

check() {
  local app=$1 developer_id=${2:-}
  [ -d "$app" ] || die "no such bundle: $app"
  [ -z "$developer_id" ] || [ "$developer_id" = --developer-id ] || die "unknown option: $developer_id"
  local hive=$app/Contents/Resources/hive plist=$app/Contents/Info.plist
  [ -f "$hive" ] || die "no hive in $app"

  codesign --verify --deep --strict "$app" || die "the bundle does not validate"
  codesign --verify --strict "$hive" || die "the nested hive does not validate"

  local hive_flags
  hive_flags=$(codesign -dv "$hive" 2>&1 | sed -n 's/.*flags=[^(]*(\([^)]*\)).*/\1/p')
  case ",$hive_flags," in *,linker-signed,*) die "the nested hive is only linker-signed" ;; esac
  [ "$(signer "$hive")" = "$(signer "$app")" ] ||
    die "the nested hive ($(signer "$hive")) and the app ($(signer "$app")) have different signers"

  local entitlements
  entitlements=$(codesign -d --entitlements - --xml "$app" 2>/dev/null) || die "no entitlements"
  grep -q '<key>com.apple.security.cs.allow-jit</key><true/>' <<<"${entitlements//[$'\t\n ']/}" ||
    die "the entitlements do not allow the WebView's JIT"
  grep -q '<key>com.apple.security.automation.apple-events</key><true/>' <<<"${entitlements//[$'\t\n ']/}" ||
    die "the entitlements do not allow the terminals' Apple Events"

  local key text
  for key in $USAGE_KEYS; do
    text=$(plutil -extract "$key" raw -o - "$plist" 2>/dev/null) || die "Info.plist has no $key"
    case $text in *Hive*) ;; *) die "$key does not name Hive: $text" ;; esac
  done

  if [ -n "$developer_id" ]; then
    case ",$hive_flags," in *,runtime,*) ;; *) die "the nested hive lacks the hardened runtime" ;; esac
    spctl -a -vv --type execute "$app" || die "Gatekeeper rejects the bundle"
    xcrun stapler validate "$app" || die "no stapled notarization ticket"
  fi
  echo "ok    $app"
}

# Which signing the release does: "developer-id" with all six secrets (release.yml passes them under
# their own names), "adhoc" with none; a partial set fails rather than ship ad-hoc by mistake.
signing_mode() {
  local v set=0
  for v in $SIGNING_SECRETS; do [ -z "${!v:-}" ] || set=$((set + 1)); done
  case $set in
    0) echo adhoc ;;
    6) echo developer-id ;;
    *) die "only $set of the 6 Developer ID secrets are set ($SIGNING_SECRETS)" ;;
  esac
}
SIGNING_SECRETS="APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_API_ISSUER
APPLE_API_KEY APPLE_API_KEY_P8"

case ${1:-} in
  signing-mode) [ $# = 1 ] || die "usage: $0 signing-mode"; signing_mode ;;
  sign-hive) [ $# = 2 ] || die "usage: $0 sign-hive <hive binary>"; sign_hive "$2" ;;
  check) [ $# -ge 2 ] && [ $# -le 3 ] || die "usage: $0 check <Hive.app> [--developer-id]"; check "$2" "${3:-}" ;;
  *) die "usage: $0 signing-mode | sign-hive <hive binary> | check <Hive.app> [--developer-id]" ;;
esac
