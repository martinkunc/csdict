# csdict

A fast, cross-platform (Windows / Linux / macOS) dictionary written in Rust
with [egui](https://github.com/emilk/egui). Czech ↔ English (plus English
definitions from WordNet), Czech ↔ German, Czech ↔ Ancient Greek,
German → English, Ancient Greek → English.

* Borderless window: a large search box replaces the title bar; window buttons
  are drawn on the platform's side (left on macOS and on Linux desktops
  configured that way, right otherwise).
* **D** menu (top-left): choose dictionaries (checkbox), set their order,
  download / remove them, toggle online lookup, theme (same as system by
  default, or light / dark).
* Both directions of a language pair are one dictionary: *Czech ↔ English*
  looks a word up in Czech → English, English → Czech and the English
  WordNet at once. Each entry is labelled with its direction and source
  (e.g. `English → Czech · GNU/FDL`, `English definitions · WordNet`).
* Left: matching words (exact, inflected form → lemma, prefix), then
  *similar words* (typo-tolerant). Typing without diacritics works
  (`zlutou` → `žlutý`, `logos` → `λόγος`).
* Right: one tab per selected dictionary, in the chosen order. Parts that
  are not downloaded are looked up online (kaikki.org for Wiktionary,
  en-word.net for WordNet).
  Online lookups are throttled: 500 ms after the word last changed, only for
  the word on screen (stale queued lookups are dropped), at most a burst of 4
  then ~1.5 requests/s with 2 in flight, a pause on HTTP 429/503 (honouring
  `Retry-After`), and results – including "not found" – are cached.
* Entries show part of speech, grammar, IPA, hyphenation, senses, examples,
  synonyms, etymology. 🔊 plays the Wiktionary recording or uses the OS
  text-to-speech. Text is selectable; right-click a translation to copy it
  or look it up.

## Layout of the repository

```
crates/dict-core   data model, .csd pack format, FST index + search, kaikki parser
crates/dict-build  CLI that turns raw dumps into .csd packs + catalog.json
crates/dict-app    the GUI
```

### Pack format and speed

Each dictionary is one memory-mapped `.csd` file:

* an [`fst`](https://docs.rs/fst) map from *folded* keys (lower-case, no
  diacritics) to postings – headwords **and all inflected forms**;
* uncompressed headword table for the result list;
* entries in zstd-compressed blocks of 32 (only the block you look at is
  decompressed).

Opening a pack is O(1); a lookup (exact + prefix + Levenshtein) takes
~0.05–1 ms.

## Building

```sh
cargo run --release -p dict-app          # the app
cargo run --release -p dict-app -- pes   # start with a lookup
cargo test --workspace                   # tests (add -- --include-ignored for online tests)
```

Linux build needs nothing beyond Rust. For pronunciation on Linux install
`espeak-ng` (or speech-dispatcher) and an audio player (`paplay`, `ffplay`
or `mpv`). Windows and macOS use built-in tools (PowerShell / `afplay`, `say`).

## Building the dictionary packs

```sh
mkdir -p data/raw/{kaikki-cs,kaikki-de,kaikki-grc,kaikki-en,slovnik,wordnet}
curl -L -o data/raw/kaikki-cs/kaikki.org-dictionary-Czech.jsonl \
  https://kaikki.org/dictionary/Czech/kaikki.org-dictionary-Czech.jsonl
curl -L -o data/raw/kaikki-de/kaikki.org-dictionary-German.jsonl.gz \
  https://kaikki.org/dictionary/German/kaikki.org-dictionary-German.jsonl.gz
curl -L -o data/raw/kaikki-grc/kaikki.org-dictionary-AncientGreek.jsonl.gz \
  "https://kaikki.org/dictionary/Ancient%20Greek/kaikki.org-dictionary-AncientGreek.jsonl.gz"
curl -L -o data/raw/kaikki-en/kaikki.org-dictionary-English.jsonl.gz \
  https://kaikki.org/dictionary/English/kaikki.org-dictionary-English.jsonl.gz   # 530 MB
mkdir -p data/raw/wordnet && (cd data/raw/wordnet && \
  curl -L -O https://github.com/globalwordnet/english-wordnet/releases/download/2025-edition/english-wordnet-2025-json.zip && \
  unzip -o english-wordnet-2025-json.zip -d json)
(cd data/raw/slovnik && \
  curl -L https://dl.cihar.com/slovnik/stardict-english-czech-latest-source.tar.gz | tar xz && \
  curl -L https://dl.cihar.com/slovnik/stardict-german-czech-latest-source.tar.gz | tar xz)

cargo run --release -p dict-build -- all --raw data/raw --out dist \
  --base-url https://example.org/csdict      # where the packs will be hosted
```

Takes about a minute and ~2 GB RAM. Result (2026-11):

| pack   | entries | size    | sources |
|--------|--------:|--------:|---------|
| cs-en  | 149 781 | 14.4 MB | Wiktionary + GNU/FDL (reversed) |
| en-cs  | 137 369 | 11.5 MB | Wiktionary translations + GNU/FDL |
| en-wn  | 135 906 | 17.0 MB | Open English WordNet 2025 (definitions) |
| de-en  | 113 264 | 27.4 MB | Wiktionary |
| grc-en |  30 359 | 10.3 MB | Wiktionary |
| de-cs  |  51 828 |  9.0 MB | GNU/FDL + Wiktionary via English |
| cs-de  |  38 868 |  5.4 MB | GNU/FDL + Wiktionary via English |
| grc-cs |   8 379 |  2.2 MB | Wiktionary via English |
| cs-grc |   9 633 |  1.3 MB | Wiktionary via English |

"Via English" pairs come from Wiktionary translation tables: Czech and
German/Greek translations listed under the same English sense are linked.
They are marked as such in the app.

### Hosting

Upload `dist/*.csd` and `dist/catalog.json` anywhere (e.g. a GitHub
Release) and put the catalog URL into the D menu (or set
`CSDICT_CATALOG`). A local path works too:

```sh
CSDICT_CATALOG=$PWD/dist/catalog.json cargo run --release -p dict-app
```

## Environment variables

| variable          | effect |
|-------------------|--------|
| `CSDICT_CATALOG`  | catalog URL or path (overrides settings) |
| `CSDICT_HOME`     | keep settings, packs and cache in this folder (portable mode) |
| `CSDICT_BUTTONS`  | `left` / `right`: force window-button side |

Developer aid: `cargo run -p dict-app --features screenshot` with
`EFRAME_SCREENSHOT_TO=shot.png` saves a screenshot of the first frames and exits.

## Data licenses

* Wiktionary content – CC BY-SA 4.0, extracted by
  [wiktextract](https://github.com/tatuylonen/wiktextract) /
  [kaikki.org](https://kaikki.org).
* GNU/FDL Anglicko-Český slovník – GNU FDL 1.1+ (no invariant sections).
  „Založeno na svobodném anglicko-českém slovníku,
  https://www.svobodneslovniky.cz/“.

* Open English WordNet – CC BY 4.0, https://en-word.net (based on Princeton
  WordNet 3.0). Used as an English definitions dictionary (English has no
  Czech translations in WordNet; the Czech WordNet is licensed for
  non-commercial use only and is not used).
