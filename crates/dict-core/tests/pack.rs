use dict_core::{DictMeta, Dictionary, Entry, MatchKind, PackBuilder, Sense};

fn entry(word: &str, pos: &str, tr: &str, forms: &[&str]) -> Entry {
    let mut e = Entry::new(word, pos, "test");
    e.senses.push(Sense { translations: vec![tr.into()], ..Default::default() });
    e.index_forms = forms.iter().map(|s| s.to_string()).collect();
    e
}

fn build() -> (tempfile::TempDir, Dictionary) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.csd");
    let meta = DictMeta {
        id: "cs-en".into(), name: "Czech → English".into(), source_lang: "cs".into(),
        target_lang: "en".into(), sources: "test".into(), license: "CC0".into(),
        built: "2026-11-01".into(), entries: 0,
    };
    let mut b = PackBuilder::new(meta);
    b.add(entry("žlutý", "adj", "yellow", &["žlutá", "žlutou", "žlutější"]));
    b.add(entry("pes", "noun", "dog", &["psa", "psovi"]));
    b.add(entry("pestrý", "adj", "colourful", &[]));
    b.add(entry("peso", "noun", "peso", &[]));
    b.add(entry("les", "noun", "forest", &["lesa"]));
    for i in 0..100 {
        b.add(entry(&format!("slovo{i:03}"), "noun", &format!("word {i}"), &[]));
    }
    b.write(&path).unwrap();
    let d = Dictionary::open(&path).unwrap();
    (dir, d)
}

#[test]
fn exact_form_prefix_similar() {
    let (_d, dict) = build();
    assert_eq!(dict.len(), 105);

    let hits = dict.search("zluty", 10);
    assert_eq!(hits[0].headword, "žlutý");
    assert_eq!(hits[0].kind, MatchKind::Exact);

    let hits = dict.search("žlutou", 10);
    assert_eq!(hits[0].headword, "žlutý");
    assert_eq!(hits[0].kind, MatchKind::Form);

    let hits = dict.search("pes", 10);
    let words: Vec<_> = hits.iter().map(|h| h.headword.as_str()).collect();
    assert_eq!(&words[..3], &["pes", "peso", "pestrý"]);
    assert!(words.contains(&"les"), "similar word expected: {words:?}");
    assert_eq!(hits.iter().find(|h| h.headword == "les").unwrap().kind, MatchKind::Similar);

    let e = dict.entry(hits[0].entry_id).unwrap();
    assert_eq!(e.senses[0].translations, vec!["dog"]);
}

#[test]
fn entries_across_blocks() {
    let (_d, dict) = build();
    for i in [0, 31, 32, 99] {
        let w = format!("slovo{i:03}");
        let e = dict.entries_for(&w);
        assert_eq!(e.len(), 1, "{w}");
        assert_eq!(e[0].senses[0].translations[0], format!("word {i}"));
    }
}
