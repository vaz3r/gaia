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
    IReadOnlyList<string> QueryVariants,
    string? Infohash = null
);

public static class TorrentQueryParser
{
    private static readonly Regex InfohashRegex = new(@"^[0-9a-fA-F]{40}$", RegexOptions.Compiled);
    private static readonly Regex TwoDigitYearRegex = new(@"\b(7\d|8\d|9\d)\b", RegexOptions.Compiled);
    private static readonly Regex FourDigitYearRegex = new(@"\b(19\d{2}|20[0-2]\d)\b", RegexOptions.Compiled);
    private static readonly Regex LeadingArticlesRegex = new(@"\b(th|teh)\b", RegexOptions.IgnoreCase | RegexOptions.Compiled);

    // Protected wordlist: never split tokens matching these words
    private static readonly HashSet<string> TheWords = new(StringComparer.OrdinalIgnoreCase)
    {
        "thx", "thq", "thunder", "third", "threat", "thor", "theft", "thefts",
        "theory", "theories", "theorist", "theorists", "theology", "theological",
        "thing", "things", "thief", "thieves", "three", "thru", "through", "throughout",
        "though", "thought", "thoughts", "throttle", "throttled", "throttling",
        "thanks", "thank", "thankful", "theater", "theatre", "theatrical", "theatricals",
        "thesis", "theses", "therapy", "therapist", "therapists", "therapeutic",
        "thermal", "theme", "themes", "thematic",
        "there", "thereby", "therefore", "therein", "thereof",
        "these", "they", "their", "theirs", "them", "themselves", "then", "thence"
    };

    public static ParsedQuery Parse(string? input)
    {
        if (string.IsNullOrWhiteSpace(input))
            return new ParsedQuery(QueryType.Standard, string.Empty, string.Empty, Array.Empty<string>());

        var trimmed = input.Trim();

        // 1. Direct infohash detection
        if (InfohashRegex.IsMatch(trimmed))
            return new ParsedQuery(QueryType.Infohash, trimmed, trimmed.ToLowerInvariant(), new[] { trimmed.ToLowerInvariant() }, trimmed.ToLowerInvariant());

        var clean = trimmed;

        // 2. Normalize glued article typos (e.g. "thmatrix" -> "the matrix", "thematrix" -> "the matrix", "tehmatrix" -> "the matrix")
        clean = Regex.Replace(clean, @"\b(the|teh|th)([a-zA-Z]{3,})\b", m =>
        {
            var full = m.Value;
            if (TheWords.Contains(full)) return full;
            return "the " + m.Groups[2].Value;
        }, RegexOptions.IgnoreCase);

        // 3. Normalize common isolated article typos (th -> the, teh -> the)
        clean = LeadingArticlesRegex.Replace(clean, "the");

        // 4. Scene release punctuation clean-up
        clean = Regex.Replace(clean, @"[_\-+\[\](){}]", " ");

        // Split dots unless version number
        clean = Regex.Replace(clean, @"(?<=[a-zA-Z])\.(?=[a-zA-Z0-9])|(?<=[0-9])\.(?=[a-zA-Z])|(?<=\b\d{4})\.(?=\d)", " ");

        // Collapse whitespace
        clean = Regex.Replace(clean, @"\s+", " ").Trim();

        // 5. Generate Year Disjunction Variants (Feature 2)
        var variants = new List<string>();

        var twoDigitMatch = TwoDigitYearRegex.Match(clean);
        var fourDigitMatch = FourDigitYearRegex.Match(clean);

        if (twoDigitMatch.Success)
        {
            int val = int.Parse(twoDigitMatch.Value);
            string expandedYear = $"19{val}";
            string expandedQuery = TwoDigitYearRegex.Replace(clean, expandedYear, 1);

            variants.Add(expandedQuery); // Primary: expanded "the matrix 1999"
            variants.Add(clean);         // Secondary: original "the matrix 99"
        }
        else if (fourDigitMatch.Success)
        {
            int val = int.Parse(fourDigitMatch.Value);
            if (val >= 1970 && val <= 1999)
            {
                string shortYear = (val % 100).ToString();
                string contractedQuery = FourDigitYearRegex.Replace(clean, shortYear, 1);

                variants.Add(clean);           // Primary: original "matrix 1999"
                variants.Add(contractedQuery); // Secondary: contracted "matrix 99"
            }
            else
            {
                variants.Add(clean);
            }
        }
        else
        {
            variants.Add(clean);
        }

        string primaryClean = variants[0];
        return new ParsedQuery(QueryType.Standard, trimmed, primaryClean, variants);
    }
}
