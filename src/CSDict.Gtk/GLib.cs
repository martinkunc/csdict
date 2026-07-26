using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace CSDict.Gtk;

/// <summary>P/Invoke bindings for the subset of libglib-2.0 needed to marshal work from a
/// background thread back onto the GTK main loop (there is no SynchronizationContext for a raw
/// GLib main loop, so this is the only safe way to touch widgets from another thread).</summary>
public static partial class GLib
{
    private const string LibName = "glib-2.0";

    public const int G_SOURCE_REMOVE = 0;

    [LibraryImport(LibName)]
    public static partial uint g_idle_add(nint function, nint data);

    /// <summary>Schedules <paramref name="action"/> to run once on the main loop's next idle
    /// iteration. Safe to call from any thread.</summary>
    public static unsafe void RunOnMainThread(Action action)
    {
        nint data = (nint)GCHandle.Alloc(action);
        g_idle_add((nint)(delegate* unmanaged[Cdecl]<nint, int>)&InvokeIdleAction, data);
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static int InvokeIdleAction(nint data)
    {
        var handle = (GCHandle)data;
        var action = (Action)handle.Target!;
        handle.Free();
        action();
        return G_SOURCE_REMOVE;
    }
}
