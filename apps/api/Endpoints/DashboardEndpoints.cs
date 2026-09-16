using System.Text.Json;
using Gaia.Api.Services;

namespace Gaia.Api.Endpoints;

public static class DashboardEndpoints
{
    public static void MapDashboardEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api").WithTags("Dashboard");

        // Health check
        group.MapGet("/health", () => Results.Ok(new
        {
            ok = true,
            status = "operational",
            now = DateTime.UtcNow.ToString("o"),
            version = "gaia-v2-dotnet10"
        }));

        // Aggregated database stats
        group.MapGet("/stats", async (DatabaseService db, CancellationToken ct) =>
        {
            var stats = await db.GetDashboardStatsAsync(ct);
            return Results.Ok(stats);
        });

        // Server-Sent Events (SSE) live telemetry stream broadcasting full operational state
        var sseHandler = async (
            HttpContext ctx,
            DatabaseService db,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            ctx.Response.Headers.Append("Content-Type", "text/event-stream");
            ctx.Response.Headers.Append("Cache-Control", "no-cache, no-transform");
            ctx.Response.Headers.Append("Connection", "keep-alive");
            ctx.Response.Headers.Append("X-Accel-Buffering", "no");

            while (!ct.IsCancellationRequested)
            {
                try
                {
                    var stats = await db.GetDashboardStatsAsync(ct);
                    var metrics = await repo.GetMetricsCurrentAsync(ct);
                    var scoringStats = await repo.GetScoringStatsAsync(ct);
                    var alerts = await repo.GetAlertsAsync("all", 1, ct);

                    var payload = JsonSerializer.Serialize(new
                    {
                        type = "tick",
                        serverStats = stats,
                        serverMetrics = metrics,
                        scoringStats,
                        alertsSummary = alerts,
                        timestamp = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()
                    });

                    await ctx.Response.WriteAsync($"data: {payload}\n\n", ct);
                    await ctx.Response.Body.FlushAsync(ct);
                }
                catch (Exception ex) when (ex is not OperationCanceledException)
                {
                    // If client disconnected, break
                    break;
                }

                await Task.Delay(2500, ct);
            }
        };

        group.MapGet("/live/stream", sseHandler);
        group.MapGet("/stats/live", sseHandler);
    }
}
