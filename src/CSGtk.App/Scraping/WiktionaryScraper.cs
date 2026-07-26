using System.IO.Compression;
using System.Text;
using System.Text.Json;
using CSGtk.App.Sqlite;

namespace CSGtk.App.Scraping;

/// <summary>C# port of dicts/scrapers/wiktionary_kaikki.py: uses two kinds of kaikki.org files - a
/// small pre-filtered per-language extract (that language's headwords, with English glosses, since
/// it's derived from the English Wiktionary's entries *about* that language) for X-&gt;en, and a
/// streamed pass over the full raw Wiktextract dump for en-&gt;X (filtering on
/// translations[].lang_code==X). Both directions are generalized over `lemmaLang`/`targetLang`
/// rather than hardcoded to Czech, so any language kaikki.org has an extract for can be added just
/// by extending KaikkiLanguageNames below - the app's Download button still downloads one direction
/// at a time, since the en-&gt;X direction alone requires ~2.8GB regardless of X.</summary>
internal static class WiktionaryScraper
{
    private const string Source = "wiktionary";
    private const string License = "CC BY-SA 4.0 / GFDL (Wiktionary); attribution to Wiktionary contributors and Wiktextract required";
    private const string Authors = "Wiktionary contributors (English Wiktionary and contributors in the source language's own Wiktionary); "
        + "extraction by Tatu Ylonen / Wiktextract (kaikki.org). Cite: Ylonen, "
        + "\"Wiktextract: Wiktionary as Machine-Readable Structured Data\", LREC 2022.";

    private const string EnRawUrl = "https://kaikki.org/dictionary/raw-wiktextract-data.jsonl.gz";
    private const string EnRawSourceUrl = "https://kaikki.org/dictionary/rawdata.html";

    /// <summary>kaikki.org's per-language extract folder name for each lemma language this app
    /// knows how to scrape X-&gt;en from - e.g. "cs" -&gt; kaikki.org/dictionary/Czech/.</summary>
    private static readonly Dictionary<string, string> KaikkiLanguageNames = new()
    {
        ["cs"] = "Czech",
        ["hi"] = "Hindi",
        ["ko"] = "Korean",
    };

    /// <summary>Occasional legacy ISO 639-3 lang_code seen in the raw dump alongside the usual
    /// 639-1 code, per language, where known.</summary>
    private static readonly Dictionary<string, string> AltLangCodes = new()
    {
        ["cs"] = "ces",
    };

    private static readonly HashSet<string> GenderTags = ["masculine", "feminine", "neuter"];

    /// <summary>X-&gt;en, where X is `lemmaLang` (must be a key of KaikkiLanguageNames): headword in
    /// X, glosses already in English because the extract is the English Wiktionary's own entries
    /// for X-language words.</summary>
    public static async Task ScrapeToEnAsync(string lemmaLang, string cacheDir, string outputPath, IProgress<string>? progress, CancellationToken ct)
    {
        string languageName = KaikkiLanguageNames[lemmaLang];
        string url = $"https://kaikki.org/dictionary/{languageName}/kaikki.org-dictionary-{languageName}.jsonl.gz";
        string sourceUrl = $"https://kaikki.org/dictionary/{languageName}/";

        Directory.CreateDirectory(cacheDir);
        string input = Path.Combine(cacheDir, $"kaikki-{lemmaLang}.jsonl.gz");

        progress?.Report($"Checking {languageName} Wiktionary extract date...");
        string year = await Downloader.FetchLastModifiedYearAsync(url, ct);

        if (!File.Exists(input))
        {
            progress?.Report($"Downloading {languageName} Wiktionary extract...");
            await Downloader.DownloadAsync(url, input, progress, ct);
        }

        using SqliteWriter writer = SqliteWriter.Create(outputPath);
        int count = 0;
        foreach (JsonDocument doc in IterateJsonl(input))
        {
            using (doc)
            {
                ct.ThrowIfCancellationRequested();
                ScrapedEntry? entry = ToEnglishEntryToScrapedEntry(doc.RootElement, year, lemmaLang, sourceUrl);
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

        progress?.Report($"Wrote {count} {lemmaLang}->en entries.");
    }

    /// <summary>en-&gt;X, where X is `targetLang`: streams the full raw Wiktextract dump (every
    /// English headword, with translations into every language Wiktionary covers) and keeps only
    /// entries with at least one translation whose language code matches `targetLang`.</summary>
    public static async Task ScrapeFromEnAsync(string targetLang, string cacheDir, string outputPath, IProgress<string>? progress, CancellationToken ct)
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
                ScrapedEntry? entry = EnEntryToScrapedEntry(doc.RootElement, year, targetLang);
                if (entry is not null)
                {
                    writer.WriteEntry(entry);
                    count++;
                }

                if (scanned % 500_000 == 0)
                {
                    progress?.Report($"...scanned {scanned:N0} lines, {count:N0} en->{targetLang} entries so far");
                }
            }
        }

        progress?.Report($"Wrote {count} en->{targetLang} entries.");
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

    /// <summary>X headword, English glosses -> X -> en entry.</summary>
    private static ScrapedEntry? ToEnglishEntryToScrapedEntry(JsonElement obj, string year, string lemmaLang, string sourceUrl)
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
            Id = ScrapedEntryId.Make(Source, lemma, lemmaLang, "en", pos ?? ""),
            Source = Source,
            SourceLicense = License,
            SourceUrl = sourceUrl,
            SourceAuthors = Authors,
            SourceYear = year,
            RetrievedAt = ScrapedEntryId.Today(),
            Lemma = lemma,
            LemmaLang = lemmaLang,
            TargetLang = "en",
            Pos = pos,
            Ipa = FirstIpa(obj),
            Gender = FirstGender(obj),
            Senses = senses,
        };
    }

    /// <summary>English headword with &gt;=1 translation into `targetLang` -> en -> X entry.</summary>
    private static ScrapedEntry? EnEntryToScrapedEntry(JsonElement obj, string year, string targetLang)
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

        string? targetLanguageName = KaikkiLanguageNames.GetValueOrDefault(targetLang);
        string? altCode = AltLangCodes.GetValueOrDefault(targetLang);
        List<JsonElement> targetTranslations = GetArray(obj, "translations")
            .Where(t => GetString(t, "lang_code") is { } code && (code == targetLang || code == altCode)
                || (targetLanguageName is not null && GetString(t, "lang") == targetLanguageName))
            .ToList();
        if (targetTranslations.Count == 0)
        {
            return null;
        }

        var groups = new Dictionary<string, List<JsonElement>>();
        var groupOrder = new List<string>();
        foreach (JsonElement t in targetTranslations)
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
            Id = ScrapedEntryId.Make(Source, lemma, "en", targetLang, pos ?? ""),
            Source = Source,
            SourceLicense = License,
            SourceUrl = EnRawSourceUrl,
            SourceAuthors = Authors,
            SourceYear = year,
            RetrievedAt = ScrapedEntryId.Today(),
            Lemma = lemma,
            LemmaLang = "en",
            TargetLang = targetLang,
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
