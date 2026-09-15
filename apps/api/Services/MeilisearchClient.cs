using System.Net.Http.Json;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Gaia.Api.Services;

/// <summary>
/// HTTP client for Meilisearch v1.x search API.
/// All search results are cached in Redis (60s TTL).
/// Search default sort: popularity_score DESC (most popular first).
/// </summary>
public class MeilisearchClient
{
    private readonly HttpClient _httpClient;
    private readonly CacheService _cache;
    private readonly ILogger<MeilisearchClient> _logger;

    public MeilisearchClient(HttpClient httpClient, CacheService cache, ILogger<MeilisearchClient> logger)
    {
        _httpClient = httpClient;
        _cache = cache;
        _logger = logger;
    }

    public async Task<MeiliSearchResponse> SearchAsync(
        string query,
        string? category = null,
        int page = 1,
        int limit = 25,
        string? sortBy = null,
        string? order = "desc",
        CancellationToken ct = default)
    {
        var safePage  = Math.Max(1, page);
        var safeLimit = Math.Clamp(limit, 1, 100);
        var offset    = (safePage - 1) * safeLimit;

        // ── Redis cache check ─────────────────────────────────────────────────
        var cacheKey = CacheService.MakeKey("ms", query, category, safePage, safeLimit, sortBy, order);
        var cached   = await _cache.GetAsync<MeiliSearchResponse>(cacheKey, ct);
        if (cached is not null)
        {
            cached.FromCache = true;
            return cached;
        }

        // ── Build filter ──────────────────────────────────────────────────────
        var filters = new List<string> { "policy_action != SUPPRESS" };
        if (!string.IsNullOrWhiteSpace(category) && !category.Equals("All", StringComparison.OrdinalIgnoreCase))
            filters.Add($"category = \"{category.Trim()}\"");

        // ── Build sort ────────────────────────────────────────────────────────
        // Default for search: popularity_score DESC (most popular first).
        // Users can override by passing sort= param.
        string[]? sort = null;
        if (!string.IsNullOrWhiteSpace(sortBy) && !sortBy.Equals("relevance", StringComparison.OrdinalIgnoreCase))
        {
            var dir = order?.ToLowerInvariant() == "asc" ? "asc" : "desc";
            sort = sortBy.ToLowerInvariant() switch
            {
                "size"        => new[] { $"total_size:{dir}" },
                "health"      => new[] { $"health_score:{dir}" },
                "popularity"  => new[] { $"popularity_score:{dir}" },
                "date" or "verified_at" => new[] { $"verified_at:{dir}" },
                _ => new[] { "popularity_score:desc" }
            };
        }
        else
        {
            // No explicit sort → default to popularity (most seeded/popular torrents first)
            sort = new[] { "popularity_score:desc" };
        }

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

        // ── Execute Meilisearch request ───────────────────────────────────────
        try
        {
            var resp = await _httpClient.PostAsJsonAsync("/indexes/torrents/search", requestBody, ct);
            resp.EnsureSuccessStatusCode();

            var raw = await resp.Content.ReadFromJsonAsync<MeiliRawResponse>(cancellationToken: ct);

            var result = new MeiliSearchResponse
            {
                Total     = raw?.EstimatedTotalHits ?? 0,
                Page      = safePage,
                Limit     = safeLimit,
                ElapsedMs = raw?.ProcessingTimeMs ?? 0,
                Hits      = raw?.Hits ?? new List<MeiliTorrentHit>(),
                FromCache = false
            };

            await _cache.SetAsync(cacheKey, result, CacheService.SearchTtl, ct);
            return result;
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Meilisearch search failed for query: {Query}", query);
            return new MeiliSearchResponse
            {
                Total = 0, Page = safePage, Limit = safeLimit, Hits = new()
            };
        }
    }
}

// ── DTOs ──────────────────────────────────────────────────────────────────────

public class MeiliRawResponse
{
    [JsonPropertyName("hits")]
    public List<MeiliTorrentHit> Hits { get; set; } = new();

    [JsonPropertyName("estimatedTotalHits")]
    public long EstimatedTotalHits { get; set; }

    [JsonPropertyName("processingTimeMs")]
    public long ProcessingTimeMs { get; set; }
}

public class MeiliSearchResponse
{
    public long Total     { get; set; }
    public int  Page      { get; set; }
    public int  Limit     { get; set; }
    public long ElapsedMs { get; set; }
    public bool FromCache { get; set; }
    public List<MeiliTorrentHit> Hits { get; set; } = new();
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
    public string? VerifiedAt { get; set; }

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
