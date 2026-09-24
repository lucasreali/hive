#!/usr/bin/env bash
# Bumps every manifest to <version>, commits and tags v<version> (TODO 4.17).
# Pushing the tag is the human's: `git push origin main v<version>` starts .github/workflows/release.yml.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH=$HOME/.cargo/bin:$PATH

version=${1:-}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "usage: $0 <major.minor.patch>" >&2; exit 1; }
[[ -z $(git status --porcelain) ]] || { echo "release: the working tree is not clean" >&2; exit 1; }

# Only each manifest's own version line, never a dependency's. The handshake compares the app's
# and the service's versions (#29); hive-protocol keeps its own.
sed -i "0,/\"version\": \"[^\"]*\"/s//\"version\": \"$version\"/" src-tauri/tauri.conf.json package.json
sed -i "0,/^version = \"[^\"]*\"/s//version = \"$version\"/" src-tauri/Cargo.toml crates/hive/Cargo.toml
cargo check --workspace --quiet   # moves the workspace crates' versions in Cargo.lock

git commit -qam "chore(release): v$version"
git tag "v$version"
echo "tagged v$version; push it with: git push origin main v$version"
