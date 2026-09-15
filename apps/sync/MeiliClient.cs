using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using Microsoft.Extensions.Logging;

namespace Gaia.Sync;

public class MeiliClient
{
    private readonly HttpClient _httpClient;
    private readonly ILogger<MeiliClient> _logger;
    private static readonly JsonSerializerOptions JsonOpts = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull
    };

    public MeiliClient(HttpClient httpClient, string meiliUrl, string apiKey, ILogger<MeiliClient> logger)
    {
        _httpClient = httpClient;
        _httpClient.BaseAddress = new Uri(meiliUrl);
        _httpClient.Timeout = TimeSpan.FromSeconds(60);
        if (!string.IsNullOrEmpty(apiKey))
        {
            _httpClient.DefaultRequestHeaders.Authorization = new AuthenticationHeaderValue("Bearer", apiKey);
        }
        _logger = logger;
    }

    public async Task EnsureIndexAndSettingsAsync()
    {
        _logger.LogInformation("Ensuring Meilisearch index 'torrents' exists and configuring canonical settings...");

        // Create index if not exists
        var createPayload = new { uid = "torrents", primaryKey = "infohash" };
        using var createContent = new StringContent(JsonSerializer.Serialize(createPayload), Encoding.UTF8, "application/json");
        var createResp = await _httpClient.PostAsync("/indexes", createContent);
        if (createResp.IsSuccessStatusCode)
        {
            var task = await createResp.Content.ReadFromJsonAsync<MeiliTaskResponse>();
            if (task != null) await WaitForTaskAsync(task.TaskUid);
        }

        // Apply settings
        var settings = new
        {
            searchableAttributes = new[] { "name_clean", "name" },
            filterableAttributes = new[] { "category", "risk_tier", "policy_action", "availability_state", "seed_confirmed", "verified_at" },
            sortableAttributes = new[] { "popularity_score", "health_score", "verified_at", "total_size", "swarm_peers" },
            rankingRules = new[] { "words", "typo", "proximity", "attribute", "sort", "exactness" },
            distinctAttribute = (string?)null,
            typoTolerance = new
            {
                enabled = true,
                minWordSizeForTypos = new { oneTypo = 5, twoTypos = 9 }
            },
            faceting = new { maxValuesPerFacet = 100 },
            pagination = new { maxTotalHits = 10000 }
        };

        using var settingsContent = new StringContent(JsonSerializer.Serialize(settings, JsonOpts), Encoding.UTF8, "application/json");
        var settingsResp = await _httpClient.PatchAsync("/indexes/torrents/settings", settingsContent);
        if (settingsResp.IsSuccessStatusCode)
        {
            var task = await settingsResp.Content.ReadFromJsonAsync<MeiliTaskResponse>();
            if (task != null) await WaitForTaskAsync(task.TaskUid);
        }
        else
        {
            var err = await settingsResp.Content.ReadAsStringAsync();
            _logger.LogWarning("Failed to update index settings: {Error}", err);
        }
    }

    public async Task<int> PushBatchAsync(IReadOnlyList<object> documents, bool waitForTask = false)
    {
        if (documents.Count == 0) return 0;

        using var content = new StringContent(JsonSerializer.Serialize(documents, JsonOpts), Encoding.UTF8, "application/json");
        var resp = await _httpClient.PostAsync("/indexes/torrents/documents?primaryKey=infohash", content);
        if (!resp.IsSuccessStatusCode)
        {
            var err = await resp.Content.ReadAsStringAsync();
            throw new InvalidOperationException($"Failed to push batch to Meilisearch ({resp.StatusCode}): {err}");
        }

        var task = await resp.Content.ReadFromJsonAsync<MeiliTaskResponse>();
        if (waitForTask && task != null)
        {
            await WaitForTaskAsync(task.TaskUid);
        }

        return documents.Count;
    }

    public async Task<int> DeleteBatchAsync(IReadOnlyList<string> infohashes, bool waitForTask = false)
    {
        if (infohashes.Count == 0) return 0;

        using var content = new StringContent(JsonSerializer.Serialize(infohashes), Encoding.UTF8, "application/json");
        var resp = await _httpClient.PostAsync("/indexes/torrents/documents/delete-batch", content);
        if (!resp.IsSuccessStatusCode)
        {
            var err = await resp.Content.ReadAsStringAsync();
            throw new InvalidOperationException($"Failed to delete batch from Meilisearch ({resp.StatusCode}): {err}");
        }

        var task = await resp.Content.ReadFromJsonAsync<MeiliTaskResponse>();
        if (waitForTask && task != null)
        {
            await WaitForTaskAsync(task.TaskUid);
        }

        return infohashes.Count;
    }

    public async Task<bool> WaitForTaskAsync(long taskUid, int maxWaitSeconds = 60)
    {
        var start = DateTime.UtcNow;
        while ((DateTime.UtcNow - start).TotalSeconds < maxWaitSeconds)
        {
            var resp = await _httpClient.GetAsync($"/tasks/{taskUid}");
            if (resp.IsSuccessStatusCode)
            {
                var doc = await resp.Content.ReadFromJsonAsync<MeiliTaskDetail>();
                if (doc != null)
                {
                    if (doc.Status == "succeeded") return true;
                    if (doc.Status == "failed")
                    {
                        _logger.LogError("Meilisearch task {Uid} failed: {Error}", taskUid, doc.Error?.Message);
                        return false;
                    }
                }
            }
            await Task.Delay(300);
        }
        _logger.LogWarning("Meilisearch task {Uid} timed out after {Sec}s", taskUid, maxWaitSeconds);
        return false;
    }

    public async Task DeleteIndexAsync()
    {
        var resp = await _httpClient.DeleteAsync("/indexes/torrents");
        if (resp.IsSuccessStatusCode)
        {
            var task = await resp.Content.ReadFromJsonAsync<MeiliTaskResponse>();
            if (task != null) await WaitForTaskAsync(task.TaskUid);
        }
    }
}

public class MeiliTaskResponse
{
    [JsonPropertyName("taskUid")]
    public long TaskUid { get; set; }

    [JsonPropertyName("status")]
    public string Status { get; set; } = string.Empty;
}

public class MeiliTaskDetail
{
    [JsonPropertyName("taskUid")]
    public long TaskUid { get; set; }

    [JsonPropertyName("status")]
    public string Status { get; set; } = string.Empty;

    [JsonPropertyName("error")]
    public MeiliTaskError? Error { get; set; }
}

public class MeiliTaskError
{
    [JsonPropertyName("message")]
    public string Message { get; set; } = string.Empty;

    [JsonPropertyName("code")]
    public string Code { get; set; } = string.Empty;
}
