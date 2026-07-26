using System.Net;
using System.Net.Http.Headers;

namespace CSDict.App.Scraping;

/// <summary>C# port of dicts/common/download.py: streaming HTTP download with resume support and
/// a best-effort Last-Modified year lookup, shared by every scraper below.</summary>
internal static class Downloader
{
    private static readonly HttpClient Http = new() { Timeout = TimeSpan.FromMinutes(30) };
    private const int BufferSize = 1024 * 1024;

    /// <summary>Best-effort year of a remote file's Last-Modified header, for source_year.
    /// Falls back to "unknown" rather than throwing - this is attribution metadata, not
    /// something that should ever break a scrape.</summary>
    public static async Task<string> FetchLastModifiedYearAsync(string url, CancellationToken ct)
    {
        try
        {
            using var request = new HttpRequestMessage(HttpMethod.Head, url);
            using HttpResponseMessage response = await Http.SendAsync(request, ct);
            if (response.Content.Headers.LastModified is { } lastModified)
            {
                return lastModified.Year.ToString();
            }
        }
        catch (Exception)
        {
            // best-effort only, see summary above.
        }

        return "unknown";
    }

    /// <summary>Downloads `url` to `destPath`, resuming via HTTP Range if a partial file already
    /// exists there (so a killed 2GB+ Wiktionary download doesn't restart from zero). Skips
    /// entirely if the file is already complete (server replies 416).</summary>
    public static async Task DownloadAsync(string url, string destPath, IProgress<string>? progress, CancellationToken ct)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(destPath) is { Length: > 0 } dir ? dir : ".");

        long existingSize = File.Exists(destPath) ? new FileInfo(destPath).Length : 0;

        using var request = new HttpRequestMessage(HttpMethod.Get, url);
        FileMode mode = FileMode.Create;
        if (existingSize > 0)
        {
            request.Headers.Range = new RangeHeaderValue(existingSize, null);
            mode = FileMode.Append;
        }

        using HttpResponseMessage response = await Http.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, ct);
        if (response.StatusCode == HttpStatusCode.RequestedRangeNotSatisfiable)
        {
            return;
        }

        response.EnsureSuccessStatusCode();

        long? total = response.Content.Headers.ContentLength is { } len ? existingSize + len : null;
        string name = Path.GetFileName(destPath);

        await using Stream input = await response.Content.ReadAsStreamAsync(ct);
        await using var output = new FileStream(destPath, mode, FileAccess.Write, FileShare.Read, BufferSize, useAsync: true);

        byte[] buffer = new byte[BufferSize];
        long written = existingSize;
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
                ? $"{name}: {written / 1e6:F1}MB / {total.Value / 1e6:F1}MB ({written * 100.0 / total.Value:F1}%)"
                : $"{name}: {written / 1e6:F1}MB");
        }
    }
}
