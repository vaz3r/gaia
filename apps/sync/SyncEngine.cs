using System.Data;
using Dapper;
using Microsoft.Extensions.Logging;
using Npgsql;
using StackExchange.Redis;

namespace Gaia.Sync;

public class SyncEngine
{
    private readonly string _pgConnString;
    private readonly MeiliClient _meili;
    private readonly SyncStateRepository _stateRepo;
    private readonly IConnectionMultiplexer _redis;
    private readonly SearchReconciler _reconciler;
    private readonly ILogger<SyncEngine> _logger;

    private DateTime _lastGenBump = DateTime.MinValue;
    private readonly object _genLock = new();

    private const int BatchSize = 5000;

    public SyncEngine(
        string pgConnString,
        MeiliClient meili,
        SyncStateRepository stateRepo,
        IConnectionMultiplexer redis,
        SearchReconciler reconciler,
        ILogger<SyncEngine> logger)
    {
        _pgConnString = pgConnString;
        _meili = meili;
        _stateRepo = stateRepo;
        _redis = redis;
        _reconciler = reconciler;
        _logger = logger;
    }

    public async Task RunAsync(CancellationToken ct)
    {
        _logger.LogInformation("Starting Gaia.Sync Engine (Decoupled Architecture)...");

        // Ensure live Meilisearch settings
        try
        {
            await _meili.EnsureIndexAndSettingsAsync("torrents");
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Failed to apply initial Meilisearch settings.");
        }

        // Generate initial dictionary in background
        _ = Task.Run(() => BuildCorpusDictionaryAsync(ct), ct);

        // Check if initial backfill is pending
        var backfillState = await _stateRepo.GetStateAsync("backfill");
        if (!backfillState.Completed)
        {
            _logger.LogInformation("Backfill incomplete. Draining backfill to Meilisearch...");
            await DrainBackfillAsync(ct);
        }

        // Start continuous decoupled loops (Fast Ingest, Suppression, Dictionary, Nightly Drift Repair, and Hourly Reconciler)
        // Note: Health Loop and Decay Sweep Loop are permanently deleted; volatile metrics reside in Redis DB 1.
        var tasks = new[]
        {
            RunFastLoopAsync(ct),
            RunSuppressionLoopAsync(ct),
            RunDictionaryRefreshLoopAsync(ct),
            RunNightlyDriftRepairLoopAsync(ct),
            _reconciler.RunHourlyLoopAsync(ct)
        };

        await Task.WhenAll(tasks);
    }

    private async Task DrainBackfillAsync(CancellationToken ct)
    {
        var state = await _stateRepo.GetStateAsync("backfill");
        var cursorTs = state.CursorTs;
        var cursorHash = state.CursorHash;

        while (!ct.IsCancellationRequested)
        {
            await using var conn = new NpgsqlConnection(_pgConnString);
            await conn.OpenAsync(ct);

            string query;
            object parameters;

            if (cursorTs == null || string.IsNullOrEmpty(cursorHash))
            {
                query = @"
                    SELECT encode(t.infohash, 'hex') AS infohash, t.name, t.category, t.total_size, t.file_count,
                           t.verified_at, t.updated_at, t.risk_tier, t.policy_action, t.availability_state,
                           (CASE WHEN COALESCE(t.popularity_score, 0) >= 75 THEN 4
                                 WHEN COALESCE(t.popularity_score, 0) >= 50 THEN 3
                                 WHEN COALESCE(t.popularity_score, 0) >= 25 THEN 2
                                 WHEN COALESCE(t.popularity_score, 0) >= 5  THEN 1
                                 ELSE 0 END) AS popularity_tier
                    FROM torrents t
                    WHERE t.policy_action IS DISTINCT FROM 'SUPPRESS'
                    ORDER BY t.verified_at ASC, t.infohash ASC
                    LIMIT @limit";
                parameters = new { limit = BatchSize };
            }
            else
            {
                query = @"
                    SELECT encode(t.infohash, 'hex') AS infohash, t.name, t.category, t.total_size, t.file_count,
                           t.verified_at, t.updated_at, t.risk_tier, t.policy_action, t.availability_state,
                           (CASE WHEN COALESCE(t.popularity_score, 0) >= 75 THEN 4
                                 WHEN COALESCE(t.popularity_score, 0) >= 50 THEN 3
                                 WHEN COALESCE(t.popularity_score, 0) >= 25 THEN 2
                                 WHEN COALESCE(t.popularity_score, 0) >= 5  THEN 1
                                 ELSE 0 END) AS popularity_tier
                    FROM torrents t
                    WHERE (t.verified_at, t.infohash) > (@cursorTs, decode(@cursorHash, 'hex'))
                      AND t.policy_action IS DISTINCT FROM 'SUPPRESS'
                    ORDER BY t.verified_at ASC, t.infohash ASC
                    LIMIT @limit";
                parameters = new { cursorTs, cursorHash, limit = BatchSize };
            }

            var rows = (await conn.QueryAsync<TorrentDocRow>(query, parameters)).ToList();
            if (rows.Count == 0)
            {
                _logger.LogInformation("Backfill finished completely! Marking completed.");
                await _stateRepo.MarkCompletedAsync("backfill");
                BumpGenerationIfAllowed();
                break;
            }

            var docs = rows.Select(MapDocument).ToList();
            await _meili.PushBatchAsync(docs, waitForTask: false);

            var last = rows.Last();
            cursorTs = last.VerifiedAt;
            cursorHash = last.Infohash;

            await _stateRepo.UpdateProgressAsync("backfill", cursorTs ?? DateTime.UtcNow, cursorHash, rows.Count);
            BumpGenerationIfAllowed();

            if (rows.Count < BatchSize)
            {
                _logger.LogInformation("Backfill drained to the end. Marking completed.");
                await _stateRepo.MarkCompletedAsync("backfill");
                break;
            }
        }
    }

    private async Task RunFastLoopAsync(CancellationToken ct)
    {
        while (!ct.IsCancellationRequested)
        {
            try
            {
                await DrainFastLoopAsync(ct);
                await _stateRepo.TouchLoopAsync("fast");
            }
            catch (Exception ex) when (!ct.IsCancellationRequested)
            {
                _logger.LogError(ex, "Error in Fast loop");
                await _stateRepo.RecordErrorAsync("fast", ex.Message);
            }

            await Task.Delay(TimeSpan.FromSeconds(30), ct);
        }
    }

    private async Task DrainFastLoopAsync(CancellationToken ct)
    {
        var state = await _stateRepo.GetStateAsync("fast");
        var cursorTs = state.CursorTs ?? DateTime.UtcNow;
        var cursorHash = state.CursorHash ?? string.Empty;

        while (!ct.IsCancellationRequested)
        {
            await using var conn = new NpgsqlConnection(_pgConnString);
            await conn.OpenAsync(ct);

            var query = @"
                SELECT encode(t.infohash, 'hex') AS infohash, t.name, t.category, t.total_size, t.file_count,
                       t.verified_at, t.updated_at, t.risk_tier, t.policy_action, t.availability_state,
                       (CASE WHEN COALESCE(t.popularity_score, 0) >= 75 THEN 4
                             WHEN COALESCE(t.popularity_score, 0) >= 50 THEN 3
                             WHEN COALESCE(t.popularity_score, 0) >= 25 THEN 2
                             WHEN COALESCE(t.popularity_score, 0) >= 5  THEN 1
                             ELSE 0 END) AS popularity_tier
                FROM torrents t
                WHERE (t.updated_at, t.infohash) > (@cursorTs, decode(@cursorHash, 'hex'))
                ORDER BY t.updated_at ASC, t.infohash ASC
                LIMIT @limit";

            var rows = (await conn.QueryAsync<TorrentDocRow>(query, new { cursorTs, cursorHash, limit = BatchSize })).ToList();
            if (rows.Count == 0) break;

            var toDelete = rows.Where(r => r.PolicyAction == "SUPPRESS").Select(r => r.Infohash).ToList();
            var toUpsert = rows.Where(r => r.PolicyAction != "SUPPRESS").Select(MapDocument).ToList();

            if (toDelete.Count > 0)
            {
                await _meili.DeleteBatchAsync(toDelete);

                // Purge suppressed from Redis DB 1
                var redisDb1 = _redis.GetDatabase(1);
                var redisBatch = redisDb1.CreateBatch();
                foreach (var h in toDelete) { _ = redisBatch.KeyDeleteAsync($"gaia:health:{h}"); }
                redisBatch.Execute();
            }
            if (toUpsert.Count > 0)
            {
                await _meili.PushBatchAsync(toUpsert);

                // Seed unprobed stub in Redis DB 1 for newly discovered torrents
                // Uses When.NotExists (HSETNX) so existing probed metrics and 't' are never overwritten
                var redisDb1 = _redis.GetDatabase(1);
                var redisBatch = redisDb1.CreateBatch();
                foreach (var row in rows.Where(r => r.PolicyAction != "SUPPRESS"))
                {
                    var key = $"gaia:health:{row.Infohash}";
                    _ = redisBatch.HashSetAsync(key, "h", 0, When.NotExists);
                    _ = redisBatch.HashSetAsync(key, "p", 0, When.NotExists);
                    _ = redisBatch.HashSetAsync(key, "s", 0, When.NotExists);
                    _ = redisBatch.HashSetAsync(key, "c", 0, When.NotExists);
                }
                redisBatch.Execute();
            }

            var last = rows.Last();
            cursorTs = last.UpdatedAt ?? cursorTs; // Strictly advances on updated_at
            cursorHash = last.Infohash;

            await _stateRepo.UpdateProgressAsync("fast", cursorTs, cursorHash, rows.Count);
            BumpGenerationIfAllowed();

            if (rows.Count < BatchSize) break;
        }
    }

    private async Task RunSuppressionLoopAsync(CancellationToken ct)
    {
        while (!ct.IsCancellationRequested)
        {
            try
            {
                var state = await _stateRepo.GetStateAsync("suppression");
                var cursorTs = state.CursorTs ?? DateTime.UtcNow;
                var cursorHash = state.CursorHash ?? string.Empty;

                while (!ct.IsCancellationRequested)
                {
                    await using var conn = new NpgsqlConnection(_pgConnString);
                    await conn.OpenAsync(ct);

                    var query = @"
                        SELECT encode(t.infohash, 'hex') AS infohash, t.updated_at
                        FROM torrents t
                        WHERE t.policy_action = 'SUPPRESS'
                          AND (t.updated_at, t.infohash) > (@cursorTs, decode(@cursorHash, 'hex'))
                        ORDER BY t.updated_at ASC, t.infohash ASC
                        LIMIT @limit";

                    var rows = (await conn.QueryAsync<dynamic>(query, new { cursorTs, cursorHash, limit = BatchSize })).ToList();
                    if (rows.Count == 0) break;

                    var hashes = rows.Select(r => (string)r.infohash).ToList();
                    await _meili.DeleteBatchAsync(hashes);

                    // Purge suppressed from Redis DB 1
                    var redisDb1 = _redis.GetDatabase(1);
                    var redisBatch = redisDb1.CreateBatch();
                    foreach (var h in hashes) { _ = redisBatch.KeyDeleteAsync($"gaia:health:{h}"); }
                    redisBatch.Execute();

                    var last = rows.Last();
                    cursorTs = (DateTime)last.updated_at;
                    cursorHash = (string)last.infohash;

                    await _stateRepo.UpdateProgressAsync("suppression", cursorTs, cursorHash, rows.Count);
                    BumpGenerationIfAllowed();

                    if (rows.Count < BatchSize) break;
                }

                await _stateRepo.TouchLoopAsync("suppression");
            }
            catch (Exception ex) when (!ct.IsCancellationRequested)
            {
                _logger.LogError(ex, "Error in Suppression loop");
            }

            await Task.Delay(TimeSpan.FromSeconds(60), ct);
        }
    }

    public async Task CatchUpTorrentsV2Async(DateTime tStart, CancellationToken ct = default)
    {
        _logger.LogInformation("Starting pre-swap catch-up drain on torrents_v2 from {TStart}...", tStart);

        // 1. Keyset Upsert Drain against torrents_v2
        var cursorTs = tStart;
        var cursorHash = string.Empty;

        while (!ct.IsCancellationRequested)
        {
            await using var conn = new NpgsqlConnection(_pgConnString);
            await conn.OpenAsync(ct);

            var query = @"
                SELECT encode(t.infohash, 'hex') AS infohash, t.name, t.category, t.total_size, t.file_count,
                       t.verified_at, t.updated_at, t.risk_tier, t.policy_action, t.availability_state,
                       (CASE WHEN COALESCE(t.popularity_score, 0) >= 75 THEN 4
                             WHEN COALESCE(t.popularity_score, 0) >= 50 THEN 3
                             WHEN COALESCE(t.popularity_score, 0) >= 25 THEN 2
                             WHEN COALESCE(t.popularity_score, 0) >= 5  THEN 1
                             ELSE 0 END) AS popularity_tier
                FROM torrents t
                WHERE (t.updated_at, t.infohash) > (@cursorTs, decode(@cursorHash, 'hex'))
                  AND t.updated_at >= @tStart
                  AND t.policy_action IS DISTINCT FROM 'SUPPRESS'
                ORDER BY t.updated_at ASC, t.infohash ASC
                LIMIT @limit;";

            var rows = (await conn.QueryAsync<TorrentDocRow>(query, new { cursorTs, cursorHash, tStart, limit = BatchSize })).ToList();
            if (rows.Count == 0) break;

            var docs = rows.Select(MapDocument).ToList();
            await _meili.PushBatchToIndexAsync("torrents_v2", docs, waitForTask: true);

            var last = rows.Last();
            cursorTs = last.UpdatedAt ?? cursorTs; // Strictly advances on updated_at
            cursorHash = last.Infohash;

            if (rows.Count < BatchSize) break;
        }

        // 2. Keyset Suppression Drain against torrents_v2 (Purges resurrected suppressions)
        cursorTs = tStart;
        cursorHash = string.Empty;

        while (!ct.IsCancellationRequested)
        {
            await using var conn = new NpgsqlConnection(_pgConnString);
            await conn.OpenAsync(ct);

            var suppQuery = @"
                SELECT encode(t.infohash, 'hex') AS infohash, t.updated_at
                FROM torrents t
                WHERE (t.updated_at, t.infohash) > (@cursorTs, decode(@cursorHash, 'hex'))
                  AND t.updated_at >= @tStart
                  AND t.policy_action = 'SUPPRESS'
                ORDER BY t.updated_at ASC, t.infohash ASC
                LIMIT @limit;";

            var suppRows = (await conn.QueryAsync<dynamic>(suppQuery, new { cursorTs, cursorHash, tStart, limit = BatchSize })).ToList();
            if (suppRows.Count == 0) break;

            var hashes = suppRows.Select(r => (string)r.infohash).ToList();
            await _meili.DeleteBatchFromIndexAsync("torrents_v2", hashes, waitForTask: true);

            // Delete from Redis DB 1
            var redisDb1 = _redis.GetDatabase(1);
            var redisBatch = redisDb1.CreateBatch();
            foreach (var h in hashes) { _ = redisBatch.KeyDeleteAsync($"gaia:health:{h}"); }
            redisBatch.Execute();

            var last = suppRows.Last();
            cursorTs = (DateTime)last.updated_at;
            cursorHash = (string)last.infohash;

            if (suppRows.Count < BatchSize) break;
        }

        // 3. Verify queue is fully drained
        await _meili.WaitForIndexIdleAsync("torrents_v2");
        _logger.LogInformation("Pre-swap catch-up drain complete. Task queue for torrents_v2 is idle.");
    }

    public async Task RebuildTorrentsV2AndSwapAsync(CancellationToken ct = default)
    {
        _logger.LogInformation("===============================================================");
        _logger.LogInformation("STARTING ZERO-DOWNTIME REBUILD INTO torrents_v2");
        _logger.LogInformation("===============================================================");

        // 1. Clock T_start from PostgreSQL
        DateTime tStart;
        await using (var conn = new NpgsqlConnection(_pgConnString))
        {
            await conn.OpenAsync(ct);
            tStart = await conn.ExecuteScalarAsync<DateTime>("SELECT now() - interval '1 minute'");
        }
        _logger.LogInformation("Recorded T_start watermark from PostgreSQL: {TStart}", tStart);

        // 2. Prepare torrents_v2 index and settings
        await _meili.DeleteIndexAsync("torrents_v2");
        await _meili.EnsureIndexAndSettingsAsync("torrents_v2");

        // 3. Stream all active rows from PostgreSQL into torrents_v2
        var cursorHash = string.Empty;
        int totalIngested = 0;
        const int bulkBatchSize = 10000;
        var sw = System.Diagnostics.Stopwatch.StartNew();

        while (!ct.IsCancellationRequested)
        {
            await using var conn = new NpgsqlConnection(_pgConnString);
            await conn.OpenAsync(ct);

            var query = string.IsNullOrEmpty(cursorHash)
                ? @"
                    SELECT encode(t.infohash, 'hex') AS infohash, t.name, t.category, t.total_size, t.file_count,
                           t.verified_at, t.updated_at, t.risk_tier, t.policy_action, t.availability_state,
                           (CASE WHEN COALESCE(t.popularity_score, 0) >= 75 THEN 4
                                 WHEN COALESCE(t.popularity_score, 0) >= 50 THEN 3
                                 WHEN COALESCE(t.popularity_score, 0) >= 25 THEN 2
                                 WHEN COALESCE(t.popularity_score, 0) >= 5  THEN 1
                                 ELSE 0 END) AS popularity_tier
                    FROM torrents t
                    WHERE t.policy_action IS DISTINCT FROM 'SUPPRESS'
                    ORDER BY t.infohash ASC
                    LIMIT @limit;"
                : @"
                    SELECT encode(t.infohash, 'hex') AS infohash, t.name, t.category, t.total_size, t.file_count,
                           t.verified_at, t.updated_at, t.risk_tier, t.policy_action, t.availability_state,
                           (CASE WHEN COALESCE(t.popularity_score, 0) >= 75 THEN 4
                                 WHEN COALESCE(t.popularity_score, 0) >= 50 THEN 3
                                 WHEN COALESCE(t.popularity_score, 0) >= 25 THEN 2
                                 WHEN COALESCE(t.popularity_score, 0) >= 5  THEN 1
                                 ELSE 0 END) AS popularity_tier
                    FROM torrents t
                    WHERE t.infohash > decode(@cursorHash, 'hex')
                      AND t.policy_action IS DISTINCT FROM 'SUPPRESS'
                    ORDER BY t.infohash ASC
                    LIMIT @limit;";

            var rows = (await conn.QueryAsync<TorrentDocRow>(query, new { cursorHash, limit = bulkBatchSize })).ToList();
            if (rows.Count == 0) break;

            var docs = rows.Select(MapDocument).ToList();
            await _meili.PushBatchToIndexAsync("torrents_v2", docs, waitForTask: false);

            var last = rows.Last();
            cursorHash = last.Infohash;
            totalIngested += rows.Count;

            // Bounded queue backpressure: if Meilisearch has >= 3 tasks pending, wait a moment
            while (await _meili.GetPendingTaskCountAsync("torrents_v2") >= 3 && !ct.IsCancellationRequested)
            {
                await Task.Delay(500, ct);
            }

            if (totalIngested % 100000 == 0)
            {
                var rate = totalIngested / sw.Elapsed.TotalSeconds;
                _logger.LogInformation("Ingested {Count} records into torrents_v2 ({Rate:F0} docs/sec)...", totalIngested, rate);
            }

            if (rows.Count < bulkBatchSize) break;
        }

        // Wait for bulk ingestion tasks to settle in Meilisearch
        _logger.LogInformation("Bulk stream complete ({Count} records). Waiting for indexing tasks to settle...", totalIngested);
        await _meili.WaitForIndexIdleAsync("torrents_v2");

        // 4. Full Keyset Pre-Swap Catch-Up Drain
        _logger.LogInformation("Executing Pre-Swap Keyset Catch-Up Drain...");
        await CatchUpTorrentsV2Async(tStart, ct);

        // 5. Atomic Index Swap
        _logger.LogInformation("Executing atomic pointer swap POST /swap-indexes ['torrents', 'torrents_v2']...");
        await _meili.SwapIndexesAsync("torrents", "torrents_v2", ct);

        sw.Stop();
        _logger.LogInformation("===============================================================");
        _logger.LogInformation("ZERO-DOWNTIME SWAP COMPLETE in {Elapsed:F1}s!", sw.Elapsed.TotalSeconds);
        _logger.LogInformation("'torrents' is now serving the newly rebuilt decoupled index.");
        _logger.LogInformation("'torrents_v2' contains the previous index and is retained for 1-hour rollback safety.");
        _logger.LogInformation("===============================================================");

        BumpGenerationIfAllowed();
    }

    private async Task RunDictionaryRefreshLoopAsync(CancellationToken ct)
    {
        while (!ct.IsCancellationRequested)
        {
            await Task.Delay(TimeSpan.FromHours(24), ct);
            try
            {
                await BuildCorpusDictionaryAsync(ct);
            }
            catch (Exception ex) when (!ct.IsCancellationRequested)
            {
                _logger.LogError(ex, "Failed to refresh corpus dictionary in Redis");
            }
        }
    }

    public async Task BuildCorpusDictionaryAsync(CancellationToken ct = default)
    {
        _logger.LogInformation("Generating corpus title vocabulary from PostgreSQL for query rewriting...");
        try
        {
            await using var conn = new NpgsqlConnection(_pgConnString);
            await conn.OpenAsync(ct);

            var sql = @"
                SELECT word FROM (
                  SELECT word, nentry FROM ts_stat($$SELECT to_tsvector('simple', name) FROM torrents WHERE policy_action IS DISTINCT FROM 'SUPPRESS' LIMIT 500000$$)
                  WHERE length(word) >= 3
                  ORDER BY nentry DESC
                  LIMIT 50000
                ) t";

            var words = (await conn.QueryAsync<string>(sql)).ToList();
            if (words.Count == 0)
            {
                _logger.LogWarning("No vocabulary words generated from database.");
                return;
            }

            var db = _redis.GetDatabase(0);
            var key = "gaia:vocab:titles";
            var tempKey = "gaia:vocab:titles:temp";

            await db.KeyDeleteAsync(tempKey);

            // Batch add to temp key
            const int redisBatchSize = 1000;
            for (int i = 0; i < words.Count; i += redisBatchSize)
            {
                var chunk = words.Skip(i).Take(redisBatchSize).Select(w => (RedisValue)w.ToLowerInvariant()).ToArray();
                await db.SetAddAsync(tempKey, chunk);
            }

            // Rename temp key to live key atomically
            await db.KeyRenameAsync(tempKey, key);
            _logger.LogInformation("Successfully populated {Count} words into Redis vocabulary set '{Key}'", words.Count, key);
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Error generating corpus title vocabulary.");
        }
    }

    private void BumpGenerationIfAllowed()
    {
        lock (_genLock)
        {
            if ((DateTime.UtcNow - _lastGenBump).TotalMinutes < 5.0) return;
            _lastGenBump = DateTime.UtcNow;
        }

        try
        {
            var db = _redis.GetDatabase(0);
            db.StringIncrement("gaia:search:gen");
        }
        catch (Exception ex)
        {
            _logger.LogWarning("Failed to bump Redis search generation: {Msg}", ex.Message);
        }
    }

    private static object MapDocument(TorrentDocRow r)
    {
        var cleanName = NameCleaner.Clean(r.Name);
        long? verifiedUnix = r.VerifiedAt.HasValue ? new DateTimeOffset(r.VerifiedAt.Value).ToUnixTimeSeconds() : null;

        return new
        {
            infohash = r.Infohash,
            name = r.Name ?? string.Empty,
            name_clean = cleanName,
            category = r.Category ?? "Other",
            total_size = r.TotalSize,
            file_count = r.FileCount,
            verified_at = verifiedUnix,
            popularity_tier = r.PopularityTier,
            risk_tier = r.RiskTier ?? "SAFE",
            policy_action = r.PolicyAction ?? "ALLOW",
            availability_state = r.AvailabilityState ?? "ACTIVE"
        };
    }

    private async Task RunNightlyDriftRepairLoopAsync(CancellationToken ct)
    {
        _logger.LogInformation("Starting scheduled daily Redis drift repair loop...");
        // Wait 1 hour after startup before first scheduled self-healing pass
        try
        {
            await Task.Delay(TimeSpan.FromHours(1), ct);
        }
        catch (OperationCanceledException)
        {
            return;
        }

        while (!ct.IsCancellationRequested)
        {
            try
            {
                _logger.LogInformation("Executing scheduled daily Redis health drift repair...");
                using var lf = LoggerFactory.Create(b => b.AddSimpleConsole());
                var seederLogger = lf.CreateLogger<RedisHealthSeeder>();
                var seeder = new RedisHealthSeeder(_pgConnString, _redis, seederLogger);
                var seeded = await seeder.SeedAsync(ct);
                _logger.LogInformation("Daily Redis health drift repair completed: {Count:N0} records synced.", seeded);
            }
            catch (OperationCanceledException) when (ct.IsCancellationRequested)
            {
                break;
            }
            catch (Exception ex)
            {
                _logger.LogError(ex, "Unexpected error during daily Redis health drift repair.");
            }

            try
            {
                await Task.Delay(TimeSpan.FromHours(24), ct);
            }
            catch (OperationCanceledException)
            {
                break;
            }
        }
    }
}

public class TorrentDocRow
{
    public string Infohash { get; set; } = string.Empty;
    public string? Name { get; set; }
    public string? Category { get; set; }
    public long TotalSize { get; set; }
    public int FileCount { get; set; }
    public DateTime? VerifiedAt { get; set; }
    public DateTime? UpdatedAt { get; set; }
    public int PopularityTier { get; set; }
    public string? RiskTier { get; set; }
    public string? PolicyAction { get; set; }
    public string? AvailabilityState { get; set; }
}
