#!/usr/bin/env bash
# Builds the terminal/editor font of task 7.11 (research: docs/spike/terminal-font.md) as
# woff2 into an output directory OUTSIDE this repository:
#
#   <FAMILY>-<Weight>.woff2        IBM Plex Mono + Fira Code ligatures (Ligaturizer)
#   SymbolsNerdFont-Regular.woff2  the Nerd Fonts icons alone, used as a fallback family
#   ligatures.json                 the character sequences the fonts join (xterm joiner list)
#   LICENSE-*.txt, README-Symbols-Nerd-Font.md  the licences that ship with the fonts
#   <FAMILY>NerdFont-<Weight>.woff2  only with NERD_PATCH=1: the ligature font patched with
#                                  every Nerd Fonts glyph (font-patcher --complete), ~1 MB each
#
#   scripts/build-terminal-font.sh [OUT_DIR]      (default: a new dir under /var/tmp)
#
# Every input is a pinned release checked against its sha256. FontForge: set FONTFORGE to a
# fontforge binary with Python (`sudo apt install fontforge python3-fontforge`,
# `brew install fontforge`); otherwise on Linux x86_64 the pinned official AppImage is
# downloaded and extracted into the work dir (nothing is installed system-wide).
#
# Env: WEIGHTS (default "Regular Medium SemiBold Bold"), FAMILY (default "Hive Mono"; must not
# contain "Plex", the Reserved Font Name of IBM Plex's OFL licence), NERD_PATCH=1.
set -euo pipefail
# FontForge stamps this time into the fonts instead of now: same inputs, same bytes.
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-1758758400}

FAMILY=${FAMILY:-Hive Mono}
WEIGHTS=${WEIGHTS:-Regular Medium SemiBold Bold}

PLEX_URL='https://github.com/IBM/plex/releases/download/%40ibm/plex-mono%402.5.0/ibm-plex-mono.zip'
PLEX_SHA=6d23f01257663d8cc49a0d64c22ced630b79e0e2a0ac08a0da86e9a38bbc481c
# Fira Code v3.1 OTFs: the revision Ligaturizer v5 pins as its submodule. Fira Code 6.x ships
# only TTFs, and saving a font with their ligatures pasted in crashes FontForge.
FIRA_URL='https://raw.githubusercontent.com/tonsky/FiraCode/e9943d2d631a4558613d7a77c58ed1d3cb790992'
FIRA_LICENSE_SHA=1d41e10031ab125302780a05ec4c91d218e47db0c7e37cf315cce5e608cdc25c
fira_sha() { # (bash 3 on macOS has no associative arrays)
  case $1 in
    Regular) echo b5639c832c98f9f4dc5bd6c0806ca0761e0af495bf1ffadf44943b0c2d634507 ;;
    Medium) echo a4ca4817a07dff8c40c923657f8b23c02e29f4d7d30761454188b0a44c2e7f8c ;;
    SemiBold) echo 803dd2d4a698c307710b7a11613e0474d423ca22f5b3a5f70a201d2bd739f067 ;;
    Bold) echo f8bc0f1234ee5639fb791d8b3cf69f5594eb422e98bb559089d7262b96eff422 ;;
  esac
}
LIGA_REV=c4065187a544a8fab40826fc91db1c6180a2d342 # Ligaturizer v5
LIGA_URL="https://github.com/ToxicFrog/Ligaturizer/archive/$LIGA_REV.tar.gz"
LIGA_SHA=8ccfcb008c28a9619fb525059dbde80320bc2fa8dd4c5c5f7b6e48a1d30808ec
NERD=https://github.com/ryanoasis/nerd-fonts/releases/download/v3.5.1
SYMBOLS_SHA=01172f37db8543edb102e5cb5c64101c9f4686630804d49b419aa07b23a69996
PATCHER_SHA=42bcb32145499a35732274c7fc48deb434ad0d2e0e118f98527c1479c6fa251a
FF_URL='https://github.com/fontforge/fontforge/releases/download/20251009/FontForge-2025-10-09-Linux-x86_64.AppImage'
FF_SHA=bf72ec45305c663da7d2288822b7d2cfc37846716322bdca4075273c0cef8294

die() { echo "build-terminal-font: $*" >&2; exit 1; }

case $FAMILY in *[Pp]lex*) die "FAMILY must not contain the Reserved Font Name \"Plex\"" ;; esac

repo=$(cd "$(dirname "$0")/.." && pwd -P)
out=${1:-$(mktemp -d /var/tmp/hive-terminal-font.XXXXXX)}
mkdir -p "$out"
out=$(cd "$out" && pwd -P)
case $out/ in "$repo"/*) die "OUT_DIR must be outside the repository ($repo)" ;; esac
work=$out/work
mkdir -p "$work"

# fetch <url> <sha256> <file>: download once, always verify.
fetch() {
  [[ -f $work/$3 ]] || curl -fsSL --retry 3 -o "$work/$3" "$1"
  echo "$2  $work/$3" | shasum -a 256 -c --status - || die "checksum mismatch for $3"
}

fetch "$PLEX_URL" "$PLEX_SHA" plex.zip
fetch "$LIGA_URL" "$LIGA_SHA" ligaturizer.tar.gz
fetch "$NERD/NerdFontsSymbolsOnly.tar.xz" "$SYMBOLS_SHA" symbols.tar.xz
unzip -oq "$work/plex.zip" -d "$work/plex"
tar -xzf "$work/ligaturizer.tar.gz" -C "$work"
mkdir -p "$work/symbols"
tar -xJf "$work/symbols.tar.xz" -C "$work/symbols"
liga=$work/Ligaturizer-$LIGA_REV

if [[ -z ${FONTFORGE:-} ]]; then
  [[ $(uname -sm) == "Linux x86_64" ]] || die "set FONTFORGE to a fontforge binary with Python"
  fetch "$FF_URL" "$FF_SHA" fontforge.AppImage
  if [[ ! -x $work/squashfs-root/AppRun ]]; then
    chmod +x "$work/fontforge.AppImage"
    (cd "$work" && ./fontforge.AppImage --appimage-extract >/dev/null)
  fi
  FONTFORGE=$work/squashfs-root/AppRun
fi
# The AppImage's AppRun changes directory: every path passed to it is absolute.
ff() { "$FONTFORGE" -quiet -lang=py -script "$@"; }

cat >"$work/woff2.py" <<'EOF'
import sys, fontforge
fontforge.open(sys.argv[1]).generate(sys.argv[2])
EOF
# The sequences whose Fira Code ligature exists: xterm draws only joined ranges as one unit.
cat >"$work/ligatures-json.py" <<'EOF'
import json, sys, fontforge
sys.path.insert(0, sys.argv[1])
from ligatures import ligatures
from char_dict import char_dict
fira = fontforge.open(sys.argv[2])
seqs = sorted({"".join(char_dict[c] for c in l["chars"]) for l in ligatures
               if l["firacode_ligature_name"] and l["firacode_ligature_name"] in fira},
              key=lambda s: (-len(s), s))
json.dump(seqs, open(sys.argv[3], "w"), indent=2)
open(sys.argv[3], "a").write("\n")
EOF

for w in $WEIGHTS; do
  src=$work/plex/ibm-plex-mono/fonts/complete/ttf/IBMPlexMono-$w.ttf
  sha=$(fira_sha "$w")
  [[ -f $src && -n $sha ]] || die "no Plex Mono / Fira Code weight \"$w\""
  fetch "$FIRA_URL/distr/otf/FiraCode-$w.otf" "$sha" "FiraCode-$w.otf"
  rm -rf "$work/liga-$w"
  mkdir -p "$work/liga-$w"
  echo "== $w: ligatures"
  ff "$liga/ligaturize.py" "$src" --ligature-font-file "$work/FiraCode-$w.otf" \
    --output-dir "$work/liga-$w" --prefix "" --output-name "$FAMILY" >"$work/liga-$w.log" 2>&1 ||
    die "Ligaturizer failed, see $work/liga-$w.log"
  ligttf=$(echo "$work/liga-$w"/*.ttf)
  ff "$work/woff2.py" "$ligttf" "$out/${FAMILY// /}-$w.woff2" >>"$work/liga-$w.log" 2>&1 ||
    die "woff2 conversion failed, see $work/liga-$w.log"
  [[ -f $out/ligatures.json ]] ||
    ff "$work/ligatures-json.py" "$liga" "$work/FiraCode-$w.otf" "$out/ligatures.json" >/dev/null 2>&1 ||
    die "could not list the ligatures"

  if [[ ${NERD_PATCH:-0} == 1 ]]; then
    fetch "$NERD/FontPatcher.zip" "$PATCHER_SHA" font-patcher.zip
    [[ -d $work/font-patcher ]] || unzip -oq "$work/font-patcher.zip" -d "$work/font-patcher"
    rm -rf "$work/nerd-$w"
    mkdir -p "$work/nerd-$w"
    echo "== $w: nerd glyphs"
    ff "$work/font-patcher/font-patcher" --complete --quiet --no-progressbars \
      --outputdir "$work/nerd-$w" "$ligttf" >"$work/nerd-$w.log" 2>&1 ||
      die "font-patcher failed, see $work/nerd-$w.log"
    for ttf in "$work/nerd-$w"/*.ttf; do
      ff "$work/woff2.py" "$ttf" "$out/$(basename "${ttf%.ttf}").woff2" >>"$work/nerd-$w.log" 2>&1 ||
        die "woff2 conversion failed, see $work/nerd-$w.log"
    done
  fi
done

ff "$work/woff2.py" "$work/symbols/SymbolsNerdFont-Regular.ttf" "$out/SymbolsNerdFont-Regular.woff2" \
  >/dev/null 2>&1 || die "woff2 conversion of the Nerd Fonts symbols failed"
fetch "$FIRA_URL/LICENSE" "$FIRA_LICENSE_SHA" fira-license.txt
# The OFL travels with the fonts; the icon sets' licences (CC BY 4.0 among them) are listed in
# the symbols README.
cp "$work/plex/ibm-plex-mono/fonts/complete/ttf/license.txt" "$out/LICENSE-IBM-Plex-Mono.txt"
cp "$work/fira-license.txt" "$out/LICENSE-Fira-Code.txt"
cp "$work/symbols/LICENSE" "$out/LICENSE-Symbols-Nerd-Font.txt"
cp "$work/symbols/README.md" "$out/README-Symbols-Nerd-Font.md"
ls -l "$out"/*.woff2 "$out/ligatures.json"
echo "Output: $out (work files in $out/work; delete it when done)"
