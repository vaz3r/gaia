using System.Text.RegularExpressions;

namespace Gaia.Sync;

public static partial class NameCleaner
{
    private static readonly Regex BracketTagRegex = new(@"\[[^\]]*\]|\([^\)]*\)", RegexOptions.Compiled);
    private static readonly Regex SeparatorRegex = new(@"[._\-+~]", RegexOptions.Compiled);
    private static readonly Regex SceneTagsRegex = new(
        @"\b(2160p|1080p|1080i|720p|480p|4k|uhd|bluray|blu-ray|remux|bdrip|brrip|web-?dl|webrip|hdtv|dvdrip|x264|x265|h264|h265|hevc|avc|atmos|truehd|dts(-?hd)?|aac|ac3|mp3|flac|repack|proper|unrated|extended|directors cut)\b",
        RegexOptions.IgnoreCase | RegexOptions.Compiled);
    private static readonly Regex MultiSpaceRegex = new(@"\s+", RegexOptions.Compiled);

    public static string Clean(string? rawName)
    {
        if (string.IsNullOrWhiteSpace(rawName)) return string.Empty;

        // Remove release groups in brackets/parentheses
        var text = BracketTagRegex.Replace(rawName, " ");

        // Replace punctuation separators with spaces
        text = SeparatorRegex.Replace(text, " ");

        // Strip scene tags
        text = SceneTagsRegex.Replace(text, " ");

        // Collapse multiple spaces & lowercase
        text = MultiSpaceRegex.Replace(text, " ").Trim().ToLowerInvariant();

        return text;
    }
}
