using System.Net.Http.Json;

namespace Gaia.Api.Services;

public class MeilisearchSearchProvider : ISearchProvider
{
    private readonly HttpClient _http;
    private readonly CacheService _redis;
    private readonly PostgresTrigramSearchProvider _fallback;
    private readonly ILogger<MeilisearchSearchProvider> _logger;

    public MeilisearchSearchProvider(
        HttpClient http,
        CacheService redis,
        PostgresTrigramSearchProvider fallback,
        ILogger<MeilisearchSearchProvider> logger)
    {
        _http = http;
        _redis = redis;
        _fallback = fallback;
        _logger = logger;
    }

    public async Task<SearchResponse> SearchAsync(
        string query, string? category = null, int page = 1, int limit = 25,
        string? sortBy = null, string? order = "desc", CancellationToken ct = default)
    {
        var safePage  = Math.Max(1, page);
        var safeLimit = Math.Clamp(limit, 1, 100);
        var offset    = (safePage - 1) * safeLimit;

        // 1. Hot-query cache
        var cacheKey = CacheService.MakeKey("ms-search", query, category, safePage, safeLimit, sortBy, order);
        var cached   = await _redis.GetAsync<SearchResponse>(cacheKey, ct);
        if (cached is not null) return cached with { FromCache = true };

        // 2. Bug B Fix: Filter strictly against SUPPRESS, allowing ALLOW and empty
        var filters = new List<string> { "policy_action != \"SUPPRESS\"" };
        if (!string.IsNullOrWhiteSpace(category) && !category.Equals("All", StringComparison.OrdinalIgnoreCase))
        {
            var cleanCat = category.Trim().Replace("\"", "");
            filters.Add($"category = \"{cleanCat}\"");
        }

        var dir = order?.ToLowerInvariant() == "asc" ? "asc" : "desc";
        string[]? sort = sortBy?.ToLowerInvariant() switch
        {
            "size" or "total_size"             => new[] { $"total_size:{dir}" },
            "health" or "health_score"         => new[] { $"health_score:{dir}" },
            "popularity" or "popularity_score" => new[] { $"popularity_score:{dir}" },
            "date" or "verified_at"            => new[] { $"verified_at:{dir}" },
            _                                  => string.IsNullOrWhiteSpace(query)
                                                    ? new[] { $"verified_at:{dir}" }
                                                    : new[] { "popularity_score:desc" }
        };

        var requestBody = new
        {
            q      = query,
            limit  = safeLimit,
            offset,
            filter = string.Join(" AND ", filters),
            sort,
            attributesToRetrieve = new[]
            {
                "infohash", "name", "category", "total_size", "file_count",
                "verified_at", "health_score", "popularity_score",
                "swarm_peers", "seed_confirmed", "risk_tier", "policy_action"
            }
        };

        try
        {
            using var timeoutCts = CancellationTokenSource.CreateLinkedTokenSource(ct);
            timeoutCts.CancelAfter(TimeSpan.FromMilliseconds(4000));

            var resp = await _http.PostAsJsonAsync("/indexes/torrents/search", requestBody, timeoutCts.Token);
            if (!resp.IsSuccessStatusCode)
            {
                _logger.LogWarning("Meilisearch HTTP {Code}, falling back to PostgreSQL", resp.StatusCode);
                return await _fallback.SearchAsync(query, category, safePage, safeLimit, sortBy, order, ct);
            }

            var raw = await resp.Content.ReadFromJsonAsync<MeiliRawResponse>(cancellationToken: timeoutCts.Token);
            var hits = (raw?.Hits ?? new()).Select(MapHit).ToList();

            var response = new SearchResponse(
                hits,
                raw?.EstimatedTotalHits ?? hits.Count,
                safePage,
                safeLimit,
                raw?.ProcessingTimeMs ?? 0,
                false,
                "meilisearch"
            );

            await _redis.SetAsync(cacheKey, response, CacheService.SearchTtl, ct);
            return response;
        }
        catch (Exception ex)
        {
            _logger.LogWarning(ex, "Meilisearch query failed or timed out. Falling back to PostgreSQL.");
            return await _fallback.SearchAsync(query, category, safePage, safeLimit, sortBy, order, ct);
        }
    }

    // Bug A Fix: Treat verified_at as numeric epoch seconds
    private static SearchResultItem MapHit(MeiliTorrentHit h)
    {
        DateTime? verifiedDate = h.VerifiedAt > 0 
            ? DateTimeOffset.FromUnixTimeSeconds(h.VerifiedAt).UtcDateTime 
            : null;

        return new SearchResultItem(
            h.Infohash, h.Name, h.Category, h.TotalSize, h.FileCount,
            verifiedDate, h.HealthScore, h.PopularityScore, h.SwarmPeers,
            h.SeedConfirmed, h.RiskTier, h.PolicyAction
        );
    }
}

public class MeiliRawResponse
{
    public List<MeiliTorrentHit> Hits { get; set; } = new();
    public int EstimatedTotalHits { get; set; }
    public long ProcessingTimeMs { get; set; }
}

public class MeiliTorrentHit
{
    public string Infohash { get; set; } = "";
    public string Name { get; set; } = "";
    public string Category { get; set; } = "Other";
    public long TotalSize { get; set; }
    public int FileCount { get; set; }
    public long VerifiedAt { get; set; } // Numeric epoch
    public int HealthScore { get; set; }
    public int PopularityScore { get; set; }
    public int SwarmPeers { get; set; }
    public bool SeedConfirmed { get; set; }
    public string RiskTier { get; set; } = "SAFE";
    public string PolicyAction { get; set; } = "ALLOW";
}
