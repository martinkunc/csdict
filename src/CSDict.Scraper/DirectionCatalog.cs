namespace CSDict.Scraper;

/// <summary>Which sources contribute to each (lemmaLang, targetLang) direction this tool knows how
/// to build - the union of FreeDictScraper.Names, WikDictScraper.Pairs and Wiktionary's
/// KaikkiLanguageNames. Replaces the old CSDict.App.Scraping.DictionaryDownloader flat list: where
/// that listed one row per (source, direction) - 26 entries, several sharing a direction - this
/// groups them by direction, since the app now ships one merged sqlite file per direction.</summary>
internal static class DirectionCatalog
{
    public static readonly IReadOnlyList<Direction> All =
    [
        new("cs", "en", ["freedict", "wikdict", "wiktionary"]),
        new("en", "cs", ["freedict", "wikdict", "wiktionary"]),
        new("es", "en", ["freedict"]),
        new("en", "es", ["freedict"]),
        new("fr", "en", ["freedict"]),
        new("en", "fr", ["freedict"]),
        new("de", "en", ["freedict"]),
        new("en", "de", ["freedict"]),
        new("ru", "en", ["freedict"]),
        new("en", "ru", ["freedict"]),
        new("pt", "en", ["freedict"]),
        new("en", "pt", ["freedict"]),
        new("ja", "en", ["freedict"]),
        new("en", "ja", ["freedict"]),
        new("ar", "en", ["freedict"]),
        new("en", "ar", ["freedict"]),
        new("zh", "en", ["wikdict"]),
        new("en", "zh", ["wikdict"]),
        new("hi", "en", ["wiktionary"]),
        new("en", "hi", ["wiktionary"]),
        new("ko", "en", ["wiktionary"]),
        new("en", "ko", ["wiktionary"]),
    ];

    public static Direction? Find(string lemmaLang, string targetLang) =>
        All.FirstOrDefault(d => d.LemmaLang == lemmaLang && d.TargetLang == targetLang);
}

/// <summary>One (lemmaLang, targetLang) direction and every source that contributes entries to it -
/// scraping all of them in turn into a single shared SqliteWriter is what produces this app's one
/// merged "{lemmaLang}_{targetLang}.sqlite3" file per direction.</summary>
internal sealed record Direction(string LemmaLang, string TargetLang, IReadOnlyList<string> Sources)
{
    public string FileName => $"{LemmaLang}_{TargetLang}.sqlite3";
    public string DisplayName => $"{LemmaLang} -> {TargetLang}";
}
