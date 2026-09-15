using System.Text.RegularExpressions;

namespace Gaia.Api.Services;

public enum QueryType
{
    Standard,
    Infohash
}

public record ParsedQuery(
    QueryType Type,
    string OriginalQuery,
    string CleanQuery,
    string? Infohash = null
);

public static class TorrentQueryParser
{
    private static readonly Regex InfohashRegex = new(@"^[0-9a-fA-F]{40}$", RegexOptions.Compiled);
    private static readonly Regex TwoDigitYearRegex = new(@"\b(7\d|8\d|9\d)\b", RegexOptions.Compiled);
    private static readonly Regex LeadingArticlesRegex = new(@"\b(th|teh)\b", RegexOptions.IgnoreCase | RegexOptions.Compiled);

    // Splits scene dots between words/numbers, but preserves version numbers (e.g. 24.04, 5.1)
    public static ParsedQuery Parse(string? input)
    {
        if (string.IsNullOrWhiteSpace(input))
            return new ParsedQuery(QueryType.Standard, string.Empty, string.Empty);

        var trimmed = input.Trim();

        // 1. O(1) Direct infohash detection
        if (InfohashRegex.IsMatch(trimmed))
            return new ParsedQuery(QueryType.Infohash, trimmed, trimmed.ToLowerInvariant(), trimmed.ToLowerInvariant());

        var clean = trimmed;

        // 2. Normalize common article typos (th -> the, teh -> the)
        clean = LeadingArticlesRegex.Replace(clean, "the");

        // 3. Scene release punctuation clean-up
        // Replace brackets, underscores, pluses with spaces
        clean = Regex.Replace(clean, @"[_\-+\[\](){}]", " ");

        // Split dots unless it's a version number or audio channel (e.g. 5.1, 24.04, 7.1)
        clean = Regex.Replace(clean, @"(?<=[a-zA-Z])\.(?=[a-zA-Z0-9])|(?<=[0-9])\.(?=[a-zA-Z])|(?<=\b\d{4})\.(?=\d)", " ");

        // 4. Contextual year expansion: 70-99 expands to 19xx (e.g. 'matrix 99' -> 'matrix 1999')
        // Does NOT expand 00-29 to avoid breaking TV show '24' or release versions
        clean = TwoDigitYearRegex.Replace(clean, m =>
        {
            var val = int.Parse(m.Value);
            return val >= 70 && val <= 99 ? $"19{val}" : m.Value;
        });

        // Collapse whitespace
        clean = Regex.Replace(clean, @"\s+", " ").Trim();

        return new ParsedQuery(QueryType.Standard, trimmed, clean);
    }

    // Helper for indexing: creates name_clean field from raw release name
    public static string CleanReleaseName(string? rawName)
    {
        if (string.IsNullOrWhiteSpace(rawName)) return string.Empty;

        // Split scene dots and separators, but preserve 1-2 digit version numbers (e.g. 24.04, 5.1)
        var s = Regex.Replace(rawName, @"[_\-+\[\](){}|/]", " ");

        // Match 4-digit years followed by resolution or numbers (e.g. 1999.1080p -> 1999 1080p)
        s = Regex.Replace(s, @"(?<=\b\d{4})\.(?=\d)", " ");

        // Match letters adjacent to dots
        s = Regex.Replace(s, @"(?<=[a-zA-Z])\.(?=[a-zA-Z0-9])|(?<=[0-9])\.(?=[a-zA-Z])", " ");

        // Split any remaining dots that are NOT between single or two-digit numbers (like 5.1 or 24.04)
        s = Regex.Replace(s, @"(?<!\b\d{1,2})\.|\.(?!\d{1,2}\b)", " ");

        return Regex.Replace(s, @"\s+", " ").Trim();
    }
}
