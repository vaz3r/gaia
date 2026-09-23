using System.Text.Json.Serialization;

namespace Gaia.Api.Services;

public record SearchResultItem(
    [property: JsonPropertyName("infohash")] string Infohash,
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("category")] string Category,
    [property: JsonPropertyName("total_size")] long TotalSize,
    [property: JsonPropertyName("file_count")] int FileCount,
    [property: JsonPropertyName("verified_at")] DateTime? VerifiedAt,
    [property: JsonPropertyName("health_score")] int? HealthScore,
    [property: JsonPropertyName("popularity_score")] int PopularityScore,
    [property: JsonPropertyName("swarm_peers")] int SwarmPeers,
    [property: JsonPropertyName("seed_confirmed")] bool SeedConfirmed,
    [property: JsonPropertyName("risk_tier")] string RiskTier,
    [property: JsonPropertyName("policy_action")] string PolicyAction,
    [property: JsonPropertyName("health_state")] string? HealthState = null,
    [property: JsonPropertyName("integrity_score")] int? IntegrityScore = null
);

public record SearchResponse(
    List<SearchResultItem> Items,
    long Total,
    int Page,
    int Limit,
    long ElapsedMs,
    bool FromCache,
    string Provider,
    string CacheTier = "none"
);

public interface ISearchProvider
{
    Task<SearchResponse> SearchAsync(
        string query,
        string? category = null,
        int page = 1,
        int limit = 25,
        string? sortBy = null,
        string? order = "desc",
        CancellationToken ct = default);
}
