using System.Text.Json;
using System.Xml.Linq;
using CSGtk.App.Sqlite;
using SharpCompress.Compressors.Xz;
using SharpCompress.Readers.Tar;

namespace CSGtk.App.Scraping;

/// <summary>C# port of dicts/scrapers/freedict.py: downloads a FreeDict ".src.tar.xz" release (TEI
/// XML source, not the compiled StarDict blob) so license/authors/year can be read straight out of
/// the &lt;teiHeader&gt; rather than assumed, then writes straight into a SqliteWriter (no JSONL
/// intermediate - the app only ever needs the SQLite file).</summary>
internal static class FreeDictScraper
{
    private const string Source = "freedict";
    private const string CatalogUrl = "https://freedict.org/freedict-database.json";
    private const string DownloadsPage = "https://freedict.org/downloads/";

    private static readonly Dictionary<string, string> FallbackReleases = new()
    {
        ["eng-ces"] = "https://download.freedict.org/dictionaries/eng-ces/0.1.3/freedict-eng-ces-0.1.3.src.tar.xz",
        ["ces-eng"] = "https://download.freedict.org/dictionaries/ces-eng/0.2.3/freedict-ces-eng-0.2.3.src.tar.xz",
        ["eng-spa"] = "https://download.freedict.org/dictionaries/eng-spa/2025.11.23/freedict-eng-spa-2025.11.23.src.tar.xz",
        ["spa-eng"] = "https://download.freedict.org/dictionaries/spa-eng/0.3.1/freedict-spa-eng-0.3.1.src.tar.xz",
        ["eng-fra"] = "https://download.freedict.org/dictionaries/eng-fra/0.1.6/freedict-eng-fra-0.1.6.src.tar.xz",
        ["fra-eng"] = "https://download.freedict.org/dictionaries/fra-eng/0.4.1/freedict-fra-eng-0.4.1.src.tar.xz",
        ["eng-deu"] = "https://download.freedict.org/dictionaries/eng-deu/1.9-fd1/freedict-eng-deu-1.9-fd1.src.tar.xz",
        ["deu-eng"] = "https://download.freedict.org/dictionaries/deu-eng/1.9-fd1/freedict-deu-eng-1.9-fd1.src.tar.xz",
        ["eng-rus"] = "https://download.freedict.org/dictionaries/eng-rus/2025.11.23/freedict-eng-rus-2025.11.23.src.tar.xz",
        ["rus-eng"] = "https://download.freedict.org/dictionaries/rus-eng/2025.11.23/freedict-rus-eng-2025.11.23.src.tar.xz",
        ["eng-por"] = "https://download.freedict.org/dictionaries/eng-por/0.3/freedict-eng-por-0.3.src.tar.xz",
        ["por-eng"] = "https://download.freedict.org/dictionaries/por-eng/0.2/freedict-por-eng-0.2.src.tar.xz",
        ["eng-jpn"] = "https://download.freedict.org/dictionaries/eng-jpn/2025.11.23/freedict-eng-jpn-2025.11.23.src.tar.xz",
        ["jpn-eng"] = "https://download.freedict.org/dictionaries/jpn-eng/0.1/freedict-jpn-eng-0.1.src.tar.xz",
        ["eng-ara"] = "https://download.freedict.org/dictionaries/eng-ara/0.6.3/freedict-eng-ara-0.6.3.src.tar.xz",
        ["ara-eng"] = "https://download.freedict.org/dictionaries/ara-eng/0.6.3/freedict-ara-eng-0.6.3.src.tar.xz",
    };

    /// <summary>FreeDict dictionary name ("eng-ces" / "ces-eng") for each (lemmaLang, targetLang)
    /// direction this app cares about.</summary>
    public static readonly Dictionary<(string LemmaLang, string TargetLang), string> Names = new()
    {
        [("en", "cs")] = "eng-ces",
        [("cs", "en")] = "ces-eng",
        [("en", "es")] = "eng-spa",
        [("es", "en")] = "spa-eng",
        [("en", "fr")] = "eng-fra",
        [("fr", "en")] = "fra-eng",
        [("en", "de")] = "eng-deu",
        [("de", "en")] = "deu-eng",
        [("en", "ru")] = "eng-rus",
        [("ru", "en")] = "rus-eng",
        [("en", "pt")] = "eng-por",
        [("pt", "en")] = "por-eng",
        [("en", "ja")] = "eng-jpn",
        [("ja", "en")] = "jpn-eng",
        [("en", "ar")] = "eng-ara",
        [("ar", "en")] = "ara-eng",
    };

    private static readonly XNamespace Tei = "http://www.tei-c.org/ns/1.0";
    private static readonly HttpClient Http = new();

    public static async Task ScrapeAsync(
        string lemmaLang, string targetLang, string cacheDir, string outputPath,
        IProgress<string>? progress, CancellationToken ct)
    {
        string name = Names[(lemmaLang, targetLang)];

        progress?.Report($"Looking up FreeDict release for {name}...");
        string url = await GetSrcTarballUrlAsync(name, ct);

        Directory.CreateDirectory(cacheDir);
        string tarball = Path.Combine(cacheDir, Path.GetFileName(new Uri(url).LocalPath));
        if (!File.Exists(tarball))
        {
            progress?.Report($"Downloading {Path.GetFileName(tarball)}...");
            await Downloader.DownloadAsync(url, tarball, progress, ct);
        }

        progress?.Report("Extracting TEI XML...");
        XDocument doc = ExtractTei(tarball);
        XElement root = doc.Root ?? throw new InvalidDataException($"{tarball} has no root element");
        FreeDictMeta meta = ParseHeader(root);

        progress?.Report($"Parsing entries (edition={meta.Edition}, year={meta.Year})...");

        using SqliteWriter writer = SqliteWriter.Create(outputPath);
        int count = 0;
        foreach (XElement entryEl in root.Descendants(Tei + "entry"))
        {
            ct.ThrowIfCancellationRequested();
            ScrapedEntry? entry = ParseEntry(entryEl, lemmaLang, targetLang, meta);
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

        progress?.Report($"Wrote {count} entries.");
    }

    private static async Task<string> GetSrcTarballUrlAsync(string name, CancellationToken ct)
    {
        try
        {
            using HttpResponseMessage resp = await Http.GetAsync(CatalogUrl, ct);
            resp.EnsureSuccessStatusCode();
            await using Stream stream = await resp.Content.ReadAsStreamAsync(ct);
            using JsonDocument doc = await JsonDocument.ParseAsync(stream, cancellationToken: ct);

            IEnumerable<JsonElement> entries = doc.RootElement.ValueKind switch
            {
                JsonValueKind.Array => doc.RootElement.EnumerateArray(),
                JsonValueKind.Object when doc.RootElement.TryGetProperty("dictionaries", out JsonElement dicts)
                    && dicts.ValueKind == JsonValueKind.Array => dicts.EnumerateArray(),
                JsonValueKind.Object => doc.RootElement.EnumerateObject().Select(p => p.Value),
                _ => [],
            };

            foreach (JsonElement d in entries)
            {
                if (d.ValueKind != JsonValueKind.Object
                    || !d.TryGetProperty("name", out JsonElement nameEl)
                    || nameEl.GetString() != name
                    || !d.TryGetProperty("releases", out JsonElement releases)
                    || releases.ValueKind != JsonValueKind.Array)
                {
                    continue;
                }

                foreach (JsonElement rel in releases.EnumerateArray())
                {
                    string? url = rel.TryGetProperty("URL", out JsonElement u1) ? u1.GetString()
                        : rel.TryGetProperty("url", out JsonElement u2) ? u2.GetString() : null;
                    string? platform = rel.TryGetProperty("platform", out JsonElement p) ? p.GetString() : null;
                    if (url is not null && (url.Contains("src.tar.xz", StringComparison.Ordinal) || platform == "src"))
                    {
                        return url;
                    }
                }
            }
        }
        catch (Exception)
        {
            // catalog lookup is a convenience, not a hard requirement - fall back below.
        }

        return FallbackReleases[name];
    }

    private static XDocument ExtractTei(string tarballPath)
    {
        // Fully buffer the decompressed tar stream rather than handing TarReader the live XZStream
        // directly - XZStream's Read() can return short reads that SharpCompress's tar block reader
        // (which expects to fill each 512-byte header block in one call) misreads as a truncated
        // archive ("Unexpected EOF reading tar file"), even though the archive is complete. FreeDict
        // releases are small (hundreds of KB to a few MB decompressed), so buffering fully is cheap.
        using var tarBuffer = new MemoryStream();
        using (FileStream fileStream = File.OpenRead(tarballPath))
        using (var xz = new XZStream(fileStream))
        {
            xz.CopyTo(tarBuffer);
        }

        tarBuffer.Position = 0;

        using TarReader reader = TarReader.Open(tarBuffer);
        while (reader.MoveToNextEntry())
        {
            if (reader.Entry.IsDirectory || reader.Entry.Key is not { } key || !key.EndsWith(".tei", StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }

            using SharpCompress.Common.EntryStream entryStream = reader.OpenEntryStream();
            return XDocument.Load(entryStream);
        }

        throw new FileNotFoundException($"no .tei file found inside {tarballPath}");
    }

    private readonly record struct FreeDictMeta(string Edition, string License, string Authors, string Year);

    private static FreeDictMeta ParseHeader(XElement root)
    {
        XElement? header = root.Element(Tei + "teiHeader");
        XElement? fileDesc = header?.Element(Tei + "fileDesc");

        string edition = fileDesc?.Descendants(Tei + "editionStmt")
            .Select(e => e.Element(Tei + "edition"))
            .FirstOrDefault(e => e is not null)?.Value.Trim() is { Length: > 0 } editionText
            ? editionText
            : "unknown";

        List<string> licenseTextParts = fileDesc?.Descendants(Tei + "availability")
            .SelectMany(av => av.Descendants(Tei + "p"))
            .Select(p => p.Value.Trim())
            .Where(t => t.Length > 0)
            .ToList() ?? [];

        string? licenseUrl = fileDesc?.Descendants(Tei + "availability")
            .SelectMany(av => av.Descendants(Tei + "ref"))
            .FirstOrDefault()?.Attribute("target")?.Value;

        string licenseSummary = licenseTextParts.Count > 1 ? licenseTextParts[1]
            : licenseTextParts.Count > 0 ? licenseTextParts[0]
            : "license unspecified in TEI header";
        if (licenseUrl is not null)
        {
            licenseSummary = $"{licenseSummary} ({licenseUrl})";
        }

        var authors = new SortedSet<string>(StringComparer.Ordinal) { "FreeDict project contributors" };
        if (header is not null)
        {
            foreach (XElement nameEl in header.Descendants(Tei + "revisionDesc").SelectMany(rd => rd.Descendants(Tei + "name")))
            {
                string text = nameEl.Value.Trim();
                if (text.Length > 0)
                {
                    authors.Add(text);
                }
            }
        }

        List<string> years = header?.Descendants(Tei + "revisionDesc")
            .SelectMany(rd => rd.Elements(Tei + "change"))
            .Select(chg => (string?)chg.Attribute("when"))
            .Where(when => !string.IsNullOrEmpty(when))
            .Select(when => when!.Length >= 4 ? when[..4] : when!)
            .ToList() ?? [];
        string year = years.Count > 0 ? years.Max()! : "unknown";

        return new FreeDictMeta(edition, licenseSummary, string.Join("; ", authors), year);
    }

    private static ScrapedEntry? ParseEntry(XElement entryEl, string lemmaLang, string targetLang, FreeDictMeta meta)
    {
        string? lemma = entryEl.Descendants(Tei + "form").Elements(Tei + "orth").FirstOrDefault()?.Value.Trim();
        if (string.IsNullOrEmpty(lemma))
        {
            return null;
        }

        string? pos = entryEl.Descendants(Tei + "gramGrp").Elements(Tei + "pos").FirstOrDefault()?.Value.Trim();
        if (string.IsNullOrEmpty(pos))
        {
            // eng-ces sometimes encodes POS as <gen> instead of <pos>; ces-eng has no <gramGrp> at
            // all in practice, so this fallback simply never fires there.
            string? gen = entryEl.Descendants(Tei + "gramGrp").Elements(Tei + "gen").FirstOrDefault()?.Value.Trim();
            pos = string.IsNullOrEmpty(gen) ? null : gen;
        }

        List<XElement> senseEls = entryEl.Elements(Tei + "sense").ToList();
        if (senseEls.Count == 0)
        {
            // Some tiny/older FreeDict files put <cit> directly under <entry>.
            senseEls = [entryEl];
        }

        var senses = new List<ScrapedSense>();
        foreach (XElement senseEl in senseEls)
        {
            List<string> translations = senseEl.Descendants(Tei + "cit")
                .Where(cit => (string?)cit.Attribute("type") == "trans")
                .Elements(Tei + "quote")
                .Select(q => q.Value.Trim())
                .Where(t => t.Length > 0)
                .ToList();
            if (translations.Count == 0)
            {
                continue;
            }

            List<string> glossParts = senseEl.Descendants(Tei + "def")
                .Select(d => d.Value.Trim())
                .Where(t => t.Length > 0)
                .ToList();
            List<string> tags = senseEl.Descendants(Tei + "usg")
                .Select(u => u.Value.Trim())
                .Where(t => t.Length > 0)
                .ToList();

            senses.Add(new ScrapedSense
            {
                Translations = translations,
                Gloss = glossParts.Count > 0 ? string.Join("; ", glossParts) : null,
                Tags = tags,
            });
        }

        if (senses.Count == 0)
        {
            return null;
        }

        return new ScrapedEntry
        {
            Id = ScrapedEntryId.Make(Source, lemma, lemmaLang, targetLang, pos ?? ""),
            Source = Source,
            SourceLicense = meta.License,
            SourceUrl = DownloadsPage,
            SourceAuthors = meta.Authors,
            SourceYear = meta.Year,
            RetrievedAt = ScrapedEntryId.Today(),
            Lemma = lemma,
            LemmaLang = lemmaLang,
            TargetLang = targetLang,
            Pos = pos,
            Senses = senses,
        };
    }
}
