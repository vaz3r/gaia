using Gaia.Api.Infrastructure;
using Gaia.Api.Services;
using Microsoft.AspNetCore.Mvc;

namespace Gaia.Api.Endpoints;

public static class SurveillanceEndpoints
{
    public static void MapSurveillanceEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/surveillance").WithTags("Surveillance");

        group.MapGet("/nodes", async (
            [FromQuery] int? page,
            [FromQuery] int? limit,
            [FromQuery] int? min_score,
            [FromQuery] bool? blocked_only,
            [FromQuery] string? category,
            [FromQuery] string? search,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            var data = await repo.GetSurveillanceNodesAsync(
                page ?? 1, limit ?? 25, min_score ?? 0, blocked_only ?? false, category, search, ct);
            return Results.Ok(data);
        });

        group.MapPost("/nodes/{ip}/toggle", async (
            string ip,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            var updated = await repo.ToggleSurveillanceNodeAsync(ip, ct);
            if (updated == null)
            {
                return Results.NotFound(new { error = "Node not found" });
            }
            return Results.Ok(new { success = true, node = updated });
        }).AddEndpointFilter<AdminAuthFilter>();

        group.MapGet("/blocklist.txt", async (
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            var text = await repo.GetSurveillanceBlocklistAsync(ct);
            return Results.Text(text, "text/plain; charset=utf-8");
        });
    }
}
