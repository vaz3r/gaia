using System.Net.Http.Json;
using Dapper;

namespace Gaia.Api.Services;

public class MeilisearchSyncService : BackgroundService
{
    private readonly IServiceProvider _services;
    private readonly IConfiguration _config;
    private readonly ILogger<MeilisearchSyncService> _logger;

    public MeilisearchSyncService(
        IServiceProvider services,
        IConfiguration config,
        ILogger<MeilisearchSyncService> logger)
    {
        _services = services;
        _config = config;
        _logger = logger;
    }

    protected override async Task ExecuteAsync(CancellationToken ct)
    {
        await Task.Delay(TimeSpan.FromSeconds(3), ct);

        using var http = new HttpClient();
        var meiliUrl = _config["Meilisearch:Url"] ?? "http://127.0.0.1:7700";
        var meiliKey = _config["Meilisearch:ApiKey"] ?? "";
        http.BaseAddress = new Uri(meiliUrl);
        if (!string.IsNullOrWhiteSpace(meiliKey))
            http.DefaultRequestHeaders.Add("Authorization", $"Bearer {meiliKey}");

        await EnsureIndexSettingsAsync(http, ct);

        // Read durable watermark from PostgreSQL
        var watermark = await GetWatermarkFromDbAsync(ct);
        if (!watermark.BackfillCompleted)
        {
            _logger.LogInformation("Resuming tie-safe composite backfill from: {CursorTs:u} / {CursorHash}...", 
                watermark.LastCursorTs, watermark.LastCursorHash);
            await KeysetBackfillAsync(http, watermark.LastCursorTs, watermark.LastCursorHash, ct);
        }

        _logger.LogInformation("Entering incremental sync mode (every 30s) with deletion tracking...");
        while (!ct.IsCancellationRequested)
        {
            try
            {
                await Task.Delay(TimeSpan.FromSeconds(30), ct);
                await SyncIncrementalWithDeletesAsync(http, DateTime.UtcNow.AddMinutes(-3), ct);
            }
            catch (OperationCanceledException) { break; }
            catch (Exception ex)
            {
                _logger.LogError(ex, "Incremental sync error — retrying in 30s");
            }
        }
    }

    // Blocker 2 Fix: Composite Keyset Pagination
    private async Task KeysetBackfillAsync(HttpClient http, DateTime? initialTs, string? initialHash, CancellationToken ct)
    {
        using var scope = _services.CreateScope();
        var db = scope.ServiceProvider.GetRequiredService<DatabaseService>();

        const int batchSize = 5000;
        DateTime? cursorTs   = initialTs;
        string?   cursorHash = initialHash;
        long totalSent = 0;

        const string sql = """
            SELECT encode(infohash, 'hex') AS infohash, name, category, total_size, file_count,
                   EXTRACT(EPOCH FROM verified_at)::bigint AS verified_at,
                   health_score, popularity_score, swarm_peers, seed_confirmed, risk_tier,
                   COALESCE(policy_action, 'ALLOW') AS policy_action,
                   verified_at AS raw_ts,
                   encode(infohash, 'hex') AS raw_hash
            FROM torrents
            WHERE (CAST(@cursorTs AS timestamptz) IS NULL OR (verified_at, infohash) < (CAST(@cursorTs AS timestamptz), decode(CAST(@cursorHash AS text), 'hex')))
              AND (policy_action IS NULL OR policy_action != 'SUPPRESS')
            ORDER BY verified_at DESC, infohash DESC
            LIMIT @lim;
        """;

        while (!ct.IsCancellationRequested)
        {
            try
            {
                await using var conn = await db.OpenConnectionAsync(ct);
                var batch = (await conn.QueryAsync(sql, new { lim = batchSize, cursorTs, cursorHash })).AsList();
                if (batch.Count == 0)
                {
                    await SaveWatermarkToDbAsync(true, cursorTs, cursorHash, ct);
                    break;
                }

                var lastRow = (IDictionary<string, object?>)batch.Last();
                var nextCursorTs   = (DateTime)lastRow["raw_ts"]!;
                var nextCursorHash = (string)lastRow["raw_hash"]!;

                var docs = batch.Select(r => (IDictionary<string, object?>)r).ToList();
                var resp = await http.PostAsJsonAsync("/indexes/torrents/documents", docs, ct);
                if (!resp.IsSuccessStatusCode)
                {
                    _logger.LogWarning("Meilisearch backfill push returned {Code}, retrying in 5s...", resp.StatusCode);
                    await Task.Delay(5000, ct);
                    continue;
                }

                cursorTs   = nextCursorTs;
                cursorHash = nextCursorHash;
                totalSent += batch.Count;

                await SaveWatermarkToDbAsync(false, cursorTs, cursorHash, ct);
                _logger.LogInformation("Backfill: {Count:N0} records sent (Cursor: {Cursor:u} / {Hash})", totalSent, cursorTs, cursorHash);

                await Task.Delay(250, ct);
            }
            catch (OperationCanceledException) { break; }
            catch (Exception ex)
            {
                _logger.LogWarning(ex, "Meilisearch backfill error (service temporarily unavailable?), retrying in 5s...");
                await Task.Delay(5000, ct);
            }
        }

        _logger.LogInformation("Composite keyset backfill complete: {Total:N0} total sent.", totalSent);
    }

    // Blocker 1 Fix: Two-phase incremental sync (Upsert live + Delete suppressed)
    private async Task SyncIncrementalWithDeletesAsync(HttpClient http, DateTime since, CancellationToken ct)
    {
        using var scope = _services.CreateScope();
        var db = scope.ServiceProvider.GetRequiredService<DatabaseService>();

        await using var conn = await db.OpenConnectionAsync(ct);

        // 1. Upsert live documents
        const string upsertSql = """
            SELECT encode(infohash, 'hex') AS infohash, name, category, total_size, file_count,
                   EXTRACT(EPOCH FROM verified_at)::bigint AS verified_at,
                   health_score, popularity_score, swarm_peers, seed_confirmed, risk_tier,
                   COALESCE(policy_action, 'ALLOW') AS policy_action
            FROM torrents
            WHERE (verified_at >= CAST(@since AS timestamptz) OR scored_at >= CAST(@since AS timestamptz) OR last_health_check >= CAST(@since AS timestamptz))
              AND (policy_action IS NULL OR policy_action != 'SUPPRESS')
            LIMIT 50000;
        """;

        var liveRows = (await conn.QueryAsync(upsertSql, new { since })).AsList();
        if (liveRows.Count > 0)
        {
            var docs = liveRows.Select(r => (IDictionary<string, object?>)r).ToList();
            await http.PostAsJsonAsync("/indexes/torrents/documents", docs, ct);
            _logger.LogInformation("Incremental upsert: {Count:N0} records pushed to Meilisearch", docs.Count);
        }

        // 2. Delete suppressed documents
        const string deleteSql = """
            SELECT encode(infohash, 'hex') AS infohash
            FROM torrents
            WHERE (scored_at >= CAST(@since AS timestamptz) OR verified_at >= CAST(@since AS timestamptz))
              AND policy_action = 'SUPPRESS'
            LIMIT 10000;
        """;

        var suppressedHashes = (await conn.QueryAsync<string>(deleteSql, new { since })).AsList();
        if (suppressedHashes.Count > 0)
        {
            await http.PostAsJsonAsync("/indexes/torrents/documents/delete-batch", suppressedHashes, ct);
            _logger.LogInformation("Incremental delete: {Count:N0} suppressed torrents removed from Meilisearch", suppressedHashes.Count);
        }
    }

    private async Task EnsureIndexSettingsAsync(HttpClient http, CancellationToken ct)
    {
        try
        {
            await http.PostAsJsonAsync("/indexes", new { uid = "torrents", primaryKey = "infohash" }, ct);
            var settings = new
            {
                searchableAttributes = new[] { "name", "infohash" },
                filterableAttributes = new[] { "category", "policy_action", "risk_tier" },
                sortableAttributes   = new[] { "verified_at", "total_size", "health_score", "popularity_score" },
                typoTolerance = new
                {
                    enabled = true,
                    minWordSizeForTypos = new { oneTypo = 3, twoTypos = 7 }
                },
                pagination = new { maxTotalHits = 100000 }
            };
            await http.PatchAsJsonAsync("/indexes/torrents/settings", settings, ct);
        }
        catch (Exception ex) { _logger.LogWarning(ex, "Index settings setup notice"); }
    }

    private async Task<SyncWatermark> GetWatermarkFromDbAsync(CancellationToken ct)
    {
        try
        {
            using var scope = _services.CreateScope();
            var db = scope.ServiceProvider.GetRequiredService<DatabaseService>();
            await using var conn = await db.OpenConnectionAsync(ct);

            await conn.ExecuteAsync("""
                CREATE TABLE IF NOT EXISTS portal_sync_state (
                    id INT PRIMARY KEY DEFAULT 1,
                    backfill_completed BOOLEAN NOT NULL DEFAULT FALSE,
                    last_cursor_ts TIMESTAMPTZ,
                    last_cursor_hash VARCHAR(40),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                );
            """);

            var row = await conn.QuerySingleOrDefaultAsync<SyncWatermark>(
                "SELECT backfill_completed AS BackfillCompleted, last_cursor_ts AS LastCursorTs, last_cursor_hash AS LastCursorHash FROM portal_sync_state WHERE id = 1;"
            );
            return row ?? new SyncWatermark();
        }
        catch { return new SyncWatermark(); }
    }

    private async Task SaveWatermarkToDbAsync(bool completed, DateTime? cursorTs, string? cursorHash, CancellationToken ct)
    {
        try
        {
            using var scope = _services.CreateScope();
            var db = scope.ServiceProvider.GetRequiredService<DatabaseService>();
            await using var conn = await db.OpenConnectionAsync(ct);

            await conn.ExecuteAsync("""
                INSERT INTO portal_sync_state (id, backfill_completed, last_cursor_ts, last_cursor_hash, updated_at)
                VALUES (1, @completed, @cursorTs, @cursorHash, NOW())
                ON CONFLICT (id) DO UPDATE SET
                    backfill_completed = EXCLUDED.backfill_completed,
                    last_cursor_ts = EXCLUDED.last_cursor_ts,
                    last_cursor_hash = EXCLUDED.last_cursor_hash,
                    updated_at = NOW();
            """, new { completed, cursorTs, cursorHash });
        }
        catch { /* non-fatal */ }
    }
}

public class SyncWatermark
{
    public bool BackfillCompleted { get; set; }
    public DateTime? LastCursorTs { get; set; }
    public string? LastCursorHash { get; set; }
}
