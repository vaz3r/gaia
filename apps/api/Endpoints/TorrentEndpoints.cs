using Gaia.Api.Services;
using Microsoft.AspNetCore.Mvc;

namespace Gaia.Api.Endpoints;

public static class TorrentEndpoints
{
    public static void MapTorrentEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/torrents").WithTags("Torrents");

        // 1. Search endpoint (Direct Quickwit sub-2ms query)
        group.MapGet("/", async (
            [FromQuery] string? q,
            [FromQuery] string? category,
            [FromQuery] int? page,
            [FromQuery] int? limit,
            [FromQuery] string? sort_by,
            [FromQuery] string? order,
            QuickwitClient quickwit,
            CancellationToken ct) =>
        {
            var results = await quickwit.SearchAsync(
                query: q,
                category: category,
                page: page ?? 1,
                limit: limit ?? 25,
                sortBy: sort_by,
                order: order ?? "desc",
                ct: ct);

            return Results.Ok(results);
        });

        // 2. Single torrent details
        group.MapGet("/{infohash}", async (
            string infohash,
            DatabaseService db,
            CancellationToken ct) =>
        {
            var torrent = await db.GetTorrentDetailsAsync(infohash, ct);
            if (torrent is null)
                return Results.NotFound(new { error = "Torrent not found" });

            return Results.Ok(torrent);
        });

        // 3. 1-Click Magnet URI generator
        group.MapGet("/{infohash}/magnet", async (
            string infohash,
            DatabaseService db,
            CancellationToken ct) =>
        {
            var torrent = await db.GetTorrentDetailsAsync(infohash, ct);
            var name = torrent?.Name ?? infohash;
            var magnetUri = TorrentBuilder.BuildMagnetUri(infohash, name);

            return Results.Ok(new { infohash, name, magnetUri });
        });

        // 4. Direct .torrent download stream
        group.MapGet("/{infohash}/torrent", async (
            string infohash,
            DatabaseService db,
            CancellationToken ct) =>
        {
            var torrent = await db.GetTorrentDetailsAsync(infohash, ct);
            var name = torrent?.Name ?? infohash;
            var torrentBytes = TorrentBuilder.BuildWrapperTorrent(infohash, name);

            var safeFilename = Uri.EscapeDataString(name.Replace("/", "_").Replace("\\", "_")) + ".torrent";
            return Results.File(torrentBytes, "application/x-bittorrent", safeFilename);
        });

        // 5. Category list
        app.MapGet("/api/categories", () =>
        {
            var categories = new[]
            {
                "All",
                "Movies",
                "Television",
                "Anime",
                "Games",
                "Applications",
                "Music",
                "Books & Learning",
                "Audiobooks",
                "Documentaries"
            };
            return Results.Ok(categories);
        }).WithTags("Categories");
    }
}
