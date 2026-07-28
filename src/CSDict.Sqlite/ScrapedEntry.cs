using System.Security.Cryptography;
using System.Text;

namespace CSDict.Sqlite;

/// <summary>C# port of dicts/common/schema.py's DictionaryEntry - kept field-for-field identical
/// so a ScrapedEntry can be written straight into the same entries/senses/translations/examples
/// SQLite schema the scrapers produce (see SqliteWriter). Lives here (rather than in
/// CSDict.Scraper, which is the only project that constructs these) since SqliteWriter - the
/// thing that consumes them - is the shared read/write core used by both CSDict.Scraper and
/// CSDict.App.</summary>
public sealed class ScrapedEntry
{
    public required string Id { get; init; }
    public required string Source { get; init; }
    public required string SourceLicense { get; init; }
    public required string SourceUrl { get; init; }
    public required string SourceAuthors { get; init; }
    public required string SourceYear { get; init; }
    public required string RetrievedAt { get; init; }
    public required string Lemma { get; init; }
    public required string LemmaLang { get; init; }
    public required string TargetLang { get; init; }
    public string? Pos { get; init; }
    public string? Ipa { get; init; }
    public string? Gender { get; init; }
    public List<ScrapedSense> Senses { get; init; } = [];
}

public sealed class ScrapedSense
{
    public List<string> Translations { get; init; } = [];
    public string? Gloss { get; init; }
    public List<ScrapedExample> Examples { get; init; } = [];
    public List<string> Tags { get; init; } = [];
}

public sealed record ScrapedExample(string SourceText, string TargetText);

public static class ScrapedEntryId
{
    /// <summary>Mirrors schema.py:make_id - stable hash-based id so re-running a scrape doesn't
    /// churn ids: sha1("{source}|{lemmaLang}|{targetLang}|{lemma}|{disambiguator}")[:16].</summary>
    public static string Make(string source, string lemma, string lemmaLang, string targetLang, string disambiguator = "")
    {
        string key = $"{source}|{lemmaLang}|{targetLang}|{lemma}|{disambiguator}";
        byte[] hash = SHA1.HashData(Encoding.UTF8.GetBytes(key));
        string hex = Convert.ToHexStringLower(hash)[..16];
        return $"{source}-{hex}";
    }

    public static string Today() => DateTime.UtcNow.ToString("yyyy-MM-dd");
}
