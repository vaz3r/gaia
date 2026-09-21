using System.Text.Json;
using Dapper;
using Gaia.Api.Services;
using Microsoft.AspNetCore.Mvc;
using Microsoft.Extensions.Caching.Memory;

namespace Gaia.Api.Endpoints;

public record UpdateCategoryPolicyRequest(bool IsEnabled, bool? AutoPurge);
public record StartPurgeRequest(int? ChunkSize, int? DelayMs);

public static class CategoryEndpoints
{
    private const string CacheKey = "admin:categories:summary";

    public static void MapCategoryEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/admin/categories")
            .WithTags("Category Governance & Policies");

        // 1. List all category policies with live torrent & tombstone counts (cached for 60s)
        group.MapGet("/", async (
            [FromServices] DatabaseService db,
            [FromServices] IMemoryCache cache) =>
        {
            if (cache.TryGetValue(CacheKey, out IEnumerable<dynamic>? cached) && cached != null)
            {
                return Results.Ok(cached);
            }

            await using var conn = await db.DataSource.OpenConnectionAsync();
            const string sql = @"
                SELECT 
                    cp.category, 
                    cp.is_enabled, 
                    cp.auto_purge, 
                    cp.description, 
                    cp.updated_at,
                    COALESCE(tc.torrent_count, 0) AS torrent_count,
                    COALESCE(tc.verified_count, 0) AS verified_count,
                    COALESCE(tc.review_count, 0) AS review_count,
                    COALESCE(bc.tombstone_count, 0) AS tombstone_count
                FROM category_policies cp
                LEFT JOIN (
                    SELECT 
                        category, 
                        count(*) AS torrent_count,
                        count(*) FILTER (WHERE (needs_review = false OR needs_review IS NULL) AND (category_confidence IS NULL OR category_confidence >= 0.85)) AS verified_count,
                        count(*) FILTER (WHERE needs_review = true) AS review_count
                    FROM torrents 
                    GROUP BY category
                ) tc ON tc.category = cp.category
                LEFT JOIN (
                    SELECT category, count(*) AS tombstone_count 
                    FROM blocked_infohashes 
                    GROUP BY category
                ) bc ON bc.category = cp.category
                ORDER BY 
                    CASE WHEN cp.category = 'Adult' THEN 1 ELSE 2 END,
                    tc.torrent_count DESC NULLS LAST;";

            var categories = (await conn.QueryAsync(sql)).ToList();
            cache.Set(CacheKey, categories, TimeSpan.FromSeconds(60));
            return Results.Ok(categories);
        });

        // 2. Update category policy (enable/disable future crawling)
        group.MapPut("/{category}/policy", async (
            string category,
            [FromBody] UpdateCategoryPolicyRequest req,
            [FromServices] DatabaseService db,
            [FromServices] CacheService redis,
            [FromServices] IMemoryCache cache,
            [FromServices] ILogger<DatabaseService> logger) =>
        {
            cache.Remove(CacheKey);
            if (string.IsNullOrWhiteSpace(category))
                return Results.BadRequest(new { error = "Category parameter cannot be empty" });

            await using var conn = await db.DataSource.OpenConnectionAsync();
            const string sql = @"
                UPDATE category_policies 
                SET is_enabled = @IsEnabled, 
                    auto_purge = COALESCE(@AutoPurge, auto_purge), 
                    updated_at = now() 
                WHERE category = @category 
                RETURNING category, is_enabled, auto_purge, description, updated_at;";

            var updated = await conn.QueryFirstOrDefaultAsync<dynamic>(sql, new
            {
                category,
                req.IsEnabled,
                req.AutoPurge
            });

            if (updated == null)
                return Results.NotFound(new { error = $"Category '{category}' not found." });

            // Broadcast policy update to Redis
            try
            {
                var payload = JsonSerializer.Serialize(new
                {
                    category,
                    is_enabled = req.IsEnabled,
                    timestamp = DateTime.UtcNow
                });
                await redis.PublishAsync("gaia:category_policy:updated", payload);
            }
            catch (Exception ex)
            {
                logger.LogWarning(ex, "Failed to publish category policy update to Redis");
            }

            return Results.Ok(updated);
        });

        // 3. Launch C# background purge worker for a disabled category
        group.MapPost("/{category}/purge", async (
            string category,
            [FromBody] StartPurgeRequest? req,
            [FromServices] CategoryPurgeService purgeService,
            [FromServices] IMemoryCache cache) =>
        {
            cache.Remove(CacheKey);
            try
            {
                var chunkSize = req?.ChunkSize ?? 250;
                var delayMs = req?.DelayMs ?? 2000;

                var progress = await purgeService.StartPurgeAsync(category, chunkSize, delayMs);
                return Results.Accepted($"/api/admin/categories/purge/status", progress);
            }
            catch (ArgumentException ex)
            {
                return Results.BadRequest(new { error = ex.Message });
            }
            catch (InvalidOperationException ex)
            {
                return Results.Conflict(new { error = ex.Message });
            }
        });

        // 4. Cancel active purge job
        group.MapPost("/purge/cancel", ([FromServices] CategoryPurgeService purgeService) =>
        {
            var cancelled = purgeService.Cancel();
            return Results.Ok(new
            {
                success = cancelled,
                message = cancelled ? "Cancellation signal sent." : "No active purge task was running."
            });
        });

        // 5. Query status of purge worker
        group.MapGet("/purge/status", ([FromServices] CategoryPurgeService purgeService) =>
        {
            return Results.Ok(purgeService.GetStatus());
        });

        // 6. SSE live stream of purge progress
        group.MapGet("/purge/stream", async (
            HttpContext context,
            [FromServices] CategoryPurgeService purgeService,
            CancellationToken ct) =>
        {
            context.Response.Headers.ContentType = "text/event-stream";
            context.Response.Headers.CacheControl = "no-cache";
            context.Response.Headers.Connection = "keep-alive";
            context.Response.Headers.Append("X-Accel-Buffering", "no");

            // Push initial status
            var initial = purgeService.GetStatus();
            await context.Response.WriteAsync($"data: {JsonSerializer.Serialize(initial)}\n\n", ct);
            await context.Response.Body.FlushAsync(ct);

            var reader = purgeService.GetProgressReader();
            while (!ct.IsCancellationRequested && await reader.WaitToReadAsync(ct))
            {
                while (reader.TryRead(out var progress))
                {
                    var json = JsonSerializer.Serialize(progress);
                    await context.Response.WriteAsync($"data: {json}\n\n", ct);
                    await context.Response.Body.FlushAsync(ct);
                }
            }
        });
    }
}
