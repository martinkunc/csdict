using CSDict.Sqlite;

namespace CSDict.App.Data;

/// <summary>Discovers and opens every *.sqlite3 dictionary snapshot in a directory at startup.
/// Each file is self-describing (lemma_lang/target_lang read from its own `entries` table, and its
/// set of distinct `source` values via a separate query - a merged direction file can hold rows
/// from several sources), so there's no filename convention to keep in sync with the scraper that
/// produced them.</summary>
internal sealed class DictionaryCatalog : IDisposable
{
    private readonly List<(DictionaryFile Meta, SqliteDatabase Db)> _entries = [];

    public IReadOnlyList<string> Sources { get; private set; } = [];
    public IReadOnlyList<string> LemmaLangs { get; private set; } = [];

    public static DictionaryCatalog Load(string directory)
    {
        var catalog = new DictionaryCatalog();
        if (!Directory.Exists(directory))
        {
            return catalog;
        }

        foreach (string path in Directory.EnumerateFiles(directory, "*.sqlite3").OrderBy(p => p, StringComparer.Ordinal))
        {
            SqliteDatabase? db = SqliteDatabase.TryOpen(path);
            if (db is null)
            {
                continue;
            }

            List<(string LemmaLang, string TargetLang)> direction = db.Query(
                "SELECT lemma_lang, target_lang FROM entries LIMIT 1",
                null,
                row => (row.GetString(0)!, row.GetString(1)!));

            if (direction.Count == 0)
            {
                db.Dispose();
                continue;
            }

            List<string> sources = db.Query("SELECT DISTINCT source FROM entries", null, row => row.GetString(0)!);
            catalog._entries.Add((new DictionaryFile(direction[0].LemmaLang, direction[0].TargetLang, path, sources), db));
        }

        catalog.Sources = catalog._entries.SelectMany(e => e.Meta.Sources).Distinct().OrderBy(s => s, StringComparer.OrdinalIgnoreCase).ToList();
        catalog.LemmaLangs = catalog._entries.Select(e => e.Meta.LemmaLang).Distinct().OrderBy(s => s, StringComparer.OrdinalIgnoreCase).ToList();
        return catalog;
    }

    public IEnumerable<(DictionaryFile Meta, SqliteDatabase Db)> ForLang(string lemmaLang) =>
        _entries.Where(e => e.Meta.LemmaLang == lemmaLang);

    public void Dispose()
    {
        foreach ((_, SqliteDatabase db) in _entries)
        {
            db.Dispose();
        }
    }
}
