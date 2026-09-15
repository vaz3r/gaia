using Dapper;

namespace Gaia.Api.Services;

public class PostgresTrigramSearchProvider : ISearchProvider
{
    private readonly DatabaseService _db;
    private readonly CacheService _redis;
    private readonly ILogger<PostgresTrigramSearchProvider> _logger;

    private static readonly HashSet<string> QualityStopTokens = new(StringComparer.OrdinalIgnoreCase)
    {
        "1080p", "720p", "480p", "2160p", "4k", "uhd", "bluray", "blu-ray", "remux",
        "webrip", "web-dl", "webdl", "dvdrip", "hdtv", "x264", "x265", "hevc", "avc",
        "aac", "dts", "ac3", "ddp5", "repack", "proper", "multi", "ita", "eng", "rus"
    };

    public PostgresTrigramSearchProvider(DatabaseService db, CacheService redis, ILogger<PostgresTrigramSearchProvider> logger)
    {
        _db = db;
        _redis = redis;
        _logger = logger;
    }

    public async Task<SearchResponse> SearchAsync(
        string query, string? category = null, int page = 1, int limit = 25,
        string? sortBy = null, string? order = "desc", CancellationToken ct = default)
    {
        var sw = System.Diagnostics.Stopwatch.StartNew();
        var safeLimit = Math.Clamp(limit, 1, 100);
        var safePage  = Math.Max(1, page);
        var offset    = (safePage - 1) * safeLimit;
        var sortDir   = order?.ToLowerInvariant() == "asc" ? "ASC" : "DESC";

        var cacheKey = CacheService.MakeKey("pg-search", query, category, safePage, safeLimit, sortBy, order);
        var cached   = await _redis.GetAsync<SearchResponse>(cacheKey, ct);
        if (cached is not null) return cached with { FromCache = true };

        var allTokens = System.Text.RegularExpressions.Regex.Split(query.Trim(), @"[\s._\-+]+")
            .Where(t => t.Length > 1 || char.IsDigit(t[0]))
            .ToList();

        var titleTokens = allTokens.Where(t => !QualityStopTokens.Contains(t)).ToList();
        var filterTokens = titleTokens.Count > 0 ? titleTokens : allTokens;

        var whereClauses = new List<string> { "(policy_action IS NULL OR policy_action != 'SUPPRESS')" };
        var dynParams    = new DynamicParameters();

        if (!string.IsNullOrWhiteSpace(category) && !category.Equals("All", StringComparison.OrdinalIgnoreCase))
        {
            whereClauses.Add("category = @category");
            dynParams.Add("category", category.Trim());
        }

        for (int i = 0; i < filterTokens.Count; i++)
        {
            whereClauses.Add($"name ILIKE @tok{i} ESCAPE '\\'");
            dynParams.Add($"tok{i}", $"%{EscapeLike(filterTokens[i])}%");
        }

        var extraTokens = allTokens.Where(t => QualityStopTokens.Contains(t)).ToList();
        for (int i = 0; i < extraTokens.Count; i++)
        {
            whereClauses.Add($"name ILIKE @qtok{i} ESCAPE '\\'");
            dynParams.Add($"qtok{i}", $"%{EscapeLike(extraTokens[i])}%");
        }

        dynParams.Add("fullPhrase", $"%{EscapeLike(query.Trim())}%");
        dynParams.Add("lim", safeLimit);
        dynParams.Add("off", offset);

        var orderBy = sortBy?.ToLowerInvariant() switch
        {
            "size"       => $"ORDER BY total_size {sortDir}",
            "popularity" => $"ORDER BY popularity_score {sortDir}",
            "health"     => $"ORDER BY health_score {sortDir}",
            "date"       => $"ORDER BY verified_at {sortDir}",
            _ => $"ORDER BY CASE WHEN name ILIKE @fullPhrase ESCAPE '\\' THEN 200 ELSE 100 END DESC, popularity_score DESC"
        };

        var sql = $"""
            SELECT encode(infohash, 'hex') AS infohash, name, category, total_size, file_count,
                   verified_at, health_score, popularity_score, swarm_peers, seed_confirmed,
                   risk_tier, policy_action
            FROM torrents
            WHERE {string.Join(" AND ", whereClauses)}
            {orderBy}
            LIMIT @lim OFFSET @off;
        """;

        await using var conn = await _db.OpenConnectionAsync(ct);
        var rows = (await conn.QueryAsync(sql, dynParams)).Select(MapRow).ToList();

        long total = rows.Count < safeLimit ? offset + rows.Count : 50000;
        sw.Stop();

        var response = new SearchResponse(rows, total, safePage, safeLimit, sw.ElapsedMilliseconds, false, "postgresql-trigram");
        await _redis.SetAsync(cacheKey, response, CacheService.SearchTtl, ct);
        return response;
    }

    private static SearchResultItem MapRow(dynamic r) => new(
        r.infohash, r.name, r.category ?? "Other", (long)(r.total_size ?? 0), (int)(r.file_count ?? 0),
        r.verified_at as DateTime?, (int)(r.health_score ?? 0), (int)(r.popularity_score ?? 0),
        (int)(r.swarm_peers ?? 0), (bool)(r.seed_confirmed ?? false), r.risk_tier ?? "SAFE", r.policy_action ?? "ALLOW"
    );

    private static string EscapeLike(string s) => s.Replace("\\", "\\\\").Replace("%", "\\%").Replace("_", "\\_");
}
