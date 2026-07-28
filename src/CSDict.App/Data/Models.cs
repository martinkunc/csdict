namespace CSDict.App.Data;

/// <summary>One loaded "{lemmaLang}_{targetLang}.sqlite3" file - a merged direction that may
/// contain rows from several distinct `source` values (freedict/wikdict/wiktionary/...), unlike
/// the old one-file-per-source layout where a file's single source was implied by its whole
/// contents.</summary>
internal sealed record DictionaryFile(string LemmaLang, string TargetLang, string FilePath, IReadOnlyList<string> Sources);

internal sealed record SourceResult(string Source, string TargetLang, List<EntryResult> Entries);

internal sealed record EntryResult(string? Pos, string? Ipa, string? Gender, List<SenseResult> Senses);

internal sealed record SenseResult(string? Gloss, List<string> Translations, List<(string SourceText, string TargetText)> Examples);
