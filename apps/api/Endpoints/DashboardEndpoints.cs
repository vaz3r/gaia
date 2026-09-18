using System.Text.Json;
using Gaia.Api.Services;

namespace Gaia.Api.Endpoints;

public static class DashboardEndpoints
{
    public static void MapDashboardEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api").WithTags("Dashboard");

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

            try
            {
                while (!ct.IsCancellationRequested)
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

                    await Task.Delay(2500, ct);
                }
            }
            catch (OperationCanceledException)
            {
                // Normal client disconnect (browser closed tab or navigated away)
            }
            catch (Exception)
            {
                // Transport or socket closed
            }
        };

        group.MapGet("/live/stream", sseHandler).CacheOutput(p => p.NoCache());
    }
}
