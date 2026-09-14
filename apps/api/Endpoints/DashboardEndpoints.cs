using Gaia.Api.Services;
using Microsoft.AspNetCore.Mvc;

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

        // Server-Sent Events (SSE) live telemetry stream for dashboard & portal clients
        var sseHandler = async (
            HttpContext ctx,
            DatabaseService db,
            CancellationToken ct) =>
        {
            ctx.Response.Headers.Append("Content-Type", "text/event-stream");
            ctx.Response.Headers.Append("Cache-Control", "no-cache");
            ctx.Response.Headers.Append("Connection", "keep-alive");
            ctx.Response.Headers.Append("X-Accel-Buffering", "no");

            while (!ct.IsCancellationRequested)
            {
                var stats = await db.GetDashboardStatsAsync(ct);
                var payload = System.Text.Json.JsonSerializer.Serialize(new
                {
                    timestamp = DateTime.UtcNow.ToString("o"),
                    torrents = stats.TryGetValue("total_torrents", out var t) ? t : 0,
                    verified24h = stats.TryGetValue("verified_last_24h", out var v) ? v : 0,
                    healthy = stats.TryGetValue("healthy_count", out var h) ? h : 0
                });

                await ctx.Response.WriteAsync($"data: {payload}\n\n", ct);
                await ctx.Response.Body.FlushAsync(ct);

                await Task.Delay(3000, ct);
            }
        };

        group.MapGet("/live/stream", sseHandler);
        group.MapGet("/stats/live", sseHandler);
    }
}
