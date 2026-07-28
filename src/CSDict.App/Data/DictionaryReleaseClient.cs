using System.IO.Compression;

namespace CSDict.App.Data;

/// <summary>Downloads a direction's merged dictionary straight from this repo's rolling
/// "dictionaries" GitHub Release - see docs/design/scraper-and-distribution.md. The app no longer
/// scrapes anything itself; CSDict.Scraper (a separate, offline tool) produces these files and
/// build-dictionaries.yml publishes them here. The URL is fixed (no GitHub API lookup for
/// "latest" needed) since that release's assets are overwritten in place on every publish.</summary>
internal static class DictionaryReleaseClient
{
    private const string BaseUrl = "https://github.com/martinkunc/csdict/releases/download/dictionaries/";
    private static readonly HttpClient Http = new() { Timeout = TimeSpan.FromMinutes(30) };

    /// <summary>Downloads "{lemmaLang}_{targetLang}.zip", extracts its single .sqlite3 member, and
    /// moves it into place at outputPath - via a temp file so a failed/cancelled run never leaves a
    /// half-written file that would otherwise look "downloaded" to the rest of the app.</summary>
    public static async Task DownloadAsync(DictionaryDirection direction, string outputPath, string tempDir, IProgress<string>? progress, CancellationToken ct)
    {
        Directory.CreateDirectory(tempDir);
        string zipPath = Path.Combine(tempDir, $"{direction.FileName}.zip.tmp");
        string extractDir = Path.Combine(tempDir, $"{direction.LemmaLang}_{direction.TargetLang}-extracted");

        try
        {
            string url = $"{BaseUrl}{direction.LemmaLang}_{direction.TargetLang}.zip";
            progress?.Report($"Downloading {direction.DisplayName}...");
            await DownloadFileAsync(url, zipPath, progress, ct);

            progress?.Report("Extracting...");
            if (Directory.Exists(extractDir))
            {
                Directory.Delete(extractDir, recursive: true);
            }

            ZipFile.ExtractToDirectory(zipPath, extractDir);

            string sqlitePath = Directory.EnumerateFiles(extractDir, "*.sqlite3", SearchOption.AllDirectories).FirstOrDefault()
                ?? throw new InvalidDataException($"no .sqlite3 file found inside {url}");

            Directory.CreateDirectory(Path.GetDirectoryName(outputPath) ?? ".");
            File.Copy(sqlitePath, outputPath, overwrite: true);
        }
        finally
        {
            if (File.Exists(zipPath))
            {
                File.Delete(zipPath);
            }

            if (Directory.Exists(extractDir))
            {
                Directory.Delete(extractDir, recursive: true);
            }
        }
    }

    private static async Task DownloadFileAsync(string url, string destPath, IProgress<string>? progress, CancellationToken ct)
    {
        using HttpResponseMessage response = await Http.GetAsync(url, HttpCompletionOption.ResponseHeadersRead, ct);
        response.EnsureSuccessStatusCode();

        long? total = response.Content.Headers.ContentLength;
        await using Stream input = await response.Content.ReadAsStreamAsync(ct);
        await using var output = new FileStream(destPath, FileMode.Create, FileAccess.Write, FileShare.Read, 1024 * 1024, useAsync: true);

        byte[] buffer = new byte[1024 * 1024];
        long written = 0;
        long lastReportedMb = -1;
        int read;
        while ((read = await input.ReadAsync(buffer, ct)) > 0)
        {
            await output.WriteAsync(buffer.AsMemory(0, read), ct);
            written += read;

            long mb = written / (1024 * 1024);
            if (mb == lastReportedMb)
            {
                continue;
            }

            lastReportedMb = mb;
            progress?.Report(total.HasValue
                ? $"{written / 1e6:F1}MB / {total.Value / 1e6:F1}MB ({written * 100.0 / total.Value:F1}%)"
                : $"{written / 1e6:F1}MB");
        }
    }
}
