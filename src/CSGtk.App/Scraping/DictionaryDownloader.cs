namespace CSGtk.App.Scraping;

/// <summary>One (source, direction) dictionary the app knows how to download, independent of
/// whether its .sqlite3 file currently exists under Dictionaries/. This is the fixed catalog shown
/// in the "D" dialog - unlike DictionaryCatalog (Data/DictionaryCatalog.cs), which only reflects
/// what's already been downloaded.</summary>
internal sealed record DictionaryDefinition(string Source, string LemmaLang, string TargetLang, string FileName)
{
    public string DisplayName => $"{Source} ({LemmaLang} → {TargetLang})";
}

internal static class DictionaryDownloader
{
    public static readonly IReadOnlyList<DictionaryDefinition> All =
    [
        new("freedict", "cs", "en", "freedict_cs_en.sqlite3"),
        new("freedict", "en", "cs", "freedict_en_cs.sqlite3"),
        new("freedict", "es", "en", "freedict_es_en.sqlite3"),
        new("freedict", "en", "es", "freedict_en_es.sqlite3"),
        new("freedict", "fr", "en", "freedict_fr_en.sqlite3"),
        new("freedict", "en", "fr", "freedict_en_fr.sqlite3"),
        new("freedict", "de", "en", "freedict_de_en.sqlite3"),
        new("freedict", "en", "de", "freedict_en_de.sqlite3"),
        new("freedict", "ru", "en", "freedict_ru_en.sqlite3"),
        new("freedict", "en", "ru", "freedict_en_ru.sqlite3"),
        new("freedict", "pt", "en", "freedict_pt_en.sqlite3"),
        new("freedict", "en", "pt", "freedict_en_pt.sqlite3"),
        new("freedict", "ja", "en", "freedict_ja_en.sqlite3"),
        new("freedict", "en", "ja", "freedict_en_ja.sqlite3"),
        new("freedict", "ar", "en", "freedict_ar_en.sqlite3"),
        new("freedict", "en", "ar", "freedict_en_ar.sqlite3"),
        new("wikdict", "cs", "en", "wikdict_cs_en.sqlite3"),
        new("wikdict", "en", "cs", "wikdict_en_cs.sqlite3"),
        new("wikdict", "zh", "en", "wikdict_zh_en.sqlite3"),
        new("wikdict", "en", "zh", "wikdict_en_zh.sqlite3"),
        new("wiktionary", "cs", "en", "wiktionary_cs_en.sqlite3"),
        new("wiktionary", "en", "cs", "wiktionary_en_cs.sqlite3"),
        new("wiktionary", "hi", "en", "wiktionary_hi_en.sqlite3"),
        new("wiktionary", "en", "hi", "wiktionary_en_hi.sqlite3"),
        new("wiktionary", "ko", "en", "wiktionary_ko_en.sqlite3"),
        new("wiktionary", "en", "ko", "wiktionary_en_ko.sqlite3"),
    ];

    /// <summary>Scrapes into a temporary file next to `outputPath` and only renames it into place on
    /// success, so a failed/cancelled run never leaves a half-written file that would otherwise look
    /// "downloaded" to the rest of the app.</summary>
    public static async Task DownloadAsync(
        DictionaryDefinition definition, string outputPath, string cacheRootDir,
        IProgress<string>? progress, CancellationToken ct)
    {
        string cacheDir = Path.Combine(cacheRootDir, definition.Source);
        string tempPath = outputPath + ".tmp";
        try
        {
            Task scrape = definition.Source switch
            {
                "freedict" => FreeDictScraper.ScrapeAsync(definition.LemmaLang, definition.TargetLang, cacheDir, tempPath, progress, ct),
                "wikdict" => WikDictScraper.ScrapeAsync(definition.LemmaLang, definition.TargetLang, cacheDir, tempPath, progress, ct),
                "wiktionary" when definition.LemmaLang == "en" => WiktionaryScraper.ScrapeFromEnAsync(definition.TargetLang, cacheDir, tempPath, progress, ct),
                "wiktionary" => WiktionaryScraper.ScrapeToEnAsync(definition.LemmaLang, cacheDir, tempPath, progress, ct),
                _ => throw new NotSupportedException($"no scraper for source '{definition.Source}'"),
            };
            await scrape;
            File.Move(tempPath, outputPath, overwrite: true);
        }
        finally
        {
            if (File.Exists(tempPath))
            {
                File.Delete(tempPath);
            }
        }
    }
}
