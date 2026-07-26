using System.Diagnostics;
using System.Reflection;
using System.Runtime.InteropServices;

namespace CSDict.App.Sqlite;

/// <summary>
/// Resolves "sqlite3" (the name Sqlite3.cs P/Invokes into) to its absolute path at runtime.
/// SQLite itself is not bundled - same policy as CSDict.Gtk.Native.* for GTK: it must already be
/// installed (it ships with the OS on Linux/macOS; on Windows install it the same way you installed
/// GTK4, e.g. `pacman -S mingw-w64-x86_64-sqlite3` under MSYS2, or drop sqlite3.dll on PATH).
/// One file covering all three platforms, since this binding is app-internal rather than a
/// redistributable per-platform NuGet package like CSDict.Gtk.Native.*.
/// </summary>
internal static class SqliteNativeResolver
{
    private static bool s_registered;
    private static Dictionary<string, string>? s_ldconfigCache;

    public static void Register()
    {
        if (s_registered)
        {
            return;
        }

        s_registered = true;
        NativeLibrary.SetDllImportResolver(typeof(Sqlite3).Assembly, Resolve);
    }

    private static nint Resolve(string libraryName, Assembly assembly, DllImportSearchPath? searchPath)
    {
        if (libraryName != "sqlite3")
        {
            return 0;
        }

        string? path = FindLibrary();
        return path is not null && NativeLibrary.TryLoad(path, out nint handle) ? handle : 0;
    }

    private static string? FindLibrary()
    {
        string? explicitDir = Environment.GetEnvironmentVariable("CSGTK_SQLITE_DIR");
        if (!string.IsNullOrEmpty(explicitDir) && Directory.Exists(explicitDir)
            && FindLibraryIn(explicitDir) is string fromExplicit)
        {
            return fromExplicit;
        }

        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return FindWindows();
        }

        if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
        {
            return FindMacOS();
        }

        return FindLinux();
    }

    // --- Linux: system libsqlite3.so.0, via ldconfig or well-known multiarch dirs ---

    private static string? FindLinux()
    {
        if (FindViaLdconfig() is string viaLdconfig)
        {
            return viaLdconfig;
        }

        string tuple = RuntimeInformation.ProcessArchitecture == Architecture.Arm64
            ? "aarch64-linux-gnu"
            : "x86_64-linux-gnu";

        string[] dirs =
        [
            $"/usr/lib/{tuple}", $"/lib/{tuple}", "/usr/lib64", "/lib64", "/usr/lib", "/lib", "/usr/local/lib",
        ];

        foreach (string dir in dirs)
        {
            if (Directory.Exists(dir) && FindLibraryIn(dir) is string found)
            {
                return found;
            }
        }

        return null;
    }

    private static string? FindLibraryIn(string dir)
    {
        string exact = Path.Combine(dir, "libsqlite3.so");
        if (File.Exists(exact))
        {
            return exact;
        }

        return Directory.Exists(dir)
            ? Directory.EnumerateFiles(dir, "libsqlite3.so.*").OrderByDescending(f => f).FirstOrDefault()
            : null;
    }

    private static string? FindViaLdconfig()
    {
        s_ldconfigCache ??= BuildLdconfigCache();
        return s_ldconfigCache.TryGetValue("sqlite3", out string? path) ? path : null;
    }

    private static Dictionary<string, string> BuildLdconfigCache()
    {
        var map = new Dictionary<string, string>();
        foreach (string ldconfig in new[] { "ldconfig", "/sbin/ldconfig", "/usr/sbin/ldconfig" })
        {
            if (TryReadLdconfigCache(ldconfig, map))
            {
                break;
            }
        }

        return map;
    }

    private static bool TryReadLdconfigCache(string ldconfigPath, Dictionary<string, string> map)
    {
        try
        {
            var startInfo = new ProcessStartInfo(ldconfigPath, "-p")
            {
                RedirectStandardOutput = true,
                UseShellExecute = false,
            };

            using Process? process = Process.Start(startInfo);
            if (process is null)
            {
                return false;
            }

            string output = process.StandardOutput.ReadToEnd();
            process.WaitForExit(2000);

            foreach (string line in output.Split('\n'))
            {
                int arrow = line.IndexOf("=>", StringComparison.Ordinal);
                if (arrow < 0)
                {
                    continue;
                }

                string soname = line[..arrow].Trim();
                string path = line[(arrow + 2)..].Trim();

                if (!soname.StartsWith("lib", StringComparison.Ordinal))
                {
                    continue;
                }

                int soIndex = soname.IndexOf(".so", StringComparison.Ordinal);
                if (soIndex < 0)
                {
                    continue;
                }

                map.TryAdd(soname[3..soIndex], path);
            }

            return true;
        }
        catch (Exception ex) when (ex is System.ComponentModel.Win32Exception or IOException)
        {
            return false;
        }
    }

    // --- macOS: system SQLite (always at /usr/lib/libsqlite3.dylib) or Homebrew ---

    private static string? FindMacOS()
    {
        string[] dirs = ["/usr/lib", "/opt/homebrew/lib", "/usr/local/lib"];
        foreach (string dir in dirs)
        {
            string exact = Path.Combine(dir, "libsqlite3.dylib");
            if (File.Exists(exact))
            {
                return exact;
            }

            if (Directory.Exists(dir)
                && Directory.EnumerateFiles(dir, "libsqlite3.*.dylib").FirstOrDefault() is string versioned)
            {
                return versioned;
            }
        }

        return null;
    }

    // --- Windows: same MSYS2/gvsbuild directories CSDict.Gtk.Native.Windows already scans for GTK ---

    private static string? FindWindows()
    {
        var candidates = new List<string>
        {
            @"C:\msys64\mingw64\bin",
            @"C:\msys64\ucrt64\bin",
            @"C:\msys64\clang64\bin",
            @"C:\msys64\clangarm64\bin",
            @"C:\gtk-build\gtk\x64\release\bin",
            @"C:\gtk\bin",
        };

        string? pathVar = Environment.GetEnvironmentVariable("PATH");
        if (pathVar is not null)
        {
            candidates.AddRange(pathVar.Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries));
        }

        foreach (string dir in candidates)
        {
            if (!Directory.Exists(dir))
            {
                continue;
            }

            string[] exact = [Path.Combine(dir, "libsqlite3.dll"), Path.Combine(dir, "sqlite3.dll")];
            foreach (string candidate in exact)
            {
                if (File.Exists(candidate))
                {
                    return candidate;
                }
            }

            if (Directory.EnumerateFiles(dir, "libsqlite3-*.dll").FirstOrDefault() is string found)
            {
                return found;
            }
        }

        return null;
    }
}
