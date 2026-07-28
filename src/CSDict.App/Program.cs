using System.Globalization;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using CSDict.App.Data;
using CSDict.Gtk;
using CSDict.Sqlite;

namespace CSDict.App;

internal static unsafe class Program
{
    /// <summary>Builds the full app stylesheet for whichever palette matches the OS's current
    /// light/dark preference (see SystemTheme.PrefersDark) - covers every widget kind the app
    /// actually uses (window, headerbar, plain/toggle buttons, the search entry, and the word-list
    /// GtkListBox/rows) so nothing is left showing the platform theme's default light chrome
    /// against our own dark (or light) background.</summary>
    private static string BuildCss(bool dark)
    {
        (string bg, string bgAlt, string bgAltHover, string fg, string fgDim, string fgHeading, string fgTranslation, string border) = dark
            ? ("#2a292c", "#3a393d", "#48474c", "#e8e7ea", "#98979b", "#cfced2", "#f5f4f7", "#201f22")
            : ("#f5f4f7", "#e7e6ea", "#dad9de", "#232227", "#6b6a6e", "#141316", "#0c0b0d", "#d3d2d6");

        return $$"""
            window {
                background-color: {{bg}};
                background-image: none;
                color: {{fg}};
            }
            headerbar {
                background-color: {{bg}};
                background-image: none;
                color: {{fg}};
                box-shadow: none;
            }
            button, entry {
                background-color: {{bgAlt}};
                background-image: none;
                color: {{fg}};
                border-color: {{border}};
                box-shadow: none;
                text-shadow: none;
            }
            button:hover, entry:hover {
                background-color: {{bgAltHover}};
                background-image: none;
            }
            button.csdict-checked {
                background-color: #3584e4;
                background-image: none;
                color: #ffffff;
            }
            list {
                background-color: {{bg}};
                color: {{fg}};
            }
            list row {
                background-color: {{bg}};
                color: {{fg}};
            }
            list row:hover {
                background-color: {{bgAlt}};
            }
            label.csdict-placeholder {
                color: {{fgDim}};
                font-size: 15px;
            }
            label.csdict-heading {
                color: {{fgHeading}};
            }
            label.csdict-translation {
                color: {{fgTranslation}};
            }
            button.csdict-a-small label {
                font-size: 12px;
            }
            button.csdict-a-large label {
                font-size: 17px;
            }
            """;
    }

    private const int MaxSimilarWords = 60;

    private static DictionaryCatalog? s_catalog;
    private static WordIndex? s_wordIndex;
    private static string? s_activeSourceFilter;
    private static (string Lemma, string Lang)? s_selected;
    private static string s_dictionariesDir = "";
    private static string s_downloadTempDir = "";

    private static nint s_mainWindow;
    private static nint s_searchEntry;
    private static nint s_dictionaryButton;
    private static nint s_tabsHost;
    private static nint s_wordList;
    private static nint s_resultsBox;

    private static nint s_dictionaryDialog;
    private static nint s_dialogList;
    private static nint s_dialogStatusLabel;
    private static nint s_dialogDownloadButton;
    private static nint s_dialogRemoveButton;
    private static nint s_dialogFromLangDropDown;
    private static nint s_dialogToLangDropDown;
    private static bool s_dialogOpen;
    private static bool s_downloadInProgress;
    private static DictionaryDirection? s_dialogSelected;

    private static readonly Dictionary<nint, (string Lemma, string Lang)> s_rowData = new();
    private static readonly Dictionary<nint, string?> s_sourceButtons = new();
    private static readonly Dictionary<nint, DictionaryDirection> s_dictionaryDialogRows = new();
    private static readonly Dictionary<(string LemmaLang, string TargetLang), nint> s_dictionaryDialogRowsByDirection = new();
    private static string[] s_dialogFromLangs = [];
    private static string[] s_dialogToLangs = [];

    [STAThread]
    private static int Main()
    {
        RegisterNativeLibraryResolver();
        SqliteNativeResolver.Register();

        s_dictionariesDir = Path.Combine(AppContext.BaseDirectory, "Dictionaries");
        s_downloadTempDir = Path.Combine(AppContext.BaseDirectory, "Cache");
        s_catalog = DictionaryCatalog.Load(s_dictionariesDir);
        s_wordIndex = WordIndex.Build(s_catalog);

        nint app = Gtk4.gtk_application_new("dev.csdict.CSDict", 0);
        GObject.Connect(app, "activate", &OnActivate);

        int status = Gio.g_application_run(app, 0, 0);
        GObject.g_object_unref(app);
        return status;
    }

    private static void RegisterNativeLibraryResolver()
    {
        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            CSDict.Gtk.Native.Windows.NativeLibraryResolver.Register();
        }
        else if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
        {
            CSDict.Gtk.Native.macOS.NativeLibraryResolver.Register();
        }
        else if (RuntimeInformation.IsOSPlatform(OSPlatform.Linux))
        {
            CSDict.Gtk.Native.Linux.NativeLibraryResolver.Register();
        }
        else
        {
            throw new PlatformNotSupportedException(
                $"No CSDict.Gtk native library resolver is available for {RuntimeInformation.OSDescription}.");
        }
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnActivate(nint application, nint userData)
    {
        ApplyStyles();

        nint window = Gtk4.gtk_application_window_new(application);
        s_mainWindow = window;
        Gtk4.gtk_window_set_default_size(window, 760, 520);
        Gtk4.gtk_window_set_titlebar(window, BuildHeaderBar());
        Gtk4.gtk_window_set_child(window, BuildContent());
        Gtk4.gtk_window_present(window);
        Gtk4.gtk_widget_grab_focus(s_searchEntry);
    }

    private static nint BuildHeaderBar()
    {
        nint headerBar = Gtk4.gtk_header_bar_new();
        // Rather than gtk_header_bar_set_show_title_buttons (which always draws the
        // minimize/maximize/close cluster with generic symbolic icons, the same on every OS), build
        // our own GtkWindowControls and opt into native rendering - GTK >= 4.18 then draws genuine
        // macOS traffic lights / Windows 11 controls where the backend supports it, on whichever
        // side that platform's window-manager convention puts them (macOS: start: everywhere else:
        // end), leaving the other side empty automatically.
        Gtk4.gtk_header_bar_set_show_title_buttons(headerBar, false);

        nint controlsStart = Gtk4.gtk_window_controls_new(GtkPackType.Start);
        Gtk4.gtk_window_controls_set_use_native_controls(controlsStart, true);
        Gtk4.gtk_header_bar_pack_start(headerBar, controlsStart);

        nint navBox = Gtk4.gtk_box_new(GtkOrientation.Horizontal, 0);
        Gtk4.gtk_widget_add_css_class(navBox, "linked");
        nint prevButton = Gtk4.gtk_button_new_from_icon_name("go-previous-symbolic");
        GObject.Connect(prevButton, "clicked", &OnPrevWordClicked);
        nint nextButton = Gtk4.gtk_button_new_from_icon_name("go-next-symbolic");
        GObject.Connect(nextButton, "clicked", &OnNextWordClicked);
        Gtk4.gtk_box_append(navBox, prevButton);
        Gtk4.gtk_box_append(navBox, nextButton);
        Gtk4.gtk_header_bar_pack_start(headerBar, navBox);

        s_dictionaryButton = Gtk4.gtk_button_new_with_label("D");
        GObject.Connect(s_dictionaryButton, "clicked", &OnDictionaryButtonClicked);
        Gtk4.gtk_header_bar_pack_start(headerBar, s_dictionaryButton);

        nint title = Gtk4.gtk_label_new(null);
        Gtk4.gtk_label_set_markup(title, "<b>CSDict</b>");
        Gtk4.gtk_header_bar_set_title_widget(headerBar, title);

        nint fontBox = Gtk4.gtk_box_new(GtkOrientation.Horizontal, 0);
        Gtk4.gtk_widget_add_css_class(fontBox, "linked");
        nint smallA = Gtk4.gtk_toggle_button_new_with_label("A");
        Gtk4.gtk_widget_add_css_class(smallA, "csdict-a-small");
        GObject.Connect(smallA, "toggled", &OnCheckedToggleChanged);
        nint largeA = Gtk4.gtk_toggle_button_new_with_label("A");
        Gtk4.gtk_widget_add_css_class(largeA, "csdict-a-large");
        GObject.Connect(largeA, "toggled", &OnCheckedToggleChanged);
        Gtk4.gtk_toggle_button_set_group(largeA, smallA);
        Gtk4.gtk_toggle_button_set_active(largeA, true);
        Gtk4.gtk_widget_add_css_class(largeA, "csdict-checked");
        Gtk4.gtk_box_append(fontBox, smallA);
        Gtk4.gtk_box_append(fontBox, largeA);

        s_searchEntry = Gtk4.gtk_search_entry_new();
        Gtk4.gtk_widget_set_size_request(s_searchEntry, 220, -1);
        GObject.Connect(s_searchEntry, "search-changed", &OnSearchChanged);
        GObject.Connect(s_searchEntry, "activate", &OnSearchActivate);

        nint controlsEnd = Gtk4.gtk_window_controls_new(GtkPackType.End);
        Gtk4.gtk_window_controls_set_use_native_controls(controlsEnd, true);
        Gtk4.gtk_header_bar_pack_end(headerBar, controlsEnd);

        nint endBox = Gtk4.gtk_box_new(GtkOrientation.Horizontal, 8);
        Gtk4.gtk_box_append(endBox, fontBox);
        Gtk4.gtk_box_append(endBox, s_searchEntry);
        Gtk4.gtk_header_bar_pack_end(headerBar, endBox);

        return headerBar;
    }

    private static nint BuildContent()
    {
        nint root = Gtk4.gtk_box_new(GtkOrientation.Vertical, 0);
        s_tabsHost = Gtk4.gtk_box_new(GtkOrientation.Vertical, 0);
        Gtk4.gtk_box_append(s_tabsHost, BuildSourceTabs());
        Gtk4.gtk_box_append(root, s_tabsHost);
        Gtk4.gtk_box_append(root, BuildLookupPane());
        return root;
    }

    /// <summary>Re-derives the catalog/word index from whatever is on disk under Dictionaries/ -
    /// called after a download or a removal in the "D" dialog, since either can add or remove a
    /// source. Also rebuilds the header source tabs, since the set of sources may have changed.</summary>
    private static void ReloadCatalog()
    {
        s_catalog?.Dispose();
        s_catalog = DictionaryCatalog.Load(s_dictionariesDir);
        s_wordIndex = WordIndex.Build(s_catalog);
        s_activeSourceFilter = null;
        RefreshSourceTabs();
        RefreshSimilarWords();
        RenderResults();
    }

    private static void RefreshSourceTabs()
    {
        nint child;
        while ((child = Gtk4.gtk_widget_get_first_child(s_tabsHost)) != 0)
        {
            Gtk4.gtk_box_remove(s_tabsHost, child);
        }

        s_sourceButtons.Clear();
        Gtk4.gtk_box_append(s_tabsHost, BuildSourceTabs());
    }

    /// <summary>One toggle per distinct dictionary source actually discovered under Dictionaries/,
    /// plus "All" (default) - replaces the previously hardcoded, fake source name list.</summary>
    private static nint BuildSourceTabs()
    {
        nint tabs = Gtk4.gtk_box_new(GtkOrientation.Horizontal, 0);
        Gtk4.gtk_widget_add_css_class(tabs, "linked");
        Gtk4.gtk_widget_set_margin_start(tabs, 12);
        Gtk4.gtk_widget_set_margin_end(tabs, 12);
        Gtk4.gtk_widget_set_margin_top(tabs, 10);
        Gtk4.gtk_widget_set_margin_bottom(tabs, 10);
        Gtk4.gtk_widget_set_halign(tabs, GtkAlign.Start);

        string[] labels = new[] { "All" }.Concat(s_catalog!.Sources).ToArray();
        nint firstTab = 0;
        foreach (string label in labels)
        {
            nint tab = Gtk4.gtk_toggle_button_new_with_label(label);
            if (firstTab == 0)
            {
                firstTab = tab;
            }
            else
            {
                Gtk4.gtk_toggle_button_set_group(tab, firstTab);
            }

            bool isAll = label == "All";
            Gtk4.gtk_toggle_button_set_active(tab, isAll);
            if (isAll)
            {
                Gtk4.gtk_widget_add_css_class(tab, "csdict-checked");
            }

            s_sourceButtons[tab] = isAll ? null : label;
            GObject.Connect(tab, "toggled", &OnSourceToggled);
            GObject.Connect(tab, "toggled", &OnCheckedToggleChanged);
            Gtk4.gtk_box_append(tabs, tab);
        }

        return tabs;
    }

    /// <summary>The moveable-slider two-pane layout: similar words on the left (~20%), the
    /// selected word's translations - aggregated across every dictionary, both directions - on
    /// the right (~80%).</summary>
    private static nint BuildLookupPane()
    {
        nint paned = Gtk4.gtk_paned_new(GtkOrientation.Horizontal);
        Gtk4.gtk_widget_set_hexpand(paned, true);
        Gtk4.gtk_widget_set_vexpand(paned, true);
        Gtk4.gtk_paned_set_position(paned, 160);
        Gtk4.gtk_paned_set_resize_start_child(paned, false);
        Gtk4.gtk_paned_set_shrink_start_child(paned, false);
        Gtk4.gtk_paned_set_resize_end_child(paned, true);
        Gtk4.gtk_paned_set_shrink_end_child(paned, false);

        s_wordList = Gtk4.gtk_list_box_new();
        Gtk4.gtk_list_box_set_selection_mode(s_wordList, GtkSelectionMode.Browse);
        Gtk4.gtk_list_box_set_activate_on_single_click(s_wordList, true);
        GObject.Connect(s_wordList, "row-activated", &OnRowActivated);

        nint leftScroller = Gtk4.gtk_scrolled_window_new();
        Gtk4.gtk_scrolled_window_set_policy(leftScroller, GtkPolicyType.Never, GtkPolicyType.Automatic);
        Gtk4.gtk_scrolled_window_set_child(leftScroller, s_wordList);
        Gtk4.gtk_paned_set_start_child(paned, leftScroller);

        s_resultsBox = Gtk4.gtk_box_new(GtkOrientation.Vertical, 10);
        Gtk4.gtk_widget_set_margin_start(s_resultsBox, 14);
        Gtk4.gtk_widget_set_margin_end(s_resultsBox, 14);
        Gtk4.gtk_widget_set_margin_top(s_resultsBox, 10);
        Gtk4.gtk_widget_set_margin_bottom(s_resultsBox, 10);
        ShowEmptyState(NoSelectionMessage());

        nint rightScroller = Gtk4.gtk_scrolled_window_new();
        Gtk4.gtk_scrolled_window_set_policy(rightScroller, GtkPolicyType.Never, GtkPolicyType.Automatic);
        Gtk4.gtk_scrolled_window_set_child(rightScroller, s_resultsBox);
        Gtk4.gtk_paned_set_end_child(paned, rightScroller);

        return paned;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnSearchChanged(nint editable, nint userData) => RefreshSimilarWords();

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnSearchActivate(nint editable, nint userData) => SelectFirstSimilarWord();

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnRowActivated(nint listBox, nint row, nint userData)
    {
        nint child = Gtk4.gtk_list_box_row_get_child(row);
        if (s_rowData.TryGetValue(child, out (string Lemma, string Lang) word))
        {
            SelectWord(word.Lemma, word.Lang);
        }
    }

    /// <summary>Keeps a "csdict-checked" class in sync with a GtkToggleButton's active state, so our
    /// own CSS (see BuildCss) can style the selected tab/font-size button reliably instead of
    /// depending on the ":checked" pseudo-class, which - unlike ":hover" or ":backdrop" - did not
    /// visibly take effect against this app's stylesheet in testing.</summary>
    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnCheckedToggleChanged(nint button, nint userData)
    {
        if (Gtk4.gtk_toggle_button_get_active(button))
        {
            Gtk4.gtk_widget_add_css_class(button, "csdict-checked");
        }
        else
        {
            Gtk4.gtk_widget_remove_css_class(button, "csdict-checked");
        }
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnSourceToggled(nint button, nint userData)
    {
        if (!Gtk4.gtk_toggle_button_get_active(button) || !s_sourceButtons.TryGetValue(button, out string? source))
        {
            return;
        }

        s_activeSourceFilter = source;
        RenderResults();
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnDictionaryButtonClicked(nint button, nint userData) => ShowDictionaryDialog();

    private const string AllLanguagesOption = "All";

    /// <summary>Two GtkDropDowns ("From"/"To" language) that filter the direction list below down
    /// to whichever source/destination language is picked - "All" (the first entry in each list)
    /// leaves that axis unfiltered, matching the one-file-per-direction model. "From" defaults to
    /// the OS's current UI language instead of "All" when that language is one of the available
    /// lemma languages, since that's overwhelmingly the language the user wants to look words up
    /// from.</summary>
    private static nint BuildLanguagePickerRow()
    {
        s_dialogFromLangs = [AllLanguagesOption, .. DictionaryDirections.LemmaLangs];
        s_dialogToLangs = [AllLanguagesOption, .. DictionaryDirections.TargetLangs];

        uint defaultFromIndex = 0;
        string osLemmaLang = CultureInfo.CurrentUICulture.TwoLetterISOLanguageName;
        int osLemmaLangIndex = Array.IndexOf(s_dialogFromLangs, osLemmaLang);
        if (osLemmaLangIndex > 0)
        {
            defaultFromIndex = (uint)osLemmaLangIndex;
        }

        nint row = Gtk4.gtk_box_new(GtkOrientation.Horizontal, 6);

        nint fromLabel = Gtk4.gtk_label_new("From");
        Gtk4.gtk_widget_add_css_class(fromLabel, "csdict-placeholder");
        Gtk4.gtk_box_append(row, fromLabel);

        s_dialogFromLangDropDown = Gtk4.gtk_drop_down_new(BuildStringList(s_dialogFromLangs), 0);
        Gtk4.gtk_widget_set_hexpand(s_dialogFromLangDropDown, true);
        if (defaultFromIndex != 0)
        {
            Gtk4.gtk_drop_down_set_selected(s_dialogFromLangDropDown, defaultFromIndex);
        }

        GObject.Connect(s_dialogFromLangDropDown, "notify::selected", &OnLanguagePickerChanged);
        Gtk4.gtk_box_append(row, s_dialogFromLangDropDown);

        nint toLabel = Gtk4.gtk_label_new("To");
        Gtk4.gtk_widget_add_css_class(toLabel, "csdict-placeholder");
        Gtk4.gtk_box_append(row, toLabel);

        s_dialogToLangDropDown = Gtk4.gtk_drop_down_new(BuildStringList(s_dialogToLangs), 0);
        Gtk4.gtk_widget_set_hexpand(s_dialogToLangDropDown, true);
        GObject.Connect(s_dialogToLangDropDown, "notify::selected", &OnLanguagePickerChanged);
        Gtk4.gtk_box_append(row, s_dialogToLangDropDown);

        return row;
    }

    private static nint BuildStringList(IReadOnlyList<string> items)
    {
        nint list = Gtk4.gtk_string_list_new(0);
        foreach (string item in items)
        {
            Gtk4.gtk_string_list_append(list, item);
        }

        return list;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnLanguagePickerChanged(nint dropDown, nint pspec, nint userData) => ApplyDictionaryDialogFilter();

    /// <summary>Shows/hides each direction's row to match the "From"/"To" dropdowns - "All" (index
    /// 0) leaves that axis unfiltered. If the currently selected row gets filtered out, falls back
    /// to the first row that's still visible (or none, clearing the selection) so the
    /// Download/Remove buttons never point at a hidden row.</summary>
    private static void ApplyDictionaryDialogFilter()
    {
        uint fromIndex = Gtk4.gtk_drop_down_get_selected(s_dialogFromLangDropDown);
        uint toIndex = Gtk4.gtk_drop_down_get_selected(s_dialogToLangDropDown);
        if (fromIndex == Gtk4.GTK_INVALID_LIST_POSITION || toIndex == Gtk4.GTK_INVALID_LIST_POSITION)
        {
            return;
        }

        string? lemmaLang = fromIndex == 0 ? null : s_dialogFromLangs[fromIndex];
        string? targetLang = toIndex == 0 ? null : s_dialogToLangs[toIndex];

        nint firstVisibleRow = 0;
        DictionaryDirection? firstVisibleDirection = null;
        bool selectedStillVisible = false;
        foreach (DictionaryDirection direction in DictionaryDirections.All)
        {
            if (!s_dictionaryDialogRowsByDirection.TryGetValue((direction.LemmaLang, direction.TargetLang), out nint row))
            {
                continue;
            }

            bool matches = (lemmaLang is null || direction.LemmaLang == lemmaLang)
                && (targetLang is null || direction.TargetLang == targetLang);
            Gtk4.gtk_widget_set_visible(row, matches);
            if (!matches)
            {
                continue;
            }

            if (firstVisibleDirection is null)
            {
                firstVisibleRow = row;
                firstVisibleDirection = direction;
            }

            if (direction == s_dialogSelected)
            {
                selectedStillVisible = true;
            }
        }

        if (!selectedStillVisible)
        {
            s_dialogSelected = firstVisibleDirection;
            if (firstVisibleDirection is not null)
            {
                Gtk4.gtk_list_box_select_row(s_dialogList, firstVisibleRow);
            }

            UpdateDialogActionState();
        }
    }

    /// <summary>A small modal library manager: lists every dictionary direction this app knows how
    /// to fetch (each merging every source that covers it into a single file - see
    /// docs/design/scraper-and-distribution.md), whether or not it's currently downloaded, and lets
    /// the user download a missing one or remove one that's already on disk. Two comboboxes above
    /// the list let you jump straight to a direction by picking its "from"/"to" language instead of
    /// scanning the list.</summary>
    private static void ShowDictionaryDialog()
    {
        nint dialog = Gtk4.gtk_window_new();
        s_dictionaryDialog = dialog;
        s_dialogOpen = true;
        s_dialogSelected = null;
        GObject.Connect(dialog, "destroy", &OnDictionaryDialogDestroyed);
        Gtk4.gtk_window_set_title(dialog, "Dictionaries");
        Gtk4.gtk_window_set_transient_for(dialog, s_mainWindow);
        Gtk4.gtk_window_set_modal(dialog, true);
        Gtk4.gtk_window_set_default_size(dialog, 340, 460);

        nint container = Gtk4.gtk_box_new(GtkOrientation.Vertical, 8);
        Gtk4.gtk_widget_set_margin_start(container, 10);
        Gtk4.gtk_widget_set_margin_end(container, 10);
        Gtk4.gtk_widget_set_margin_top(container, 10);
        Gtk4.gtk_widget_set_margin_bottom(container, 10);

        Gtk4.gtk_box_append(container, BuildLanguagePickerRow());

        s_dialogList = Gtk4.gtk_list_box_new();
        Gtk4.gtk_list_box_set_selection_mode(s_dialogList, GtkSelectionMode.Browse);
        Gtk4.gtk_list_box_set_activate_on_single_click(s_dialogList, true);
        GObject.Connect(s_dialogList, "row-activated", &OnDictionaryDialogRowActivated);

        s_dictionaryDialogRows.Clear();
        s_dictionaryDialogRowsByDirection.Clear();
        foreach (DictionaryDirection direction in DictionaryDirections.All)
        {
            AppendDictionaryDialogRow(direction);
        }

        // GTK auto-selects the first row for a Browse-mode list box as soon as it has children,
        // but that doesn't raise "row-activated" - so the action-button state has to be primed
        // manually to match, or Download/Remove stay hidden until the user clicks the (already
        // visibly selected) first row themselves. This also applies whatever filter the dropdowns'
        // (possibly non-"All") defaults imply, hiding rows and selecting the first one left visible.
        ApplyDictionaryDialogFilter();

        nint scroller = Gtk4.gtk_scrolled_window_new();
        Gtk4.gtk_scrolled_window_set_policy(scroller, GtkPolicyType.Never, GtkPolicyType.Automatic);
        Gtk4.gtk_scrolled_window_set_child(scroller, s_dialogList);
        Gtk4.gtk_widget_set_vexpand(scroller, true);
        Gtk4.gtk_box_append(container, scroller);

        s_dialogStatusLabel = Gtk4.gtk_label_new("Select a dictionary above.");
        Gtk4.gtk_label_set_xalign(s_dialogStatusLabel, 0);
        Gtk4.gtk_label_set_wrap(s_dialogStatusLabel, true);
        Gtk4.gtk_widget_add_css_class(s_dialogStatusLabel, "csdict-placeholder");
        Gtk4.gtk_box_append(container, s_dialogStatusLabel);

        nint actionBox = Gtk4.gtk_box_new(GtkOrientation.Horizontal, 8);
        s_dialogDownloadButton = Gtk4.gtk_button_new_with_label("Download");
        GObject.Connect(s_dialogDownloadButton, "clicked", &OnDownloadButtonClicked);
        Gtk4.gtk_widget_set_visible(s_dialogDownloadButton, false);
        Gtk4.gtk_box_append(actionBox, s_dialogDownloadButton);

        s_dialogRemoveButton = Gtk4.gtk_button_new_with_label("Remove");
        GObject.Connect(s_dialogRemoveButton, "clicked", &OnRemoveButtonClicked);
        Gtk4.gtk_widget_set_visible(s_dialogRemoveButton, false);
        Gtk4.gtk_box_append(actionBox, s_dialogRemoveButton);
        Gtk4.gtk_box_append(container, actionBox);

        Gtk4.gtk_window_set_child(dialog, container);
        UpdateDialogActionState();
        Gtk4.gtk_window_present(dialog);
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnDictionaryDialogDestroyed(nint window, nint userData) => s_dialogOpen = false;

    private static void AppendDictionaryDialogRow(DictionaryDirection direction)
    {
        nint label = Gtk4.gtk_label_new(DictionaryRowText(direction));
        Gtk4.gtk_label_set_xalign(label, 0);
        Gtk4.gtk_widget_set_margin_start(label, 8);
        Gtk4.gtk_widget_set_margin_end(label, 8);
        Gtk4.gtk_widget_set_margin_top(label, 6);
        Gtk4.gtk_widget_set_margin_bottom(label, 6);
        s_dictionaryDialogRows[label] = direction;
        Gtk4.gtk_list_box_append(s_dialogList, label);
        s_dictionaryDialogRowsByDirection[(direction.LemmaLang, direction.TargetLang)] = GetLastChild(s_dialogList);
    }

    /// <summary>The listbox's own children are the GtkListBoxRow wrappers GtkListBox creates
    /// around each appended child - gtk_list_box_append itself returns void, so the row for the
    /// child just appended is simply whichever one is currently last.</summary>
    private static nint GetLastChild(nint widget)
    {
        nint child = Gtk4.gtk_widget_get_first_child(widget);
        if (child == 0)
        {
            return 0;
        }

        nint next;
        while ((next = Gtk4.gtk_widget_get_next_sibling(child)) != 0)
        {
            child = next;
        }

        return child;
    }

    private static string DictionaryRowText(DictionaryDirection direction) =>
        $"{direction.DisplayName} — {(File.Exists(GetDictionaryPath(direction)) ? "Downloaded" : "Not downloaded")}";

    private static string GetDictionaryPath(DictionaryDirection direction) => Path.Combine(s_dictionariesDir, direction.FileName);

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnDictionaryDialogRowActivated(nint listBox, nint row, nint userData)
    {
        if (s_downloadInProgress)
        {
            return;
        }

        nint child = Gtk4.gtk_list_box_row_get_child(row);
        if (!s_dictionaryDialogRows.TryGetValue(child, out DictionaryDirection? def))
        {
            return;
        }

        s_dialogSelected = def;
        UpdateDialogActionState();
    }

    private static void UpdateDialogActionState()
    {
        if (s_dialogSelected is not { } def)
        {
            Gtk4.gtk_widget_set_visible(s_dialogDownloadButton, false);
            Gtk4.gtk_widget_set_visible(s_dialogRemoveButton, false);
            return;
        }

        bool present = File.Exists(GetDictionaryPath(def));
        Gtk4.gtk_widget_set_visible(s_dialogDownloadButton, !present);
        Gtk4.gtk_widget_set_visible(s_dialogRemoveButton, present);
        Gtk4.gtk_widget_set_sensitive(s_dialogDownloadButton, !s_downloadInProgress);
        Gtk4.gtk_widget_set_sensitive(s_dialogRemoveButton, !s_downloadInProgress);
        if (!s_downloadInProgress)
        {
            Gtk4.gtk_label_set_text(s_dialogStatusLabel, $"{def.DisplayName}: {(present ? "downloaded" : "not downloaded")}");
        }
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnDownloadButtonClicked(nint button, nint userData)
    {
        if (s_downloadInProgress || s_dialogSelected is not { } def)
        {
            return;
        }

        s_downloadInProgress = true;
        Gtk4.gtk_widget_set_sensitive(s_dialogDownloadButton, false);
        Gtk4.gtk_widget_set_sensitive(s_dialogRemoveButton, false);
        Gtk4.gtk_label_set_text(s_dialogStatusLabel, $"Starting download of {def.DisplayName}...");

        string outputPath = GetDictionaryPath(def);
        string downloadTempDir = s_downloadTempDir;
        var progress = new Progress<string>(message =>
            GLib.RunOnMainThread(() =>
            {
                if (s_dialogOpen)
                {
                    Gtk4.gtk_label_set_text(s_dialogStatusLabel, message);
                }
            }));

        DownloadRunner.Run(def, outputPath, downloadTempDir, progress, error => GLib.RunOnMainThread(() => OnDownloadFinished(def, error)));
    }

    private static void OnDownloadFinished(DictionaryDirection def, Exception? error)
    {
        s_downloadInProgress = false;
        if (error is null)
        {
            ReloadCatalog();
        }

        if (!s_dialogOpen)
        {
            return;
        }

        Gtk4.gtk_label_set_text(s_dialogStatusLabel, error is null
            ? $"{def.DisplayName}: downloaded."
            : $"Download failed: {error.Message}");
        RefreshDictionaryDialogRows();
        UpdateDialogActionState();
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnRemoveButtonClicked(nint button, nint userData)
    {
        if (s_downloadInProgress || s_dialogSelected is not { } def)
        {
            return;
        }

        string path = GetDictionaryPath(def);
        try
        {
            if (File.Exists(path))
            {
                File.Delete(path);
            }

            Gtk4.gtk_label_set_text(s_dialogStatusLabel, $"{def.DisplayName}: removed.");
        }
        catch (IOException ex)
        {
            Gtk4.gtk_label_set_text(s_dialogStatusLabel, $"Remove failed: {ex.Message}");
        }

        RefreshDictionaryDialogRows();
        UpdateDialogActionState();
        ReloadCatalog();
    }

    private static void RefreshDictionaryDialogRows()
    {
        foreach (nint label in s_dictionaryDialogRows.Keys)
        {
            Gtk4.gtk_label_set_text(label, DictionaryRowText(s_dictionaryDialogRows[label]));
        }
    }

    /// <summary>Queries both the Cs and En tries with the current search text - this is the
    /// "two-way" lookup: whichever language the user typed in, its trie yields matches.</summary>
    private static void RefreshSimilarWords()
    {
        string text = GetEditableText(s_searchEntry);
        ClearListBox();

        if (string.IsNullOrWhiteSpace(text))
        {
            return;
        }

        var matches = new List<(string Lemma, string Lang)>();
        foreach (string lang in s_wordIndex!.Languages)
        {
            foreach (string word in s_wordIndex.ForLang(lang)!.WordsWithPrefix(text, MaxSimilarWords))
            {
                matches.Add((word, lang));
            }
        }

        matches.Sort((a, b) => string.Compare(a.Lemma, b.Lemma, StringComparison.InvariantCultureIgnoreCase));
        if (matches.Count > MaxSimilarWords)
        {
            matches = matches.GetRange(0, MaxSimilarWords);
        }

        foreach ((string lemma, string lang) in matches)
        {
            AppendWordRow(lemma, lang);
        }
    }

    private static void SelectFirstSimilarWord()
    {
        nint firstChild = Gtk4.gtk_widget_get_first_child(s_wordList);
        if (firstChild == 0)
        {
            return;
        }

        nint label = Gtk4.gtk_list_box_row_get_child(firstChild);
        if (s_rowData.TryGetValue(label, out (string Lemma, string Lang) word))
        {
            Gtk4.gtk_list_box_select_row(s_wordList, firstChild);
            SelectWord(word.Lemma, word.Lang);
        }
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnPrevWordClicked(nint button, nint userData) => StepSelectedWord(-1);

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnNextWordClicked(nint button, nint userData) => StepSelectedWord(1);

    /// <summary>Moves the highlighted row in the similar-words list to its previous/next sibling
    /// (falling back to the first row if nothing is selected yet) and shows that word's
    /// translations - the header bar's back/forward buttons walk the list the same way arrow keys
    /// or a click would.</summary>
    private static void StepSelectedWord(int direction)
    {
        nint current = Gtk4.gtk_list_box_get_selected_row(s_wordList);
        nint target = current == 0
            ? Gtk4.gtk_widget_get_first_child(s_wordList)
            : direction < 0 ? Gtk4.gtk_widget_get_prev_sibling(current) : Gtk4.gtk_widget_get_next_sibling(current);

        if (target == 0)
        {
            return;
        }

        nint label = Gtk4.gtk_list_box_row_get_child(target);
        if (!s_rowData.TryGetValue(label, out (string Lemma, string Lang) word))
        {
            return;
        }

        Gtk4.gtk_list_box_select_row(s_wordList, target);
        SelectWord(word.Lemma, word.Lang);
    }

    private static void AppendWordRow(string lemma, string lang)
    {
        nint label = Gtk4.gtk_label_new($"{lemma}  ({lang})");
        Gtk4.gtk_label_set_xalign(label, 0);
        Gtk4.gtk_widget_set_margin_start(label, 8);
        Gtk4.gtk_widget_set_margin_end(label, 8);
        Gtk4.gtk_widget_set_margin_top(label, 4);
        Gtk4.gtk_widget_set_margin_bottom(label, 4);
        s_rowData[label] = (lemma, lang);
        Gtk4.gtk_list_box_append(s_wordList, label);
    }

    private static void ClearListBox()
    {
        s_rowData.Clear();
        nint child;
        while ((child = Gtk4.gtk_widget_get_first_child(s_wordList)) != 0)
        {
            Gtk4.gtk_list_box_remove(s_wordList, child);
        }
    }

    private static void SelectWord(string lemma, string lang)
    {
        s_selected = (lemma, lang);
        RenderResults();
    }

    /// <summary>Rebuilds the right pane for the current selection, unioning results from every
    /// dictionary in both directions (or just the active source-tab filter, if not "All").</summary>
    private static void RenderResults()
    {
        ClearResultsBox();

        if (s_selected is not { } selected)
        {
            ShowEmptyState(NoSelectionMessage());
            return;
        }

        nint heading = Gtk4.gtk_label_new(null);
        Gtk4.gtk_label_set_markup(heading,
            $"<span size='large' weight='bold'>{Escape(selected.Lemma)}</span>  <span alpha='60%'>({selected.Lang})</span>");
        Gtk4.gtk_label_set_xalign(heading, 0);
        Gtk4.gtk_widget_add_css_class(heading, "csdict-heading");
        Gtk4.gtk_box_append(s_resultsBox, heading);

        List<SourceResult> results = LookupService.GetTranslations(s_catalog!, selected.Lemma, selected.Lang, s_activeSourceFilter);
        if (results.Count == 0)
        {
            ShowEmptyState("No translations found in the selected dictionary.");
            return;
        }

        foreach (SourceResult source in results)
        {
            AppendSourceSection(source);
        }
    }

    private static void AppendSourceSection(SourceResult source)
    {
        nint sourceHeading = Gtk4.gtk_label_new(null);
        Gtk4.gtk_label_set_markup(sourceHeading, $"<b>{Escape(source.Source)}</b>");
        Gtk4.gtk_label_set_xalign(sourceHeading, 0);
        Gtk4.gtk_widget_set_margin_top(sourceHeading, 6);
        Gtk4.gtk_widget_add_css_class(sourceHeading, "csdict-heading");
        Gtk4.gtk_box_append(s_resultsBox, sourceHeading);

        foreach (EntryResult entry in source.Entries)
        {
            string meta = string.Join(" · ", new[] { entry.Pos, entry.Ipa, entry.Gender }.Where(v => !string.IsNullOrEmpty(v)));
            if (meta.Length > 0)
            {
                nint metaLabel = Gtk4.gtk_label_new(meta);
                Gtk4.gtk_label_set_xalign(metaLabel, 0);
                Gtk4.gtk_widget_add_css_class(metaLabel, "csdict-placeholder");
                Gtk4.gtk_box_append(s_resultsBox, metaLabel);
            }

            foreach (SenseResult sense in entry.Senses)
            {
                AppendSense(sense);
            }
        }
    }

    private static void AppendSense(SenseResult sense)
    {
        var parts = new List<string>();
        if (sense.Translations.Count > 0)
        {
            parts.Add(string.Join(", ", sense.Translations));
        }

        if (!string.IsNullOrEmpty(sense.Gloss))
        {
            parts.Add(sense.Gloss!);
        }

        nint senseLabel = Gtk4.gtk_label_new("• " + string.Join("  —  ", parts));
        Gtk4.gtk_label_set_wrap(senseLabel, true);
        Gtk4.gtk_label_set_xalign(senseLabel, 0);
        Gtk4.gtk_label_set_selectable(senseLabel, true);
        Gtk4.gtk_widget_add_css_class(senseLabel, "csdict-translation");
        Gtk4.gtk_box_append(s_resultsBox, senseLabel);

        foreach ((string sourceText, string targetText) in sense.Examples)
        {
            nint exampleLabel = Gtk4.gtk_label_new($"    {sourceText} — {targetText}");
            Gtk4.gtk_label_set_wrap(exampleLabel, true);
            Gtk4.gtk_label_set_xalign(exampleLabel, 0);
            Gtk4.gtk_widget_add_css_class(exampleLabel, "csdict-placeholder");
            Gtk4.gtk_box_append(s_resultsBox, exampleLabel);
        }
    }

    /// <summary>What to show in the results pane when no word is selected - guides a first-run
    /// user toward the "D" (Dictionaries) button instead of the generic search hint if there's
    /// nothing to search yet.</summary>
    private static string NoSelectionMessage() =>
        s_catalog!.Sources.Count == 0
            ? "Open Dictionaries to download a dictionary first."
            : "Type a word above to look it up.";

    private static void ShowEmptyState(string text)
    {
        nint label = Gtk4.gtk_label_new(text);
        Gtk4.gtk_widget_add_css_class(label, "csdict-placeholder");
        Gtk4.gtk_label_set_xalign(label, 0);
        Gtk4.gtk_box_append(s_resultsBox, label);
    }

    private static void ClearResultsBox()
    {
        nint child;
        while ((child = Gtk4.gtk_widget_get_first_child(s_resultsBox)) != 0)
        {
            Gtk4.gtk_box_remove(s_resultsBox, child);
        }
    }

    private static string GetEditableText(nint editable)
    {
        nint ptr = Gtk4.gtk_editable_get_text(editable);
        return ptr == 0 ? "" : Marshal.PtrToStringUTF8(ptr) ?? "";
    }

    private static string Escape(string s) => s.Replace("&", "&amp;").Replace("<", "&lt;").Replace(">", "&gt;");

    private static void ApplyStyles()
    {
        nint provider = Gtk4.gtk_css_provider_new();
        Gtk4.gtk_css_provider_load_from_string(provider, BuildCss(SystemTheme.PrefersDark()));
        Gtk4.gtk_style_context_add_provider_for_display(Gtk4.gdk_display_get_default(), provider, Gtk4.GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}

/// <summary>Kept outside Program (which is an `unsafe` class - `await` isn't allowed inside an
/// unsafe context) so the actual download can be a normal async method; fires the completion
/// callback with whatever exception (if any) it threw instead of letting it go unobserved.</summary>
internal static class DownloadRunner
{
    public static void Run(DictionaryDirection direction, string outputPath, string downloadTempDir, IProgress<string> progress, Action<Exception?> onFinished)
    {
        _ = RunAsync(direction, outputPath, downloadTempDir, progress, onFinished);
    }

    private static async Task RunAsync(DictionaryDirection direction, string outputPath, string downloadTempDir, IProgress<string> progress, Action<Exception?> onFinished)
    {
        Exception? error = null;
        try
        {
            await DictionaryReleaseClient.DownloadAsync(direction, outputPath, downloadTempDir, progress, CancellationToken.None);
        }
        catch (Exception ex)
        {
            error = ex;
        }

        onFinished(error);
    }
}
