using CSDict.Sqlite;

namespace CSDict.Scraper;

/// <summary>CLI entry point: scrapes one or more (lemmaLang, targetLang) directions, merging every
/// contributing source (see DirectionCatalog) into a single "{lemmaLang}_{targetLang}.sqlite3" per
/// direction. Runs offline/in CI - this is the only place scraping code lives now; CSDict.App just
/// downloads the files this produces from a GitHub Release (see docs/design/scraper-and-distribution.md).
///
/// Usage:
///   dotnet run --project src/CSDict.Scraper -- --all [--output dist/dictionaries] [--cache dist/cache]
///   dotnet run --project src/CSDict.Scraper -- --lang cs:en --lang en:cs [--output dist/dictionaries]
/// </summary>
internal static class Program
{
    private static async Task<int> Main(string[] args)
    {
        SqliteNativeResolver.Register();

        Options? options = Options.Parse(args);
        if (options is null)
        {
            PrintUsage();
            return 1;
        }

        List<Direction> directions = options.All
            ? DirectionCatalog.All.ToList()
            : options.Languages.Select(l => DirectionCatalog.Find(l.LemmaLang, l.TargetLang)
                ?? throw new ArgumentException($"no known direction '{l.LemmaLang}:{l.TargetLang}' - see DirectionCatalog.All")).ToList();

        if (directions.Count == 0)
        {
            PrintUsage();
            return 1;
        }

        Directory.CreateDirectory(options.OutputDir);

        using var cts = new CancellationTokenSource();
        Console.CancelKeyPress += (_, e) =>
        {
            e.Cancel = true;
            cts.Cancel();
        };

        foreach (Direction direction in directions)
        {
            await ScrapeDirectionAsync(direction, options.OutputDir, options.CacheDir, cts.Token);
        }

        return 0;
    }

    /// <summary>Opens one SqliteWriter for the whole direction and runs every contributing source's
    /// scraper against it in turn, so their entries end up merged into a single file (each still
    /// tagged with its own `source` column - see SqliteWriter.Schema). A source that fails is logged
    /// and skipped rather than aborting the other sources for this direction; re-run to retry it.</summary>
    private static async Task ScrapeDirectionAsync(Direction direction, string outputDir, string cacheRootDir, CancellationToken ct)
    {
        string outputPath = Path.Combine(outputDir, direction.FileName);
        Console.WriteLine($"=== {direction.DisplayName} -> {outputPath} ({string.Join(", ", direction.Sources)}) ===");

        var progress = new Progress<string>(message => Console.WriteLine($"  {message}"));
        using SqliteWriter writer = SqliteWriter.Create(outputPath);

        foreach (string source in direction.Sources)
        {
            ct.ThrowIfCancellationRequested();
            string cacheDir = Path.Combine(cacheRootDir, source);
            try
            {
                Console.WriteLine($"-- {source} --");
                await RunSourceAsync(source, direction.LemmaLang, direction.TargetLang, cacheDir, writer, progress, ct);
            }
            catch (Exception ex) when (ct.IsCancellationRequested is false)
            {
                Console.Error.WriteLine($"  {source} failed: {ex.Message}");
            }
        }

        Console.WriteLine($"Wrote {outputPath} ({writer.Count} entries total).");
    }

    private static Task RunSourceAsync(
        string source, string lemmaLang, string targetLang, string cacheDir, SqliteWriter writer,
        IProgress<string> progress, CancellationToken ct) => source switch
    {
        "freedict" => FreeDictScraper.ScrapeAsync(lemmaLang, targetLang, cacheDir, writer, progress, ct),
        "wikdict" => WikDictScraper.ScrapeAsync(lemmaLang, targetLang, cacheDir, writer, progress, ct),
        "wiktionary" when lemmaLang == "en" => WiktionaryScraper.ScrapeFromEnAsync(targetLang, cacheDir, writer, progress, ct),
        "wiktionary" => WiktionaryScraper.ScrapeToEnAsync(lemmaLang, cacheDir, writer, progress, ct),
        _ => throw new NotSupportedException($"no scraper for source '{source}'"),
    };

    private static void PrintUsage()
    {
        Console.WriteLine("""
            Usage:
              dotnet run --project src/CSDict.Scraper -- --all [--output dist/dictionaries] [--cache dist/cache]
              dotnet run --project src/CSDict.Scraper -- --lang cs:en --lang en:cs [--output dist/dictionaries]

            --all              Scrape every direction in DirectionCatalog.All.
            --lang X:Y         Scrape just the X->Y direction (repeatable).
            --output <dir>     Where to write "{lemmaLang}_{targetLang}.sqlite3" files (default: dist/dictionaries).
            --cache <dir>      Where to cache downloaded source archives between runs (default: dist/cache).
            """);
    }

    private sealed record Options(bool All, List<(string LemmaLang, string TargetLang)> Languages, string OutputDir, string CacheDir)
    {
        public static Options? Parse(string[] args)
        {
            bool all = false;
            var languages = new List<(string, string)>();
            string outputDir = "dist/dictionaries";
            string cacheDir = "dist/cache";

            for (int i = 0; i < args.Length; i++)
            {
                switch (args[i])
                {
                    case "--all":
                        all = true;
                        break;
                    case "--lang" when i + 1 < args.Length:
                        string[] parts = args[++i].Split(':', 2);
                        if (parts.Length != 2)
                        {
                            return null;
                        }

                        languages.Add((parts[0], parts[1]));
                        break;
                    case "--output" when i + 1 < args.Length:
                        outputDir = args[++i];
                        break;
                    case "--cache" when i + 1 < args.Length:
                        cacheDir = args[++i];
                        break;
                    default:
                        return null;
                }
            }

            return !all && languages.Count == 0 ? null : new Options(all, languages, outputDir, cacheDir);
        }
    }
}
