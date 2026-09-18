using Gaia.Api.Services;
using Microsoft.AspNetCore.Mvc;

namespace Gaia.Api.Endpoints;

public static class MetricsEndpoints
{
    public static void MapMetricsEndpoints(this IEndpointRouteBuilder app)
    {
        var metricsGroup = app.MapGroup("/api/metrics").WithTags("Metrics");

        metricsGroup.MapGet("/current", async (DashboardRepository repo, CancellationToken ct) =>
        {
            var data = await repo.GetMetricsCurrentAsync(ct);
            return Results.Ok(data);
        });

        metricsGroup.MapGet("/history", async (
            [FromQuery] string metric,
            [FromQuery] DateTime? from,
            [FromQuery] DateTime? to,
            [FromQuery] string? interval,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            if (string.IsNullOrWhiteSpace(metric) || metric.StartsWith('_'))
            {
                return Results.BadRequest(new { error = "metric is required" });
            }

            var toDate = to ?? DateTime.UtcNow;
            var fromDate = from ?? toDate.AddHours(-1);
            var data = await repo.GetMetricsHistoryAsync(metric, fromDate, toDate, interval ?? "minute", ct);
            return Results.Ok(data);
        });

        app.MapGet("/api/analytics", async (DashboardRepository repo, CancellationToken ct) =>
        {
            var data = await repo.GetAnalyticsSummaryAsync(ct);
            return Results.Ok(data);
        }).WithTags("Analytics");

        app.MapGet("/api/routing/security", async (DashboardRepository repo, CancellationToken ct) =>
        {
            var data = await repo.GetRoutingSecurityAsync(ct);
            return Results.Ok(data);
        }).WithTags("Routing Security");
    }
}
