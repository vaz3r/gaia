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

        // Server-Sent Events (SSE) live telemetry stream for dashboard client
        group.MapGet("/live/stream", async (
            HttpContext ctx,
            DatabaseService db,
            CancellationToken ct) =>
        {
            ctx.Response.Headers.Append("Content-Type", "text/event-stream");
            ctx.Response.Headers.Append("Cache-Control", "no-cache");
            ctx.Response.Headers.Append("Connection", "keep-alive");

            while (!ct.IsCancellationRequested)
            {
                var stats = await db.GetDashboardStatsAsync(ct);
                var payload = System.Text.Json.JsonSerializer.Serialize(new
                {
                    timestamp = DateTime.UtcNow.ToString("o"),
                    torrents = stats.TotalTorrents,
                    verified24h = stats.VerifiedLast24h,
                    healthy = stats.HealthyCount
                });

                await ctx.Response.WriteAsync($"data: {payload}\n\n", ct);
                await ctx.Response.Body.FlushAsync(ct);

                await Task.Delay(3000, ct);
            }
        });
    }
}
