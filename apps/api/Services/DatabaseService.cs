using Dapper;
using Npgsql;

namespace Gaia.Api.Services;

public class DatabaseService
{
    private readonly NpgsqlDataSource _dataSource;
    private readonly ILogger<DatabaseService> _logger;

    public DatabaseService(IConfiguration config, ILogger<DatabaseService> logger)
    {
        _logger = logger;
        var connectionString = config.GetConnectionString("Postgres") 
            ?? config["DATABASE_URL"]
            ?? "Host=localhost;Port=5432;Database=craw;Username=crawler;Password=83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b;Pooling=true;Maximum Pool Size=50;";

        var builder = new NpgsqlDataSourceBuilder(connectionString);
        _dataSource = builder.Build();
    }

    public async Task<TorrentDetailModel?> GetTorrentDetailsAsync(string infohashHex, CancellationToken ct = default)
    {
        if (string.IsNullOrWhiteSpace(infohashHex) || infohashHex.Length != 40)
            return null;

        const string sql = """
            SELECT 
                encode(infohash, 'hex') AS infohash,
                name,
                piece_length AS piecelength,
                total_size AS totalsize,
                file_count AS filecount,
                files::text AS filesjson,
                verified_at AS verifiedat,
                first_seen AS firstseen,
                last_seen AS lastseen,
                total_seen AS totalseen,
                health_score AS healthscore,
                popularity_score AS popularityscore,
                swarm_peers AS swarmpeers,
                seed_confirmed AS seedconfirmed,
                category,
                risk_tier AS risktier,
                policy_action AS policyaction
            FROM torrents
            WHERE infohash = decode(@InfohashHex, 'hex')
            LIMIT 1;
        """;

        try
        {
            await using var conn = await _dataSource.OpenConnectionAsync(ct);
            return await conn.QueryFirstOrDefaultAsync<TorrentDetailModel>(sql, new { InfohashHex = infohashHex.ToLowerInvariant() });
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Failed to get torrent details for infohash {Infohash}", infohashHex);
            return null;
        }
    }

    public async Task<DashboardStatsModel> GetDashboardStatsAsync(CancellationToken ct = default)
    {
        const string sql = """
            SELECT
                (SELECT count(*) FROM torrents) AS total_torrents,
                (SELECT count(*) FROM torrents WHERE verified_at >= NOW() - INTERVAL '24 hours') AS verified_last_24h,
                (SELECT count(*) FROM torrents WHERE seed_confirmed = true) AS seed_confirmed_count,
                (SELECT count(*) FROM torrents WHERE health_score >= 70) AS healthy_count,
                (SELECT count(*) FROM torrents WHERE category IS NOT NULL) AS categorized_count;
        """;

        const string catSql = """
            SELECT COALESCE(category, 'Uncategorized') AS category, count(*) AS count
            FROM torrents
            GROUP BY category
            ORDER BY count DESC;
        """;

        try
        {
            await using var conn = await _dataSource.OpenConnectionAsync(ct);
            var stats = await conn.QueryFirstAsync<DashboardStatsModel>(sql);
            var categories = await conn.QueryAsync<CategoryCountModel>(catSql);
            stats.CategoryBreakdown = categories.ToList();
            return stats;
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Failed to query dashboard stats");
            return new DashboardStatsModel();
        }
    }
}

public class TorrentDetailModel
{
    public string Infohash { get; set; } = "";
    public string Name { get; set; } = "";
    public long? PieceLength { get; set; }
    public long TotalSize { get; set; }
    public int FileCount { get; set; }
    public string? FilesJson { get; set; }
    public DateTime VerifiedAt { get; set; }
    public DateTime? FirstSeen { get; set; }
    public DateTime? LastSeen { get; set; }
    public long TotalSeen { get; set; }
    public short HealthScore { get; set; }
    public short PopularityScore { get; set; }
    public int SwarmPeers { get; set; }
    public bool SeedConfirmed { get; set; }
    public string? Category { get; set; }
    public string? RiskTier { get; set; }
    public string? PolicyAction { get; set; }
}

public class DashboardStatsModel
{
    public long TotalTorrents { get; set; }
    public long VerifiedLast24h { get; set; }
    public long SeedConfirmedCount { get; set; }
    public long HealthyCount { get; set; }
    public long CategorizedCount { get; set; }
    public List<CategoryCountModel> CategoryBreakdown { get; set; } = new();
}

public class CategoryCountModel
{
    public string Category { get; set; } = "";
    public long Count { get; set; }
}
