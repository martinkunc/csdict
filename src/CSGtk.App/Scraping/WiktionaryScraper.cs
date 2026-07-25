using System.IO.Compression;
using System.Text;
using System.Text.Json;
using CSGtk.App.Sqlite;

namespace CSGtk.App.Scraping;

/// <summary>C# port of dicts/scrapers/wiktionary_kaikki.py: uses two kaikki.org files - the small
/// pre-filtered Czech-only extract for cs-&gt;en (glosses already in English), and a streamed pass
/// over the full raw Wiktextract dump for en-&gt;cs (filtering on translations[].lang_code=="cs").
/// The two directions are scraped independently here (unlike the Python script's combined `run`),
/// since the app's Download button downloads one direction at a time and the en-&gt;cs direction
/// alone requires ~2.8GB.</summary>
internal static class WiktionaryScraper
{
    private const string Source = "wiktionary";
    private const string License = "CC BY-SA 4.0 / GFDL (Wiktionary); attribution to Wiktionary contributors and Wiktextract required";
    private const string Authors = "Wiktionary contributors (English Wiktionary & Czech Wiktionary); "
        + "extraction by Tatu Ylonen / Wiktextract (kaikki.org). Cite: Ylonen, "
        + "\"Wiktextract: Wiktionary as Machine-Readable Structured Data\", LREC 2022.";

    private const string CsUrl = "https://kaikki.org/dictionary/Czech/kaikki.org-dictionary-Czech.jsonl.gz";
    private const string CsSourceUrl = "https://kaikki.org/dictionary/Czech/";
    private const string EnRawUrl = "https://kaikki.org/dictionary/raw-wiktextract-data.jsonl.gz";
    private const string EnRawSourceUrl = "https://kaikki.org/dictionary/rawdata.html";

    private static readonly HashSet<string> GenderTags = ["masculine", "feminine", "neuter"];

    public static async Task ScrapeCsToEnAsync(string cacheDir, string outputPath, IProgress<string>? progress, CancellationToken ct)
    {
        Directory.CreateDirectory(cacheDir);
        string csInput = Path.Combine(cacheDir, "kaikki-cs.jsonl.gz");

        progress?.Report("Checking Czech Wiktionary extract date...");
        string year = await Downloader.FetchLastModifiedYearAsync(CsUrl, ct);

        if (!File.Exists(csInput))
        {
            progress?.Report("Downloading Czech Wiktionary extract (~19MB)...");
            await Downloader.DownloadAsync(CsUrl, csInput, progress, ct);
        }

        using SqliteWriter writer = SqliteWriter.Create(outputPath);
        int count = 0;
        foreach (JsonDocument doc in IterateJsonl(csInput))
        {
            using (doc)
            {
                ct.ThrowIfCancellationRequested();
                ScrapedEntry? entry = CsEntryToScrapedEntry(doc.RootElement, year);
                if (entry is null)
                {
                    continue;
                }

                writer.WriteEntry(entry);
                count++;
                if (count % 5000 == 0)
                {
                    progress?.Report($"Wrote {count} entries so far...");
                }
            }
        }

        progress?.Report($"Wrote {count} cs->en entries.");
    }

    public static async Task ScrapeEnToCsAsync(string cacheDir, string outputPath, IProgress<string>? progress, CancellationToken ct)
    {
        Directory.CreateDirectory(cacheDir);
        string enInput = Path.Combine(cacheDir, "kaikki-raw-en.jsonl.gz");

        progress?.Report("Checking raw Wiktextract dump date...");
        string year = await Downloader.FetchLastModifiedYearAsync(EnRawUrl, ct);

        if (!File.Exists(enInput))
        {
            progress?.Report("Downloading raw Wiktextract data (~2.8GB, this will take a while)...");
            await Downloader.DownloadAsync(EnRawUrl, enInput, progress, ct);
        }

        using SqliteWriter writer = SqliteWriter.Create(outputPath);
        int count = 0;
        long scanned = 0;
        foreach (JsonDocument doc in IterateJsonl(enInput))
        {
            using (doc)
            {
                ct.ThrowIfCancellationRequested();
                scanned++;
                ScrapedEntry? entry = EnEntryToScrapedEntry(doc.RootElement, year);
                if (entry is not null)
                {
                    writer.WriteEntry(entry);
                    count++;
                }

                if (scanned % 500_000 == 0)
                {
                    progress?.Report($"...scanned {scanned:N0} lines, {count:N0} en->cs entries so far");
                }
            }
        }

        progress?.Report($"Wrote {count} en->cs entries.");
    }

    private static IEnumerable<JsonDocument> IterateJsonl(string path)
    {
        using FileStream fileStream = File.OpenRead(path);
        using Stream decompressed = path.EndsWith(".gz", StringComparison.OrdinalIgnoreCase)
            ? new GZipStream(fileStream, CompressionMode.Decompress)
            : fileStream;
        using var reader = new StreamReader(decompressed, Encoding.UTF8);

        string? line;
        while ((line = reader.ReadLine()) != null)
        {
            string trimmed = line.Trim();
            if (trimmed.Length == 0)
            {
                continue;
            }

            JsonDocument? doc;
            try
            {
                doc = JsonDocument.Parse(trimmed);
            }
            catch (JsonException)
            {
                continue;
            }

            yield return doc;
        }
    }

    /// <summary>Czech headword, English glosses -> cs -> en entry.</summary>
    private static ScrapedEntry? CsEntryToScrapedEntry(JsonElement obj, string year)
    {
        string? lemma = GetString(obj, "word");
        if (string.IsNullOrEmpty(lemma))
        {
            return null;
        }

        var senses = new List<ScrapedSense>();
        foreach (JsonElement sense in GetArray(obj, "senses"))
        {
            List<string> glosses = GetArray(sense, "glosses").Select(g => g.GetString() ?? "").Where(s => s.Length > 0).ToList();
            if (glosses.Count == 0)
            {
                glosses = GetArray(sense, "raw_glosses").Select(g => g.GetString() ?? "").Where(s => s.Length > 0).ToList();
            }

            if (glosses.Count == 0)
            {
                continue;
            }

            var examples = new List<ScrapedExample>();
            foreach (JsonElement ex in GetArray(sense, "examples"))
            {
                string? text = GetString(ex, "text");
                if (string.IsNullOrEmpty(text))
                {
                    continue;
                }

                examples.Add(new ScrapedExample(text, GetString(ex, "english") ?? ""));
            }

            senses.Add(new ScrapedSense
            {
                Gloss = string.Join("; ", glosses),
                Examples = examples,
                Tags = GetArray(sense, "tags").Select(t => t.GetString() ?? "").Where(s => s.Length > 0).ToList(),
            });
        }

        if (senses.Count == 0)
        {
            return null;
        }

        string? pos = GetString(obj, "pos");
        return new ScrapedEntry
        {
            Id = ScrapedEntryId.Make(Source, lemma, "cs", "en", pos ?? ""),
            Source = Source,
            SourceLicense = License,
            SourceUrl = CsSourceUrl,
            SourceAuthors = Authors,
            SourceYear = year,
            RetrievedAt = ScrapedEntryId.Today(),
            Lemma = lemma,
            LemmaLang = "cs",
            TargetLang = "en",
            Pos = pos,
            Ipa = FirstIpa(obj),
            Gender = FirstGender(obj),
            Senses = senses,
        };
    }

    /// <summary>English headword with &gt;=1 Czech translation -> en -> cs entry.</summary>
    private static ScrapedEntry? EnEntryToScrapedEntry(JsonElement obj, string year)
    {
        if (GetString(obj, "lang_code") != "en")
        {
            return null;
        }

        string? lemma = GetString(obj, "word");
        if (string.IsNullOrEmpty(lemma))
        {
            return null;
        }

        List<JsonElement> csTranslations = GetArray(obj, "translations")
            .Where(t => GetString(t, "lang_code") is "cs" or "ces" || GetString(t, "lang") == "Czech")
            .ToList();
        if (csTranslations.Count == 0)
        {
            return null;
        }

        var groups = new Dictionary<string, List<JsonElement>>();
        var groupOrder = new List<string>();
        foreach (JsonElement t in csTranslations)
        {
            string key = GetString(t, "sense") ?? "";
            if (!groups.TryGetValue(key, out List<JsonElement>? list))
            {
                list = [];
                groups[key] = list;
                groupOrder.Add(key);
            }

            list.Add(t);
        }

        var senses = new List<ScrapedSense>();
        foreach (string senseText in groupOrder)
        {
            List<JsonElement> group = groups[senseText];
            List<string> translations = group.Select(t => GetString(t, "word")).Where(w => !string.IsNullOrEmpty(w)).Select(w => w!).ToList();
            if (translations.Count == 0)
            {
                continue;
            }

            var tags = new SortedSet<string>(StringComparer.Ordinal);
            foreach (JsonElement t in group)
            {
                foreach (string tag in GetArray(t, "tags").Select(x => x.GetString() ?? "").Where(s => s.Length > 0))
                {
                    tags.Add(tag);
                }
            }

            senses.Add(new ScrapedSense
            {
                Translations = translations,
                Gloss = senseText.Length > 0 ? senseText : null,
                Tags = tags.ToList(),
            });
        }

        if (senses.Count == 0)
        {
            return null;
        }

        string? pos = GetString(obj, "pos");
        return new ScrapedEntry
        {
            Id = ScrapedEntryId.Make(Source, lemma, "en", "cs", pos ?? ""),
            Source = Source,
            SourceLicense = License,
            SourceUrl = EnRawSourceUrl,
            SourceAuthors = Authors,
            SourceYear = year,
            RetrievedAt = ScrapedEntryId.Today(),
            Lemma = lemma,
            LemmaLang = "en",
            TargetLang = "cs",
            Pos = pos,
            Ipa = FirstIpa(obj),
            Senses = senses,
        };
    }

    private static string? FirstIpa(JsonElement obj)
    {
        foreach (JsonElement sound in GetArray(obj, "sounds"))
        {
            string? ipa = GetString(sound, "ipa");
            if (!string.IsNullOrEmpty(ipa))
            {
                return ipa;
            }
        }

        return null;
    }

    private static string? FirstGender(JsonElement obj)
    {
        foreach (JsonElement sense in GetArray(obj, "senses"))
        {
            List<string> hit = GetArray(sense, "tags")
                .Select(t => t.GetString() ?? "")
                .Where(GenderTags.Contains)
                .OrderBy(s => s, StringComparer.Ordinal)
                .ToList();
            if (hit.Count > 0)
            {
                return hit[0];
            }
        }

        return null;
    }

    private static string? GetString(JsonElement obj, string property) =>
        obj.ValueKind == JsonValueKind.Object && obj.TryGetProperty(property, out JsonElement v) && v.ValueKind == JsonValueKind.String
            ? v.GetString()
            : null;

    private static IEnumerable<JsonElement> GetArray(JsonElement obj, string property)
    {
        if (obj.ValueKind == JsonValueKind.Object && obj.TryGetProperty(property, out JsonElement v) && v.ValueKind == JsonValueKind.Array)
        {
            foreach (JsonElement item in v.EnumerateArray())
            {
                yield return item;
            }
        }
    }
}
