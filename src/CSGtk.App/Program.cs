using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using CSGtk.App.Data;
using CSGtk.App.Scraping;
using CSGtk.App.Sqlite;
using CSGtk.Gtk;

namespace CSGtk.App;

internal static unsafe class Program
{
    private const string Css = """
        window {
            background-color: #2a292c;
        }
        headerbar {
            background-color: #2a292c;
            color: #e8e7ea;
        }
        label.csgtk-placeholder {
            color: #98979b;
            font-size: 15px;
        }
        label.csgtk-heading {
            color: #cfced2;
        }
        label.csgtk-translation {
            color: #f5f4f7;
        }
        togglebutton.csgtk-a-small label {
            font-size: 12px;
        }
        togglebutton.csgtk-a-large label {
            font-size: 17px;
        }
        """;

    private const int MaxSimilarWords = 60;

    private static DictionaryCatalog? s_catalog;
    private static WordIndex? s_wordIndex;
    private static string? s_activeSourceFilter;
    private static (string Lemma, string Lang)? s_selected;
    private static string s_dictionariesDir = "";
    private static string s_cacheRootDir = "";

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
    private static bool s_dialogOpen;
    private static bool s_downloadInProgress;
    private static DictionaryDefinition? s_dialogSelected;

    private static readonly Dictionary<nint, (string Lemma, string Lang)> s_rowData = new();
    private static readonly Dictionary<nint, string?> s_sourceButtons = new();
    private static readonly Dictionary<nint, DictionaryDefinition> s_dictionaryDialogRows = new();

    [STAThread]
    private static int Main()
    {
        RegisterNativeLibraryResolver();
        SqliteNativeResolver.Register();

        s_dictionariesDir = Path.Combine(AppContext.BaseDirectory, "Dictionaries");
        s_cacheRootDir = Path.Combine(AppContext.BaseDirectory, "Cache");
        s_catalog = DictionaryCatalog.Load(s_dictionariesDir);
        s_wordIndex = WordIndex.Build(s_catalog);

        nint app = Gtk4.gtk_application_new("dev.csgtk.Dictionary", 0);
        GObject.Connect(app, "activate", &OnActivate);

        int status = Gio.g_application_run(app, 0, 0);
        GObject.g_object_unref(app);
        return status;
    }

    private static void RegisterNativeLibraryResolver()
    {
        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            CSGtk.Gtk.Native.Windows.NativeLibraryResolver.Register();
        }
        else if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
        {
            CSGtk.Gtk.Native.macOS.NativeLibraryResolver.Register();
        }
        else if (RuntimeInformation.IsOSPlatform(OSPlatform.Linux))
        {
            CSGtk.Gtk.Native.Linux.NativeLibraryResolver.Register();
        }
        else
        {
            throw new PlatformNotSupportedException(
                $"No CSGtk.Gtk native library resolver is available for {RuntimeInformation.OSDescription}.");
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
        Gtk4.gtk_header_bar_set_show_title_buttons(headerBar, true);

        nint navBox = Gtk4.gtk_box_new(GtkOrientation.Horizontal, 0);
        Gtk4.gtk_widget_add_css_class(navBox, "linked");
        Gtk4.gtk_box_append(navBox, Gtk4.gtk_button_new_from_icon_name("go-previous-symbolic"));
        Gtk4.gtk_box_append(navBox, Gtk4.gtk_button_new_from_icon_name("go-next-symbolic"));
        Gtk4.gtk_header_bar_pack_start(headerBar, navBox);

        nint title = Gtk4.gtk_label_new(null);
        Gtk4.gtk_label_set_markup(title, "<b>Dictionary</b>");
        Gtk4.gtk_header_bar_set_title_widget(headerBar, title);

        nint fontBox = Gtk4.gtk_box_new(GtkOrientation.Horizontal, 0);
        Gtk4.gtk_widget_add_css_class(fontBox, "linked");
        nint smallA = Gtk4.gtk_toggle_button_new_with_label("A");
        Gtk4.gtk_widget_add_css_class(smallA, "csgtk-a-small");
        nint largeA = Gtk4.gtk_toggle_button_new_with_label("A");
        Gtk4.gtk_widget_add_css_class(largeA, "csgtk-a-large");
        Gtk4.gtk_toggle_button_set_group(largeA, smallA);
        Gtk4.gtk_toggle_button_set_active(largeA, true);
        Gtk4.gtk_box_append(fontBox, smallA);
        Gtk4.gtk_box_append(fontBox, largeA);

        s_dictionaryButton = Gtk4.gtk_button_new_with_label("D");
        GObject.Connect(s_dictionaryButton, "clicked", &OnDictionaryButtonClicked);

        s_searchEntry = Gtk4.gtk_search_entry_new();
        Gtk4.gtk_widget_set_size_request(s_searchEntry, 220, -1);
        GObject.Connect(s_searchEntry, "search-changed", &OnSearchChanged);
        GObject.Connect(s_searchEntry, "activate", &OnSearchActivate);

        nint endBox = Gtk4.gtk_box_new(GtkOrientation.Horizontal, 8);
        Gtk4.gtk_box_append(endBox, fontBox);
        Gtk4.gtk_box_append(endBox, s_dictionaryButton);
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
            s_sourceButtons[tab] = isAll ? null : label;
            GObject.Connect(tab, "toggled", &OnSourceToggled);
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
        ShowEmptyState("Type a word above to look it up.");

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

    /// <summary>A small modal library manager: lists every (source, direction) dictionary this app
    /// knows how to fetch, whether or not it's currently downloaded, and lets the user download a
    /// missing one or remove one that's already on disk.</summary>
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
        Gtk4.gtk_window_set_default_size(dialog, 340, 420);

        nint container = Gtk4.gtk_box_new(GtkOrientation.Vertical, 8);
        Gtk4.gtk_widget_set_margin_start(container, 10);
        Gtk4.gtk_widget_set_margin_end(container, 10);
        Gtk4.gtk_widget_set_margin_top(container, 10);
        Gtk4.gtk_widget_set_margin_bottom(container, 10);

        s_dialogList = Gtk4.gtk_list_box_new();
        Gtk4.gtk_list_box_set_selection_mode(s_dialogList, GtkSelectionMode.Browse);
        Gtk4.gtk_list_box_set_activate_on_single_click(s_dialogList, true);
        GObject.Connect(s_dialogList, "row-activated", &OnDictionaryDialogRowActivated);

        s_dictionaryDialogRows.Clear();
        foreach (DictionaryDefinition def in DictionaryDownloader.All)
        {
            AppendDictionaryDialogRow(def);
        }

        // GTK auto-selects the first row for a Browse-mode list box as soon as it has children,
        // but that doesn't raise "row-activated" - so the action-button state has to be primed
        // manually to match, or Download/Remove stay hidden until the user clicks the (already
        // visibly selected) first row themselves.
        s_dialogSelected = DictionaryDownloader.All.Count > 0 ? DictionaryDownloader.All[0] : null;

        nint scroller = Gtk4.gtk_scrolled_window_new();
        Gtk4.gtk_scrolled_window_set_policy(scroller, GtkPolicyType.Never, GtkPolicyType.Automatic);
        Gtk4.gtk_scrolled_window_set_child(scroller, s_dialogList);
        Gtk4.gtk_widget_set_vexpand(scroller, true);
        Gtk4.gtk_box_append(container, scroller);

        s_dialogStatusLabel = Gtk4.gtk_label_new("Select a dictionary above.");
        Gtk4.gtk_label_set_xalign(s_dialogStatusLabel, 0);
        Gtk4.gtk_label_set_wrap(s_dialogStatusLabel, true);
        Gtk4.gtk_widget_add_css_class(s_dialogStatusLabel, "csgtk-placeholder");
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

    private static void AppendDictionaryDialogRow(DictionaryDefinition def)
    {
        nint label = Gtk4.gtk_label_new(DictionaryRowText(def));
        Gtk4.gtk_label_set_xalign(label, 0);
        Gtk4.gtk_widget_set_margin_start(label, 8);
        Gtk4.gtk_widget_set_margin_end(label, 8);
        Gtk4.gtk_widget_set_margin_top(label, 6);
        Gtk4.gtk_widget_set_margin_bottom(label, 6);
        s_dictionaryDialogRows[label] = def;
        Gtk4.gtk_list_box_append(s_dialogList, label);
    }

    private static string DictionaryRowText(DictionaryDefinition def) =>
        $"{def.DisplayName} — {(File.Exists(GetDictionaryPath(def)) ? "Downloaded" : "Not downloaded")}";

    private static string GetDictionaryPath(DictionaryDefinition def) => Path.Combine(s_dictionariesDir, def.FileName);

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnDictionaryDialogRowActivated(nint listBox, nint row, nint userData)
    {
        if (s_downloadInProgress)
        {
            return;
        }

        nint child = Gtk4.gtk_list_box_row_get_child(row);
        if (!s_dictionaryDialogRows.TryGetValue(child, out DictionaryDefinition? def))
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
        string cacheRootDir = s_cacheRootDir;
        var progress = new Progress<string>(message =>
            GLib.RunOnMainThread(() =>
            {
                if (s_dialogOpen)
                {
                    Gtk4.gtk_label_set_text(s_dialogStatusLabel, message);
                }
            }));

        DownloadRunner.Run(def, outputPath, cacheRootDir, progress, error => GLib.RunOnMainThread(() => OnDownloadFinished(def, error)));
    }

    private static void OnDownloadFinished(DictionaryDefinition def, Exception? error)
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
            SelectWord(word.Lemma, word.Lang);
        }
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
            ShowEmptyState("Type a word above to look it up.");
            return;
        }

        nint heading = Gtk4.gtk_label_new(null);
        Gtk4.gtk_label_set_markup(heading,
            $"<span size='large' weight='bold'>{Escape(selected.Lemma)}</span>  <span alpha='60%'>({selected.Lang})</span>");
        Gtk4.gtk_label_set_xalign(heading, 0);
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
        Gtk4.gtk_box_append(s_resultsBox, sourceHeading);

        foreach (EntryResult entry in source.Entries)
        {
            string meta = string.Join(" · ", new[] { entry.Pos, entry.Ipa, entry.Gender }.Where(v => !string.IsNullOrEmpty(v)));
            if (meta.Length > 0)
            {
                nint metaLabel = Gtk4.gtk_label_new(meta);
                Gtk4.gtk_label_set_xalign(metaLabel, 0);
                Gtk4.gtk_widget_add_css_class(metaLabel, "csgtk-placeholder");
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
        Gtk4.gtk_widget_add_css_class(senseLabel, "csgtk-translation");
        Gtk4.gtk_box_append(s_resultsBox, senseLabel);

        foreach ((string sourceText, string targetText) in sense.Examples)
        {
            nint exampleLabel = Gtk4.gtk_label_new($"    {sourceText} — {targetText}");
            Gtk4.gtk_label_set_wrap(exampleLabel, true);
            Gtk4.gtk_label_set_xalign(exampleLabel, 0);
            Gtk4.gtk_widget_add_css_class(exampleLabel, "csgtk-placeholder");
            Gtk4.gtk_box_append(s_resultsBox, exampleLabel);
        }
    }

    private static void ShowEmptyState(string text)
    {
        nint label = Gtk4.gtk_label_new(text);
        Gtk4.gtk_widget_add_css_class(label, "csgtk-placeholder");
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
        Gtk4.gtk_css_provider_load_from_string(provider, Css);
        Gtk4.gtk_style_context_add_provider_for_display(Gtk4.gdk_display_get_default(), provider, Gtk4.GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}

/// <summary>Kept outside Program (which is an `unsafe` class - `await` isn't allowed inside an
/// unsafe context) so the actual scrape can be a normal async method; fires the completion
/// callback with whatever exception (if any) it threw instead of letting it go unobserved.</summary>
internal static class DownloadRunner
{
    public static void Run(DictionaryDefinition def, string outputPath, string cacheRootDir, IProgress<string> progress, Action<Exception?> onFinished)
    {
        _ = RunAsync(def, outputPath, cacheRootDir, progress, onFinished);
    }

    private static async Task RunAsync(DictionaryDefinition def, string outputPath, string cacheRootDir, IProgress<string> progress, Action<Exception?> onFinished)
    {
        Exception? error = null;
        try
        {
            await DictionaryDownloader.DownloadAsync(def, outputPath, cacheRootDir, progress, CancellationToken.None);
        }
        catch (Exception ex)
        {
            error = ex;
        }

        onFinished(error);
    }
}
