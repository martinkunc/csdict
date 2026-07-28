namespace CSDict.App.Data;

/// <summary>Maps a two-letter language code to its English name (e.g. "cs" -> "Czech"), for display
/// in the results pane. Hardcoded rather than looked up via CultureInfo, since CSDict.App.csproj
/// builds with InvariantGlobalization enabled, which makes CultureInfo unable to resolve real
/// culture/language data - see SystemLanguage.cs for the same constraint. Covers exactly the
/// languages DictionaryDirections.All knows about; falls back to the raw code for anything else.</summary>
internal static class LanguageNames
{
    private static readonly IReadOnlyDictionary<string, string> ByCode = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
    {
        ["cs"] = "Czech",
        ["en"] = "English",
        ["es"] = "Spanish",
        ["fr"] = "French",
        ["de"] = "German",
        ["ru"] = "Russian",
        ["pt"] = "Portuguese",
        ["ja"] = "Japanese",
        ["ar"] = "Arabic",
        ["zh"] = "Chinese",
        ["hi"] = "Hindi",
        ["ko"] = "Korean",
    };

    public static string English(string twoLetterCode) => ByCode.GetValueOrDefault(twoLetterCode, twoLetterCode);
}
