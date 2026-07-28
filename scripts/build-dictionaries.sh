#!/usr/bin/env bash
# Regenerates the merged per-direction dictionary sqlite files with CSDict.Scraper.
# See docs/design/scraper-and-distribution.md for the overall architecture.
#
# Usage:
#   scripts/build-dictionaries.sh                 # scrape every direction
#   scripts/build-dictionaries.sh cs:en en:cs      # scrape just these directions
#
# Output goes to dist/dictionaries/{lemmaLang}_{targetLang}.sqlite3 (gitignored - these are
# regenerated locally/in CI and published as GitHub Release assets, never committed to git).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [ "$#" -eq 0 ]; then
  args=(--all)
else
  args=()
  for lang in "$@"; do
    args+=(--lang "$lang")
  done
fi

dotnet run --project src/CSDict.Scraper -c Release -- "${args[@]}" --output dist/dictionaries --cache dist/cache
