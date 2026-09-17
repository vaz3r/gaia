using System.Data;
using System.Diagnostics;
using Dapper;
using Microsoft.Extensions.Logging;
using Npgsql;
using StackExchange.Redis;

namespace Gaia.Sync;

public class SyncEngine
{
    private readonly string _pgConnString;
    private readonly MeiliClient _meili;
    private readonly IConnectionMultiplexer _redis;
    private readonly ILogger<SyncEngine> _logger;
    private readonly TimeSpan _cadence;

    // Concurrency lock: Gaia.Sync is deployed as a single-instance container on CT 115.
    // This process-wide Semaphore prevents overlapping runs between the scheduled timer and manual/CLI triggers.
    private static readonly SemaphoreSlim _rebuildLock = new(1, 1);
    private const int BulkBatchSize = 20000;

    public SyncEngine(
        string pgConnString,
        MeiliClient meili,
        IConnectionMultiplexer redis,
        ILogger<SyncEngine> logger)
    {
        _pgConnString = pgConnString;
        _meili = meili;
        _redis = redis;
        _logger = logger;

        var cadenceMinutes = int.TryParse(Environment.GetEnvironmentVariable("REBUILD_CADENCE_MINUTES"), out var m) && m > 0 ? m : 120;
        _cadence = TimeSpan.FromMinutes(cadenceMinutes);
    }

    public async Task RunAsync(CancellationToken ct)
    {
        _logger.LogInformation("Starting Gaia.Sync Stateless Rebuild Engine (Cadence: {Cadence}m)...", _cadence.TotalMinutes);

        // Ensure live index exists on startup
        await _meili.EnsureIndexAndSettingsAsync("torrents");

        var tasks = new[]
        {
            RunPeriodicRebuildLoopAsync(ct),
            RunHealthMonitorLoopAsync(ct)
        };

        await Task.WhenAll(tasks);
    }

    private async Task RunPeriodicRebuildLoopAsync(CancellationToken ct)
    {
        if (Environment.GetEnvironmentVariable("INITIAL_REBUILD_ON_STARTUP") == "true")
        {
            _logger.LogInformation("INITIAL_REBUILD_ON_STARTUP is enabled. Launching initial rebuild...");
            await ExecuteRebuildAndSwapAsync(ct);
        }

        while (!ct.IsCancellationRequested)
        {
            try
            {
                await Task.Delay(_cadence, ct);
                await ExecuteRebuildAndSwapAsync(ct);
            }
            catch (OperationCanceledException) when (ct.IsCancellationRequested)
            {
                break;
            }
            catch (Exception ex)
            {
                _logger.LogError(ex, "Error occurred during periodic index rebuild. Retrying at next scheduled cadence window.");
            }
        }
    }

    public async Task ExecuteRebuildAndSwapAsync(CancellationToken ct)
    {
        // Guard 1: Concurrency overlap prevention
        if (!await _rebuildLock.WaitAsync(0, ct))
        {
            _logger.LogWarning("Rebuild is already in progress. Skipping overlapping trigger.");
            return;
        }

        const string shadowIndex = "torrents_shadow";
        const string liveIndex = "torrents";

        try
        {
            _logger.LogInformation("=== STARTING PERIODIC FULL REBUILD INTO {Shadow} ===", shadowIndex);
            var sw = Stopwatch.StartNew();

            // 1. Fetch baseline active count from PostgreSQL for Low-Watermark Guard
            long baselineCount;
            await using (var countConn = new NpgsqlConnection(_pgConnString))
            {
                await countConn.OpenAsync(ct);
                baselineCount = await countConn.ExecuteScalarAsync<long>(
                    "SELECT count(*) FROM torrents WHERE policy_action IS DISTINCT FROM 'SUPPRESS';");
            }
            _logger.LogInformation("Baseline active records in PostgreSQL: {Count:N0}", baselineCount);

            // 2. Clean up any leftover shadow index and configure canonical settings
            await _meili.DeleteIndexAsync(shadowIndex);
            await _meili.EnsureIndexAndSettingsAsync(shadowIndex);

            // 3. Stream all active rows ordered strictly by infohash primary key
            var cursorHash = string.Empty;
            int totalIngested = 0;

            while (!ct.IsCancellationRequested)
            {
                await using var conn = new NpgsqlConnection(_pgConnString);
                await conn.OpenAsync(ct);

                var query = string.IsNullOrEmpty(cursorHash)
                    ? @"
                        SELECT encode(t.infohash, 'hex') AS infohash, t.name, t.category, t.total_size, t.file_count,
                               t.verified_at, t.risk_tier, t.policy_action, t.availability_state
                        FROM torrents t
                        WHERE t.policy_action IS DISTINCT FROM 'SUPPRESS'
                        ORDER BY t.infohash ASC
                        LIMIT @limit;"
                    : @"
                        SELECT encode(t.infohash, 'hex') AS infohash, t.name, t.category, t.total_size, t.file_count,
                               t.verified_at, t.risk_tier, t.policy_action, t.availability_state
                        FROM torrents t
                        WHERE t.infohash > decode(@cursorHash, 'hex')
                          AND t.policy_action IS DISTINCT FROM 'SUPPRESS'
                        ORDER BY t.infohash ASC
                        LIMIT @limit;";

                var rows = (await conn.QueryAsync<TorrentDocRow>(query, new { cursorHash, limit = BulkBatchSize })).ToList();
                if (rows.Count == 0) break;

                var docs = rows.Select(MapDocument).ToList();
                await _meili.PushBatchToIndexAsync(shadowIndex, docs, waitForTask: false);

                cursorHash = rows.Last().Infohash;
                totalIngested += rows.Count;

                // Backpressure throttle: keep task queue between 2 and 4
                while (await _meili.GetPendingTaskCountAsync(shadowIndex) >= 4 && !ct.IsCancellationRequested)
                {
                    await Task.Delay(500, ct);
                }

                if (totalIngested % 250000 == 0)
                {
                    var rate = totalIngested / sw.Elapsed.TotalSeconds;
                    _logger.LogInformation("Streamed {Count:N0} records ({Rate:F0} docs/sec)...", totalIngested, rate);
                }

                if (rows.Count < BulkBatchSize) break;
            }

            // 4. Wait for indexing queue to drain completely
            _logger.LogInformation("Extracted {Count:N0} records. Waiting for {Shadow} to finish indexing...", totalIngested, shadowIndex);
            await _meili.WaitForIndexIdleAsync(shadowIndex);

            // Guard 2: Task-Failure Check (Zero failed tasks in Meilisearch)
            var failedTasks = await _meili.GetFailedTasksAsync(shadowIndex);
            if (failedTasks.Count > 0)
            {
                _logger.LogCritical("❌ ABORTING SWAP: Detected {Count} failed tasks in {Shadow}! Errors: {Errors}",
                    failedTasks.Count, shadowIndex, string.Join("; ", failedTasks));
                throw new InvalidOperationException($"Rebuild aborted due to {failedTasks.Count} failed Meilisearch indexing tasks. Existing live index preserved.");
            }

            // Guard 3: Row-Count Sanity Guard (Low-watermark check >= 90% of baseline)
            var minRequired = (long)(baselineCount * 0.90);
            if (totalIngested < minRequired)
            {
                _logger.LogCritical("❌ ABORTING SWAP: Ingested row count ({Total:N0}) is below 90% of PostgreSQL baseline ({Min:N0} required of {Baseline:N0})!",
                    totalIngested, minRequired, baselineCount);
                throw new InvalidOperationException($"Rebuild aborted: ingested {totalIngested:N0} docs, expected at least {minRequired:N0}. Existing live index preserved.");
            }

            // 5. Atomic Zero-Downtime Pointer Swap
            _logger.LogInformation("Safety invariants verified. Executing atomic pointer swap POST /swap-indexes ['{Live}', '{Shadow}']...", liveIndex, shadowIndex);
            await _meili.SwapIndexesAsync(liveIndex, shadowIndex, ct);

            // 6. Delete old index to reclaim disk space immediately
            await _meili.DeleteIndexAsync(shadowIndex);

            // 7. Invalidate search cache & record completion stats (NO blanket suppression set clear!)
            var db0 = _redis.GetDatabase(0);
            await db0.StringIncrementAsync("gaia:search:gen");
            await db0.StringSetAsync("gaia:search:last_rebuild", DateTimeOffset.UtcNow.ToUnixTimeSeconds());
            await db0.StringSetAsync("gaia:search:last_doc_count", totalIngested);

            sw.Stop();
            _logger.LogInformation("=== REBUILD & ATOMIC SWAP COMPLETE in {Elapsed:F1}s! ({Count:N0} docs live) ===", sw.Elapsed.TotalSeconds, totalIngested);
        }
        finally
        {
            _rebuildLock.Release();
        }
    }

    private async Task RunHealthMonitorLoopAsync(CancellationToken ct)
    {
        while (!ct.IsCancellationRequested)
        {
            await Task.Delay(TimeSpan.FromMinutes(15), ct);
            try
            {
                var db0 = _redis.GetDatabase(0);
                var lastRebuildRaw = await db0.StringGetAsync("gaia:search:last_rebuild");
                if (lastRebuildRaw.HasValue && lastRebuildRaw.TryParse(out long lastTs))
                {
                    var ageMinutes = (DateTimeOffset.UtcNow.ToUnixTimeSeconds() - lastTs) / 60.0;
                    if (ageMinutes > (_cadence.TotalMinutes * 2.5))
                    {
                        _logger.LogError("⚠️ [ALERT] Periodic index rebuild has not completed in {Age:F0} minutes (expected every {Cadence}m)!", ageMinutes, _cadence.TotalMinutes);
                    }
                    else
                    {
                        _logger.LogInformation("✅ Search health check passed: last rebuild was {Age:F0}m ago.", ageMinutes);
                    }
                }
            }
            catch (Exception ex)
            {
                _logger.LogWarning(ex, "Health monitor check encountered an error.");
            }
        }
    }

    private static object MapDocument(TorrentDocRow r)
    {
        return new
        {
            infohash = r.Infohash,
            name = r.Name,
            name_clean = r.Name,
            category = r.Category ?? "Other",
            total_size = r.TotalSize,
            file_count = r.FileCount,
            verified_at = r.VerifiedAt.HasValue ? new DateTimeOffset(r.VerifiedAt.Value, TimeSpan.Zero).ToUnixTimeSeconds() : 0,
            risk_tier = r.RiskTier ?? "SAFE",
            policy_action = r.PolicyAction ?? "ALLOW",
            availability_state = r.AvailabilityState ?? "ALIVE"
        };
    }
}

public class TorrentDocRow
{
    public string Infohash { get; set; } = string.Empty;
    public string Name { get; set; } = string.Empty;
    public string? Category { get; set; }
    public long TotalSize { get; set; }
    public int FileCount { get; set; }
    public DateTime? VerifiedAt { get; set; }
    public string? RiskTier { get; set; }
    public string? PolicyAction { get; set; }
    public string? AvailabilityState { get; set; }
}
