# CSDict

A fast, cross-platform (Windows / Linux / macOS) dictionary written in Rust
with [egui](https://github.com/emilk/egui). Czech ↔ English (plus English
definitions from WordNet), Czech ↔ German, Czech ↔ Ancient Greek, and
English ↔ German, Ancient Greek, Spanish, French, Russian, Portuguese,
Japanese, Arabic, Chinese, Hindi, Korean.

## Install

Download the file for your system from the
[latest release](https://github.com/martinkunc/csdict/releases/latest).
Dictionaries are not bundled: pick and download them in the **D** menu
(until then words are looked up online).

* **Windows** – unzip, run `csdict.exe`.
* **macOS** – open the `.dmg`, drag *CSDict* to Applications. It is not
  notarized: on first start right-click → Open, or
  `xattr -dr com.apple.quarantine /Applications/CSDict.app`.
* **Linux** – unpack, run `./csdict`; `./install-desktop-icon.sh` adds a
  menu entry with icon.

* Borderless window: a large search box replaces the title bar; window buttons
  are drawn on the platform's side (left on macOS and on Linux desktops
  configured that way, right otherwise).
* **D** menu (top-left): choose dictionaries (checkbox), set their order,
  download / remove / update them, toggle online lookup, theme (same as system by
  default, or light / dark). On first start the dictionary of the system
  language is selected too. Ctrl/⌘ + / − / 0 zooms (remembered).
* Both directions of a language pair are one dictionary: *Czech ↔ English*
  looks a word up in Czech → English, English → Czech and the English
  WordNet at once. Each entry is labelled with its direction and source
  (e.g. `English → Czech · GNU/FDL`, `English definitions · WordNet`).
* Left: matching words (exact, inflected form → lemma, prefix), then
  *similar words* (typo-tolerant). Typing without diacritics works
  (`zlutou` → `žlutý`, `logos` → `λόγος`); Japanese can be typed in kana
  or rōmaji (`inu` → 犬), Chinese in pinyin (`gou` → 狗).
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
cargo run --release -p dict-app          # the app (binary: csdict)
cargo run --release -p dict-app -- pes   # start with a lookup
cargo test --workspace                   # tests (add -- --include-ignored for online tests)
```

Linux build needs nothing beyond Rust. For pronunciation on Linux install
`espeak-ng` (or speech-dispatcher) and an audio player (`paplay`, `ffplay`
or `mpv`). Windows and macOS use built-in tools (PowerShell / `afplay`, `say`).

## Building the dictionary packs

```sh
scripts/fetch-raw.sh data/raw      # ~1.5 GB: kaikki.org dumps, GNU/FDL slovník, WordNet
cargo run --release -p dict-build -- all --raw data/raw --out dist
```

Takes about two minutes and ~3.5 GB RAM. Result (2026-11):

| pack   | entries | size    | sources |
|--------|--------:|--------:|---------|
| cs-en  | 149 780 |  14.4 MB | Wiktionary + GNU/FDL |
| en-cs  | 137 368 |  11.5 MB | Wiktionary + GNU/FDL |
| en-wn  | 135 906 |  17.0 MB | Open English WordNet |
| de-cs  |  51 828 |   9.0 MB | GNU/FDL + Wiktionary via English |
| cs-de  |  38 868 |   5.4 MB | GNU/FDL + Wiktionary via English |
| grc-cs |   8 379 |   2.2 MB | Wiktionary via English |
| cs-grc |   9 633 |   1.3 MB | Wiktionary via English |
| de-en  | 113 264 |  27.4 MB | Wiktionary |
| en-de  |  73 643 |   8.9 MB | Wiktionary |
| grc-en |  30 359 |  10.3 MB | Wiktionary |
| en-grc |  11 575 |   1.5 MB | Wiktionary |
| es-en  | 142 292 |  20.0 MB | Wiktionary |
| en-es  |  68 791 |   8.2 MB | Wiktionary |
| fr-en  | 117 953 |  16.4 MB | Wiktionary |
| en-fr  |  78 037 |   8.8 MB | Wiktionary |
| ru-en  |  77 393 |  18.4 MB | Wiktionary |
| en-ru  |  64 631 |   8.5 MB | Wiktionary |
| pt-en  |  90 343 |  13.7 MB | Wiktionary |
| en-pt  |  58 693 |   6.9 MB | Wiktionary |
| ja-en  | 112 111 |  19.7 MB | Wiktionary |
| en-ja  |  39 959 |   5.0 MB | Wiktionary |
| ar-en  |  28 726 |   7.2 MB | Wiktionary |
| en-ar  |  24 698 |   3.3 MB | Wiktionary |
| zh-en  | 195 061 |  28.8 MB | Wiktionary |
| en-zh  |  53 256 |   6.6 MB | Wiktionary |
| hi-en  |  26 507 |   6.1 MB | Wiktionary |
| en-hi  |  20 154 |   2.6 MB | Wiktionary |
| ko-en  |  47 361 |   8.2 MB | Wiktionary |
| en-ko  |  27 521 |   3.4 MB | Wiktionary |

X → English packs are the English Wiktionary's entries for words of
language X; English → X packs come from the translation tables of English
entries. Japanese can be looked up by kana and rōmaji, Chinese by pinyin
(index keys only).

"Via English" pairs come from Wiktionary translation tables: Czech and
German/Greek translations listed under the same English sense are linked.
They are marked as such in the app.

### Hosting

The app downloads packs on demand from the catalog at
`https://github.com/martinkunc/csdict/releases/download/dictionaries/catalog.json`.
The **Build dictionaries** workflow (`.github/workflows/build-dictionaries.yml`,
started manually) rebuilds every pack and overwrites the assets of that
rolling `dictionaries` release; the app then offers *Update* for packs whose
build differs from the downloaded one.

Packs can be hosted anywhere else too: upload `dist/*.csd` and
`dist/catalog.json` and put the catalog URL into the D menu (or set
`CSDICT_CATALOG`). A local path works too:

```sh
CSDICT_CATALOG=$PWD/dist/catalog.json cargo run --release -p dict-app
```

## Releases

Pushing a version tag (`v1.2.0` or `1.2.0`) runs `.github/workflows/release.yml`:
it builds Windows x64 (`.zip`, static CRT, embedded icon), Linux x64 and
arm64 (`.tar.gz` with `.desktop` file, icons and `install-desktop-icon.sh`)
and a universal macOS app (`.dmg`, ad-hoc signed), and attaches them to the
GitHub Release of the tag. The packaging scripts are in `packaging/`.
Dictionaries are not bundled. Running the workflow manually builds the
packages as workflow artifacts without releasing.

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
