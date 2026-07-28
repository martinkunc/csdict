namespace CSDict.App.Data;

/// <summary>Given a headword and the language it was found in, looks it up across every loaded
/// dictionary whose lemma_lang matches - which, since each direction file covers both a lemma
/// language and a target language, is exactly "search all dictionaries in both directions" for
/// that word. Each matching row carries its own `source` (a merged direction file can hold rows
/// from several sources), so filtering/grouping by source happens per-row, not per-file.</summary>
internal static class LookupService
{
    public static List<SourceResult> GetTranslations(DictionaryCatalog catalog, string lemma, string lang, string? sourceFilter)
    {
        // Keyed by (TargetLang, Source), not just Source - ForLang(lang) can return several
        // directions out of "lang" (e.g. en_cs and en_de), and the same source name (e.g.
        // "wiktionary") can show up in more than one of them, so grouping by Source alone would
        // silently merge translations into different target languages under one heading.
        var bySource = new Dictionary<(string TargetLang, string Source), List<EntryResult>>();

        foreach (var (meta, db) in catalog.ForLang(lang))
        {
            List<(string Id, string Source, string? Pos, string? Ipa, string? Gender)> entries = db.Query(
                "SELECT id, source, pos, ipa, gender FROM entries WHERE lemma_lang = ? AND lemma = ?",
                [lang, lemma],
                row => (row.GetString(0)!, row.GetString(1)!, row.GetString(2), row.GetString(3), row.GetString(4)));

            foreach (IGrouping<string, (string Id, string Source, string? Pos, string? Ipa, string? Gender)> group in entries
                .Where(e => sourceFilter is null || e.Source == sourceFilter)
                .GroupBy(e => e.Source))
            {
                var entryResults = new List<EntryResult>();
                foreach ((string id, _, string? pos, string? ipa, string? gender) in group)
                {
                    List<(string Id, string? Gloss)> senses = db.Query(
                        "SELECT id, gloss FROM senses WHERE entry_id = ? ORDER BY position",
                        [id],
                        row => (row.GetString(0)!, row.GetString(1)));

                    var senseResults = new List<SenseResult>();
                    foreach ((string senseId, string? gloss) in senses)
                    {
                        List<string> translations = db.Query(
                            "SELECT text FROM translations WHERE sense_id = ? ORDER BY position",
                            [senseId],
                            row => row.GetString(0)!);

                        List<(string, string)> examples = db.Query(
                            "SELECT source_text, target_text FROM examples WHERE sense_id = ? ORDER BY position",
                            [senseId],
                            row => (row.GetString(0) ?? "", row.GetString(1) ?? ""));

                        if (translations.Count == 0 && string.IsNullOrEmpty(gloss))
                        {
                            continue;
                        }

                        senseResults.Add(new SenseResult(gloss, translations, examples));
                    }

                    if (senseResults.Count > 0)
                    {
                        entryResults.Add(new EntryResult(pos, ipa, gender, senseResults));
                    }
                }

                if (entryResults.Count == 0)
                {
                    continue;
                }

                (string TargetLang, string Source) key = (meta.TargetLang, group.Key);
                if (!bySource.TryGetValue(key, out List<EntryResult>? list))
                {
                    list = [];
                    bySource[key] = list;
                }

                list.AddRange(entryResults);
            }
        }

        return bySource
            .OrderBy(kv => kv.Key.TargetLang, StringComparer.OrdinalIgnoreCase)
            .ThenBy(kv => kv.Key.Source, StringComparer.OrdinalIgnoreCase)
            .Select(kv => new SourceResult(kv.Key.Source, kv.Key.TargetLang, kv.Value))
            .ToList();
    }
}
