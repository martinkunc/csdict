namespace CSGtk.App.Data;

/// <summary>The "two-way" index: one trie of every Czech headword across all loaded dictionaries,
/// one of every English headword. Typing a Czech word walks Cs, typing an English word walks En -
/// both are searched on every keystroke so lookup works regardless of which language was typed.</summary>
internal sealed class WordIndex
{
    public Trie Cs { get; } = new();
    public Trie En { get; } = new();

    public static WordIndex Build(DictionaryCatalog catalog)
    {
        var index = new WordIndex();
        BuildTrie(catalog, "cs", index.Cs);
        BuildTrie(catalog, "en", index.En);
        return index;
    }

    private static void BuildTrie(DictionaryCatalog catalog, string lang, Trie trie)
    {
        foreach (var (_, db) in catalog.ForLang(lang))
        {
            List<string> lemmas = db.Query(
                "SELECT DISTINCT lemma FROM entries WHERE lemma_lang = ?",
                [lang],
                row => row.GetString(0)!);

            foreach (string lemma in lemmas)
            {
                trie.Insert(lemma);
            }
        }
    }
}
