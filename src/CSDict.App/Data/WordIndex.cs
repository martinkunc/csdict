namespace CSDict.App.Data;

/// <summary>The "two-way" index: one trie of every headword per language actually present across
/// loaded dictionaries. Typing a word in any indexed language walks that language's trie - every
/// lemma_lang found in the catalog gets a trie, not just Czech and English, so newly downloaded
/// dictionaries (e.g. Spanish, French) are searchable the same way cs/en already are.</summary>
internal sealed class WordIndex
{
    private readonly Dictionary<string, Trie> _tries = new();

    public IReadOnlyCollection<string> Languages => _tries.Keys;

    public Trie? ForLang(string lang) => _tries.GetValueOrDefault(lang);

    public static WordIndex Build(DictionaryCatalog catalog)
    {
        var index = new WordIndex();
        foreach (string lang in catalog.LemmaLangs)
        {
            var trie = new Trie();
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

            index._tries[lang] = trie;
        }

        return index;
    }
}
