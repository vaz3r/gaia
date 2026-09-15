using Gaia.Api.Services;
using Microsoft.AspNetCore.Mvc;

namespace Gaia.Api.Endpoints;

public static class TorrentEndpoints
{
    public static void MapTorrentEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/torrents").WithTags("Torrents");

        // 1. List / Search endpoint
        group.MapGet("/", async (
            HttpContext httpContext,
            [FromQuery] string? q,
            [FromQuery] string? search,
            [FromQuery] string? category,
            [FromQuery] int? page,
            [FromQuery] int? limit,
            [FromQuery] string? sort,
            [FromQuery] string? sort_by,
            [FromQuery] string? order,
            ISearchProvider searchProvider,
            DatabaseService db,
            CancellationToken ct) =>
        {
            var effectiveQuery = !string.IsNullOrWhiteSpace(search) ? search : q;
            var effectiveSort  = !string.IsNullOrWhiteSpace(sort) ? sort : sort_by;
            var safeLimit      = Math.Clamp(limit ?? 25, 1, 100);
            var safePage       = Math.Max(1, page ?? 1);

            // ── Search & Browse path (Meilisearch primary, PostgreSQL trigram fallback) ─
            var results = await searchProvider.SearchAsync(
                query:    effectiveQuery ?? "",
                category: category,
                page:     safePage,
                limit:    safeLimit,
                sortBy:   effectiveSort,
                order:    order ?? "desc",
                ct:       ct);

            // Diagnostic observability headers
            SetDiagnosticHeaders(httpContext, results);

            return Results.Ok(new
            {
                data       = results.Items,
                page       = safePage,
                limit      = safeLimit,
                total      = results.Total,
                pages      = Math.Max(1, (int)Math.Ceiling((double)results.Total / safeLimit)),
                elapsed_ms = results.ElapsedMs,
                from_cache = results.FromCache,
                source     = results.Provider
            });
        });

        // 2. Single torrent details
        group.MapGet("/{infohash}", async (
            HttpContext httpContext,
            string infohash,
            DatabaseService db,
            CancellationToken ct) =>
        {
            var torrent = await db.GetTorrentDetailsAsync(infohash, ct);
            if (torrent is null)
                return Results.NotFound(new { error = "Torrent not found" });

            if (ShouldEmitHeaders(httpContext))
            {
                httpContext.Response.Headers["X-Search-Source"] = "postgresql";
            }

            return Results.Ok(torrent);
        });

        // 3. Magnet URI generator
        group.MapGet("/{infohash}/magnet", async (
            string infohash,
            DatabaseService db,
            CancellationToken ct) =>
        {
            var torrent   = await db.GetTorrentDetailsAsync(infohash, ct);
            var name      = torrent?.TryGetValue("name", out var n) == true && n != null ? n.ToString() : infohash;
            var magnetUri = TorrentBuilder.BuildMagnetUri(infohash, name ?? infohash);

            return Results.Ok(new { infohash, name, magnetUri });
        });

        // 4. Direct .torrent download
        group.MapGet("/{infohash}/torrent", async (
            string infohash,
            DatabaseService db,
            CancellationToken ct) =>
        {
            var torrent      = await db.GetTorrentDetailsAsync(infohash, ct);
            var name         = torrent?.TryGetValue("name", out var n) == true && n != null ? n.ToString() : infohash;
            var torrentBytes = TorrentBuilder.BuildWrapperTorrent(infohash, name ?? infohash);
            var safeFilename = Uri.EscapeDataString((name ?? infohash).Replace("/", "_").Replace("\\", "_")) + ".torrent";

            return Results.File(torrentBytes, "application/x-bittorrent", safeFilename);
        });

        // 5. Category list
        app.MapGet("/api/categories", () =>
        {
            var categories = new[]
            {
                "All", "Movies", "Television", "Anime", "Games",
                "Applications", "Music", "Books & Learning", "Audiobooks", "Documentaries"
            };
            return Results.Ok(categories);
        }).WithTags("Categories");
    }

    private static void SetDiagnosticHeaders(HttpContext ctx, SearchResponse results)
    {
        if (ShouldEmitHeaders(ctx))
        {
            ctx.Response.Headers["X-Search-Source"] = results.Provider;
            ctx.Response.Headers["X-Cache"] = results.FromCache ? "HIT" : "MISS";
            ctx.Response.Headers["X-Cache-Tier"] = results.CacheTier;
            ctx.Response.Headers["X-Elapsed-Ms"] = results.ElapsedMs.ToString();
            ctx.Response.Headers["X-Total-Hits"] = results.Total.ToString();
        }
    }

    private static bool ShouldEmitHeaders(HttpContext ctx)
    {
        // Emit if explicitly requested via X-Debug header
        if (ctx.Request.Headers.ContainsKey("X-Debug")) return true;

        // Emit if client is internal/Tailscale/LAN
        var ip = ctx.Connection.RemoteIpAddress?.ToString() ?? "";
        return ip == "127.0.0.1" || ip == "::1" 
            || ip.StartsWith("100.")    // Tailscale
            || ip.StartsWith("192.168.") // Private LAN
            || ip.StartsWith("10.") 
            || ip.StartsWith("172.");
    }
}
