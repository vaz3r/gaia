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
        await Task.Delay(TimeSpan.FromSeconds(2), ct);

        using var http = new HttpClient();
        var meiliUrl = _config["Meilisearch:Url"] ?? "http://127.0.0.1:7700";
        var meiliKey = _config["Meilisearch:ApiKey"] ?? "";
        http.BaseAddress = new Uri(meiliUrl);
        if (!string.IsNullOrWhiteSpace(meiliKey))
            http.DefaultRequestHeaders.Add("Authorization", $"Bearer {meiliKey}");

        await EnsureIndexSettingsAsync(http, ct);

        var watermark = await GetWatermarkFromDbAsync(ct);
        if (!watermark.BackfillCompleted)
        {
            _logger.LogInformation("Resuming tie-safe composite backfill from: {CursorTs:u} / {CursorHash}...", 
                watermark.LastCursorTs, watermark.LastCursorHash);
            await KeysetBackfillAsync(http, watermark.LastCursorTs, watermark.LastCursorHash, ct);
        }

        _logger.LogInformation("Starting two-cadence continuous sync: fast loop (60s) + slow health loop (15m)...");
        
        var nextSlowRun = DateTime.UtcNow;

        while (!ct.IsCancellationRequested)
        {
            try
            {
                // 1. Fast Loop (every 60s): metadata, new torrents, classifier changes, suppression
                await RunFastSyncLoopAsync(http, ct);

                // 2. Slow Loop (every 15m): drain swarm health updates
                if (DateTime.UtcNow >= nextSlowRun)
                {
                    await RunSlowHealthSyncLoopAsync(http, ct);
                    nextSlowRun = DateTime.UtcNow.AddMinutes(15);
                }

                await Task.Delay(TimeSpan.FromSeconds(60), ct);
            }
            catch (OperationCanceledException) { break; }
            catch (Exception ex)
            {
                _logger.LogError(ex, "Sync cycle error — retrying in 30s");
                await Task.Delay(TimeSpan.FromSeconds(30), ct);
            }
        }
    }

    // 1. Initial Keyset Backfill
    public async Task KeysetBackfillAsync(HttpClient http, DateTime? initialTs, string? initialHash, CancellationToken ct)
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

                var docs = batch.Select(r =>
                {
                    var dict = (IDictionary<string, object?>)r;
                    dict["name_clean"] = TorrentQueryParser.CleanReleaseName(dict["name"]?.ToString());
                    return dict;
                }).ToList();

                var resp = await http.PutAsJsonAsync("/indexes/torrents/documents", docs, ct);
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

                await Task.Delay(200, ct);
            }
            catch (OperationCanceledException) { break; }
            catch (Exception ex)
            {
                _logger.LogWarning(ex, "Meilisearch backfill error, retrying in 5s...");
                await Task.Delay(5000, ct);
            }
        }

        _logger.LogInformation("Composite keyset backfill complete: {Total:N0} total sent.", totalSent);
    }

    // 2. Fast Loop (60s): metadata + classifier updates + deletions
    private async Task RunFastSyncLoopAsync(HttpClient http, CancellationToken ct)
    {
        using var scope = _services.CreateScope();
        var db = scope.ServiceProvider.GetRequiredService<DatabaseService>();
        await using var conn = await db.OpenConnectionAsync(ct);

        var watermark = await GetWatermarkFromDbAsync(ct);
        DateTime? cursorTs = watermark.FastCursorTs;
        string? cursorHash = watermark.FastCursorHash;

        const int batchSize = 5000;
        const string upsertSql = """
            SELECT encode(infohash, 'hex') AS infohash, name, category, total_size, file_count,
                   EXTRACT(EPOCH FROM verified_at)::bigint AS verified_at,
                   health_score, popularity_score, swarm_peers, seed_confirmed, risk_tier,
                   COALESCE(policy_action, 'ALLOW') AS policy_action,
                   GREATEST(verified_at, scored_at) AS raw_ts,
                   encode(infohash, 'hex') AS raw_hash
            FROM torrents
            WHERE (CAST(@cursorTs AS timestamptz) IS NULL OR (GREATEST(verified_at, scored_at), infohash) > (CAST(@cursorTs AS timestamptz), decode(CAST(@cursorHash AS text), 'hex')))
              AND (policy_action IS NULL OR policy_action != 'SUPPRESS')
            ORDER BY GREATEST(verified_at, scored_at) ASC, infohash ASC
            LIMIT @lim;
        """;

        int totalFast = 0;
        while (!ct.IsCancellationRequested)
        {
            var batch = (await conn.QueryAsync(upsertSql, new { lim = batchSize, cursorTs, cursorHash })).AsList();
            if (batch.Count == 0) break;

            var lastRow = (IDictionary<string, object?>)batch.Last();
            var nextCursorTs = (DateTime)lastRow["raw_ts"]!;
            var nextCursorHash = (string)lastRow["raw_hash"]!;

            var docs = batch.Select(r =>
            {
                var dict = (IDictionary<string, object?>)r;
                dict["name_clean"] = TorrentQueryParser.CleanReleaseName(dict["name"]?.ToString());
                return dict;
            }).ToList();

            var resp = await http.PutAsJsonAsync("/indexes/torrents/documents", docs, ct);
            if (resp.IsSuccessStatusCode)
            {
                cursorTs = nextCursorTs;
                cursorHash = nextCursorHash;
                totalFast += batch.Count;
                await UpdateFastWatermarkAsync(cursorTs, cursorHash, ct);
            }

            if (batch.Count < batchSize) break;
        }

        if (totalFast > 0)
        {
            _logger.LogInformation("Fast loop: {Count:N0} records pushed to Meilisearch", totalFast);
        }

        // Handle deletions for suppressed torrents
        if (cursorTs.HasValue)
        {
            const string deleteSql = """
                SELECT encode(infohash, 'hex') AS infohash
                FROM torrents
                WHERE scored_at >= CAST(@since AS timestamptz)
                  AND policy_action = 'SUPPRESS'
                LIMIT 5000;
            """;

            var suppressedHashes = (await conn.QueryAsync<string>(deleteSql, new { since = cursorTs.Value.AddHours(-1) })).AsList();
            if (suppressedHashes.Count > 0)
            {
                await http.PostAsJsonAsync("/indexes/torrents/documents/delete-batch", suppressedHashes, ct);
                _logger.LogInformation("Fast loop delete: {Count:N0} suppressed torrents removed from Meilisearch", suppressedHashes.Count);
            }
        }
    }

    // 3. Slow Loop (15m): drain swarm health updates without clobbering or phantoms
    private async Task RunSlowHealthSyncLoopAsync(HttpClient http, CancellationToken ct)
    {
        using var scope = _services.CreateScope();
        var db = scope.ServiceProvider.GetRequiredService<DatabaseService>();
        await using var conn = await db.OpenConnectionAsync(ct);

        var watermark = await GetWatermarkFromDbAsync(ct);
        DateTime? cursorTs = watermark.SlowCursorTs;
        string? cursorHash = watermark.SlowCursorHash;

        const int batchSize = 10000;
        // Include full document fields to guarantee 0 phantom documents!
        const string healthSql = """
            SELECT encode(infohash, 'hex') AS infohash, name, category, total_size, file_count,
                   EXTRACT(EPOCH FROM verified_at)::bigint AS verified_at,
                   health_score, popularity_score, swarm_peers, seed_confirmed, risk_tier,
                   COALESCE(policy_action, 'ALLOW') AS policy_action,
                   last_health_check AS raw_ts,
                   encode(infohash, 'hex') AS raw_hash
            FROM torrents
            WHERE last_health_check IS NOT NULL
              AND (CAST(@cursorTs AS timestamptz) IS NULL OR (last_health_check, infohash) > (CAST(@cursorTs AS timestamptz), decode(CAST(@cursorHash AS text), 'hex')))
              AND (policy_action IS NULL OR policy_action != 'SUPPRESS')
            ORDER BY last_health_check ASC, infohash ASC
            LIMIT @lim;
        """;

        int totalHealth = 0;
        while (!ct.IsCancellationRequested)
        {
            var batch = (await conn.QueryAsync(healthSql, new { lim = batchSize, cursorTs, cursorHash })).AsList();
            if (batch.Count == 0) break;

            var lastRow = (IDictionary<string, object?>)batch.Last();
            var nextCursorTs = (DateTime)lastRow["raw_ts"]!;
            var nextCursorHash = (string)lastRow["raw_hash"]!;

            var docs = batch.Select(r =>
            {
                var dict = (IDictionary<string, object?>)r;
                dict["name_clean"] = TorrentQueryParser.CleanReleaseName(dict["name"]?.ToString());
                return dict;
            }).ToList();

            var resp = await http.PutAsJsonAsync("/indexes/torrents/documents", docs, ct);
            if (resp.IsSuccessStatusCode)
            {
                cursorTs = nextCursorTs;
                cursorHash = nextCursorHash;
                totalHealth += batch.Count;
                await UpdateSlowWatermarkAsync(cursorTs, cursorHash, ct);
            }

            if (batch.Count < batchSize) break;
        }

        if (totalHealth > 0)
        {
            _logger.LogInformation("Slow health loop: {Count:N0} health records synced to Meilisearch", totalHealth);
        }
    }

    public static async Task EnsureIndexSettingsAsync(HttpClient http, CancellationToken ct)
    {
        await http.PostAsJsonAsync("/indexes", new { uid = "torrents", primaryKey = "infohash" }, ct);
        var settings = new
        {
            searchableAttributes = new[] { "name", "name_clean" },
            filterableAttributes = new[] { "category", "policy_action", "risk_tier" },
            sortableAttributes   = new[] { "verified_at", "total_size", "health_score", "popularity_score" },
            rankingRules         = new[]
            {
                "words",
                "typo",
                "proximity",
                "attribute",
                "exactness",
                "popularity_score:desc",
                "verified_at:desc"
            },
            stopWords = new[] { "a", "an", "the", "and", "or", "of", "in", "for", "to", "with", "on", "at", "by", "from", "www", "com", "net", "org" },
            separatorTokens = new[] { "_", "-", "+", "[", "]", "(", ")" },
            synonyms = new Dictionary<string, string[]>
            {
                ["4k"] = new[] { "2160p", "uhd" },
                ["2160p"] = new[] { "4k", "uhd" },
                ["uhd"] = new[] { "4k", "2160p" },
                ["1080p"] = new[] { "fhd" },
                ["fhd"] = new[] { "1080p" },
                ["720p"] = new[] { "hd" },
                ["hd"] = new[] { "720p" },
                ["x265"] = new[] { "hevc", "h265" },
                ["hevc"] = new[] { "x265", "h265" },
                ["x264"] = new[] { "avc", "h264" },
                ["avc"] = new[] { "x264", "h264" },
                ["atmos"] = new[] { "truehd" }
            },
            typoTolerance = new
            {
                enabled = true,
                minWordSizeForTypos = new { oneTypo = 3, twoTypos = 7 }
            },
            pagination = new { maxTotalHits = 100000 }
        };
        await http.PatchAsJsonAsync("/indexes/torrents/settings", settings, ct);
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
                    fast_cursor_ts TIMESTAMPTZ,
                    fast_cursor_hash VARCHAR(40),
                    slow_cursor_ts TIMESTAMPTZ,
                    slow_cursor_hash VARCHAR(40),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                );
                ALTER TABLE portal_sync_state ADD COLUMN IF NOT EXISTS fast_cursor_ts TIMESTAMPTZ;
                ALTER TABLE portal_sync_state ADD COLUMN IF NOT EXISTS fast_cursor_hash VARCHAR(40);
                ALTER TABLE portal_sync_state ADD COLUMN IF NOT EXISTS slow_cursor_ts TIMESTAMPTZ;
                ALTER TABLE portal_sync_state ADD COLUMN IF NOT EXISTS slow_cursor_hash VARCHAR(40);
            """);

            var row = await conn.QuerySingleOrDefaultAsync<SyncWatermark>(
                """
                SELECT backfill_completed AS BackfillCompleted, 
                       last_cursor_ts AS LastCursorTs, 
                       last_cursor_hash AS LastCursorHash,
                       fast_cursor_ts AS FastCursorTs,
                       fast_cursor_hash AS FastCursorHash,
                       slow_cursor_ts AS SlowCursorTs,
                       slow_cursor_hash AS SlowCursorHash
                FROM portal_sync_state WHERE id = 1;
                """
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

    private async Task UpdateFastWatermarkAsync(DateTime? cursorTs, string? cursorHash, CancellationToken ct)
    {
        try
        {
            using var scope = _services.CreateScope();
            var db = scope.ServiceProvider.GetRequiredService<DatabaseService>();
            await using var conn = await db.OpenConnectionAsync(ct);

            await conn.ExecuteAsync("""
                UPDATE portal_sync_state 
                SET fast_cursor_ts = @cursorTs, fast_cursor_hash = @cursorHash, updated_at = NOW()
                WHERE id = 1;
            """, new { cursorTs, cursorHash });
        }
        catch { /* non-fatal */ }
    }

    private async Task UpdateSlowWatermarkAsync(DateTime? cursorTs, string? cursorHash, CancellationToken ct)
    {
        try
        {
            using var scope = _services.CreateScope();
            var db = scope.ServiceProvider.GetRequiredService<DatabaseService>();
            await using var conn = await db.OpenConnectionAsync(ct);

            await conn.ExecuteAsync("""
                UPDATE portal_sync_state 
                SET slow_cursor_ts = @cursorTs, slow_cursor_hash = @cursorHash, updated_at = NOW()
                WHERE id = 1;
            """, new { cursorTs, cursorHash });
        }
        catch { /* non-fatal */ }
    }
}

public class SyncWatermark
{
    public bool BackfillCompleted { get; set; }
    public DateTime? LastCursorTs { get; set; }
    public string? LastCursorHash { get; set; }
    public DateTime? FastCursorTs { get; set; }
    public string? FastCursorHash { get; set; }
    public DateTime? SlowCursorTs { get; set; }
    public string? SlowCursorHash { get; set; }
}
