using System.Runtime.InteropServices;

namespace CSDict.Gtk;

/// <summary>P/Invoke bindings for the subset of libgtk-4 needed to build the initial UI:
/// application/window, header bar, boxes, labels, buttons and the search entry.</summary>
public static partial class Gtk4
{
    private const string LibName = "gtk-4";

    // Application / window

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial nint gtk_application_new(string applicationId, uint flags);

    [LibraryImport(LibName)]
    public static partial nint gtk_application_window_new(nint application);

    [LibraryImport(LibName)]
    public static partial nint gtk_window_new();

    [LibraryImport(LibName)]
    public static partial void gtk_window_set_default_size(nint window, int width, int height);

    [LibraryImport(LibName)]
    public static partial void gtk_window_set_titlebar(nint window, nint titlebar);

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial void gtk_window_set_title(nint window, string title);

    [LibraryImport(LibName)]
    public static partial void gtk_window_set_transient_for(nint window, nint parent);

    [LibraryImport(LibName)]
    public static partial void gtk_window_set_modal(nint window, [MarshalAs(UnmanagedType.Bool)] bool modal);

    [LibraryImport(LibName)]
    public static partial void gtk_window_set_child(nint window, nint child);

    [LibraryImport(LibName)]
    public static partial void gtk_window_present(nint window);

    [LibraryImport(LibName)]
    public static partial void gtk_window_destroy(nint window);

    // Header bar

    [LibraryImport(LibName)]
    public static partial nint gtk_header_bar_new();

    [LibraryImport(LibName)]
    public static partial void gtk_header_bar_set_show_title_buttons(nint bar, [MarshalAs(UnmanagedType.Bool)] bool settings);

    [LibraryImport(LibName)]
    public static partial void gtk_header_bar_pack_start(nint bar, nint child);

    [LibraryImport(LibName)]
    public static partial void gtk_header_bar_pack_end(nint bar, nint child);

    [LibraryImport(LibName)]
    public static partial void gtk_header_bar_set_title_widget(nint bar, nint titleWidget);

    // Window controls (the minimize/maximize/close cluster) - gtk_header_bar_set_show_title_buttons
    // draws these with generic symbolic icons on every platform; building our own GtkWindowControls
    // and opting into use-native-controls (GTK >= 4.18) instead renders genuine platform chrome
    // (macOS traffic lights, Windows 11 controls) where the backend supports it.

    [LibraryImport(LibName)]
    public static partial nint gtk_window_controls_new(GtkPackType side);

    [LibraryImport(LibName)]
    public static partial void gtk_window_controls_set_use_native_controls(nint controls, [MarshalAs(UnmanagedType.Bool)] bool setting);

    // Box layout

    [LibraryImport(LibName)]
    public static partial nint gtk_box_new(GtkOrientation orientation, int spacing);

    [LibraryImport(LibName)]
    public static partial void gtk_box_append(nint box, nint child);

    [LibraryImport(LibName)]
    public static partial void gtk_box_remove(nint box, nint child);

    // Label

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial nint gtk_label_new(string? text);

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial void gtk_label_set_markup(nint label, string markup);

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial void gtk_label_set_text(nint label, string text);

    [LibraryImport(LibName)]
    public static partial void gtk_label_set_wrap(nint label, [MarshalAs(UnmanagedType.Bool)] bool wrap);

    [LibraryImport(LibName)]
    public static partial void gtk_label_set_xalign(nint label, float xalign);

    [LibraryImport(LibName)]
    public static partial void gtk_label_set_selectable(nint label, [MarshalAs(UnmanagedType.Bool)] bool selectable);

    // Buttons

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial nint gtk_button_new_from_icon_name(string iconName);

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial nint gtk_button_new_with_label(string label);

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial void gtk_button_set_label(nint button, string label);

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial nint gtk_toggle_button_new_with_label(string label);

    [LibraryImport(LibName)]
    public static partial void gtk_toggle_button_set_active(nint button, [MarshalAs(UnmanagedType.Bool)] bool active);

    [LibraryImport(LibName)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool gtk_toggle_button_get_active(nint button);

    [LibraryImport(LibName)]
    public static partial void gtk_toggle_button_set_group(nint button, nint groupSource);

    // Search entry

    [LibraryImport(LibName)]
    public static partial nint gtk_search_entry_new();

    // Editable (implemented by GtkSearchEntry/GtkEntry)

    /// <summary>Returns a pointer owned by the widget - copy it out (e.g. via
    /// Marshal.PtrToStringUTF8) immediately; do not free it.</summary>
    [LibraryImport(LibName)]
    public static partial nint gtk_editable_get_text(nint editable);

    // Paned (the draggable-divider two-pane container)

    [LibraryImport(LibName)]
    public static partial nint gtk_paned_new(GtkOrientation orientation);

    [LibraryImport(LibName)]
    public static partial void gtk_paned_set_start_child(nint paned, nint child);

    [LibraryImport(LibName)]
    public static partial void gtk_paned_set_end_child(nint paned, nint child);

    [LibraryImport(LibName)]
    public static partial void gtk_paned_set_position(nint paned, int position);

    [LibraryImport(LibName)]
    public static partial void gtk_paned_set_resize_start_child(nint paned, [MarshalAs(UnmanagedType.Bool)] bool resize);

    [LibraryImport(LibName)]
    public static partial void gtk_paned_set_resize_end_child(nint paned, [MarshalAs(UnmanagedType.Bool)] bool resize);

    [LibraryImport(LibName)]
    public static partial void gtk_paned_set_shrink_start_child(nint paned, [MarshalAs(UnmanagedType.Bool)] bool shrink);

    [LibraryImport(LibName)]
    public static partial void gtk_paned_set_shrink_end_child(nint paned, [MarshalAs(UnmanagedType.Bool)] bool shrink);

    // Scrolled window

    [LibraryImport(LibName)]
    public static partial nint gtk_scrolled_window_new();

    [LibraryImport(LibName)]
    public static partial void gtk_scrolled_window_set_child(nint scrolledWindow, nint child);

    [LibraryImport(LibName)]
    public static partial void gtk_scrolled_window_set_policy(nint scrolledWindow, GtkPolicyType hPolicy, GtkPolicyType vPolicy);

    // List box

    [LibraryImport(LibName)]
    public static partial nint gtk_list_box_new();

    [LibraryImport(LibName)]
    public static partial void gtk_list_box_append(nint listBox, nint child);

    [LibraryImport(LibName)]
    public static partial void gtk_list_box_remove(nint listBox, nint child);

    [LibraryImport(LibName)]
    public static partial void gtk_list_box_set_selection_mode(nint listBox, GtkSelectionMode mode);

    [LibraryImport(LibName)]
    public static partial void gtk_list_box_set_activate_on_single_click(nint listBox, [MarshalAs(UnmanagedType.Bool)] bool single);

    /// <summary>Returns the child widget of a GtkListBoxRow* (as received by "row-activated").</summary>
    [LibraryImport(LibName)]
    public static partial nint gtk_list_box_row_get_child(nint row);

    /// <summary>Programmatically selects (and visually highlights) a GtkListBoxRow*, the same
    /// highlight a user click already produces.</summary>
    [LibraryImport(LibName)]
    public static partial void gtk_list_box_select_row(nint listBox, nint row);

    /// <summary>Returns the currently selected GtkListBoxRow*, or 0 if none (Browse selection mode
    /// only ever has zero or one).</summary>
    [LibraryImport(LibName)]
    public static partial nint gtk_list_box_get_selected_row(nint listBox);

    // Generic widget properties

    [LibraryImport(LibName)]
    public static partial void gtk_widget_set_halign(nint widget, GtkAlign align);

    [LibraryImport(LibName)]
    public static partial void gtk_widget_set_valign(nint widget, GtkAlign align);

    [LibraryImport(LibName)]
    public static partial void gtk_widget_set_hexpand(nint widget, [MarshalAs(UnmanagedType.Bool)] bool expand);

    [LibraryImport(LibName)]
    public static partial void gtk_widget_set_vexpand(nint widget, [MarshalAs(UnmanagedType.Bool)] bool expand);

    [LibraryImport(LibName)]
    public static partial void gtk_widget_set_size_request(nint widget, int width, int height);

    [LibraryImport(LibName)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool gtk_widget_grab_focus(nint widget);

    [LibraryImport(LibName)]
    public static partial void gtk_widget_set_margin_start(nint widget, int margin);

    [LibraryImport(LibName)]
    public static partial void gtk_widget_set_margin_end(nint widget, int margin);

    [LibraryImport(LibName)]
    public static partial void gtk_widget_set_margin_top(nint widget, int margin);

    [LibraryImport(LibName)]
    public static partial void gtk_widget_set_margin_bottom(nint widget, int margin);

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial void gtk_widget_add_css_class(nint widget, string cssClass);

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial void gtk_widget_remove_css_class(nint widget, string cssClass);

    [LibraryImport(LibName)]
    public static partial nint gtk_widget_get_first_child(nint widget);

    [LibraryImport(LibName)]
    public static partial nint gtk_widget_get_next_sibling(nint widget);

    [LibraryImport(LibName)]
    public static partial nint gtk_widget_get_prev_sibling(nint widget);

    [LibraryImport(LibName)]
    public static partial void gtk_widget_set_visible(nint widget, [MarshalAs(UnmanagedType.Bool)] bool visible);

    [LibraryImport(LibName)]
    public static partial void gtk_widget_set_sensitive(nint widget, [MarshalAs(UnmanagedType.Bool)] bool sensitive);

    // CSS / styling

    [LibraryImport(LibName)]
    public static partial nint gtk_css_provider_new();

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial void gtk_css_provider_load_from_string(nint provider, string data);

    [LibraryImport(LibName)]
    public static partial void gtk_style_context_add_provider_for_display(nint display, nint provider, uint priority);

    [LibraryImport(LibName)]
    public static partial nint gdk_display_get_default();

    public const uint GTK_STYLE_PROVIDER_PRIORITY_APPLICATION = 600;
}
