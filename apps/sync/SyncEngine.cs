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
    private readonly ILogger<SyncEngine> _logger;

    private DateTime _lastGenBump = DateTime.MinValue;
    private readonly object _genLock = new();

    private const int BatchSize = 5000;

    public SyncEngine(
        string pgConnString,
        MeiliClient meili,
        SyncStateRepository stateRepo,
        IConnectionMultiplexer redis,
        ILogger<SyncEngine> logger)
    {
        _pgConnString = pgConnString;
        _meili = meili;
        _stateRepo = stateRepo;
        _redis = redis;
        _logger = logger;
    }

    public async Task RunAsync(CancellationToken ct)
    {
        _logger.LogInformation("Starting Gaia.Sync Engine...");

        // Ensure Meilisearch settings
        try
        {
            await _meili.EnsureIndexAndSettingsAsync();
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Failed to apply initial Meilisearch settings.");
        }

        // Generate initial dictionary in background
        _ = Task.Run(() => BuildCorpusDictionaryAsync(ct), ct);

        // Check if backfill is pending
        var backfillState = await _stateRepo.GetStateAsync("backfill");
        if (!backfillState.Completed)
        {
            _logger.LogInformation("Backfill incomplete. Draining backfill to Meilisearch...");
            await DrainBackfillAsync(ct);
        }

        // Start continuous loops
        var tasks = new[]
        {
            RunFastLoopAsync(ct),
            RunHealthLoopAsync(ct),
            RunSuppressionLoopAsync(ct),
            RunDecaySweepLoopAsync(ct),
            RunDictionaryRefreshLoopAsync(ct)
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
                           t.verified_at, t.health_score, t.popularity_score, t.swarm_peers, t.seed_confirmed,
                           t.risk_tier, t.policy_action, t.availability_state,
                           COALESCE(t.last_health_attempt, t.last_health_check, t.verified_at) AS last_health_attempt
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
                           t.verified_at, t.health_score, t.popularity_score, t.swarm_peers, t.seed_confirmed,
                           t.risk_tier, t.policy_action, t.availability_state,
                           COALESCE(t.last_health_attempt, t.last_health_check, t.verified_at) AS last_health_attempt
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
            }
            catch (Exception ex) when (!ct.IsCancellationRequested)
            {
                _logger.LogError(ex, "Error in Fast loop");
                await _stateRepo.RecordErrorAsync("fast", ex.Message);
            }

            await Task.Delay(TimeSpan.FromSeconds(60), ct);
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
                       t.verified_at, t.updated_at, t.health_score, t.popularity_score, t.swarm_peers, t.seed_confirmed,
                       t.risk_tier, t.policy_action, t.availability_state,
                       COALESCE(t.last_health_attempt, t.last_health_check, t.verified_at) AS last_health_attempt
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
            }
            if (toUpsert.Count > 0)
            {
                await _meili.PushBatchAsync(toUpsert);
            }

            var last = rows.Last();
            cursorTs = last.UpdatedAt ?? cursorTs;
            cursorHash = last.Infohash;

            await _stateRepo.UpdateProgressAsync("fast", cursorTs, cursorHash, rows.Count);
            BumpGenerationIfAllowed();

            if (rows.Count < BatchSize) break;
        }
    }

    private async Task RunHealthLoopAsync(CancellationToken ct)
    {
        while (!ct.IsCancellationRequested)
        {
            try
            {
                await DrainHealthLoopAsync(ct);
            }
            catch (Exception ex) when (!ct.IsCancellationRequested)
            {
                _logger.LogError(ex, "Error in Health loop");
                await _stateRepo.RecordErrorAsync("health", ex.Message);
            }

            await Task.Delay(TimeSpan.FromMinutes(15), ct);
        }
    }

    private async Task DrainHealthLoopAsync(CancellationToken ct)
    {
        var state = await _stateRepo.GetStateAsync("health");
        var cursorTs = state.CursorTs ?? DateTime.UtcNow;
        var cursorHash = state.CursorHash ?? string.Empty;

        while (!ct.IsCancellationRequested)
        {
            await using var conn = new NpgsqlConnection(_pgConnString);
            await conn.OpenAsync(ct);

            var query = @"
                SELECT encode(t.infohash, 'hex') AS infohash, t.name, t.category, t.total_size, t.file_count,
                       t.verified_at, t.updated_at, t.health_score, t.popularity_score, t.swarm_peers, t.seed_confirmed,
                       t.risk_tier, t.policy_action, t.availability_state,
                       t.last_health_attempt
                FROM torrents t
                WHERE (t.last_health_attempt, t.infohash) > (@cursorTs, decode(@cursorHash, 'hex'))
                  AND t.policy_action IS DISTINCT FROM 'SUPPRESS'
                ORDER BY t.last_health_attempt ASC, t.infohash ASC
                LIMIT @limit";

            var rows = (await conn.QueryAsync<TorrentDocRow>(query, new { cursorTs, cursorHash, limit = BatchSize })).ToList();
            if (rows.Count == 0) break;

            var docs = rows.Select(MapDocument).ToList();
            await _meili.PushBatchAsync(docs);

            var last = rows.Last();
            cursorTs = last.LastHealthAttempt ?? cursorTs;
            cursorHash = last.Infohash;

            await _stateRepo.UpdateProgressAsync("health", cursorTs, cursorHash, rows.Count);
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

                    var last = rows.Last();
                    cursorTs = (DateTime)last.updated_at;
                    cursorHash = (string)last.infohash;

                    await _stateRepo.UpdateProgressAsync("suppression", cursorTs, cursorHash, rows.Count);
                    BumpGenerationIfAllowed();

                    if (rows.Count < BatchSize) break;
                }
            }
            catch (Exception ex) when (!ct.IsCancellationRequested)
            {
                _logger.LogError(ex, "Error in Suppression loop");
            }

            await Task.Delay(TimeSpan.FromMinutes(10), ct);
        }
    }

    private async Task RunDecaySweepLoopAsync(CancellationToken ct)
    {
        while (!ct.IsCancellationRequested)
        {
            try
            {
                await DrainDecaySweepAsync(ct);
            }
            catch (Exception ex) when (!ct.IsCancellationRequested)
            {
                _logger.LogError(ex, "Error in Decay sweep loop");
                await _stateRepo.RecordErrorAsync("decay", ex.Message);
            }

            await Task.Delay(TimeSpan.FromHours(6), ct);
        }
    }

    private async Task DrainDecaySweepAsync(CancellationToken ct)
    {
        var state = await _stateRepo.GetStateAsync("decay");
        var cursorTs = state.CursorTs ?? new DateTime(1970, 1, 1, 0, 0, 0, DateTimeKind.Utc);
        var cursorHash = state.CursorHash ?? string.Empty;

        while (!ct.IsCancellationRequested)
        {
            await using var conn = new NpgsqlConnection(_pgConnString);
            await conn.OpenAsync(ct);

            var query = @"
                SELECT encode(t.infohash, 'hex') AS infohash, t.name, t.category, t.total_size, t.file_count,
                       t.verified_at, t.health_score, t.popularity_score, t.swarm_peers, t.seed_confirmed,
                       t.risk_tier, t.policy_action, t.availability_state,
                       COALESCE(t.last_health_attempt, t.last_health_check, t.verified_at) AS last_health_attempt,
                       t.last_decay_sweep
                FROM torrents t
                WHERE (COALESCE(t.last_decay_sweep, '1970-01-01 00:00:00+00'::timestamptz), t.infohash) > (@cursorTs, decode(@cursorHash, 'hex'))
                  AND COALESCE(t.last_health_attempt, t.last_health_check, t.verified_at) < now() - interval '7 days'
                  AND t.health_score > 0
                  AND t.policy_action IS DISTINCT FROM 'SUPPRESS'
                ORDER BY COALESCE(t.last_decay_sweep, '1970-01-01 00:00:00+00'::timestamptz) ASC, t.infohash ASC
                LIMIT @limit";

            var rows = (await conn.QueryAsync<TorrentDocRow>(query, new { cursorTs, cursorHash, limit = BatchSize })).ToList();
            if (rows.Count == 0)
            {
                // Reset cursor for next 6-hour cycle
                await _stateRepo.UpdateProgressAsync("decay", new DateTime(1970, 1, 1, 0, 0, 0, DateTimeKind.Utc), string.Empty, 0);
                break;
            }

            var docs = rows.Select(MapDocument).ToList();
            await _meili.PushBatchAsync(docs);

            // Stamp last_decay_sweep in Postgres
            var byteaHashes = rows.Select(r => Convert.FromHexString(r.Infohash)).ToArray();
            await conn.ExecuteAsync(
                "UPDATE torrents SET last_decay_sweep = now() WHERE infohash = ANY(@byteaHashes)",
                new { byteaHashes });

            var last = rows.Last();
            cursorTs = last.LastDecaySweep ?? new DateTime(1970, 1, 1, 0, 0, 0, DateTimeKind.Utc);
            cursorHash = last.Infohash;

            await _stateRepo.UpdateProgressAsync("decay", cursorTs, cursorHash, rows.Count);
            BumpGenerationIfAllowed();

            if (rows.Count < BatchSize)
            {
                await _stateRepo.UpdateProgressAsync("decay", new DateTime(1970, 1, 1, 0, 0, 0, DateTimeKind.Utc), string.Empty, 0);
                break;
            }
        }
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

            var db = _redis.GetDatabase();
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
            var db = _redis.GetDatabase();
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
        var decayedHealth = ScoreDecay.ComputeDecayedHealth(r.HealthScore, r.LastHealthAttempt, r.SwarmPeers, r.SeedConfirmed);

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
            health_score = decayedHealth,
            popularity_score = r.PopularityScore,
            swarm_peers = r.SwarmPeers,
            seed_confirmed = r.SeedConfirmed,
            risk_tier = r.RiskTier ?? "SAFE",
            policy_action = r.PolicyAction ?? "ALLOW",
            availability_state = r.AvailabilityState ?? "ACTIVE"
        };
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
    public short HealthScore { get; set; }
    public short PopularityScore { get; set; }
    public int SwarmPeers { get; set; }
    public bool SeedConfirmed { get; set; }
    public string? RiskTier { get; set; }
    public string? PolicyAction { get; set; }
    public string? AvailabilityState { get; set; }
    public DateTime? LastHealthAttempt { get; set; }
    public DateTime? LastDecaySweep { get; set; }
}
