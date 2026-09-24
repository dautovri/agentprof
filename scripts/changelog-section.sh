#!/usr/bin/env bash
# Prints the CHANGELOG.md section for a release, for use as release notes.
#   scripts/changelog-section.sh v0.3.0 [CHANGELOG.md]
set -euo pipefail

version="${1:?usage: changelog-section.sh <version> [changelog]}"
version="${version#v}"
changelog="${2:-CHANGELOG.md}"

section="$(awk -v ver="$version" '
  index($0, "## [" ver "]") == 1 { found = 1; next }
  found && /^## \[/ { exit }
  found { print }
' "$changelog")"

if [ -z "$(printf '%s' "$section" | tr -d '[:space:]')" ]; then
  echo "See [CHANGELOG.md](https://github.com/dautovri/agentprof/blob/main/CHANGELOG.md)."
else
  printf '%s\n' "$section" | sed -e '/./,$!d'
fi
