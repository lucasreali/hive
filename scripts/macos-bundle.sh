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
#       The app's binary and the nested `hive` hold both architectures (`lipo -archs`, TODO 26.5). The
#       bundle validates (`codesign --verify --deep --strict`); in each architecture the nested `hive`
#       is signed (not only linker-signed) with the app's identity and the app's entitlements allow
#       the WebView's JIT; the Info.plist names Hive in the folder prompts; the Liquid Glass icon is there
#       (Assets.car and CFBundleIconName, else macOS 26 boxes the icon in grey). With --developer-id also
#       Gatekeeper's verdict (`spctl -a`) and the stapled notarization ticket. With --self-signed, in
#       each architecture, the designated requirement of the app and of the nested `hive` names their
#       identifier and that certificate, not a cdhash (what lets macOS's grants survive an update),
#       under the hardened runtime. The remote Linux services are there too (TODO 26.13):
#       `hive-linux-x86_64` and `hive-linux-aarch64`, each an executable ELF for its CPU with no
#       interpreter (static, so it needs no libc on the host) that carries the app's version
#       (CFBundleShortVersionString). They are data to codesign, sealed by the app's signature like
#       any resource; a Mac cannot run them, so linux-remote.yml's smoke test is what runs them and
#       checks their `--version`.
#   macos-bundle.sh smoke <Hive.app>
#       CI only (it opens the app's window; a running Hive would take the launch over): the nested
#       `hive` answers `--version` the same under both architectures, and the app's Intel slice starts
#       and stays up under Rosetta, with a temporary HOME.
#   macos-bundle.sh release-notes <signing mode>
#       Given the earlier releases' notes on stdin: the one note of the first self-signed release
#       (Hive's identity changes that once), else nothing. scripts/release-notes.sh puts it first.
#   macos-bundle.sh dsyms <Hive.app> <folder>
#       The app's binary and its nested `hive` each have a dSYM in <folder> (`hive-app.dSYM`,
#       `hive.dSYM`, as cargo makes them with `split-debuginfo = "packed"`) whose UUIDs are the
#       binary's (`dwarfdump --uuid`), so a crash report of that release can be symbolicated.
#   macos-bundle.sh join-dsyms <arm64 folder> <x86_64 folder> <new folder>
#       Joins each architecture's `hive.dSYM` and `hive-app.dSYM` into one that `dsyms` accepts for
#       the universal binaries, as `lipo` joined those.
#   macos-bundle.sh check-updater <latest.json>
#       Every macOS key the updater may look up (`darwin-<arch>` and `darwin-<arch>-app`, TODO 26.5)
#       has the one universal tarball and its signature. An installed Hive reads only these keys,
#       never an asset's name, so this is what keeps it updating.
set -euo pipefail

die() { echo "macos-bundle: $*" >&2; exit 1; }

# The keys of the prompts terminals and watchers trigger (src-tauri/Info.plist).
USAGE_KEYS="NSDocumentsFolderUsageDescription NSDesktopFolderUsageDescription
NSDownloadsFolderUsageDescription NSRemovableVolumesUsageDescription
NSAppleEventsUsageDescription NSLocalNetworkUsageDescription"

# Every architecture a release holds, as `lipo` and `codesign --arch` name them.
ARCHS="arm64 x86_64"

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

# names_certificate <path> <arch> <identifier> <lowercase SHA-1>: that slice's designated
# requirement is theirs.
names_certificate() {
  local path=$1 arch=$2 id=$3 sha1=$4 requirement
  requirement=$(codesign --arch "$arch" -d -r- "$path" 2>&1) || die "no requirement: $path ($arch)"
  grep -q "identifier \"$id\"" <<<"$requirement" ||
    die "$path: the designated requirement does not name $id: $requirement"
  grep -Eq "certificate (root|leaf) = H\"$sha1\"" <<<"$requirement" ||
    die "$path: the designated requirement does not name the certificate $sha1: $requirement"
}

# flags <path> <arch>: that slice's signature flags, comma separated.
flags() { codesign --arch "$2" -dv "$1" 2>&1 | sed -n 's/.*flags=[^(]*(\([^)]*\)).*/\1/p'; }

# both_archs <path>: it holds every architecture of a release.
both_archs() {
  local archs arch
  archs=$(lipo -archs "$1" 2>/dev/null) || die "not a Mach-O binary: $1"
  for arch in $ARCHS; do
    case " $archs " in *" $arch "*) ;; *) die "$1 has no $arch slice (it has: $archs)" ;; esac
  done
}

# signer <path> <arch>: who signed that slice, one line: the certificate chain, else "adhoc", and
# the team.
signer() {
  local info
  info=$(codesign --arch "$2" -dvv "$1" 2>&1) || die "not signed: $1 ($2)"
  local chain team
  chain=$(sed -n 's/^Authority=//p' <<<"$info" | paste -sd '>' -)
  team=$(sed -n 's/^TeamIdentifier=//p' <<<"$info")
  echo "${chain:-adhoc} team=${team:-none}"
}

# The remote Linux services the app carries (TODO 26.13): each name and its ELF e_machine.
LINUX_BUILDS="hive-linux-x86_64:62 hive-linux-aarch64:183"

# le <file> <offset> <bytes>: the unsigned number stored there (little-endian, as `od` reads it on
# the Macs and Linux machines this runs on).
le() { od -An -tu"$3" -j "$2" -N "$3" "$1" | tr -d ' \n'; }

# linux_build <path> <e_machine> <version>: an executable 64-bit little-endian ELF for that CPU,
# with no PT_INTERP (static: no dynamic loader, so no libc of the host's), carrying <version>.
linux_build() {
  local bin=$1 machine=$2 version=$3 phoff size count i type load=
  [ -f "$bin" ] || die "no ${bin##*/} in the app"
  [ -x "$bin" ] || die "${bin##*/} is not executable"
  [ "$(od -An -tx1 -N 6 "$bin" | tr -d ' \n')" = 7f454c460201 ] || die "${bin##*/} is not a 64-bit little-endian ELF"
  [ "$(le "$bin" 18 2)" = "$machine" ] || die "${bin##*/} is not built for its CPU (e_machine $(le "$bin" 18 2), not $machine)"
  phoff=$(le "$bin" 32 8) size=$(le "$bin" 54 2) count=$(le "$bin" 56 2)
  [ -n "$count" ] && [ "$count" -le 64 ] || die "${bin##*/} has no readable program headers"
  for ((i = 0; i < count; i++)); do
    type=$(le "$bin" $((phoff + i * size)) 4)
    [ "$type" != 3 ] || die "${bin##*/} is dynamically linked (it names an interpreter)"
    [ "$type" != 1 ] || load=1
  done
  [ -n "$load" ] || die "${bin##*/} has nothing to load"
  LC_ALL=C grep -aqE "(^|[^0-9])${version//./\\.}([^0-9]|$)" "$bin" || die "${bin##*/} does not carry version $version"
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
  local exe
  exe=$(plutil -extract CFBundleExecutable raw -o - "$plist" 2>/dev/null) || die "Info.plist has no CFBundleExecutable"
  both_archs "$app/Contents/MacOS/$exe"
  both_archs "$hive"
  local version build
  version=$(plutil -extract CFBundleShortVersionString raw -o - "$plist" 2>/dev/null) ||
    die "Info.plist has no CFBundleShortVersionString"
  [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "not a version: $version"
  for build in $LINUX_BUILDS; do
    linux_build "$app/Contents/Resources/${build%:*}" "${build#*:}" "$version"
  done

  # --verify checks every architecture; what codesign displays is only this machine's slice unless
  # --arch names another, so each is asked for by name.
  codesign --verify --deep --strict "$app" || die "the bundle does not validate"
  codesign --verify --strict "$hive" || die "the nested hive does not validate"

  local arch entitlements hive_signer app_signer
  for arch in $ARCHS; do
    case ",$(flags "$hive" "$arch")," in *,linker-signed,*) die "the nested hive's $arch slice is only linker-signed" ;; esac
    hive_signer=$(signer "$hive" "$arch")
    app_signer=$(signer "$app" "$arch")
    [ "$hive_signer" = "$app_signer" ] ||
      die "the nested hive ($hive_signer) and the app ($app_signer) have different signers in $arch"

    entitlements=$(codesign --arch "$arch" -d --entitlements - --xml "$app" 2>/dev/null) || die "no entitlements in $arch"
    grep -q '<key>com.apple.security.cs.allow-jit</key><true/>' <<<"${entitlements//[$'\t\n ']/}" ||
      die "the $arch entitlements do not allow the WebView's JIT"
    grep -q '<key>com.apple.security.automation.apple-events</key><true/>' <<<"${entitlements//[$'\t\n ']/}" ||
      die "the $arch entitlements do not allow the terminals' Apple Events"
  done

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
    for arch in $ARCHS; do
      case ",$(flags "$hive" "$arch")," in *,runtime,*) ;; *) die "the nested hive's $arch slice lacks the hardened runtime" ;; esac
    done
  fi
  if [ "$option" = --self-signed ]; then
    local id
    id=$(plutil -extract CFBundleIdentifier raw -o - "$plist") || die "Info.plist has no CFBundleIdentifier"
    for arch in $ARCHS; do
      case ",$(flags "$app" "$arch")," in *,runtime,*) ;; *) die "the app's $arch slice lacks the hardened runtime" ;; esac
      names_certificate "$app" "$arch" "$id" "$certificate"
      names_certificate "$hive" "$arch" com.lucas.hive.service "$certificate"
    done
  fi
  if [ "$option" = --developer-id ]; then
    spctl -a -vv --type execute "$app" || die "Gatekeeper rejects the bundle"
    xcrun stapler validate "$app" || die "no stapled notarization ticket"
  fi
  echo "ok    $app"
}

# How long the app's Intel slice must stay up, in half seconds.
SMOKE_APP_TICKS=${SMOKE_APP_TICKS:-20}

smoke() {
  [ -n "${GITHUB_ACTIONS:-}" ] || die "smoke opens the app's window: CI only"
  local app=$1 hive=$1/Contents/Resources/hive plist=$1/Contents/Info.plist
  [ -f "$hive" ] || die "no hive in $app"
  local arch version first=
  for arch in $ARCHS; do
    version=$(arch "-$arch" "$hive" --version) || die "the $arch hive does not run (for x86_64: is Rosetta installed?)"
    case $version in "hive "*) ;; *) die "the $arch hive answers: $version" ;; esac
    [ -z "$first" ] || [ "$first" = "$version" ] || die "the slices disagree: $first, and in $arch $version"
    first=$version
    echo "ok    $arch $version"
  done

  local exe home pid tick
  exe=$(plutil -extract CFBundleExecutable raw -o - "$plist") || die "Info.plist has no CFBundleExecutable"
  home=$(mktemp -d "${TMPDIR:-/tmp}/hive-smoke.XXXXXX")
  HOME=$home arch -x86_64 "$app/Contents/MacOS/$exe" &
  pid=$!
  for ((tick = 0; tick < SMOKE_APP_TICKS; tick++)); do
    sleep 0.5
    if ! kill -0 "$pid" 2>/dev/null; then
      rm -rf "$home"
      die "the x86_64 app ended after $tick half seconds"
    fi
  done
  # An app that quit between the last look and now did not stay up either: say so, never a bare
  # `set -e` exit.
  if ! kill "$pid" 2>/dev/null; then
    rm -rf "$home"
    die "the x86_64 app ended before it was stopped"
  fi
  wait "$pid" 2>/dev/null || true
  rm -rf "$home" || true
  echo "ok    x86_64 app stayed up"
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

# join_dsym <arm64 dSYM> <x86_64 dSYM> <new dSYM>: the first bundle, its DWARF file holding both.
# Cargo names that file after the build's hash, which differs between the two.
join_dsym() {
  local arm=$1 intel=$2 new=$3 arm_dwarf intel_dwarf
  [ -d "$arm" ] || die "no dSYM: $arm"
  [ -d "$intel" ] || die "no dSYM: $intel"
  arm_dwarf=$(find -L "$arm/Contents/Resources/DWARF" -type f)
  intel_dwarf=$(find -L "$intel/Contents/Resources/DWARF" -type f)
  [ -n "$arm_dwarf" ] && [ "$(wc -l <<<"$arm_dwarf")" -eq 1 ] || die "not one DWARF file in $arm"
  [ -n "$intel_dwarf" ] && [ "$(wc -l <<<"$intel_dwarf")" -eq 1 ] || die "not one DWARF file in $intel"
  cp -RL "$arm" "$new"
  lipo -create -output "$new/Contents/Resources/DWARF/${arm_dwarf##*/}" "$arm_dwarf" "$intel_dwarf" ||
    die "lipo could not join $arm and $intel"
}

join_dsyms() {
  local arm=$1 intel=$2 new=$3 name
  mkdir "$new" || die "cannot make a new folder: $new"
  for name in hive hive-app; do join_dsym "$arm/$name.dSYM" "$intel/$name.dSYM" "$new/$name.dSYM"; done
}

# The keys the updater looks up, {os}-{arch}-{bundle type} first (tauri-plugin-updater).
check_updater() {
  local file=$1 key
  [ -f "$file" ] || die "no such file: $file"
  jq -e '.platforms["darwin-aarch64"] | (.url | endswith(".app.tar.gz")) and (.signature | length > 0)' "$file" >/dev/null ||
    die "$file: darwin-aarch64 has no updater tarball and signature"
  for key in darwin-aarch64-app darwin-x86_64 darwin-x86_64-app; do
    jq -e --arg key "$key" '.platforms[$key] == .platforms["darwin-aarch64"]' "$file" >/dev/null ||
      die "$file: $key is not darwin-aarch64's tarball and signature"
  done
  echo "ok    $file"
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
  join-dsyms) [ $# = 4 ] || die "usage: $0 join-dsyms <arm64 folder> <x86_64 folder> <new folder>"; join_dsyms "$2" "$3" "$4" ;;
  check-updater) [ $# = 2 ] || die "usage: $0 check-updater <latest.json>"; check_updater "$2" ;;
  smoke) [ $# = 2 ] || die "usage: $0 smoke <Hive.app>"; smoke "$2" ;;
  check)
    [ $# -ge 2 ] && [ $# -le 4 ] || die "usage: $0 check <Hive.app> [--developer-id | --self-signed <SHA-1>]"
    check "$2" "${3:-}" "${4:-}"
    ;;
  *) die "usage: $0 signing-mode | new-certificate <new folder> | import-certificate <new keychain> | sign-hive <hive binary> | release-notes <signing mode> | check <Hive.app> [--developer-id | --self-signed <SHA-1>] | dsyms <Hive.app> <folder> | join-dsyms <arm64 folder> <x86_64 folder> <new folder> | check-updater <latest.json> | smoke <Hive.app>" ;;
esac
