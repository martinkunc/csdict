using System.Diagnostics;
using System.Reflection;
using System.Runtime.InteropServices;

namespace CSGtk.Gtk.Native.Linux;

/// <summary>
/// Resolves the native libraries the CSGtk.Gtk bindings P/Invoke into (gtk-4, gobject-2.0, gio-2.0)
/// to their absolute path on Linux. GTK 4 itself is not bundled in this package - it must already be
/// installed via the distro's package manager (e.g. `apt install libgtk-4-1`, `dnf install gtk4`,
/// `pacman -S gtk4`).
/// </summary>
public static class NativeLibraryResolver
{
    // Runtime-only installs (no matching -dev package) typically only ship the ld.so.cache and the
    // versioned soname (e.g. libgtk-4.so.1), not the unversioned symlink a linker would use, so we
    // fall back to scanning these multiarch/legacy locations directly if ldconfig isn't available.
    private static readonly string[] WellKnownDirectories =
    [
        $"/usr/lib/{MultiarchTuple}",
        $"/lib/{MultiarchTuple}",
        "/usr/lib64",
        "/lib64",
        "/usr/lib",
        "/lib",
        "/usr/local/lib",
    ];

    private static readonly string[] LdconfigPaths = ["ldconfig", "/sbin/ldconfig", "/usr/sbin/ldconfig"];

    private static string MultiarchTuple => RuntimeInformation.ProcessArchitecture switch
    {
        Architecture.X64 => "x86_64-linux-gnu",
        Architecture.Arm64 => "aarch64-linux-gnu",
        _ => "x86_64-linux-gnu",
    };

    private static bool s_registered;
    private static Dictionary<string, string>? s_ldconfigCache;

    public static void Register()
    {
        if (s_registered)
        {
            return;
        }

        s_registered = true;
        NativeLibrary.SetDllImportResolver(typeof(Gtk4).Assembly, Resolve);
    }

    private static nint Resolve(string libraryName, Assembly assembly, DllImportSearchPath? searchPath)
    {
        string? path = libraryName switch
        {
            "gtk-4" or "gobject-2.0" or "gio-2.0" or "glib-2.0" => FindLibrary(libraryName),
            _ => null,
        };

        return path is not null && NativeLibrary.TryLoad(path, out nint handle) ? handle : 0;
    }

    private static string? FindLibrary(string name)
    {
        string? explicitDir = Environment.GetEnvironmentVariable("CSGTK_GTK_DIR");
        if (!string.IsNullOrEmpty(explicitDir) && Directory.Exists(explicitDir))
        {
            string? found = FindLibraryIn(explicitDir, name);
            if (found is not null)
            {
                return found;
            }
        }

        string? viaLdconfig = FindViaLdconfig(name);
        if (viaLdconfig is not null)
        {
            return viaLdconfig;
        }

        var candidates = new List<string>(WellKnownDirectories);
        string? ldLibraryPath = Environment.GetEnvironmentVariable("LD_LIBRARY_PATH");
        if (ldLibraryPath is not null)
        {
            candidates.AddRange(ldLibraryPath.Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries));
        }

        foreach (string dir in candidates)
        {
            if (Directory.Exists(dir) && FindLibraryIn(dir, name) is string found)
            {
                return found;
            }
        }

        return null;
    }

    private static string? FindLibraryIn(string dir, string name)
    {
        string exact = Path.Combine(dir, $"lib{name}.so");
        if (File.Exists(exact))
        {
            return exact;
        }

        // Prefer the shortest match (the soname-style symlink, e.g. "libgobject-2.0.so.0") over a
        // longer fully-versioned real file ("libgobject-2.0.so.0.8800.0") that might sit alongside
        // it in a bundled directory. GTK's own ELF NEEDED entries reference the short soname, so our
        // explicit load must resolve to the exact same path string - otherwise the dynamic linker
        // ends up with two independent instances of a library GObject expects to be a process-wide
        // singleton (its global GType registry), which corrupts type registration and segfaults.
        return Directory.EnumerateFiles(dir, $"lib{name}.so.*")
            .OrderBy(f => f.Length)
            .ThenBy(f => f, StringComparer.Ordinal)
            .FirstOrDefault();
    }

    private static string? FindViaLdconfig(string name)
    {
        s_ldconfigCache ??= BuildLdconfigCache();
        return s_ldconfigCache.TryGetValue(name, out string? path) ? path : null;
    }

    private static Dictionary<string, string> BuildLdconfigCache()
    {
        var map = new Dictionary<string, string>();

        foreach (string ldconfig in LdconfigPaths)
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

                string libName = soname[3..soIndex];
                map.TryAdd(libName, path);
            }

            return true;
        }
        catch (Exception ex) when (ex is System.ComponentModel.Win32Exception or IOException)
        {
            // ldconfig isn't at this path or isn't executable (e.g. musl-based distros like Alpine
            // ship no ldconfig cache at all); the caller falls back to scanning well-known directories.
            return false;
        }
    }
}
