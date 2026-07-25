using System.Reflection;
using System.Runtime.InteropServices;

namespace CSGtk.Gtk.Native.macOS;

/// <summary>
/// Resolves the native libraries the CSGtk.Gtk bindings P/Invoke into (libgtk-4, libgobject-2.0,
/// libgio-2.0, libglib-2.0) to their absolute path on macOS. Prefers the copy bundled in this
/// package's own runtimes/{osx-arm64,osx-x64}/native/ directory (GTK 4 plus its full runtime
/// dependency closure, fetched from Homebrew's bottles via scripts/fetch-macos-native-deps.sh at
/// the repo root, with install names rewritten to be self-contained) so the app runs standalone
/// without requiring GTK to be installed on the target system. Falls back to a Homebrew install
/// (e.g. `brew install gtk4`) if the bundled copy is ever missing.
/// </summary>
public static class NativeLibraryResolver
{
    private static readonly string[] HomebrewLibDirectories =
    [
        "/opt/homebrew/lib", // Homebrew on Apple Silicon
        "/usr/local/lib",    // Homebrew on Intel
    ];

    private static bool s_registered;

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
        string rid = RuntimeInformation.ProcessArchitecture == Architecture.Arm64 ? "osx-arm64" : "osx-x64";
        string bundledDir = Path.Combine(AppContext.BaseDirectory, "runtimes", rid, "native");
        if (Directory.Exists(bundledDir) && FindLibraryIn(bundledDir, name) is string bundled)
        {
            return bundled;
        }

        foreach (string dir in HomebrewLibDirectories)
        {
            if (FindLibraryIn(dir, name) is string found)
            {
                return found;
            }
        }

        return null;
    }

    private static string? FindLibraryIn(string dir, string name)
    {
        string exact = Path.Combine(dir, $"lib{name}.dylib");
        if (File.Exists(exact))
        {
            return exact;
        }

        return Directory.Exists(dir) ? Directory.EnumerateFiles(dir, $"lib{name}.*.dylib").FirstOrDefault() : null;
    }
}
