namespace CSDict.App.Data;

/// <summary>One (lemmaLang, targetLang) dictionary direction the app knows how to download - the
/// fixed catalog shown in the "D" dialog, independent of whether its .sqlite3 file currently
/// exists under Dictionaries/. Unlike the old per-source DictionaryDefinition list, there is
/// exactly one row per direction: each merged file bundles every source that has one for that
/// direction (see CSDict.Scraper/DirectionCatalog.cs, which is the source of truth this list must
/// be kept in sync with - the app itself no longer contains any scraping/source knowledge, only
/// which directions exist to download).</summary>
internal sealed record DictionaryDirection(string LemmaLang, string TargetLang)
{
    public string FileName => $"{LemmaLang}_{TargetLang}.sqlite3";
    public string DisplayName => $"{LemmaLang} → {TargetLang}";
}

internal static class DictionaryDirections
{
    public static readonly IReadOnlyList<DictionaryDirection> All =
    [
        new("cs", "en"),
        new("en", "cs"),
        new("es", "en"),
        new("en", "es"),
        new("fr", "en"),
        new("en", "fr"),
        new("de", "en"),
        new("en", "de"),
        new("ru", "en"),
        new("en", "ru"),
        new("pt", "en"),
        new("en", "pt"),
        new("ja", "en"),
        new("en", "ja"),
        new("ar", "en"),
        new("en", "ar"),
        new("zh", "en"),
        new("en", "zh"),
        new("hi", "en"),
        new("en", "hi"),
        new("ko", "en"),
        new("en", "ko"),
    ];

    public static IReadOnlyList<string> LemmaLangs => All.Select(d => d.LemmaLang).Distinct().OrderBy(l => l, StringComparer.Ordinal).ToList();

    public static IReadOnlyList<string> TargetLangs => All.Select(d => d.TargetLang).Distinct().OrderBy(l => l, StringComparer.Ordinal).ToList();

    public static DictionaryDirection? Find(string lemmaLang, string targetLang) =>
        All.FirstOrDefault(d => d.LemmaLang == lemmaLang && d.TargetLang == targetLang);
}
