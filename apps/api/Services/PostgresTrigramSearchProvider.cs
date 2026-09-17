using Dapper;

namespace Gaia.Api.Services;

public class PostgresTrigramSearchProvider : ISearchProvider
{
    private readonly DatabaseService _db;
    private readonly ILogger<PostgresTrigramSearchProvider> _logger;

    public PostgresTrigramSearchProvider(DatabaseService db, ILogger<PostgresTrigramSearchProvider> logger)
    {
        _db = db;
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

        var whereClauses = new List<string> { "policy_action IS DISTINCT FROM 'SUPPRESS'" };
        var dynParams    = new DynamicParameters();

        if (!string.IsNullOrWhiteSpace(category) && !category.Equals("All", StringComparison.OrdinalIgnoreCase))
        {
            whereClauses.Add("category = @category");
            dynParams.Add("category", category.Trim());
        }

        var trimmedQuery = query?.Trim() ?? "";
        if (!string.IsNullOrWhiteSpace(trimmedQuery))
        {
            // Use GIN index idx_torrents_name_fts
            whereClauses.Add("to_tsvector('simple', name) @@ websearch_to_tsquery('simple', @q)");
            dynParams.Add("q", trimmedQuery);
        }

        dynParams.Add("lim", safeLimit);
        dynParams.Add("off", offset);

        var orderBy = sortBy?.ToLowerInvariant() switch
        {
            "size" or "total_size"             => $"ORDER BY total_size {sortDir}",
            "popularity" or "popularity_score" => $"ORDER BY popularity_score {sortDir}",
            "health" or "health_score"         => $"ORDER BY health_score {sortDir}",
            "date" or "verified_at"            => $"ORDER BY verified_at {sortDir}",
            _ => string.IsNullOrWhiteSpace(trimmedQuery)
                ? $"ORDER BY verified_at {sortDir}"
                : $"ORDER BY popularity_score DESC, verified_at DESC"
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
        var commandDefinition = new CommandDefinition(sql, dynParams, cancellationToken: ct, commandTimeout: 5);
        var rows = (await conn.QueryAsync(commandDefinition)).Select(MapRow).ToList();

        long total = rows.Count < safeLimit ? offset + rows.Count : 50000;
        sw.Stop();

        // G4: Never cache fallback results in hot cache!
        return new SearchResponse(rows, total, safePage, safeLimit, sw.ElapsedMilliseconds, false, "postgresql-fallback");
    }

    private static SearchResultItem MapRow(dynamic r) => new(
        r.infohash, r.name, r.category ?? "Other", (long)(r.total_size ?? 0), (int)(r.file_count ?? 0),
        r.verified_at as DateTime?, (int?)r.health_score, (int)(r.popularity_score ?? 0),
        (int)(r.swarm_peers ?? 0), (bool)(r.seed_confirmed ?? false), r.risk_tier ?? "SAFE", r.policy_action ?? "ALLOW"
    );
}
