using Gaia.Api.Infrastructure;
using Gaia.Api.Services;
using Microsoft.AspNetCore.Mvc;

namespace Gaia.Api.Endpoints;

public static class ScoringEndpoints
{
    public static void MapScoringEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/scoring").WithTags("Scoring");

        group.MapPost("/override", async (
            [FromBody] OverrideRequest req,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            if (string.IsNullOrWhiteSpace(req.Infohash) || req.Infohash.Length != 40)
            {
                return Results.BadRequest(new { error = "Valid 40-hex infohash is required" });
            }

            var updated = await repo.OverrideScoringAsync(
                req.Infohash, req.Action ?? "ALLOW", req.RiskTier, req.IntegrityScore, req.Notes, ct);

            if (updated == null)
            {
                return Results.NotFound(new { error = $"Torrent {req.Infohash} not found in database" });
            }

            return Results.Ok(new
            {
                success = true,
                message = $"Torrent successfully overridden to {req.Action} with MANUAL decision source.",
                data = updated
            });
        }).AddEndpointFilter<AdminAuthFilter>();

        group.MapGet("/pending", async (
            [FromQuery] int? page,
            [FromQuery] int? limit,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            var data = await repo.GetPendingReviewsAsync(page ?? 1, limit ?? 25, ct);
            return Results.Ok(data);
        });

        group.MapGet("/blocked", async (
            [FromQuery] int? page,
            [FromQuery] int? limit,
            [FromQuery] string? search,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            var data = await repo.GetBlockedTorrentsAsync(page ?? 1, limit ?? 25, search, ct);
            return Results.Ok(data);
        });

        group.MapPost("/unblock", async (
            [FromBody] UnblockRequest req,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            if (req.Infohashes == null || req.Infohashes.Length == 0)
            {
                return Results.BadRequest(new { error = "Array of infohashes is required" });
            }

            var updatedCount = await repo.UnblockTorrentsAsync(
                req.Infohashes, req.TargetAction ?? "ALLOW", req.Notes, ct);

            return Results.Ok(new
            {
                success = true,
                updated_count = updatedCount,
                target_action = req.TargetAction ?? "ALLOW",
                message = $"Successfully unblocked {updatedCount} torrent(s) to {req.TargetAction ?? "ALLOW"}."
            });
        }).AddEndpointFilter<AdminAuthFilter>();

        group.MapPost("/batch-rescore", async (
            [FromBody] BatchRescoreRequest req,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            var count = await repo.BatchRescoreAsync(
                req.Scope ?? "review", req.Category, req.Limit ?? 500, ct);

            return Results.Ok(new
            {
                success = true,
                count,
                message = $"Reset scored_at for {count} torrents (scope: {req.Scope ?? "review"})."
            });
        }).AddEndpointFilter<AdminAuthFilter>();

        group.MapGet("/stats", async (DashboardRepository repo, CancellationToken ct) =>
        {
            var stats = await repo.GetScoringStatsAsync(ct);
            return Results.Ok(stats);
        });
    }

    public record OverrideRequest(string? Infohash, string? Action, string? RiskTier, int? IntegrityScore, string? Notes);
    public record UnblockRequest(string[]? Infohashes, string? TargetAction, string? Notes);
    public record BatchRescoreRequest(string? Scope, string? Category, int? Limit);
}
