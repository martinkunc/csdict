namespace CSGtk.App.Data;

internal sealed record DictionarySource(string Source, string LemmaLang, string TargetLang, string FilePath);

internal sealed record SourceResult(string Source, List<EntryResult> Entries);

internal sealed record EntryResult(string? Pos, string? Ipa, string? Gender, List<SenseResult> Senses);

internal sealed record SenseResult(string? Gloss, List<string> Translations, List<(string SourceText, string TargetText)> Examples);
