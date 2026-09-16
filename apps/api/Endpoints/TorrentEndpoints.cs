using Gaia.Api.Services;
using Microsoft.AspNetCore.Mvc;

namespace Gaia.Api.Endpoints;

public static class TorrentEndpoints
{
    public static void MapTorrentEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/torrents").WithTags("Torrents");

        // 1. List / Search endpoint (Public Portal)
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

            // Search & Browse path (Meilisearch primary, PostgreSQL fallback)
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

        // 4. Direct .torrent download (BEP-9 Wire Metadata Fetcher with Fallback Magnet)
        group.MapGet("/{infohash}/torrent", async (
            string infohash,
            DashboardRepository repo,
            WireMetadataFetcher wireFetcher,
            CancellationToken ct) =>
        {
            var cleanIh = (infohash ?? "").Trim().ToLowerInvariant();
            if (cleanIh.Length != 40 || !cleanIh.All(Uri.IsHexDigit))
            {
                return Results.BadRequest(new { error = "infohash must be 40 hex chars" });
            }

            var torrent = await repo.GetTorrentDetailsAsync(cleanIh, ct);
            if (torrent == null)
            {
                return Results.NotFound(new { error = "Torrent not found" });
            }

            var name = (string?)torrent.name ?? $"payload-{cleanIh[..8]}";
            var candidatePeers = await repo.GetCandidatePeersAsync(cleanIh, ct);
            var turboMagnet = TorrentBuilder.BuildMagnetUri(cleanIh, name);

            if (candidatePeers.Count == 0)
            {
                return Results.Json(new
                {
                    error = "No active seeders recorded to assemble metadata on the fly",
                    magnet = turboMagnet
                }, statusCode: StatusCodes.Status504GatewayTimeout);
            }

            // Race candidate peers with 3.5s timeout over BEP-9
            var rawInfoBytes = await wireFetcher.FetchMetadataOnTheFlyAsync(cleanIh, candidatePeers, 3500, ct);
            if (rawInfoBytes == null || rawInfoBytes.Length == 0)
            {
                return Results.Json(new
                {
                    error = "Could not reach live seeders in time to assemble metadata on the fly",
                    magnet = turboMagnet
                }, statusCode: StatusCodes.Status504GatewayTimeout);
            }

            // Wrap verified raw info dict into full .torrent bytes
            var torrentBytes = WireMetadataFetcher.BuildTorrentBuffer(rawInfoBytes);
            var cleanFilename = Uri.EscapeDataString(name.Replace("/", "_").Replace("\\", "_").Trim()) + ".torrent";

            return Results.File(torrentBytes, "application/x-bittorrent", cleanFilename);
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
        var expectedToken = Environment.GetEnvironmentVariable("GAIA_DEBUG_TOKEN") ?? "gaia_debug_secret_token_v2";
        if (ctx.Request.Headers.TryGetValue("X-Gaia-Debug", out var token) && token == expectedToken)
            return true;

        return false;
    }
}
