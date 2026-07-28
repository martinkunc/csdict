using System.Diagnostics;

namespace CSDict.App;

/// <summary>Best-effort read of the OS's current light/dark appearance preference, so the app's
/// own hand-rolled CSS palette (see Program.BuildCss) can match it rather than always rendering
/// dark regardless of the platform setting. Each platform is queried through whatever the OS
/// exposes for this - there's no single GTK API that reports it reliably across macOS/Windows/Linux
/// backends, so this shells out to the native mechanism per platform and falls back to light (the
/// safer default - a light UI in a dark OS is merely inconsistent, whereas a dark UI in a light OS
/// - our old hardcoded behavior - reads as broken) if detection fails.</summary>
internal static class SystemTheme
{
    public static bool PrefersDark()
    {
        try
        {
            if (OperatingSystem.IsMacOS())
            {
                return PrefersDarkMacOS();
            }

            if (OperatingSystem.IsWindows())
            {
                return PrefersDarkWindows();
            }

            if (OperatingSystem.IsLinux())
            {
                return PrefersDarkLinux();
            }
        }
        catch
        {
            // best-effort only - fall through to the light default.
        }

        return false;
    }

    /// <summary>macOS has no "light" key - AppleInterfaceStyle is only present at all when dark
    /// mode is on, so a missing key (non-zero exit) means light.</summary>
    private static bool PrefersDarkMacOS()
    {
        string output = RunAndReadStdout("defaults", "read -g AppleInterfaceStyle");
        return output.Equals("Dark", StringComparison.OrdinalIgnoreCase);
    }

    [System.Runtime.Versioning.SupportedOSPlatform("windows")]
    private static bool PrefersDarkWindows()
    {
        using Microsoft.Win32.RegistryKey? key = Microsoft.Win32.Registry.CurrentUser.OpenSubKey(
            @"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        return key?.GetValue("AppsUseLightTheme") is int lightValue && lightValue == 0;
    }

    /// <summary>Covers GNOME and GNOME-derived desktops (Ubuntu, Fedora Workstation, etc.) via the
    /// standard gsettings key; other desktop environments (KDE, etc.) fall back to light.</summary>
    private static bool PrefersDarkLinux()
    {
        string output = RunAndReadStdout("gsettings", "get org.gnome.desktop.interface color-scheme");
        return output.Contains("dark", StringComparison.OrdinalIgnoreCase);
    }

    internal static string RunAndReadStdout(string fileName, string arguments)
    {
        using var process = Process.Start(new ProcessStartInfo
        {
            FileName = fileName,
            Arguments = arguments,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
        });

        if (process is null)
        {
            return "";
        }

        string output = process.StandardOutput.ReadToEnd();
        process.WaitForExit();
        return output.Trim();
    }
}
