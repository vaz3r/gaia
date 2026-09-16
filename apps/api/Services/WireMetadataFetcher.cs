using System.Buffers.Binary;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;

namespace Gaia.Api.Services;

public class WireMetadataFetcher
{
    private readonly ILogger<WireMetadataFetcher> _logger;

    public static readonly string[] DefaultTrackers = new[]
    {
        "udp://tracker.opentrackr.org:1337/announce",
        "udp://open.stealth.si:80/announce",
        "udp://tracker.torrent.eu.org:451/announce",
        "udp://explodie.org:6969/announce"
    };

    public WireMetadataFetcher(ILogger<WireMetadataFetcher> logger)
    {
        _logger = logger;
    }

    /// <summary>
    /// Races multiple candidate peers to fetch raw BEP-9 bencoded info dictionary within timeout.
    /// Returns raw info dictionary bytes if successful.
    /// </summary>
    public async Task<byte[]?> FetchMetadataOnTheFlyAsync(string infohashHex, IEnumerable<string> peerAddrs, int timeoutMs = 3500, CancellationToken ct = default)
    {
        var peers = peerAddrs.Where(p => !string.IsNullOrWhiteSpace(p)).Distinct().Take(5).ToList();
        if (peers.Count == 0)
        {
            return null;
        }

        using var linkedCts = CancellationTokenSource.CreateLinkedTokenSource(ct);
        linkedCts.CancelAfter(timeoutMs);

        var tasks = peers.Select(p => FetchFromSinglePeerAsync(infohashHex, p, timeoutMs, linkedCts.Token)).ToList();

        while (tasks.Count > 0)
        {
            var completedTask = await Task.WhenAny(tasks);
            tasks.Remove(completedTask);

            try
            {
                var result = await completedTask;
                if (result != null && result.Length > 0)
                {
                    linkedCts.Cancel(); // Cancel others once a peer succeeds
                    return result;
                }
            }
            catch
            {
                // Peer failed or timed out, continue checking other candidate peers
            }
        }

        return null;
    }

    private async Task<byte[]?> FetchFromSinglePeerAsync(string infohashHex, string peerAddr, int timeoutMs, CancellationToken ct)
    {
        var parts = peerAddr.Split(':');
        if (parts.Length != 2 || !int.TryParse(parts[1], out var port))
        {
            return null;
        }
        var ip = parts[0];

        using var client = new TcpClient();
        try
        {
            using var reg = ct.Register(() => client.Close());
            await client.ConnectAsync(ip, port, ct);

            await using var stream = client.GetStream();
            stream.ReadTimeout = timeoutMs;
            stream.WriteTimeout = timeoutMs;

            var infohashBytes = Convert.FromHexString(infohashHex ?? "");
            var peerId = Encoding.ASCII.GetBytes("-GA0002-" + Guid.NewGuid().ToString("N")[..12]);

            // Handshake: 19 + "BitTorrent protocol" (19) + 8 reserved (byte 5 = 0x10 for BEP-10) + 20 infohash + 20 peerId
            var handshake = new byte[68];
            handshake[0] = 19;
            Encoding.ASCII.GetBytes("BitTorrent protocol").CopyTo(handshake, 1);
            handshake[25] = 0x10; // BEP-10 extension support
            infohashBytes.CopyTo(handshake, 28);
            peerId.CopyTo(handshake, 48);

            await stream.WriteAsync(handshake, ct);

            // Read peer handshake (68 bytes)
            var peerHandshake = new byte[68];
            var read = 0;
            while (read < 68)
            {
                var n = await stream.ReadAsync(peerHandshake.AsMemory(read, 68 - read), ct);
                if (n == 0) return null;
                read += n;
            }

            if ((peerHandshake[25] & 0x10) == 0)
            {
                return null; // Peer does not support extensions
            }

            // Send BEP-10 Extended Handshake: length (4) + ID 20 + ExtID 0 + bencoded payload
            var extPayload = "d1:md11:ut_metadatai1eee"u8.ToArray();
            var extMsg = new byte[4 + 1 + 1 + extPayload.Length];
            BinaryPrimitives.WriteUInt32BigEndian(extMsg.AsSpan(0, 4), (uint)(1 + 1 + extPayload.Length));
            extMsg[4] = 20; // Extended message ID
            extMsg[5] = 0;  // Extended handshake ID
            extPayload.CopyTo(extMsg, 6);

            await stream.WriteAsync(extMsg, ct);

            // Process message stream to find ut_metadata response
            int? remoteUtMetadataId = null;
            int metadataSize = 0;
            int numPieces = 0;
            var piecesReceived = new Dictionary<int, byte[]>();

            var buffer = new byte[65536];
            var memStream = new MemoryStream();

            while (!ct.IsCancellationRequested)
            {
                var bytesRead = await stream.ReadAsync(buffer, ct);
                if (bytesRead == 0) break;
                memStream.Write(buffer, 0, bytesRead);

                var data = memStream.ToArray();
                var offset = 0;

                while (offset + 4 <= data.Length)
                {
                    var msgLen = (int)BinaryPrimitives.ReadUInt32BigEndian(data.AsSpan(offset, 4));
                    if (msgLen == 0)
                    {
                        // Keepalive
                        offset += 4;
                        continue;
                    }

                    if (offset + 4 + msgLen > data.Length)
                    {
                        // Incomplete message
                        break;
                    }

                    var msg = data.AsSpan(offset + 4, msgLen);
                    offset += 4 + msgLen;

                    if (msg.Length > 2 && msg[0] == 20) // Extended message
                    {
                        var extId = msg[1];
                        var payload = msg[2..];

                        if (extId == 0) // Extended Handshake from peer
                        {
                            var payloadStr = Encoding.Latin1.GetString(payload);
                            var utMatch = Regex.Match(payloadStr, @"11:ut_metadatai([0-9]+)e");
                            var sizeMatch = Regex.Match(payloadStr, @"13:metadata_sizei([0-9]+)e");

                            if (utMatch.Success && sizeMatch.Success)
                            {
                                remoteUtMetadataId = int.Parse(utMatch.Groups[1].Value);
                                metadataSize = int.Parse(sizeMatch.Groups[1].Value);

                                if (metadataSize > 0 && metadataSize <= 15 * 1024 * 1024)
                                {
                                    numPieces = (int)Math.Ceiling(metadataSize / 16384.0);

                                    // Request all pieces
                                    for (var p = 0; p < numPieces; p++)
                                    {
                                        var reqPayload = Encoding.UTF8.GetBytes($"d8:msg_typei0e5:piecei{p}ee");
                                        var reqMsg = new byte[4 + 1 + 1 + reqPayload.Length];
                                        BinaryPrimitives.WriteUInt32BigEndian(reqMsg.AsSpan(0, 4), (uint)(1 + 1 + reqPayload.Length));
                                        reqMsg[4] = 20;
                                        reqMsg[5] = (byte)remoteUtMetadataId.Value;
                                        reqPayload.CopyTo(reqMsg, 6);
                                        await stream.WriteAsync(reqMsg, ct);
                                    }
                                }
                            }
                        }
                        else if (extId == 1 && remoteUtMetadataId.HasValue) // ut_metadata piece response
                        {
                            // Find dictionary end ('ee')
                            var payloadBytes = payload.ToArray();
                            var dictEnd = FindDictEnd(payloadBytes);
                            if (dictEnd != -1)
                            {
                                var headerStr = Encoding.Latin1.GetString(payloadBytes, 0, dictEnd);
                                var pieceMatch = Regex.Match(headerStr, @"5:piecei([0-9]+)e");
                                var typeMatch = Regex.Match(headerStr, @"8:msg_typei([0-9]+)e");

                                if (typeMatch.Success && typeMatch.Groups[1].Value == "1" && pieceMatch.Success)
                                {
                                    var pieceIdx = int.Parse(pieceMatch.Groups[1].Value);
                                    var pieceData = payloadBytes[dictEnd..];
                                    piecesReceived[pieceIdx] = pieceData;

                                    if (piecesReceived.Count == numPieces && numPieces > 0)
                                    {
                                        var fullInfo = new byte[metadataSize];
                                        var writeOffset = 0;
                                        for (var i = 0; i < numPieces; i++)
                                        {
                                            var part = piecesReceived[i];
                                            Buffer.BlockCopy(part, 0, fullInfo, writeOffset, Math.Min(part.Length, metadataSize - writeOffset));
                                            writeOffset += part.Length;
                                        }

                                        var actualHash = Convert.ToHexString(SHA1.HashData(fullInfo)).ToLowerInvariant();
                                        if (actualHash == infohashHex.ToLowerInvariant())
                                        {
                                            return fullInfo; // Verified SHA-1 matching infohash
                                        }
                                        else
                                        {
                                            _logger.LogWarning("Wire metadata hash mismatch for {Infohash}: got {Actual}", infohashHex, actualHash);
                                            return null;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Keep unprocessed trailing bytes in memStream
                var remaining = data.Length - offset;
                memStream.SetLength(0);
                if (remaining > 0)
                {
                    memStream.Write(data, offset, remaining);
                }
            }
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            _logger.LogDebug("Peer {Peer} wire fetch failed: {Message}", peerAddr, ex.Message);
        }

        return null;
    }

    private static int FindDictEnd(byte[] buf, int start = 0)
    {
        if (start >= buf.Length || buf[start] != (byte)'d') return -1;
        var i = start + 1;
        while (i < buf.Length)
        {
            var b = buf[i];
            if (b == (byte)'e')
            {
                return i + 1;
            }
            if (b == (byte)'i')
            {
                i++;
                while (i < buf.Length && buf[i] != (byte)'e') i++;
                if (i < buf.Length) i++;
            }
            else if (b >= (byte)'0' && b <= (byte)'9')
            {
                var colon = Array.IndexOf(buf, (byte)':', i);
                if (colon == -1) return -1;
                var lenStr = Encoding.ASCII.GetString(buf, i, colon - i);
                if (!int.TryParse(lenStr, out var strLen)) return -1;
                i = colon + 1 + strLen;
            }
            else if (b == (byte)'l' || b == (byte)'d')
            {
                var end = FindDictEnd(buf, i);
                if (end == -1) return -1;
                i = end;
            }
            else
            {
                i++;
            }
        }
        return -1;
    }

    /// <summary>
    /// Wraps verified raw bencoded info dictionary bytes into a complete valid .torrent file.
    /// </summary>
    public static byte[] BuildTorrentBuffer(byte[] rawInfoBuffer, string? primaryAnnounce = null, IEnumerable<string>? trackers = null)
    {
        var trList = (trackers ?? DefaultTrackers).ToList();
        var mainTracker = primaryAnnounce ?? trList.FirstOrDefault() ?? DefaultTrackers[0];

        using var ms = new MemoryStream();
        using var writer = new BinaryWriter(ms, Encoding.UTF8);

        writer.Write("d8:announce"u8);
        writer.Write(Encoding.UTF8.GetBytes($"{Encoding.UTF8.GetByteCount(mainTracker)}:{mainTracker}"));

        writer.Write("13:announce-listl"u8);
        foreach (var tr in trList)
        {
            writer.Write("l"u8);
            writer.Write(Encoding.UTF8.GetBytes($"{Encoding.UTF8.GetByteCount(tr)}:{tr}e"));
        }
        writer.Write("e"u8);

        writer.Write("7:comment31:Downloaded from GAIA Indexer"u8);
        writer.Write("10:created by12:GAIA V2 .NET"u8);
        writer.Write(Encoding.UTF8.GetBytes($"13:creation datei{DateTimeOffset.UtcNow.ToUnixTimeSeconds()}e"));

        // Raw info dictionary
        writer.Write("4:info"u8);
        writer.Write(rawInfoBuffer);
        writer.Write("e"u8); // End root dict

        return ms.ToArray();
    }
}
