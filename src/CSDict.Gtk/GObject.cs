using System.Runtime.InteropServices;

namespace CSDict.Gtk;

/// <summary>P/Invoke bindings for the subset of libgobject-2.0 needed for signal handling and lifetime.</summary>
public static partial class GObject
{
    private const string LibName = "gobject-2.0";

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    private static partial ulong g_signal_connect_data(nint instance, string detailedSignal, nint cHandler, nint data, nint destroyData, uint connectFlags);

    [LibraryImport(LibName)]
    public static partial void g_object_unref(nint obj);

    /// <summary>Connects a signal using the common (instance, user_data) -> void callback shape shared by
    /// "activate", "clicked" and "toggled".</summary>
    public static unsafe ulong Connect(nint instance, string signal, delegate* unmanaged[Cdecl]<nint, nint, void> handler, nint userData = 0)
        => g_signal_connect_data(instance, signal, (nint)handler, userData, 0, 0);

    /// <summary>Connects a signal using the (instance, extra_arg, user_data) -> void callback shape
    /// used by e.g. GtkListBox's "row-activated" (extra_arg is the activated GtkListBoxRow*).</summary>
    public static unsafe ulong Connect(nint instance, string signal, delegate* unmanaged[Cdecl]<nint, nint, nint, void> handler, nint userData = 0)
        => g_signal_connect_data(instance, signal, (nint)handler, userData, 0, 0);
}
