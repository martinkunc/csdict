using CSGtk.App.Sqlite;

namespace CSGtk.App.Data;

/// <summary>Discovers and opens every *.sqlite3 dictionary snapshot in a directory at startup.
/// Each file is self-describing (source/lemma_lang/target_lang read from its own `entries` table),
/// so there's no filename convention to keep in sync with the scrapers that produced them.</summary>
internal sealed class DictionaryCatalog : IDisposable
{
    private readonly List<(DictionarySource Meta, SqliteDatabase Db)> _entries = [];

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

            List<DictionarySource> meta = db.Query(
                "SELECT source, lemma_lang, target_lang FROM entries LIMIT 1",
                null,
                row => new DictionarySource(row.GetString(0)!, row.GetString(1)!, row.GetString(2)!, path));

            if (meta.Count == 0)
            {
                db.Dispose();
                continue;
            }

            catalog._entries.Add((meta[0], db));
        }

        catalog.Sources = catalog._entries.Select(e => e.Meta.Source).Distinct().OrderBy(s => s, StringComparer.OrdinalIgnoreCase).ToList();
        catalog.LemmaLangs = catalog._entries.Select(e => e.Meta.LemmaLang).Distinct().OrderBy(s => s, StringComparer.OrdinalIgnoreCase).ToList();
        return catalog;
    }

    public IEnumerable<(DictionarySource Meta, SqliteDatabase Db)> ForLang(string lemmaLang) =>
        _entries.Where(e => e.Meta.LemmaLang == lemmaLang);

    public void Dispose()
    {
        foreach ((_, SqliteDatabase db) in _entries)
        {
            db.Dispose();
        }
    }
}
