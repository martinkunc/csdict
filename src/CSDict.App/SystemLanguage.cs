namespace CSDict.App;

/// <summary>Best-effort read of the OS's current UI language, used to preselect the "From" language
/// in the dictionaries dialog. CultureInfo.CurrentUICulture can't be used for this - the app builds
/// with InvariantGlobalization enabled (see CSDict.App.csproj), which makes every CultureInfo
/// always report the invariant culture regardless of the actual OS setting. So, same as
/// SystemTheme, this shells out to whatever each platform exposes for it instead.</summary>
internal static class SystemLanguage
{
    /// <returns>A lowercase two-letter language code (e.g. "en"), or null if detection failed.</returns>
    public static string? CurrentTwoLetterLanguage()
    {
        try
        {
            if (OperatingSystem.IsMacOS())
            {
                return TwoLetterLanguageMacOS();
            }

            if (OperatingSystem.IsWindows())
            {
                return TwoLetterLanguageWindows();
            }

            if (OperatingSystem.IsLinux())
            {
                return TwoLetterLanguageLinux();
            }
        }
        catch
        {
            // best-effort only - fall through to no hint (caller defaults to "All").
        }

        return null;
    }

    /// <summary>"defaults read -g AppleLocale" reports the user's Language & Region setting (e.g.
    /// "en_US") straight from the OS, unlike LANG/LC_*, which are usually unset for GUI apps
    /// launched outside a shell (e.g. double-clicked from Finder).</summary>
    private static string? TwoLetterLanguageMacOS() =>
        TwoLetterPrefix(SystemTheme.RunAndReadStdout("defaults", "read -g AppleLocale"));

    [System.Runtime.Versioning.SupportedOSPlatform("windows")]
    private static string? TwoLetterLanguageWindows()
    {
        using Microsoft.Win32.RegistryKey? key = Microsoft.Win32.Registry.CurrentUser.OpenSubKey(@"Control Panel\International");
        return TwoLetterPrefix(key?.GetValue("LocaleName") as string);
    }

    /// <summary>No GUI-level setting to shell out to on Linux the way macOS/Windows have one - the
    /// POSIX locale environment variables are the standard source instead, checked in the usual
    /// override order (LANGUAGE > LC_ALL > LANG).</summary>
    private static string? TwoLetterLanguageLinux()
    {
        foreach (string name in new[] { "LANGUAGE", "LC_ALL", "LANG" })
        {
            string? lang = TwoLetterPrefix(Environment.GetEnvironmentVariable(name)?.Split(':')[0]);
            if (lang is not null)
            {
                return lang;
            }
        }

        return null;
    }

    private static string? TwoLetterPrefix(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return null;
        }

        int sep = value.IndexOfAny(['_', '-', '.']);
        string prefix = sep > 1 ? value[..sep] : value;
        return prefix.Length == 2 ? prefix.ToLowerInvariant() : null;
    }
}
