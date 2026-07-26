using System.Reflection;
using System.Runtime.InteropServices;

namespace CSDict.Gtk.Native.Windows;

/// <summary>
/// Resolves the native libraries the CSDict.Gtk bindings P/Invoke into (gtk-4, gobject-2.0, gio-2.0,
/// glib-2.0) to their absolute path on Windows. Prefers the copy bundled in this package's own
/// runtimes/win-x64/native/ directory (GTK 4 plus its full runtime dependency closure, fetched from
/// MSYS2's UCRT64 repository - see scripts/fetch-windows-native-deps.{sh,ps1} at the repo root) so
/// the app runs standalone without requiring GTK to be installed on the target system. Falls back to
/// an MSYS2/gvsbuild/portable install if the bundled copy is ever missing.
/// </summary>
public static class NativeLibraryResolver
{
    private static readonly string[] WellKnownDirectories =
    [
        @"C:\msys64\mingw64\bin",            // MSYS2, mingw-w64 (most common)
        @"C:\msys64\ucrt64\bin",             // MSYS2, UCRT64 toolchain
        @"C:\msys64\clang64\bin",            // MSYS2, clang64 toolchain
        @"C:\msys64\clangarm64\bin",         // MSYS2, arm64
        @"C:\gtk-build\gtk\x64\release\bin", // gvsbuild default output
        @"C:\gtk\bin",                       // portable/manual GTK layout
    ];

    private static bool s_registered;
    private static string? s_gtkBinDirectory;

    public static void Register()
    {
        if (s_registered)
        {
            return;
        }

        s_registered = true;
        s_gtkBinDirectory = FindGtkBinDirectory();

        if (s_gtkBinDirectory is not null)
        {
            // gtk-4 pulls in a chain of sibling dependencies (glib, cairo, pango, gdk-pixbuf, harfbuzz, ...).
            // Loading it by an absolute path does not add its own directory to the search path Windows uses
            // to resolve those transitive imports, so put it on PATH for this process.
            PrependToPath(s_gtkBinDirectory);
        }

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

    private static string? FindLibrary(string name) =>
        s_gtkBinDirectory is not null ? FindLibraryIn(s_gtkBinDirectory, name) : null;

    private static string? FindGtkBinDirectory()
    {
        var candidates = new List<string>();

        string? explicitDir = Environment.GetEnvironmentVariable("CSGTK_GTK_DIR");
        if (!string.IsNullOrEmpty(explicitDir))
        {
            candidates.Add(explicitDir);
        }

        // Framework-dependent builds keep the package's native assets under this RID-specific
        // subdirectory. `dotnet publish --self-contained` (what the release workflow uses) instead
        // flattens them directly into the app's own output directory, so that must be checked too.
        candidates.Add(Path.Combine(AppContext.BaseDirectory, "runtimes", "win-x64", "native"));
        candidates.Add(AppContext.BaseDirectory);
        candidates.AddRange(WellKnownDirectories);

        string? pathVar = Environment.GetEnvironmentVariable("PATH");
        if (pathVar is not null)
        {
            candidates.AddRange(pathVar.Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries));
        }

        foreach (string dir in candidates)
        {
            if (Directory.Exists(dir) && FindLibraryIn(dir, "gtk-4") is not null)
            {
                return dir;
            }
        }

        return null;
    }

    private static string? FindLibraryIn(string dir, string name)
    {
        // MSYS2/mingw-w64 packages keep the Unix "lib" prefix and a numeric ABI suffix before the
        // extension (e.g. libgtk-4-1.dll, libgobject-2.0-0.dll). gvsbuild drops the "lib" prefix
        // (e.g. gtk-4-1.dll). Try the unversioned name first, then fall back to a wildcard search.
        string[] exact = [Path.Combine(dir, $"lib{name}.dll"), Path.Combine(dir, $"{name}.dll")];
        foreach (string candidate in exact)
        {
            if (File.Exists(candidate))
            {
                return candidate;
            }
        }

        return Directory.EnumerateFiles(dir, $"lib{name}-*.dll")
            .Concat(Directory.EnumerateFiles(dir, $"{name}-*.dll"))
            .FirstOrDefault();
    }

    private static void PrependToPath(string dir)
    {
        string path = Environment.GetEnvironmentVariable("PATH") ?? "";
        bool alreadyPresent = path
            .Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
            .Any(p => string.Equals(p, dir, StringComparison.OrdinalIgnoreCase));

        if (!alreadyPresent)
        {
            Environment.SetEnvironmentVariable("PATH", $"{dir}{Path.PathSeparator}{path}");
        }
    }
}
