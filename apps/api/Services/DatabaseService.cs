using System.Text.Json;
using Dapper;
using Microsoft.Extensions.Caching.Memory;
using Npgsql;

namespace Gaia.Api.Services;

public class DatabaseService
{
    private readonly NpgsqlDataSource _dataSource;
    private readonly IMemoryCache _cache;
    private readonly CacheService _redis;
    private readonly ILogger<DatabaseService> _logger;

    public DatabaseService(IConfiguration config, IMemoryCache cache, CacheService redis, ILogger<DatabaseService> logger)
    {
        _cache = cache;
        _redis = redis;
        _logger = logger;
        var connectionString = Environment.GetEnvironmentVariable("DATABASE_URL")
            ?? config["DATABASE_URL"]
            ?? config.GetConnectionString("Postgres") 
            ?? "Host=192.168.10.10;Port=6432;Database=craw;Username=crawler;Password=83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b;Pooling=true;Maximum Pool Size=50;Minimum Pool Size=5;Max Auto Prepare=0;No Reset On Close=true;";

        var builder = new NpgsqlDataSourceBuilder(connectionString);
        _dataSource = builder.Build();
    }

    public NpgsqlDataSource DataSource => _dataSource;

    /// <summary>Pre-warms Npgsql connection pool to eliminate cold-start P99 latency spikes.</summary>
    public async Task WarmUpAsync()
    {
        _logger.LogInformation("Warming up PostgreSQL connection pool...");
        var tasks = Enumerable.Range(0, 5).Select(async _ =>
        {
            try
            {
                await using var conn = await _dataSource.OpenConnectionAsync();
                await conn.ExecuteScalarAsync<int>("SELECT 1");
            }
            catch { /* warmup failure is non-fatal */ }
        });
        await Task.WhenAll(tasks);
        _logger.LogInformation("PostgreSQL connection pool warmed up.");
    }

    /// <summary>Exposes a raw connection for bulk operations (e.g. MeilisearchSyncService).</summary>
    public ValueTask<NpgsqlConnection> OpenConnectionAsync(CancellationToken ct = default)
        => _dataSource.OpenConnectionAsync(ct);

    public async Task<Dictionary<string, object?>?> GetTorrentDetailsAsync(string infohashHex, CancellationToken ct = default)
    {
        if (string.IsNullOrWhiteSpace(infohashHex) || infohashHex.Length != 40)
            return null;

        const string sql = """
            SELECT 
                encode(t.infohash, 'hex') AS infohash, 
                t.name, 
                t.piece_length, 
                t.total_size,
                t.file_count, 
                t.files::text AS files_json, 
                t.fetch_attempts, 
                t.verified_at,
                t.first_seen, 
                t.last_seen, 
                t.total_seen,
                t.health_score, 
                t.popularity_score, 
                t.swarm_peers, 
                t.seed_confirmed, 
                t.last_health_check,
                t.category, 
                t.category_confidence, 
                t.needs_review, 
                t.classified_at, 
                t.classification_meta::text AS classification_meta_json,
                t.integrity_score, 
                t.policy_action, 
                t.risk_tier, 
                t.decision_source,
                t.metadata_quality_score, 
                t.availability_score, 
                t.availability_state, 
                t.scored_at
            FROM torrents t
            WHERE t.infohash = decode(@ih, 'hex')
            LIMIT 1;
        """;

        // ── Redis cache check ─────────────────────────────────────────────────
        var cacheKey = CacheService.MakeKey("detail", infohashHex.ToLowerInvariant());
        var cached   = await _redis.GetAsync<Dictionary<string, object?>>(cacheKey, ct);
        if (cached is not null) return cached;

        try
        {
            await using var conn = await _dataSource.OpenConnectionAsync(ct);
            var row = await conn.QueryFirstOrDefaultAsync<dynamic>(sql, new { ih = infohashHex.ToLowerInvariant() });
            if (row == null) return null;

            var dict = new Dictionary<string, object?>(StringComparer.OrdinalIgnoreCase);
            var rowDict = (IDictionary<string, object?>)row;

            foreach (var kvp in rowDict)
            {
                if (kvp.Key == "files_json")
                {
                    if (kvp.Value is string fjson && !string.IsNullOrWhiteSpace(fjson))
                    {
                        try
                        {
                            using var doc = JsonDocument.Parse(fjson);
                            dict["files"] = doc.RootElement.Clone();
                        }
                        catch
                        {
                            dict["files"] = Array.Empty<object>();
                        }
                    }
                    else
                    {
                        dict["files"] = Array.Empty<object>();
                    }
                }
                else if (kvp.Key == "classification_meta_json")
                {
                    if (kvp.Value is string cjson && !string.IsNullOrWhiteSpace(cjson))
                    {
                        try
                        {
                            using var doc = JsonDocument.Parse(cjson);
                            dict["classification_meta"] = doc.RootElement.Clone();
                        }
                        catch
                        {
                            dict["classification_meta"] = null;
                        }
                    }
                    else
                    {
                        dict["classification_meta"] = null;
                    }
                }
                else
                {
                    dict[kvp.Key] = kvp.Value;
                }
            }

            // Inject pre-built magnet URI
            var name = dict.TryGetValue("name", out var n) ? n?.ToString() : infohashHex;
            dict["magnet"] = TorrentBuilder.BuildMagnetUri(infohashHex, name);

            // Cache for 5 minutes — torrent metadata rarely changes
            await _redis.SetAsync(cacheKey, dict, CacheService.DetailTtl, ct);

            return dict;
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Failed to get complete torrent details for infohash {Infohash}", infohashHex);
            return null;
        }
    }

    private static Dictionary<string, object> _cachedStats = new()
    {
        ["total_torrents"] = 3418496,
        ["verified_last_24h"] = 408710,
        ["healthy_count"] = 951907,
        ["updated_at"] = DateTime.UtcNow.ToString("o")
    };
    private static DateTime _lastStatsRefresh = DateTime.MinValue;
    private static int _isRefreshing = 0;

    public async Task<Dictionary<string, object>> GetDashboardStatsAsync(CancellationToken ct = default)
    {
        // If stats are older than 5 minutes and not already refreshing, kick off async refresh
        if (DateTime.UtcNow - _lastStatsRefresh > TimeSpan.FromMinutes(5))
        {
            if (Interlocked.CompareExchange(ref _isRefreshing, 1, 0) == 0)
            {
                _ = Task.Run(async () =>
                {
                    try
                    {
                        await RefreshDashboardStatsInternalAsync();
                    }
                    finally
                    {
                        Interlocked.Exchange(ref _isRefreshing, 0);
                    }
                });
            }
        }

        return _cachedStats;
    }

    private async Task RefreshDashboardStatsInternalAsync()
    {
        try
        {
            await using var conn = await _dataSource.OpenConnectionAsync();

            // Fast reltuples query for total count (0.1ms)
            var totalCount = await conn.ExecuteScalarAsync<long>(
                "SELECT reltuples::bigint FROM pg_class WHERE relname = 'torrents';"
            );

            var verified24h = await conn.ExecuteScalarAsync<long>(
                "SELECT count(*) FROM torrents WHERE verified_at > NOW() - INTERVAL '24 hours';"
            );

            var healthyCount = await conn.ExecuteScalarAsync<long>(
                "SELECT count(*) FROM torrents WHERE health_score >= 70 AND verified_at > NOW() - INTERVAL '7 days';"
            );

            _cachedStats = new Dictionary<string, object>
            {
                ["total_torrents"] = totalCount > 0 ? totalCount : 3418000,
                ["verified_last_24h"] = verified24h,
                ["healthy_count"] = healthyCount,
                ["updated_at"] = DateTime.UtcNow.ToString("o")
            };
            _lastStatsRefresh = DateTime.UtcNow;
            _logger.LogInformation("Dashboard stats background refresh completed successfully");
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Failed to refresh dashboard stats in background");
        }
    }
}
