using Gaia.Api.Services;
using Microsoft.AspNetCore.Mvc;

namespace Gaia.Api.Endpoints;

public static class DashboardTorrentEndpoints
{
    public static void MapDashboardTorrentEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api").WithTags("Dashboard Torrents");

        // 1. Direct PostgreSQL Admin Table
        group.MapGet("/dashboard/torrents", async (
            [FromQuery] string? search,
            [FromQuery] string? category,
            [FromQuery] string? risk,
            [FromQuery] string? availability,
            [FromQuery] string? policy,
            [FromQuery] string? sort,
            [FromQuery] string? order,
            [FromQuery] int? page,
            [FromQuery] int? limit,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            var result = await repo.GetDashboardTorrentsAsync(
                search, category, risk, availability, policy, sort, order, page ?? 1, limit ?? 25, ct);
            return Results.Ok(result);
        });

        // 2. Batch Lookup
        group.MapPost("/torrents/batch-lookup", async (
            [FromBody] BatchLookupRequest req,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            var dict = await repo.BatchLookupAsync(req.Hashes ?? Array.Empty<string>(), ct);
            return Results.Ok(new { torrents = dict });
        });

        // 3. On-demand Swarm Health Recalculation
        group.MapPost("/torrents/{infohash}/refresh-health", async (
            string infohash,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            if (infohash.Length != 40)
            {
                return Results.BadRequest(new { error = "infohash must be 40 hex chars" });
            }

            var updated = await repo.RefreshTorrentHealthAsync(infohash, ct);
            if (updated == null)
            {
                return Results.NotFound(new { error = "Torrent not found" });
            }

            return Results.Ok(updated);
        });
    }

    public record BatchLookupRequest(string[]? Hashes);
}
