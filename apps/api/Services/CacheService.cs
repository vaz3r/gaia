using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using StackExchange.Redis;

namespace Gaia.Api.Services;

/// <summary>
/// Redis-backed response cache for search and browse results.
/// Key strategy: SHA1(normalized query params) → compressed JSON string.
/// TTLs: search results 60s, browse 30s, details 5min.
/// </summary>
public class CacheService
{
    private readonly IDatabase? _db;
    private readonly ILogger<CacheService> _logger;
    private readonly bool _enabled;

    // TTL constants
    public static readonly TimeSpan SearchTtl  = TimeSpan.FromSeconds(60);
    public static readonly TimeSpan BrowseTtl  = TimeSpan.FromSeconds(30);
    public static readonly TimeSpan DetailTtl  = TimeSpan.FromMinutes(5);

    public CacheService(IConnectionMultiplexer? redis, ILogger<CacheService> logger)
    {
        _logger = logger;
        if (redis is null || !redis.IsConnected)
        {
            _logger.LogWarning("Redis not available — caching disabled.");
            _enabled = false;
            return;
        }
        _db = redis.GetDatabase();
        _enabled = true;
        _logger.LogInformation("Redis cache enabled.");
    }

    public async Task<T?> GetAsync<T>(string key, CancellationToken ct = default)
    {
        if (!_enabled || _db is null) return default;
        try
        {
            var val = await _db.StringGetAsync(key);
            if (val.IsNullOrEmpty) return default;
            return JsonSerializer.Deserialize<T>(val.ToString());
        }
        catch (Exception ex)
        {
            _logger.LogWarning(ex, "Cache GET failed for key {Key}", key);
            return default;
        }
    }

    public async Task SetAsync<T>(string key, T value, TimeSpan ttl, CancellationToken ct = default)
    {
        if (!_enabled || _db is null) return;
        try
        {
            var json = JsonSerializer.Serialize(value);
            await _db.StringSetAsync(key, json, ttl);
        }
        catch (Exception ex)
        {
            _logger.LogWarning(ex, "Cache SET failed for key {Key}", key);
        }
    }

    /// <summary>Builds a short stable cache key from arbitrary key components.</summary>
    public static string MakeKey(string prefix, params object?[] parts)
    {
        var raw = string.Join("|", parts.Select(p => p?.ToString()?.ToLowerInvariant() ?? ""));
        var hash = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(raw)))[..16];
        return $"gaia:{prefix}:{hash}";
    }
}
