#!/usr/bin/env bash
# Signing and checking Hive.app on macOS (TODO 17.11, 25.5, D17.1).
#
#   macos-bundle.sh signing-mode
#       Prints "developer-id" when all six APPLE_* secrets are in the environment, else
#       "self-signed" when all three HIVE_CERTIFICATE* secrets are, else "adhoc"; fails on a partial
#       set of either.
#   macos-bundle.sh new-certificate <new folder>
#       Makes the self-signed code-signing certificate (once, by hand: README "Signing with Hive's
#       certificate"): <folder>/certificate.p12 and <folder>/password, and prints its SHA-1. Touches
#       no keychain.
#   macos-bundle.sh import-certificate <new keychain>
#       CI only (it changes the keychain search list): puts $SIGNING_CERTIFICATE (a .p12 in base64)
#       and $SIGNING_CERTIFICATE_PASSWORD in a new keychain for `codesign`.
#   macos-bundle.sh sign-hive <hive binary>
#       Signs the service binary before Tauri bundles it, with the identity Tauri then signs the app
#       with: $APPLE_SIGNING_IDENTITY (a Developer ID, with the hardened runtime and a secure
#       timestamp, as notarization requires), else $HIVE_CERTIFICATE_SHA1 (the self-signed
#       certificate, with the hardened runtime; no timestamp, which nothing here would check), else
#       ad-hoc ("-").
#   macos-bundle.sh check <Hive.app> [--developer-id | --self-signed <certificate SHA-1>]
#       The bundle validates (`codesign --verify --deep --strict`); the nested `hive` is signed
#       (not only linker-signed) with the app's identity; the app's entitlements allow the
#       WebView's JIT; the Info.plist names Hive in the folder prompts; the Liquid Glass icon is there
#       (Assets.car and CFBundleIconName, else macOS 26 boxes the icon in grey). With --developer-id also
#       Gatekeeper's verdict (`spctl -a`) and the stapled notarization ticket. With --self-signed
#       the designated requirement of the app and of the nested `hive` names their identifier and that
#       certificate, not a cdhash (what lets macOS's grants survive an update), under the hardened
#       runtime.
#   macos-bundle.sh release-notes <signing mode>
#       The notes of a release, given the earlier releases' notes on stdin: the one note of the
#       first self-signed release (Hive's identity changes that once), else nothing.
#   macos-bundle.sh dsyms <Hive.app> <folder>
#       The app's binary and its nested `hive` each have a dSYM in <folder> (`hive-app.dSYM`,
#       `hive.dSYM`, as cargo makes them with `split-debuginfo = "packed"`) whose UUIDs are the
#       binary's (`dwarfdump --uuid`), so a crash report of that release can be symbolicated.
set -euo pipefail

die() { echo "macos-bundle: $*" >&2; exit 1; }

# The keys of the prompts terminals and watchers trigger (src-tauri/Info.plist).
USAGE_KEYS="NSDocumentsFolderUsageDescription NSDesktopFolderUsageDescription
NSDownloadsFolderUsageDescription NSRemovableVolumesUsageDescription
NSAppleEventsUsageDescription NSLocalNetworkUsageDescription"

sign_hive() {
  local bin=$1 identity=${APPLE_SIGNING_IDENTITY:-${HIVE_CERTIFICATE_SHA1:--}}
  [ -f "$bin" ] || die "no such binary: $bin"
  local args=(--force --sign "$identity" --identifier com.lucas.hive.service)
  if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
    args+=(--options runtime --timestamp)
  elif [ "$identity" != - ]; then
    args+=(--options runtime --timestamp=none)
  fi
  codesign "${args[@]}" "$bin"
}

# Twenty years: a new certificate is a new identity, and every macOS grant is asked again.
new_certificate() {
  umask 077
  local name="Hive Self-Signed" sha1
  certificate_folder=$1
  mkdir -m 700 "$certificate_folder" || die "cannot make a new folder: $certificate_folder"
  local folder=$certificate_folder
  # Set only now: an existing folder is not ours to clean. A global, as the trap runs at exit.
  trap 'rm -f "$certificate_folder/key.pem" "$certificate_folder/certificate.pem"' EXIT
  # The system's LibreSSL: a Homebrew OpenSSL 3 writes a .p12 `security import` refuses.
  /usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 7300 -subj "/CN=$name" \
    -keyout "$folder/key.pem" -out "$folder/certificate.pem" \
    -addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning" \
    2>/dev/null || die "openssl could not make the certificate"
  uuidgen >"$folder/password"
  /usr/bin/openssl pkcs12 -export -inkey "$folder/key.pem" -in "$folder/certificate.pem" \
    -out "$folder/certificate.p12" -name "$name" -passout "file:$folder/password" ||
    die "openssl could not make the .p12"
  sha1=$(/usr/bin/openssl x509 -in "$folder/certificate.pem" -noout -fingerprint -sha1)
  sha1=${sha1#*=}
  echo "${sha1//:/}"
}

import_certificate() {
  [ -n "${GITHUB_ACTIONS:-}" ] || die "import-certificate changes the keychain search list: CI only"
  local keychain=$1 password p12
  umask 077
  password=$(uuidgen)
  p12=$keychain.p12
  printf '%s' "${SIGNING_CERTIFICATE:?}" | base64 --decode >"$p12"
  security create-keychain -p "$password" "$keychain"
  security set-keychain-settings -lut 21600 "$keychain"
  security unlock-keychain -p "$password" "$keychain"
  security import "$p12" -k "$keychain" -P "${SIGNING_CERTIFICATE_PASSWORD:?}" -T /usr/bin/codesign
  rm -f "$p12"
  security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" >/dev/null
  # shellcheck disable=SC2046 # one path per word, as `security` prints them
  security list-keychains -d user -s "$keychain" $(security list-keychains -d user | tr -d '"')
}

# names_certificate <path> <identifier> <lowercase SHA-1>: its designated requirement is theirs.
names_certificate() {
  local path=$1 id=$2 sha1=$3 requirement
  requirement=$(codesign -d -r- "$path" 2>&1) || die "no requirement: $path"
  grep -q "identifier \"$id\"" <<<"$requirement" ||
    die "$path: the designated requirement does not name $id: $requirement"
  grep -Eq "certificate (root|leaf) = H\"$sha1\"" <<<"$requirement" ||
    die "$path: the designated requirement does not name the certificate $sha1: $requirement"
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
  local app=$1 option=${2:-} certificate=${3:-}
  [ -d "$app" ] || die "no such bundle: $app"
  case $option in
    "" | --developer-id) [ -z "$certificate" ] || die "unknown option: $certificate" ;;
    --self-signed)
      certificate=$(tr 'A-F' 'a-f' <<<"$certificate")
      [[ $certificate =~ ^[0-9a-f]{40}$ ]] || die "--self-signed needs the certificate's SHA-1"
      ;;
    *) die "unknown option: $option" ;;
  esac
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

  local icon
  icon=$(plutil -extract CFBundleIconName raw -o - "$plist" 2>/dev/null) || die "Info.plist has no CFBundleIconName"
  assetutil --info "$app/Contents/Resources/Assets.car" 2>/dev/null | grep -q "\"Name\" : \"$icon\"" ||
    die "Assets.car has no $icon icon"

  if [ -n "$option" ]; then
    case ",$hive_flags," in *,runtime,*) ;; *) die "the nested hive lacks the hardened runtime" ;; esac
  fi
  if [ "$option" = --self-signed ]; then
    local app_flags id
    app_flags=$(codesign -dv "$app" 2>&1 | sed -n 's/.*flags=[^(]*(\([^)]*\)).*/\1/p')
    case ",$app_flags," in *,runtime,*) ;; *) die "the app lacks the hardened runtime" ;; esac
    id=$(plutil -extract CFBundleIdentifier raw -o - "$plist") || die "Info.plist has no CFBundleIdentifier"
    names_certificate "$app" "$id" "$certificate"
    names_certificate "$hive" com.lucas.hive.service "$certificate"
  fi
  if [ "$option" = --developer-id ]; then
    spctl -a -vv --type execute "$app" || die "Gatekeeper rejects the bundle"
    xcrun stapler validate "$app" || die "no stapled notarization ticket"
  fi
  echo "ok    $app"
}

# uuids <path>: its Mach-O UUIDs, one per line.
uuids() { dwarfdump --uuid "$1" | awk '/^UUID: / {print $2}' | sort; }

# dsym <binary> <dSYM>
dsym() {
  local bin=$1 dsym=$2 want got
  [ -f "$bin" ] || die "no such binary: $bin"
  [ -d "$dsym" ] || die "no dSYM: $dsym"
  want=$(uuids "$bin")
  got=$(uuids "$dsym")
  [ -n "$want" ] && [ "$want" = "$got" ] || die "$dsym (${got:-no UUID}) does not match $bin (${want:-no UUID})"
  echo "ok    $dsym $want"
}

dsyms() {
  local app=$1 folder=$2 exe
  [ -d "$app" ] || die "no such bundle: $app"
  exe=$(plutil -extract CFBundleExecutable raw -o - "$app/Contents/Info.plist") ||
    die "no CFBundleExecutable in $app"
  dsym "$app/Contents/MacOS/$exe" "$folder/hive-app.dSYM"
  dsym "$app/Contents/Resources/hive" "$folder/hive.dSYM"
}

# How many of these variables are set, not empty (an absent GitHub secret is an empty string).
count_set() {
  local v set=0
  for v in "$@"; do [ -z "${!v:-}" ] || set=$((set + 1)); done
  echo "$set"
}

# Which signing the release does: "developer-id" with all six Apple secrets (release.yml passes them
# under their own names), else "self-signed" with all three of Hive's certificate, "adhoc" with
# none; a partial set fails rather than ship the weaker signature by mistake.
signing_mode() {
  local apple hive
  # shellcheck disable=SC2086 # the names, one per word
  apple=$(count_set $DEVELOPER_ID_SECRETS)
  # shellcheck disable=SC2086 # the names, one per word
  hive=$(count_set $SELF_SIGNED_SECRETS)
  case $apple in
    0 | 6) ;;
    *) die "only $apple of the 6 Developer ID secrets are set ($DEVELOPER_ID_SECRETS)" ;;
  esac
  case $hive in
    0 | 3) ;;
    *) die "only $hive of the 3 certificate secrets are set ($SELF_SIGNED_SECRETS)" ;;
  esac
  if [ "$apple" = 6 ]; then
    echo developer-id
  elif [ "$hive" = 3 ]; then
    echo self-signed
  else
    echo adhoc
  fi
}
DEVELOPER_ID_SECRETS="APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_API_ISSUER
APPLE_API_KEY APPLE_API_KEY_P8"
SELF_SIGNED_SECRETS="HIVE_CERTIFICATE HIVE_CERTIFICATE_PASSWORD HIVE_CERTIFICATE_SHA1"

# Its first line also says the note was given: a later release, seeing it, stays silent.
FIRST_SIGNED_NOTE="Hive is now signed with its own certificate, so what you allow in macOS lasts across updates.

This one update changes Hive's identity, so allow it one last time: in System Settings, Privacy & Security, remove Hive from Screen & System Audio Recording and from Accessibility, add it again and restart Hive. macOS also asks again for your folders. See [Computer Use and screen recording](https://github.com/lucasreali/hive#computer-use-and-screen-recording)."

release_notes() {
  local earlier
  earlier=$(cat)
  [ "$1" = self-signed ] || return 0
  grep -qF "${FIRST_SIGNED_NOTE%%$'\n'*}" <<<"$earlier" || echo "$FIRST_SIGNED_NOTE"
}

case ${1:-} in
  signing-mode) [ $# = 1 ] || die "usage: $0 signing-mode"; signing_mode ;;
  new-certificate) [ $# = 2 ] || die "usage: $0 new-certificate <new folder>"; new_certificate "$2" ;;
  import-certificate) [ $# = 2 ] || die "usage: $0 import-certificate <new keychain>"; import_certificate "$2" ;;
  sign-hive) [ $# = 2 ] || die "usage: $0 sign-hive <hive binary>"; sign_hive "$2" ;;
  release-notes) [ $# = 2 ] || die "usage: $0 release-notes <signing mode>"; release_notes "$2" ;;
  dsyms) [ $# = 3 ] || die "usage: $0 dsyms <Hive.app> <folder>"; dsyms "$2" "$3" ;;
  check)
    [ $# -ge 2 ] && [ $# -le 4 ] || die "usage: $0 check <Hive.app> [--developer-id | --self-signed <SHA-1>]"
    check "$2" "${3:-}" "${4:-}"
    ;;
  *) die "usage: $0 signing-mode | new-certificate <new folder> | import-certificate <new keychain> | sign-hive <hive binary> | release-notes <signing mode> | check <Hive.app> [--developer-id | --self-signed <SHA-1>] | dsyms <Hive.app> <folder>" ;;
esac
