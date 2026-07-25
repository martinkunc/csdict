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
        new("wikdict", "cs", "en", "wikdict_cs_en.sqlite3"),
        new("wikdict", "en", "cs", "wikdict_en_cs.sqlite3"),
        new("wiktionary", "cs", "en", "wiktionary_cs_en.sqlite3"),
        new("wiktionary", "en", "cs", "wiktionary_en_cs.sqlite3"),
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
                "wiktionary" when definition.LemmaLang == "cs" => WiktionaryScraper.ScrapeCsToEnAsync(cacheDir, tempPath, progress, ct),
                "wiktionary" => WiktionaryScraper.ScrapeEnToCsAsync(cacheDir, tempPath, progress, ct),
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
