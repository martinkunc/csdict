//! The `.csd` pack format.
//!
//! ```text
//! "CSDICT\0\x01"                          8 bytes magic + version
//! SECTION_COUNT × (offset: u64, len: u64)  section table (little endian)
//! sections…
//! ```
//!
//! Sections:
//! * `META`       – JSON [`DictMeta`]
//! * `FST`        – `fst::Map` from folded key → offset into `POSTINGS`
//! * `POSTINGS`   – per key: varint count, then varint `(entry_id << 1) | is_form`
//! * `HEADS_IDX`  – `u32` offsets into `HEADS` (entries + 1)
//! * `HEADS`      – `headword \0 pos` for every entry (uncompressed, for result lists)
//! * `BLOCK_IDX`  – `u64` offsets into `BLOCKS` (blocks + 1)
//! * `BLOCKS`     – zstd(postcard(Vec<Entry>)), [`BLOCK_SIZE`] entries per block
//!
//! The file is memory-mapped; opening a pack is O(1) and lookups touch only
//! the pages they need.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, Mutex};

use fst::automaton::{Levenshtein, Str};
use fst::{Automaton, IntoStreamer, Map, Streamer};
use memmap2::Mmap;

use crate::model::{DictMeta, Entry};
use crate::normalize::{edit_distance, fold, nfc};
use crate::Error;

const MAGIC: &[u8; 8] = b"CSDICT\0\x01";
const SECTION_COUNT: usize = 7;
const S_META: usize = 0;
const S_FST: usize = 1;
const S_POSTINGS: usize = 2;
const S_HEADS_IDX: usize = 3;
const S_HEADS: usize = 4;
const S_BLOCK_IDX: usize = 5;
const S_BLOCKS: usize = 6;
/// Entries per compressed block.
pub const BLOCK_SIZE: usize = 32;
const HEADER_LEN: usize = 8 + SECTION_COUNT * 16;

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Collects entries and writes a pack.
pub struct PackBuilder {
    meta: DictMeta,
    entries: Vec<Entry>,
}

impl PackBuilder {
    pub fn new(meta: DictMeta) -> Self {
        PackBuilder { meta, entries: Vec::new() }
    }

    pub fn add(&mut self, mut entry: Entry) {
        entry.headword = nfc(&entry.headword);
        if entry.headword.is_empty() || entry.senses.is_empty() {
            return;
        }
        self.entries.push(entry);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn write(mut self, path: &Path) -> Result<DictMeta, Error> {
        // Sort so related entries share blocks (better compression) and ids
        // follow alphabetical order. Stable sort keeps source order.
        self.entries
            .sort_by_cached_key(|e| (fold(&e.headword), e.headword.clone()));
        self.meta.entries = self.entries.len() as u32;

        // Index keys.
        let mut keys: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        for (id, e) in self.entries.iter().enumerate() {
            let id = id as u32;
            push_posting(&mut keys, fold(&e.headword), id << 1);
            for f in &e.index_forms {
                let k = fold(f);
                if !k.is_empty() {
                    push_posting(&mut keys, k, (id << 1) | 1);
                }
            }
        }

        let mut postings = Vec::new();
        let mut fst_builder = fst::MapBuilder::memory();
        for (key, mut list) in keys {
            // Headword postings first, then forms; drop a form posting if the
            // same entry already matches as headword.
            list.sort_unstable_by_key(|p| (p & 1, p >> 1));
            list.dedup_by_key(|p| *p >> 1);
            fst_builder.insert(key.as_bytes(), postings.len() as u64)?;
            write_varint(&mut postings, list.len() as u64);
            for p in list {
                write_varint(&mut postings, u64::from(p));
            }
        }
        let fst_bytes = fst_builder.into_inner()?;

        let mut heads_idx = Vec::with_capacity((self.entries.len() + 1) * 4);
        let mut heads = Vec::new();
        for e in &self.entries {
            heads_idx.extend_from_slice(&(heads.len() as u32).to_le_bytes());
            heads.extend_from_slice(e.headword.as_bytes());
            heads.push(0);
            heads.extend_from_slice(e.pos.as_bytes());
        }
        heads_idx.extend_from_slice(&(heads.len() as u32).to_le_bytes());

        let mut block_idx = Vec::new();
        let mut blocks = Vec::new();
        for chunk in self.entries.chunks(BLOCK_SIZE) {
            block_idx.extend_from_slice(&(blocks.len() as u64).to_le_bytes());
            let raw = postcard::to_stdvec(chunk)?;
            let compressed = compress(&raw)?;
            blocks.extend_from_slice(&compressed);
        }
        block_idx.extend_from_slice(&(blocks.len() as u64).to_le_bytes());

        let meta_json = serde_json::to_vec_pretty(&self.meta)?;
        let sections: [&[u8]; SECTION_COUNT] = [
            &meta_json, &fst_bytes, &postings, &heads_idx, &heads, &block_idx, &blocks,
        ];

        let tmp = path.with_extension("csd.tmp");
        {
            let mut w = BufWriter::new(File::create(&tmp)?);
            w.write_all(MAGIC)?;
            let mut offset = HEADER_LEN as u64;
            for s in &sections {
                w.write_all(&offset.to_le_bytes())?;
                w.write_all(&(s.len() as u64).to_le_bytes())?;
                offset += s.len() as u64;
            }
            for s in &sections {
                w.write_all(s)?;
            }
            w.flush()?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(self.meta)
    }
}

#[cfg(feature = "write")]
fn compress(raw: &[u8]) -> Result<Vec<u8>, Error> {
    Ok(zstd::bulk::compress(raw, 19)?)
}

/// Without the `write` feature, use ruzstd's (faster, weaker) encoder.
#[cfg(not(feature = "write"))]
fn compress(raw: &[u8]) -> Result<Vec<u8>, Error> {
    Ok(ruzstd::encoding::compress_to_vec(raw, ruzstd::encoding::CompressionLevel::Fastest))
}

fn push_posting(keys: &mut BTreeMap<String, Vec<u32>>, key: String, posting: u32) {
    keys.entry(key).or_default().push(posting);
}

fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn read_varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let b = *data.get(*pos)?;
        *pos += 1;
        v |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

/// A slice of the shared memory map, usable as fst backing storage.
#[derive(Clone)]
struct MapSlice {
    mmap: Arc<Mmap>,
    range: Range<usize>,
}

impl AsRef<[u8]> for MapSlice {
    fn as_ref(&self) -> &[u8] {
        &self.mmap[self.range.clone()]
    }
}

/// How a hit matched the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MatchKind {
    /// Headword equals the query (ignoring case/diacritics).
    Exact,
    /// The query is an inflected form of the headword.
    Form,
    /// Headword starts with the query.
    Prefix,
    /// Headword is within a small edit distance of the query.
    Similar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub entry_id: u32,
    pub headword: String,
    pub pos: String,
    pub kind: MatchKind,
    /// Lower is better; combines kind and closeness.
    pub rank: u32,
}

/// An opened, memory-mapped dictionary pack.
pub struct Dictionary {
    meta: DictMeta,
    mmap: Arc<Mmap>,
    sections: [Range<usize>; SECTION_COUNT],
    fst: Map<MapSlice>,
    cache: Mutex<HashMap<usize, Arc<Vec<Entry>>>>,
}

impl std::fmt::Debug for Dictionary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dictionary").field("meta", &self.meta).finish()
    }
}

impl Dictionary {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let file = File::open(path)?;
        // SAFETY: packs are written atomically (temp file + rename) and never
        // modified in place; the app drops the map before deleting a pack.
        let mmap = Arc::new(unsafe { Mmap::map(&file)? });
        if mmap.len() < HEADER_LEN || &mmap[..8] != MAGIC {
            return Err(Error::Format("not a csdict pack".into()));
        }
        let mut sections: [Range<usize>; SECTION_COUNT] = Default::default();
        for (i, s) in sections.iter_mut().enumerate() {
            let at = 8 + i * 16;
            let off = u64::from_le_bytes(mmap[at..at + 8].try_into().unwrap()) as usize;
            let len = u64::from_le_bytes(mmap[at + 8..at + 16].try_into().unwrap()) as usize;
            let end = off.checked_add(len).filter(|&e| e <= mmap.len());
            let Some(end) = end else {
                return Err(Error::Format(format!("section {i} out of bounds")));
            };
            *s = off..end;
        }
        let meta: DictMeta = serde_json::from_slice(&mmap[sections[S_META].clone()])?;
        let fst = Map::new(MapSlice { mmap: mmap.clone(), range: sections[S_FST].clone() })?;
        Ok(Dictionary { meta, mmap, sections, fst, cache: Mutex::new(HashMap::new()) })
    }

    /// Read only the metadata of a pack file.
    pub fn read_meta(path: &Path) -> Result<DictMeta, Error> {
        Ok(Self::open(path)?.meta)
    }

    pub fn meta(&self) -> &DictMeta {
        &self.meta
    }

    pub fn len(&self) -> usize {
        self.meta.entries as usize
    }

    pub fn is_empty(&self) -> bool {
        self.meta.entries == 0
    }

    fn section(&self, i: usize) -> &[u8] {
        &self.mmap[self.sections[i].clone()]
    }

    /// Headword and part of speech of an entry, without decompressing it.
    pub fn head(&self, id: u32) -> Option<(&str, &str)> {
        let idx = self.section(S_HEADS_IDX);
        let i = id as usize * 4;
        let start = u32::from_le_bytes(idx.get(i..i + 4)?.try_into().ok()?) as usize;
        let end = u32::from_le_bytes(idx.get(i + 4..i + 8)?.try_into().ok()?) as usize;
        let raw = std::str::from_utf8(self.section(S_HEADS).get(start..end)?).ok()?;
        Some(raw.split_once('\0').unwrap_or((raw, "")))
    }

    /// Full entry by id.
    pub fn entry(&self, id: u32) -> Result<Entry, Error> {
        let block = id as usize / BLOCK_SIZE;
        let entries = self.block(block)?;
        entries
            .get(id as usize % BLOCK_SIZE)
            .cloned()
            .ok_or_else(|| Error::Format(format!("entry {id} missing")))
    }

    fn block(&self, block: usize) -> Result<Arc<Vec<Entry>>, Error> {
        if let Some(b) = self.cache.lock().unwrap().get(&block) {
            return Ok(b.clone());
        }
        let idx = self.section(S_BLOCK_IDX);
        let read = |i: usize| -> Option<usize> {
            Some(u64::from_le_bytes(idx.get(i * 8..i * 8 + 8)?.try_into().ok()?) as usize)
        };
        let (Some(start), Some(end)) = (read(block), read(block + 1)) else {
            return Err(Error::Format(format!("block {block} missing")));
        };
        let data = self
            .section(S_BLOCKS)
            .get(start..end)
            .ok_or_else(|| Error::Format("block out of bounds".into()))?;
        let mut raw = Vec::new();
        ruzstd::decoding::StreamingDecoder::new(data)
            .map_err(|e| Error::Format(format!("zstd: {e}")))
            .and_then(|mut d| std::io::Read::read_to_end(&mut d, &mut raw).map_err(Error::from))?;
        let entries: Arc<Vec<Entry>> = Arc::new(postcard::from_bytes(&raw)?);
        let mut cache = self.cache.lock().unwrap();
        if cache.len() > 256 {
            cache.clear();
        }
        cache.insert(block, entries.clone());
        Ok(entries)
    }

    /// All entries whose headword is exactly `word` (after folding).
    pub fn entries_for(&self, word: &str) -> Vec<Entry> {
        let key = fold(word);
        let want = nfc(word);
        let mut out: Vec<Entry> = self
            .postings_for_key(&key)
            .into_iter()
            .filter(|(_, form)| !form)
            .filter_map(|(id, _)| self.entry(id).ok())
            .collect();
        // Prefer exact spelling (e.g. `Pes` vs `pes`), keep the rest.
        out.sort_by_key(|e| e.headword != want);
        out
    }

    fn postings_at(&self, offset: u64) -> Vec<(u32, bool)> {
        let data = self.section(S_POSTINGS);
        let mut pos = offset as usize;
        let Some(n) = read_varint(data, &mut pos) else { return Vec::new() };
        (0..n)
            .filter_map(|_| read_varint(data, &mut pos))
            .map(|p| ((p >> 1) as u32, p & 1 == 1))
            .collect()
    }

    fn postings_for_key(&self, key: &str) -> Vec<(u32, bool)> {
        self.fst.get(key).map(|o| self.postings_at(o)).unwrap_or_default()
    }

    /// Search for `query`: exact / inflected-form matches first, then
    /// headwords starting with the query, then similar spellings.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let key = fold(query);
        if key.is_empty() || limit == 0 {
            return Vec::new();
        }
        let query_nfc = nfc(query);
        let mut hits: Vec<Hit> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let mut push = |hits: &mut Vec<Hit>, id: u32, kind: MatchKind, rank: u32| {
            if seen.insert(id)
                && let Some((h, p)) = self.head(id)
            {
                hits.push(Hit { entry_id: id, headword: h.to_owned(), pos: p.to_owned(), kind, rank });
            }
        };

        // 1. Exact key (headword or inflected form).
        for (id, is_form) in self.postings_for_key(&key) {
            let (kind, base) = if is_form { (MatchKind::Form, 100) } else { (MatchKind::Exact, 0) };
            let exact_spelling = self.head(id).is_some_and(|(h, _)| h == query_nfc);
            push(&mut hits, id, kind, base + u32::from(!exact_spelling));
        }

        // 2. Prefix matches, shortest/alphabetical first (fst order is
        //    alphabetical; rank by length so `pes` < `pesky` < `pestrý`).
        let budget = limit * 4;
        let mut prefix: Vec<(String, u64)> = Vec::new();
        let mut stream = self.fst.search(Str::new(&key).starts_with()).into_stream();
        while let Some((k, v)) = stream.next() {
            if k.len() == key.len() {
                continue; // exact key, already handled
            }
            prefix.push((String::from_utf8_lossy(k).into_owned(), v));
            if prefix.len() >= budget {
                break;
            }
        }
        for (k, v) in &prefix {
            let extra = (k.chars().count() - key.chars().count()) as u32;
            for (id, is_form) in self.postings_at(*v) {
                if !is_form {
                    push(&mut hits, id, MatchKind::Prefix, 1000 + extra);
                }
            }
        }

        // 3. Similar spellings (only when we still have room).
        if hits.len() < limit {
            let n = key.chars().count();
            // Distance-2 automata get expensive for long keys (~ms); long
            // words rarely need more than one typo fixed anyway.
            let dist = match n {
                0..=2 => 0,
                3..=5 => 1,
                6..=12 => 2,
                _ => 1,
            };
            if dist > 0
                && let Ok(lev) = Levenshtein::new(&key, dist)
            {
                let mut stream = self.fst.search(lev).into_stream();
                let mut similar = Vec::new();
                while let Some((k, v)) = stream.next() {
                    let k = String::from_utf8_lossy(k).into_owned();
                    if k == key {
                        continue;
                    }
                    similar.push((edit_distance(&k, &key) as u32, k, v));
                    if similar.len() >= budget {
                        break;
                    }
                }
                similar.sort();
                for (d, _, v) in similar {
                    for (id, is_form) in self.postings_at(v) {
                        if !is_form {
                            push(&mut hits, id, MatchKind::Similar, 10_000 + d * 100);
                        }
                    }
                }
            }
        }

        hits.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.headword.cmp(&b.headword)));
        hits.truncate(limit);
        hits
    }

    /// Number of keys in the index (headwords + inflected forms).
    pub fn key_count(&self) -> usize {
        self.fst.len()
    }

    /// Iterate all keys matching an automaton (used by tools / tests).
    pub fn keys_matching<A: Automaton>(&self, aut: A, max: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut s = self.fst.search(aut).into_stream();
        while let Some((k, _)) = s.next() {
            out.push(String::from_utf8_lossy(k).into_owned());
            if out.len() >= max {
                break;
            }
        }
        out
    }
}
