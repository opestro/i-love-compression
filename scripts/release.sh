#!/usr/bin/env bash
# Releases a new version: sets X.Y.Z in the engine, the macOS app and the Windows app,
# commits, tags vX.Y.Z and pushes. CI then builds both apps and publishes the GitHub
# release with the .dmg and the Windows .zip attached.
#   scripts/release.sh 0.1.1     (fixes: bump the last number; features: the middle one)
set -euo pipefail
cd "$(dirname "$0")/.."

v="${1:?usage: scripts/release.sh X.Y.Z}"
[[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version must look like 0.1.1" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "commit or stash your changes first" >&2; exit 1; }
git rev-parse -q --verify "refs/tags/v$v" >/dev/null && { echo "v$v already exists" >&2; exit 1; }

perl -0pi -e "s/^version = \"[^\"]+\"/version = \"$v\"/m" engine/Cargo.toml
perl -0pi -e "s/(name = \"ilc-engine\"\nversion = \")[^\"]+/\${1}$v/" engine/Cargo.lock
perl -0pi -e "s#(<key>CFBundleShortVersionString</key>\\s*<string>)[^<]+#\${1}$v#; s#(<key>CFBundleVersion</key>\\s*<string>)[^<]+#\${1}$v#" macos/Info.plist
perl -0pi -e "s#<Version>[^<]+</Version>#<Version>$v</Version>#" windows/ILoveCompression/ILoveCompression.csproj

git diff --quiet || git commit -qam "Release v$v" # nothing to commit if already at v$v
git tag -a "v$v" -m "I Love Compression $v"
git push -q origin HEAD "v$v"
echo "pushed v$v; CI is building the release: $(git remote get-url origin | sed 's/\.git$//')/actions"
