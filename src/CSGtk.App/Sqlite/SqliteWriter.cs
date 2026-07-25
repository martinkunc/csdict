using System.Text.Json;
using CSGtk.App.Scraping;

namespace CSGtk.App.Sqlite;

/// <summary>C# port of dicts/common/sqlite_io.py's SqliteWriter: writes ScrapedEntry records into
/// the same entries/senses/translations/examples schema the Python scrapers produce, so the app's
/// existing read-side queries (DictionaryCatalog, LookupService, WordIndex) work unchanged against
/// dictionaries generated here. Each run starts from a fresh file, same as the Python writer.</summary>
internal sealed class SqliteWriter : IDisposable
{
    private const string Schema = """
        CREATE TABLE entries (
            id TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            source_license TEXT NOT NULL,
            source_url TEXT NOT NULL,
            source_authors TEXT NOT NULL,
            source_year TEXT NOT NULL,
            retrieved_at TEXT NOT NULL,
            lemma TEXT NOT NULL,
            lemma_lang TEXT NOT NULL,
            target_lang TEXT NOT NULL,
            pos TEXT,
            ipa TEXT,
            gender TEXT
        );
        CREATE INDEX idx_entries_lemma ON entries (lemma_lang, lemma COLLATE NOCASE);

        CREATE TABLE senses (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
            position INTEGER NOT NULL,
            gloss TEXT,
            tags TEXT NOT NULL DEFAULT '[]'
        );
        CREATE INDEX idx_senses_entry ON senses (entry_id);

        CREATE TABLE translations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            sense_id INTEGER NOT NULL REFERENCES senses(id) ON DELETE CASCADE,
            position INTEGER NOT NULL,
            text TEXT NOT NULL
        );
        CREATE INDEX idx_translations_sense ON translations (sense_id);

        CREATE TABLE examples (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            sense_id INTEGER NOT NULL REFERENCES senses(id) ON DELETE CASCADE,
            position INTEGER NOT NULL,
            source_text TEXT NOT NULL,
            target_text TEXT NOT NULL
        );
        CREATE INDEX idx_examples_sense ON examples (sense_id);
        """;

    private readonly nint _db;
    private readonly nint _insertEntry;
    private readonly nint _insertSense;
    private readonly nint _insertTranslation;
    private readonly nint _insertExample;
    private int _count;

    private SqliteWriter(nint db)
    {
        _db = db;
        Exec(Schema);
        _insertEntry = Prepare(
            "INSERT OR REPLACE INTO entries " +
            "(id, source, source_license, source_url, source_authors, source_year, " +
            " retrieved_at, lemma, lemma_lang, target_lang, pos, ipa, gender) " +
            "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)");
        _insertSense = Prepare("INSERT INTO senses (entry_id, position, gloss, tags) VALUES (?, ?, ?, ?)");
        _insertTranslation = Prepare("INSERT INTO translations (sense_id, position, text) VALUES (?, ?, ?)");
        _insertExample = Prepare("INSERT INTO examples (sense_id, position, source_text, target_text) VALUES (?, ?, ?, ?)");
        Exec("BEGIN");
    }

    public static SqliteWriter Create(string path)
    {
        if (File.Exists(path))
        {
            File.Delete(path);
        }

        Directory.CreateDirectory(Path.GetDirectoryName(path) ?? ".");

        int rc = Sqlite3.sqlite3_open_v2(path, out nint db, Sqlite3.SQLITE_OPEN_READWRITE | Sqlite3.SQLITE_OPEN_CREATE, 0);
        if (rc != Sqlite3.SQLITE_OK)
        {
            throw new InvalidOperationException($"sqlite3_open_v2 failed ({rc}) for {path}");
        }

        return new SqliteWriter(db);
    }

    public void WriteEntry(ScrapedEntry entry)
    {
        nint stmt = _insertEntry;
        Sqlite3.sqlite3_bind_text(stmt, 1, entry.Id, -1, Sqlite3.SQLITE_TRANSIENT);
        Sqlite3.sqlite3_bind_text(stmt, 2, entry.Source, -1, Sqlite3.SQLITE_TRANSIENT);
        Sqlite3.sqlite3_bind_text(stmt, 3, entry.SourceLicense, -1, Sqlite3.SQLITE_TRANSIENT);
        Sqlite3.sqlite3_bind_text(stmt, 4, entry.SourceUrl, -1, Sqlite3.SQLITE_TRANSIENT);
        Sqlite3.sqlite3_bind_text(stmt, 5, entry.SourceAuthors, -1, Sqlite3.SQLITE_TRANSIENT);
        Sqlite3.sqlite3_bind_text(stmt, 6, entry.SourceYear, -1, Sqlite3.SQLITE_TRANSIENT);
        Sqlite3.sqlite3_bind_text(stmt, 7, entry.RetrievedAt, -1, Sqlite3.SQLITE_TRANSIENT);
        Sqlite3.sqlite3_bind_text(stmt, 8, entry.Lemma, -1, Sqlite3.SQLITE_TRANSIENT);
        Sqlite3.sqlite3_bind_text(stmt, 9, entry.LemmaLang, -1, Sqlite3.SQLITE_TRANSIENT);
        Sqlite3.sqlite3_bind_text(stmt, 10, entry.TargetLang, -1, Sqlite3.SQLITE_TRANSIENT);
        BindNullableText(stmt, 11, entry.Pos);
        BindNullableText(stmt, 12, entry.Ipa);
        BindNullableText(stmt, 13, entry.Gender);
        StepAndReset(stmt);

        for (int sensePos = 0; sensePos < entry.Senses.Count; sensePos++)
        {
            ScrapedSense sense = entry.Senses[sensePos];
            nint senseStmt = _insertSense;
            Sqlite3.sqlite3_bind_text(senseStmt, 1, entry.Id, -1, Sqlite3.SQLITE_TRANSIENT);
            Sqlite3.sqlite3_bind_int64(senseStmt, 2, sensePos);
            BindNullableText(senseStmt, 3, sense.Gloss);
            Sqlite3.sqlite3_bind_text(senseStmt, 4, JsonSerializer.Serialize(sense.Tags), -1, Sqlite3.SQLITE_TRANSIENT);
            StepAndReset(senseStmt);
            long senseId = Sqlite3.sqlite3_last_insert_rowid(_db);

            for (int tPos = 0; tPos < sense.Translations.Count; tPos++)
            {
                nint tStmt = _insertTranslation;
                Sqlite3.sqlite3_bind_int64(tStmt, 1, senseId);
                Sqlite3.sqlite3_bind_int64(tStmt, 2, tPos);
                Sqlite3.sqlite3_bind_text(tStmt, 3, sense.Translations[tPos], -1, Sqlite3.SQLITE_TRANSIENT);
                StepAndReset(tStmt);
            }

            for (int ePos = 0; ePos < sense.Examples.Count; ePos++)
            {
                ScrapedExample example = sense.Examples[ePos];
                nint eStmt = _insertExample;
                Sqlite3.sqlite3_bind_int64(eStmt, 1, senseId);
                Sqlite3.sqlite3_bind_int64(eStmt, 2, ePos);
                Sqlite3.sqlite3_bind_text(eStmt, 3, example.SourceText, -1, Sqlite3.SQLITE_TRANSIENT);
                Sqlite3.sqlite3_bind_text(eStmt, 4, example.TargetText, -1, Sqlite3.SQLITE_TRANSIENT);
                StepAndReset(eStmt);
            }
        }

        _count++;
        if (_count % 5000 == 0)
        {
            Exec("COMMIT");
            Exec("BEGIN");
        }
    }

    public int Count => _count;

    private static void BindNullableText(nint stmt, int index, string? value)
    {
        if (value is null)
        {
            Sqlite3.sqlite3_bind_null(stmt, index);
        }
        else
        {
            Sqlite3.sqlite3_bind_text(stmt, index, value, -1, Sqlite3.SQLITE_TRANSIENT);
        }
    }

    private static void StepAndReset(nint stmt)
    {
        int rc = Sqlite3.sqlite3_step(stmt);
        if (rc != Sqlite3.SQLITE_DONE)
        {
            throw new InvalidOperationException($"sqlite3_step failed ({rc})");
        }

        Sqlite3.sqlite3_reset(stmt);
        Sqlite3.sqlite3_clear_bindings(stmt);
    }

    private nint Prepare(string sql)
    {
        int rc = Sqlite3.sqlite3_prepare_v2(_db, sql, -1, out nint stmt, 0);
        if (rc != Sqlite3.SQLITE_OK)
        {
            string message = System.Runtime.InteropServices.Marshal.PtrToStringUTF8(Sqlite3.sqlite3_errmsg(_db)) ?? "unknown error";
            throw new InvalidOperationException($"sqlite3_prepare_v2 failed ({rc}): {message}, for: {sql}");
        }

        return stmt;
    }

    private void Exec(string sql)
    {
        int rc = Sqlite3.sqlite3_exec(_db, sql, 0, 0, out nint errmsg);
        if (rc != Sqlite3.SQLITE_OK)
        {
            string message = errmsg != 0 ? System.Runtime.InteropServices.Marshal.PtrToStringUTF8(errmsg) ?? "unknown error" : "unknown error";
            Sqlite3.sqlite3_free(errmsg);
            throw new InvalidOperationException($"sqlite3_exec failed ({rc}): {message}");
        }
    }

    public void Dispose()
    {
        Exec("COMMIT");
        Sqlite3.sqlite3_finalize(_insertEntry);
        Sqlite3.sqlite3_finalize(_insertSense);
        Sqlite3.sqlite3_finalize(_insertTranslation);
        Sqlite3.sqlite3_finalize(_insertExample);
        Sqlite3.sqlite3_close_v2(_db);
    }
}
