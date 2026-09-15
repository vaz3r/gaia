using System.Net.Http.Json;
using System.Text.Json.Serialization;
using Microsoft.Extensions.Caching.Memory;

namespace Gaia.Api.Services;

public class MeilisearchSearchProvider : ISearchProvider
{
    private readonly HttpClient _http;
    private readonly CacheService _redis;
    private readonly IMemoryCache _memCache;
    private readonly PostgresTrigramSearchProvider _fallback;
    private readonly CircuitBreaker _circuitBreaker;
    private readonly ILogger<MeilisearchSearchProvider> _logger;

    public MeilisearchSearchProvider(
        HttpClient http,
        CacheService redis,
        IMemoryCache memCache,
        PostgresTrigramSearchProvider fallback,
        CircuitBreaker circuitBreaker,
        ILogger<MeilisearchSearchProvider> logger)
    {
        _http = http;
        _redis = redis;
        _memCache = memCache;
        _fallback = fallback;
        _circuitBreaker = circuitBreaker;
        _logger = logger;
    }

    public async Task<SearchResponse> SearchAsync(
        string query, string? category = null, int page = 1, int limit = 25,
        string? sortBy = null, string? order = "desc", CancellationToken ct = default)
    {
        var safePage  = Math.Max(1, page);
        var safeLimit = Math.Clamp(limit, 1, 100);
        var offset    = (safePage - 1) * safeLimit;

        // 1. Smart Query Preprocessing
        var parsed = TorrentQueryParser.Parse(query);

        // 2. Multi-tier cache check (L1 MemoryCache -> L2 Redis)
        var cacheKey = CacheService.MakeKey("ms-search", parsed.CleanQuery, category, safePage, safeLimit, sortBy, order);

        // L1: In-Memory (0.2ms)
        if (_memCache.TryGetValue<SearchResponse>(cacheKey, out var memCached) && memCached is not null)
        {
            return memCached with { FromCache = true, CacheTier = "memory" };
        }

        // L2: Redis (1-3ms)
        var redisCached = await _redis.GetAsync<SearchResponse>(cacheKey, ct);
        if (redisCached is not null)
        {
            _memCache.Set(cacheKey, redisCached, TimeSpan.FromSeconds(15));
            return redisCached with { FromCache = true, CacheTier = "redis" };
        }

        // 3. Direct O(1) Infohash Lookup
        if (parsed.Type == QueryType.Infohash && !string.IsNullOrWhiteSpace(parsed.Infohash))
        {
            return await FetchByInfohashDirectAsync(parsed.Infohash, cacheKey, ct);
        }

        // 4. Circuit Breaker Check
        if (!_circuitBreaker.CanExecute())
        {
            _logger.LogWarning("Circuit breaker is OPEN. Fast-failing to PostgreSQL fallback (0ms delay).");
            var fallbackResp = await _fallback.SearchAsync(parsed.OriginalQuery, category, safePage, safeLimit, sortBy, order, ct);
            // CRITICAL: NEVER cache degraded fallback responses in hot cache!
            return fallbackResp;
        }

        // 5. Build Meilisearch Query
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
            _                                  => string.IsNullOrWhiteSpace(parsed.CleanQuery)
                                                    ? new[] { $"verified_at:{dir}" }
                                                    : null // Let ranking rules govern text relevance!
        };

        var requestBody = new
        {
            q                = parsed.CleanQuery,
            limit            = safeLimit,
            offset,
            filter           = string.Join(" AND ", filters),
            sort,
            matchingStrategy = "all",
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
            timeoutCts.CancelAfter(TimeSpan.FromMilliseconds(800)); // 800ms SLA

            var resp = await _http.PostAsJsonAsync("/indexes/torrents/search", requestBody, timeoutCts.Token);
            if (!resp.IsSuccessStatusCode)
            {
                _circuitBreaker.RecordFailure();
                _logger.LogWarning("Meilisearch HTTP {Code}, tripping failure and calling fallback", resp.StatusCode);
                return await _fallback.SearchAsync(parsed.OriginalQuery, category, safePage, safeLimit, sortBy, order, ct);
            }

            var raw = await resp.Content.ReadFromJsonAsync<MeiliRawResponse>(cancellationToken: timeoutCts.Token);
            var hits = (raw?.Hits ?? new()).Select(MapHit).ToList();

            // 6. Relaxed Retry: If matchingStrategy: 'all' returned 0 hits for a multi-word query (>= 2 words), retry with 'last'
            if (hits.Count == 0 && !string.IsNullOrWhiteSpace(parsed.CleanQuery) && parsed.CleanQuery.Split(' ').Length >= 2)
            {
                var relaxedBody = new
                {
                    q                = parsed.CleanQuery,
                    limit            = safeLimit,
                    offset,
                    filter           = string.Join(" AND ", filters),
                    sort,
                    matchingStrategy = "last",
                    attributesToRetrieve = requestBody.attributesToRetrieve
                };

                var relaxedResp = await _http.PostAsJsonAsync("/indexes/torrents/search", relaxedBody, timeoutCts.Token);
                if (relaxedResp.IsSuccessStatusCode)
                {
                    var relaxedRaw = await relaxedResp.Content.ReadFromJsonAsync<MeiliRawResponse>(cancellationToken: timeoutCts.Token);
                    hits = (relaxedRaw?.Hits ?? new()).Select(MapHit).ToList();
                    raw = relaxedRaw;
                }
            }

            _circuitBreaker.RecordSuccess();

            var response = new SearchResponse(
                hits,
                raw?.EstimatedTotalHits ?? hits.Count,
                safePage,
                safeLimit,
                raw?.ProcessingTimeMs ?? 0,
                false,
                "meilisearch",
                "none"
            );

            // 7. ONLY cache high-quality primary Meilisearch responses
            _memCache.Set(cacheKey, response, TimeSpan.FromSeconds(15));
            await _redis.SetAsync(cacheKey, response, CacheService.SearchTtl, ct);

            return response;
        }
        catch (Exception ex)
        {
            _circuitBreaker.RecordFailure();
            _logger.LogWarning(ex, "Meilisearch query failed or timed out. Falling back to PostgreSQL.");
            return await _fallback.SearchAsync(parsed.OriginalQuery, category, safePage, safeLimit, sortBy, order, ct);
        }
    }

    private async Task<SearchResponse> FetchByInfohashDirectAsync(string infohash, string cacheKey, CancellationToken ct)
    {
        try
        {
            using var cts = CancellationTokenSource.CreateLinkedTokenSource(ct);
            cts.CancelAfter(TimeSpan.FromMilliseconds(500));

            var resp = await _http.GetAsync($"/indexes/torrents/documents/{infohash}", cts.Token);
            if (resp.IsSuccessStatusCode)
            {
                var hit = await resp.Content.ReadFromJsonAsync<MeiliTorrentHit>(cancellationToken: cts.Token);
                if (hit is not null)
                {
                    var item = MapHit(hit);
                    var result = new SearchResponse(new List<SearchResultItem> { item }, 1, 1, 1, 1, false, "meilisearch", "none");
                    _memCache.Set(cacheKey, result, TimeSpan.FromSeconds(60));
                    await _redis.SetAsync(cacheKey, result, CacheService.SearchTtl, ct);
                    return result;
                }
            }
        }
        catch (Exception ex)
        {
            _logger.LogWarning(ex, "Direct infohash lookup failed in Meilisearch, falling back to database");
        }

        return await _fallback.SearchAsync(infohash, null, 1, 1, null, "desc", ct);
    }

    private static SearchResultItem MapHit(MeiliTorrentHit h)
    {
        DateTime? verifiedDate = (h.VerifiedAt.HasValue && h.VerifiedAt.Value > 0)
            ? DateTimeOffset.FromUnixTimeSeconds(h.VerifiedAt.Value).UtcDateTime 
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
    [JsonPropertyName("hits")]
    public List<MeiliTorrentHit> Hits { get; set; } = new();

    [JsonPropertyName("estimatedTotalHits")]
    public int EstimatedTotalHits { get; set; }

    [JsonPropertyName("processingTimeMs")]
    public long ProcessingTimeMs { get; set; }
}

public class MeiliTorrentHit
{
    [JsonPropertyName("infohash")]
    public string Infohash { get; set; } = "";

    [JsonPropertyName("name")]
    public string Name { get; set; } = "";

    [JsonPropertyName("category")]
    public string Category { get; set; } = "Other";

    [JsonPropertyName("total_size")]
    public long TotalSize { get; set; }

    [JsonPropertyName("file_count")]
    public int FileCount { get; set; }

    [JsonPropertyName("verified_at")]
    public long? VerifiedAt { get; set; }

    [JsonPropertyName("health_score")]
    public int HealthScore { get; set; }

    [JsonPropertyName("popularity_score")]
    public int PopularityScore { get; set; }

    [JsonPropertyName("swarm_peers")]
    public int SwarmPeers { get; set; }

    [JsonPropertyName("seed_confirmed")]
    public bool SeedConfirmed { get; set; }

    [JsonPropertyName("risk_tier")]
    public string RiskTier { get; set; } = "SAFE";

    [JsonPropertyName("policy_action")]
    public string PolicyAction { get; set; } = "ALLOW";
}
