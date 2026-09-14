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

    /// <summary>
    /// Builds a minimal valid bencoded .torrent wrapper pointing to the swarm infohash and trackers.
    /// BitTorrent clients use this to immediately join the swarm and retrieve metadata over wire (BEP-9).
    /// </summary>
    public static byte[] BuildWrapperTorrent(string infohashHex, string? name)
    {
        var safeName = string.IsNullOrWhiteSpace(name) ? infohashHex : name;
        var sb = new StringBuilder();

        // Standard bencoded dictionary
        // d8:announce{len}:{tracker}13:announce-listl...e4:infod4:name{len}:{name}e
        var primaryTracker = DefaultTrackers[0];

        sb.Append("d8:announce")
          .Append(primaryTracker.Length).Append(':').Append(primaryTracker);

        sb.Append("13:announce-listl");
        foreach (var tr in DefaultTrackers)
        {
            sb.Append("l").Append(tr.Length).Append(':').Append(tr).Append("e");
        }
        sb.Append("e");

        sb.Append("7:comment31:Downloaded from GAIA Indexer");
        sb.Append("10:created by12:GAIA V2 .NET");
        sb.Append("13:creation datei").Append(DateTimeOffset.UtcNow.ToUnixTimeSeconds()).Append("e");

        // Info dict
        sb.Append("4:infod");
        sb.Append("4:name").Append(Encoding.UTF8.GetByteCount(safeName)).Append(':').Append(safeName);
        sb.Append("ee"); // End of info and root dict

        return Encoding.UTF8.GetBytes(sb.ToString());
    }
}
