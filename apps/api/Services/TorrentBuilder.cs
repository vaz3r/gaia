using System.Text;

namespace Gaia.Api.Services;

public static class TorrentBuilder
{
    public static readonly string[] DefaultTrackers = new[]
    {
        "udp://tracker.opentrackr.org:1337/announce",
        "udp://open.demonii.com:1337/announce",
        "udp://open.stealth.si:80/announce",
        "udp://tracker.torrent.eu.org:451/announce",
        "udp://explodie.org:6969/announce",
        "udp://tracker.coppersurfer.tk:6969/announce"
    };

    public static string BuildMagnetUri(string infohashHex, string? name)
    {
        var sb = new StringBuilder();
        sb.Append("magnet:?xt=urn:btih:").Append(infohashHex.ToLowerInvariant());

        if (!string.IsNullOrWhiteSpace(name))
        {
            sb.Append("&dn=").Append(Uri.EscapeDataString(name));
        }

        foreach (var tr in DefaultTrackers)
        {
            sb.Append("&tr=").Append(Uri.EscapeDataString(tr));
        }

        return sb.ToString();
    }
}
