#!/usr/bin/env bash
# Downloads the raw inputs of `dict-build all` into data/raw (or $1).
# Files that already exist are kept; delete them to refresh.
#
#   scripts/fetch-raw.sh [raw-dir]
set -euo pipefail

raw="${1:-data/raw}"
ua="csdict-build (https://github.com/martinkunc/csdict)"

fetch() { # url dest
  if [ -s "$2" ]; then
    echo "have $2"
    return
  fi
  mkdir -p "$(dirname "$2")"
  echo "get  $1"
  curl -fL --no-progress-meter --retry 3 -A "$ua" -o "$2.part" "$1"
  mv "$2.part" "$2"
}

# Wiktionary, extracted by wiktextract (kaikki.org). code:Language name
for pair in en:English cs:Czech de:German "grc:Ancient Greek" es:Spanish fr:French ru:Russian \
            pt:Portuguese ja:Japanese ar:Arabic zh:Chinese hi:Hindi ko:Korean; do
  code="${pair%%:*}"
  name="${pair#*:}"
  stem="kaikki.org-dictionary-${name// /}"
  fetch "https://kaikki.org/dictionary/${name// /%20}/$stem.jsonl.gz" "$raw/kaikki-$code/$stem.jsonl.gz"
done

# GNU/FDL English–Czech and German–Czech (svobodneslovniky.cz)
for pair in english-czech german-czech; do
  tgz="$raw/slovnik/stardict-$pair-latest-source.tar.gz"
  fetch "https://dl.cihar.com/slovnik/stardict-$pair-latest-source.tar.gz" "$tgz"
  tar -xzf "$tgz" -C "$raw/slovnik"
done

# Open English WordNet
zip="$raw/wordnet/english-wordnet-2025-json.zip"
fetch "https://github.com/globalwordnet/english-wordnet/releases/download/2025-edition/english-wordnet-2025-json.zip" "$zip"
unzip -oq "$zip" -d "$raw/wordnet/json"
