using System.IO.Compression;
using System.Text.RegularExpressions;
using CSGtk.App.Sqlite;

namespace CSGtk.App.Scraping;

/// <summary>C# port of dicts/scrapers/wikdict.py: downloads WikDict's StarDict export
/// (download.wikdict.com/dictionaries/stardict/wikdict-{cs-en,en-cs}.zip), flattens each HTML
/// entry via StarDict.ExtractPosIpaTranslationsGloss, and writes straight into a SqliteWriter.</summary>
internal static class WikDictScraper
{
    private const string Source = "wikdict";
    private const string BaseUrl = "https://download.wikdict.com/dictionaries/stardict/";

    private static readonly Dictionary<(string LemmaLang, string TargetLang), string> Pairs = new()
    {
        [("cs", "en")] = "cs-en",
        [("en", "cs")] = "en-cs",
        [("zh", "en")] = "zh-en",
        [("en", "zh")] = "en-zh",
    };

    private static readonly Regex LicenseUrlRegex = new(
        "href=\"([^\"]*creativecommons\\.org[^\"]*)\"", RegexOptions.Compiled);

    public static async Task ScrapeAsync(
        string lemmaLang, string targetLang, string cacheDir, string outputPath,
        IProgress<string>? progress, CancellationToken ct)
    {
        string pair = Pairs[(lemmaLang, targetLang)];
        Directory.CreateDirectory(cacheDir);

        string zipPath = Path.Combine(cacheDir, $"wikdict-{pair}.zip");
        if (!File.Exists(zipPath))
        {
            progress?.Report($"Downloading wikdict-{pair}.zip...");
            await Downloader.DownloadAsync($"{BaseUrl}wikdict-{pair}.zip", zipPath, progress, ct);
        }

        string extractDir = Path.Combine(cacheDir, $"wikdict-{pair}-extracted");
        if (!Directory.Exists(extractDir))
        {
            progress?.Report("Extracting archive...");
            ZipFile.ExtractToDirectory(zipPath, extractDir);
        }

        string ifoPath = Directory.EnumerateFiles(extractDir, "*.ifo", SearchOption.AllDirectories).First();
        string withoutExt = Path.Combine(Path.GetDirectoryName(ifoPath)!, Path.GetFileNameWithoutExtension(ifoPath));
        string dictPath = $"{withoutExt}.dict";
        if (!File.Exists(dictPath))
        {
            dictPath = $"{withoutExt}.dict.dz";
        }

        Dictionary<string, string> ifo = StarDict.ReadIfo(ifoPath);
        (string license, string authors, string year) = ParseIfoMeta(ifo);
        progress?.Report($"wordcount={ifo.GetValueOrDefault("wordcount")} year={year} license={license}");

        byte[] dictBlob = StarDict.ReadDictBlob(dictPath);
        List<StarDict.IdxRecord> records = StarDict.ReadIdx($"{withoutExt}.idx");

        using SqliteWriter writer = SqliteWriter.Create(outputPath);
        int count = 0;
        foreach (StarDict.IdxRecord record in records)
        {
            ct.ThrowIfCancellationRequested();
            string html = System.Text.Encoding.UTF8.GetString(dictBlob, (int)record.Offset, (int)record.Size);
            StarDict.Node root = StarDict.ParseHtmlFragment(html);
            (string? pos, string? ipa, List<string> translations, string? gloss) = StarDict.ExtractPosIpaTranslationsGloss(root);
            if (translations.Count == 0)
            {
                continue;
            }

            var entry = new ScrapedEntry
            {
                Id = ScrapedEntryId.Make(Source, record.Word, lemmaLang, targetLang, pos ?? record.Offset.ToString()),
                Source = Source,
                SourceLicense = license,
                SourceUrl = $"{BaseUrl}wikdict-{pair}.zip",
                SourceAuthors = authors,
                SourceYear = year,
                RetrievedAt = ScrapedEntryId.Today(),
                Lemma = record.Word,
                LemmaLang = lemmaLang,
                TargetLang = targetLang,
                Pos = pos,
                Ipa = ipa,
                Senses = [new ScrapedSense { Translations = translations, Gloss = gloss }],
            };
            writer.WriteEntry(entry);
            count++;
            if (count % 5000 == 0)
            {
                progress?.Report($"Wrote {count} entries so far...");
            }
        }

        progress?.Report($"Wrote {count} entries.");
    }

    private static (string License, string Authors, string Year) ParseIfoMeta(Dictionary<string, string> ifo)
    {
        string description = ifo.GetValueOrDefault("description", "");
        Match match = LicenseUrlRegex.Match(description);
        string license = match.Success ? $"CC BY-SA 4.0 ({match.Groups[1].Value})" : "CC BY-SA (see WikDict description)";

        string date = ifo.GetValueOrDefault("date", "");
        string year = date.Length >= 4 && int.TryParse(date[..4], out _) ? date[..4] : "unknown";

        const string authors = "Karl Bartel (WikDict); Wiktionary contributors (via DBnary); FreeDict project contributors";
        return (license, authors, year);
    }
}
