using Gaia.Api.Infrastructure;
using Gaia.Api.Services;
using Microsoft.AspNetCore.Mvc;

namespace Gaia.Api.Endpoints;

public static class AlertsEndpoints
{
    public static void MapAlertsEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/alerts").WithTags("Alerts");

        group.MapGet("/", async (
            [FromQuery] string? status,
            [FromQuery] int? limit,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            var data = await repo.GetAlertsAsync(status ?? "all", limit ?? 25, ct);
            return Results.Ok(data);
        });

        group.MapPost("/{id:int}/resolve", async (
            int id,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            var alert = await repo.ResolveAlertAsync(id, ct);
            if (alert == null)
            {
                return Results.NotFound(new { error = "Alert not found" });
            }
            return Results.Ok(new { success = true, alert });
        }).AddEndpointFilter<AdminAuthFilter>();
    }
}
