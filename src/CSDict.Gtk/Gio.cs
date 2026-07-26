using System.Runtime.InteropServices;

namespace CSDict.Gtk;

/// <summary>P/Invoke bindings for the subset of libgio-2.0 needed to drive a GtkApplication main loop.</summary>
public static partial class Gio
{
    private const string LibName = "gio-2.0";

    [LibraryImport(LibName)]
    public static partial int g_application_run(nint application, int argc, nint argv);
}
