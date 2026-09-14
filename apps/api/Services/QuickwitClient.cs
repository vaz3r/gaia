using System.Text.Json;
using System.Text.Json.Serialization;

namespace Gaia.Api.Services;

public class QuickwitClient
{
    private readonly HttpClient _httpClient;
    private readonly ILogger<QuickwitClient> _logger;

    public QuickwitClient(HttpClient httpClient, ILogger<QuickwitClient> logger)
    {
        _httpClient = httpClient;
        _logger = logger;
    }

    public async Task<QuickwitSearchResponse> SearchAsync(
        string? query,
        string? category = null,
        int page = 1,
        int limit = 25,
        string? sortBy = null,
        string? order = "desc",
        CancellationToken ct = default)
    {
        var safePage = Math.Max(1, page);
        var safeLimit = Math.Clamp(limit, 1, 100);
        var offset = (safePage - 1) * safeLimit;

        var queryParts = new List<string>();

        if (string.IsNullOrWhiteSpace(query))
        {
            queryParts.Add("*");
        }
        else
        {
            queryParts.Add(query.Trim());
        }

        if (!string.IsNullOrWhiteSpace(category) && !category.Equals("All", StringComparison.OrdinalIgnoreCase))
        {
            queryParts.Add($"category:\"{category.Trim()}\"");
        }

        // Exclude suppressed items by default
        queryParts.Add("NOT policy_action:SUPPRESS");

        var finalQuery = string.Join(" AND ", queryParts);

        var url = $"/api/v1/torrents/search?query={Uri.EscapeDataString(finalQuery)}&max_hits={safeLimit}&start_offset={offset}";

        if (!string.IsNullOrWhiteSpace(sortBy))
        {
            var sortField = sortBy.ToLowerInvariant() switch
            {
                "size" => "total_size",
                "health" => "health_score",
                "popularity" => "popularity_score",
                "date" => "verified_at",
                _ => "verified_at"
            };
            var sortDir = order?.ToLowerInvariant() == "asc" ? "asc" : "desc";
            url += $"&sort_by={sortField}:{sortDir}";
        }

        try
        {
            var response = await _httpClient.GetAsync(url, ct);
            response.EnsureSuccessStatusCode();

            var json = await response.Content.ReadAsStringAsync(ct);
            var result = JsonSerializer.Deserialize<QuickwitRawSearchResponse>(json, new JsonSerializerOptions
            {
                PropertyNameCaseInsensitive = true
            });

            return new QuickwitSearchResponse
            {
                Total = result?.NumHits ?? 0,
                Page = safePage,
                Limit = safeLimit,
                ElapsedMicros = result?.ElapsedTimeMicros ?? 0,
                Hits = result?.Hits ?? new List<QuickwitTorrentHit>()
            };
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Failed to execute Quickwit search query: {Query}", finalQuery);
            return new QuickwitSearchResponse
            {
                Total = 0,
                Page = safePage,
                Limit = safeLimit,
                ElapsedMicros = 0,
                Hits = new List<QuickwitTorrentHit>()
            };
        }
    }
}

public class QuickwitRawSearchResponse
{
    [JsonPropertyName("num_hits")]
    public long NumHits { get; set; }

    [JsonPropertyName("hits")]
    public List<QuickwitTorrentHit> Hits { get; set; } = new();

    [JsonPropertyName("elapsed_time_micros")]
    public long ElapsedTimeMicros { get; set; }
}

public class QuickwitSearchResponse
{
    public long Total { get; set; }
    public int Page { get; set; }
    public int Limit { get; set; }
    public long ElapsedMicros { get; set; }
    public List<QuickwitTorrentHit> Hits { get; set; } = new();
}

public class QuickwitTorrentHit
{
    public string Infohash { get; set; } = "";
    public string Name { get; set; } = "";
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
