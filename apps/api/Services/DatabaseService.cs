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

    public async Task<(List<IDictionary<string, object?>> Items, long Total)> GetBrowseTorrentsAsync(
        string? category = null,
        int page = 1,
        int limit = 25,
        string? sortBy = null,
        string? order = "desc",
        CancellationToken ct = default)
    {
        var safeLimit = Math.Clamp(limit, 1, 100);
        var safePage = Math.Max(1, page);
        var offset = (safePage - 1) * safeLimit;

        var sortCol = sortBy?.ToLowerInvariant() switch
        {
            "size"        => "total_size",
            "files"       => "file_count",
            "health"      => "health_score",
            "popularity"  => "popularity_score",
            "first_seen"  => "first_seen",
            "sightings"   => "total_seen",
            "name"        => "name",
            _             => "verified_at"
        };
        var sortDir = order?.ToLowerInvariant() == "asc" ? "ASC" : "DESC";

        var hasCategory = !string.IsNullOrWhiteSpace(category) && !category.Equals("All", StringComparison.OrdinalIgnoreCase);

        // ── Redis cache check ─────────────────────────────────────────────────
        var browseKey = CacheService.MakeKey("browse", category, safePage, safeLimit, sortCol, sortDir);
        var cachedBrowse = await _redis.GetAsync<BrowseResult>(browseKey, ct);
        if (cachedBrowse is not null)
            return (cachedBrowse.Items, cachedBrowse.Total);

        var sql = $"""
            SELECT 
                encode(infohash, 'hex') AS infohash,
                name,
                category,
                total_size,
                file_count,
                verified_at,
                health_score,
                popularity_score,
                swarm_peers,
                seed_confirmed,
                risk_tier,
                policy_action
            FROM torrents
            WHERE (policy_action IS NULL OR policy_action != 'SUPPRESS')
              {(hasCategory ? "AND category = @cat" : "")}
            ORDER BY {sortCol} {sortDir}
            LIMIT @lim OFFSET @off;
        """;

        try
        {
            await using var conn = await _dataSource.OpenConnectionAsync(ct);
            var rows = await conn.QueryAsync(sql, new
            {
                cat = category?.Trim(),
                lim = safeLimit,
                off = offset
            });

            var list  = rows.Select(r => (IDictionary<string, object?>)r).ToList();
            var total = _cachedStats.TryGetValue("total_torrents", out var t) && t is long tl ? tl : 3418496;

            // Cache for 30 seconds — browse results are hot and stable
            await _redis.SetAsync(browseKey, new BrowseResult { Items = list, Total = total }, CacheService.BrowseTtl, ct);

            return (list, total);
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Failed to browse torrents from database");
            return (new List<IDictionary<string, object?>>(), 0);
        }
    }

    private record BrowseResult
    {
        public List<IDictionary<string, object?>> Items { get; init; } = new();
        public long Total { get; init; }
    }

    /// <summary>
    /// PostgreSQL trigram full-text search — fallback used while Meilisearch is building its index.
    /// Uses the existing idx_torrents_name_trgm GIN index (~77-226ms depending on term specificity).
    /// Results are Redis-cached with the same 60s TTL as Meilisearch results.
    /// </summary>
    public async Task<(List<IDictionary<string, object?>> Items, long Total)> SearchTorrentsAsync(
        string query,
        string? category = null,
        int page = 1,
        int limit = 25,
        string? sortBy = null,
        string? order = "desc",
        CancellationToken ct = default)
    {
        var safeLimit   = Math.Clamp(limit, 1, 100);
        var safePage    = Math.Max(1, page);
        var offset      = (safePage - 1) * safeLimit;
        var hasCategory = !string.IsNullOrWhiteSpace(category)
            && !category.Equals("All", StringComparison.OrdinalIgnoreCase);

        var sortDir = order?.ToLowerInvariant() == "asc" ? "ASC" : "DESC";
        var sortCol = sortBy?.ToLowerInvariant() switch
        {
            "size"        => "total_size",
            "health"      => "health_score",
            "popularity"  => "popularity_score",
            "date" or "verified_at" => "verified_at",
            _             => "popularity_score"   // most popular first by default
        };

        // Redis cache — same key prefix 'search' so it's shared when Meili takes over
        var cacheKey     = CacheService.MakeKey("search-pg", query, category, safePage, safeLimit, sortCol, sortDir);
        var cachedBrowse = await _redis.GetAsync<BrowseResult>(cacheKey, ct);
        if (cachedBrowse is not null)
            return (cachedBrowse.Items, cachedBrowse.Total);

        // Detect infohash lookup (8-40 hex chars)
        var isHash = System.Text.RegularExpressions.Regex.IsMatch(
            query.Trim(), @"^[0-9a-fA-F]{8,40}$");

        var sql = isHash
            ? $"""
                SELECT encode(infohash,'hex') AS infohash, name, category, total_size, file_count,
                       verified_at, health_score, popularity_score, swarm_peers, seed_confirmed, risk_tier, policy_action
                FROM torrents
                WHERE encode(infohash,'hex') ILIKE @pattern
                  AND (policy_action IS NULL OR policy_action != 'SUPPRESS')
                  {(hasCategory ? "AND category = @cat" : "")}
                ORDER BY {sortCol} {sortDir}
                LIMIT @lim OFFSET @off
              """
            : $"""
                SELECT encode(infohash,'hex') AS infohash, name, category, total_size, file_count,
                       verified_at, health_score, popularity_score, swarm_peers, seed_confirmed, risk_tier, policy_action
                FROM torrents
                WHERE name ILIKE @pattern
                  AND (policy_action IS NULL OR policy_action != 'SUPPRESS')
                  {(hasCategory ? "AND category = @cat" : "")}
                ORDER BY {sortCol} {sortDir}
                LIMIT @lim OFFSET @off
              """;

        try
        {
            await using var conn = await _dataSource.OpenConnectionAsync(ct);
            var rows = await conn.QueryAsync(sql, new
            {
                pattern = isHash ? query.Trim().ToLowerInvariant() + "%" : "%" + query.Trim() + "%",
                cat     = category?.Trim(),
                lim     = safeLimit,
                off     = offset
            });

            var list = rows.Select(r => (IDictionary<string, object?>)r).ToList();
            // Use cached stats for total (exact count on trgm is slow)
            var total = _cachedStats.TryGetValue("total_torrents", out var t) && t is long tl ? tl / 100 : 50_000;

            await _redis.SetAsync(cacheKey, new BrowseResult { Items = list, Total = total }, CacheService.SearchTtl, ct);
            return (list, total);
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "PostgreSQL trigram search failed for query: {Query}", query);
            return (new List<IDictionary<string, object?>>(), 0);
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
