using System.Net.Http.Json;
using System.Text.Json;
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

    private static readonly string[] RetrievalAttributes = new[]
    {
        "infohash", "name", "category", "total_size", "file_count",
        "verified_at", "popularity_tier", "risk_tier", "policy_action"
    };

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

        // 1. Smart Query Preprocessing (glued words + year disjunction)
        var parsed = TorrentQueryParser.Parse(query);

        // 2. Generation key for safe cache invalidation
        var gen = await _redis.GetGenerationAsync(ct);
        var cacheKey = CacheService.MakeKey("ms-search", gen, parsed.CleanQuery, category, safePage, safeLimit, sortBy, order);

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
            return await _fallback.SearchAsync(parsed.CleanQuery, category, safePage, safeLimit, sortBy, order, ct);
        }

        // 5. Build Filter and Sort
        var filters = new List<string> { "policy_action != \"SUPPRESS\"" };
        if (!string.IsNullOrWhiteSpace(category) && !category.Equals("All", StringComparison.OrdinalIgnoreCase))
        {
            var cleanCat = category.Trim().Replace("\"", "");
            filters.Add($"category = \"{cleanCat}\"");
        }
        var filterExpr = string.Join(" AND ", filters);

        var dir = order?.ToLowerInvariant() == "asc" ? "asc" : "desc";
        string[]? sort = sortBy?.ToLowerInvariant() switch
        {
            "size" or "total_size"             => new[] { $"total_size:{dir}" },
            "popularity" or "popularity_score" => new[] { $"verified_at:{dir}" },
            "date" or "verified_at"            => new[] { $"verified_at:{dir}" },
            _                                  => string.IsNullOrWhiteSpace(parsed.CleanQuery)
                                                    ? new[] { $"verified_at:{dir}" }
                                                    : null
        };

        // 6. Execute with SLA budget (2000ms max)
        using var budgetCts = CancellationTokenSource.CreateLinkedTokenSource(ct);
        budgetCts.CancelAfter(TimeSpan.FromMilliseconds(2000));

        try
        {
            MeiliRawResponse? raw = null;

            if (parsed.QueryVariants.Count > 1)
            {
                // Feature 2: Multi-Search Native Federation for year disjunctions (e.g. matrix 1999 OR matrix 99)
                var multiSearchPayload = new
                {
                    federation = new { limit = safeLimit, offset },
                    queries = parsed.QueryVariants.Select(v => new
                    {
                        indexUid = "torrents",
                        q = v,
                        filter = filterExpr,
                        sort,
                        attributesToRetrieve = RetrievalAttributes
                    }).ToArray()
                };

                var multiResp = await _http.PostAsJsonAsync("/multi-search", multiSearchPayload, budgetCts.Token);
                if (!multiResp.IsSuccessStatusCode)
                {
                    if (_circuitBreaker.RecordFailure())
                    {
                        TriggerAutoCacheInvalidation("Multi-search failure tripped circuit OPEN");
                    }
                    return await _fallback.SearchAsync(parsed.CleanQuery, category, safePage, safeLimit, sortBy, order, ct);
                }

                raw = await multiResp.Content.ReadFromJsonAsync<MeiliRawResponse>(cancellationToken: budgetCts.Token);
            }
            else
            {
                // Single query search
                var requestBody = new
                {
                    q                = parsed.CleanQuery,
                    limit            = safeLimit,
                    offset,
                    filter           = filterExpr,
                    sort,
                    matchingStrategy = "all",
                    attributesToRetrieve = RetrievalAttributes
                };

                var resp = await _http.PostAsJsonAsync("/indexes/torrents/search", requestBody, budgetCts.Token);
                if (!resp.IsSuccessStatusCode)
                {
                    if (_circuitBreaker.RecordFailure())
                    {
                        TriggerAutoCacheInvalidation("Search HTTP failure (" + resp.StatusCode + ") tripped circuit OPEN");
                    }
                    return await _fallback.SearchAsync(parsed.CleanQuery, category, safePage, safeLimit, sortBy, order, ct);
                }

                raw = await resp.Content.ReadFromJsonAsync<MeiliRawResponse>(cancellationToken: budgetCts.Token);

                // Relaxed retry if 0 hits on multi-word query
                if ((raw?.Hits == null || raw.Hits.Count == 0) &&
                    !string.IsNullOrWhiteSpace(parsed.CleanQuery) &&
                    parsed.CleanQuery.Split(' ').Length >= 2 &&
                    !parsed.CleanQuery.Contains("\""))
                {
                    var relaxedBody = new
                    {
                        q                = parsed.CleanQuery,
                        limit            = safeLimit,
                        offset,
                        filter           = filterExpr,
                        sort,
                        matchingStrategy = "last",
                        attributesToRetrieve = RetrievalAttributes
                    };

                    var relaxedResp = await _http.PostAsJsonAsync("/indexes/torrents/search", relaxedBody, budgetCts.Token);
                    if (relaxedResp.IsSuccessStatusCode)
                    {
                        raw = await relaxedResp.Content.ReadFromJsonAsync<MeiliRawResponse>(cancellationToken: budgetCts.Token);
                    }
                }
            }

            var rawHits = raw?.Hits ?? new();

            // Double-Lock / Race-Free TTL Suppression Filter: Check Redis DB 0 for any suppressed items
            if (rawHits.Count > 0)
            {
                var db0 = _redis.GetDatabase(0);
                if (db0 != null)
                {
                    var batch = db0.CreateBatch();
                    var suppressionTasks = rawHits.Select(h => batch.KeyExistsAsync($"gaia:suppressed:{h.Infohash.ToLowerInvariant()}")).ToList();
                    batch.Execute();
                    var suppressionResults = await Task.WhenAll(suppressionTasks);

                    for (int i = rawHits.Count - 1; i >= 0; i--)
                    {
                        if (suppressionResults[i])
                        {
                            rawHits.RemoveAt(i);
                        }
                    }
                }
            }

            await HydrateVolatileMetricsAsync(rawHits, budgetCts.Token);
            var hits = rawHits.Select(MapHit).ToList();
            if (_circuitBreaker.RecordSuccess())
            {
                TriggerAutoCacheInvalidation("Circuit breaker recovered to CLOSED");
            }

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

            // ONLY cache primary Meilisearch responses
            _memCache.Set(cacheKey, response, TimeSpan.FromSeconds(15));
            await _redis.SetAsync(cacheKey, response, CacheService.SearchTtl, ct);

            return response;
        }
        catch (Exception ex)
        {
            if (_circuitBreaker.RecordFailure())
            {
                TriggerAutoCacheInvalidation("Circuit breaker opened on exception: " + ex.Message);
            }
            _logger.LogWarning(ex, "Meilisearch execution failed or timed out. Falling back to PostgreSQL.");
            return await _fallback.SearchAsync(parsed.CleanQuery, category, safePage, safeLimit, sortBy, order, ct);
        }
    }

    private void TriggerAutoCacheInvalidation(string reason)
    {
        _ = Task.Run(async () =>
        {
            try
            {
                var db0 = _redis.GetDatabase(0);
                if (db0 != null)
                {
                    var newGen = await db0.StringIncrementAsync("gaia:search:gen");
                    _logger.LogWarning("⚡ AUTOMATIC CACHE INVALIDATION: {Reason}. Incremented gaia:search:gen to {Gen}.", reason, newGen);
                }
            }
            catch (Exception ex)
            {
                _logger.LogError(ex, "Failed to auto-increment gaia:search:gen during circuit state transition.");
            }
        });
    }

    private async Task<SearchResponse> FetchByInfohashDirectAsync(string infohash, string cacheKey, CancellationToken ct)
    {
        try
        {
            var db0 = _redis.GetDatabase(0);
            if (db0 != null && await db0.KeyExistsAsync($"gaia:suppressed:{infohash.ToLowerInvariant()}"))
            {
                return new SearchResponse(new List<SearchResultItem>(), 0, 1, 1, 0, false, "meilisearch", "suppressed");
            }

            using var cts = CancellationTokenSource.CreateLinkedTokenSource(ct);
            cts.CancelAfter(TimeSpan.FromMilliseconds(500));

            var resp = await _http.GetAsync($"/indexes/torrents/documents/{infohash}", cts.Token);
            if (resp.IsSuccessStatusCode)
            {
                var hit = await resp.Content.ReadFromJsonAsync<MeiliTorrentHit>(cancellationToken: cts.Token);
                if (hit is not null)
                {
                    await HydrateVolatileMetricsAsync(new List<MeiliTorrentHit> { hit }, cts.Token);
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

    private async Task HydrateVolatileMetricsAsync(List<MeiliTorrentHit> hits, CancellationToken ct)
    {
        if (hits.Count == 0) return;

        try
        {
            var db1 = _redis.GetDatabase(1);
            if (db1 == null) return;

            var batch = db1.CreateBatch();
            var tasks = new Task<StackExchange.Redis.RedisValue[]>[hits.Count];
            var hashFields = new StackExchange.Redis.RedisValue[] { "h", "p", "s", "c", "t" };

            for (int i = 0; i < hits.Count; i++)
            {
                tasks[i] = batch.HashGetAsync($"gaia:health:{hits[i].Infohash}", hashFields);
            }

            batch.Execute();
            await Task.WhenAll(tasks);

            var nowUnix = DateTimeOffset.UtcNow.ToUnixTimeSeconds();

            for (int i = 0; i < hits.Count; i++)
            {
                var vals = tasks[i].Result;
                if (vals == null || vals.Length < 5 || !vals[0].HasValue)
                {
                    hits[i].HealthScore = null;
                    continue;
                }

                int rawH = vals[0].TryParse(out int parsedH) ? parsedH : 0;
                int p = vals[1].TryParse(out int parsedP) ? parsedP : 0;
                int s = vals[2].TryParse(out int parsedS) ? parsedS : 0;
                bool c = vals[3].TryParse(out int parsedC) && parsedC == 1;
                long? t = (vals[4].HasValue && vals[4].TryParse(out long parsedT)) ? parsedT : null;

                hits[i].PopularityScore = p;
                hits[i].SwarmPeers = s;

                if (!t.HasValue)
                {
                    // Unprobed: never actively health-probed -> Unknown (no seeder claim)
                    hits[i].HealthScore = null;
                    hits[i].SeedConfirmed = false;
                }
                else
                {
                    hits[i].SeedConfirmed = c;
                    var days = (nowUnix - t.Value) / 86400.0;
                    int h = rawH;

                    // 1. Linear age decay (> 7 days decays to 0 by day 30)
                    if (days > 7.0)
                    {
                        if (days >= 30.0)
                        {
                            h = 0;
                        }
                        else
                        {
                            double ratio = 1.0 - ((days - 7.0) / 23.0);
                            h = (int)Math.Max(0, Math.Round(rawH * ratio));
                        }
                    }

                    // 2. Peer penalty composed after age: only floor to 0 if data is old enough (> 7 days) and peers == 0
                    if (days > 7.0 && s == 0)
                    {
                        h = 0;
                    }

                    hits[i].HealthScore = h;
                }
            }
        }
        catch (Exception ex)
        {
            _logger.LogWarning(ex, "Failed to hydrate volatile metrics from Redis DB 1; rendering default/cached metrics.");
        }
    }

    private static SearchResultItem MapHit(MeiliTorrentHit h)
    {
        DateTime? verifiedDate = null;
        if (h.VerifiedAt.ValueKind == JsonValueKind.Number && h.VerifiedAt.TryGetInt64(out var unixSec))
        {
            verifiedDate = DateTimeOffset.FromUnixTimeSeconds(unixSec).UtcDateTime;
        }
        else if (h.VerifiedAt.ValueKind == JsonValueKind.String && DateTime.TryParse(h.VerifiedAt.GetString(), out var dt))
        {
            verifiedDate = dt.ToUniversalTime();
        }

        return new SearchResultItem(
            h.Infohash,
            h.Name,
            h.Category,
            h.TotalSize,
            h.FileCount,
            verifiedDate,
            h.HealthScore,
            h.PopularityScore,
            h.SwarmPeers,
            h.SeedConfirmed,
            h.RiskTier,
            h.PolicyAction
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
    public JsonElement VerifiedAt { get; set; }

    [JsonPropertyName("health_score")]
    public int? HealthScore { get; set; }

    [JsonPropertyName("popularity_score")]
    public int PopularityScore { get; set; }

    [JsonPropertyName("swarm_peers")]
    public int SwarmPeers { get; set; }

    [JsonPropertyName("seed_confirmed")]
    public bool SeedConfirmed { get; set; }

    [JsonPropertyName("popularity_tier")]
    public int PopularityTier { get; set; }

    [JsonPropertyName("risk_tier")]
    public string RiskTier { get; set; } = "SAFE";

    [JsonPropertyName("policy_action")]
    public string PolicyAction { get; set; } = "ALLOW";
}
