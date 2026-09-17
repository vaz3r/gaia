using System.Net.Http.Headers;
using System.Net.Http.Json;
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

    public async Task EnsureIndexAndSettingsAsync(string indexUid = "torrents")
    {
        _logger.LogInformation("Ensuring Meilisearch index '{IndexUid}' exists and configuring canonical settings...", indexUid);

        // Create index if not exists
        var createPayload = new { uid = indexUid, primaryKey = "infohash" };
        using var createContent = new StringContent(JsonSerializer.Serialize(createPayload), Encoding.UTF8, "application/json");
        var createResp = await _httpClient.PostAsync("/indexes", createContent);
        if (createResp.IsSuccessStatusCode)
        {
            var task = await createResp.Content.ReadFromJsonAsync<MeiliTaskResponse>();
            if (task != null) await WaitForTaskAsync(task.TaskUid);
        }

        // Apply decoupled canonical settings
        var settings = new
        {
            searchableAttributes = new[] { "name_clean", "name" },
            filterableAttributes = new[] { "category", "risk_tier", "policy_action", "availability_state", "verified_at" },
            sortableAttributes = new[] { "total_size", "verified_at" },
            rankingRules = new[] { "words", "typo", "proximity", "attribute", "exactness", "verified_at:desc" },
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
        var settingsResp = await _httpClient.PatchAsync($"/indexes/{indexUid}/settings", settingsContent);
        if (settingsResp.IsSuccessStatusCode)
        {
            var task = await settingsResp.Content.ReadFromJsonAsync<MeiliTaskResponse>();
            if (task != null) await WaitForTaskAsync(task.TaskUid);
        }
        else
        {
            var err = await settingsResp.Content.ReadAsStringAsync();
            _logger.LogWarning("Failed to update index settings for {IndexUid}: {Error}", indexUid, err);
        }
    }

    public async Task<int> PushBatchAsync(IReadOnlyList<object> documents, bool waitForTask = false)
    {
        return await PushBatchToIndexAsync("torrents", documents, waitForTask);
    }

    public async Task<int> PushBatchToIndexAsync(string indexUid, IReadOnlyList<object> documents, bool waitForTask = false)
    {
        if (documents.Count == 0) return 0;

        using var content = new StringContent(JsonSerializer.Serialize(documents, JsonOpts), Encoding.UTF8, "application/json");
        var resp = await _httpClient.PostAsync($"/indexes/{indexUid}/documents?primaryKey=infohash", content);
        if (!resp.IsSuccessStatusCode)
        {
            var err = await resp.Content.ReadAsStringAsync();
            throw new InvalidOperationException($"Failed to push batch to Meilisearch index {indexUid} ({resp.StatusCode}): {err}");
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
        return await DeleteBatchFromIndexAsync("torrents", infohashes, waitForTask);
    }

    public async Task<int> DeleteBatchFromIndexAsync(string indexUid, IReadOnlyList<string> infohashes, bool waitForTask = false)
    {
        if (infohashes.Count == 0) return 0;

        using var content = new StringContent(JsonSerializer.Serialize(infohashes), Encoding.UTF8, "application/json");
        var resp = await _httpClient.PostAsync($"/indexes/{indexUid}/documents/delete-batch", content);
        if (!resp.IsSuccessStatusCode)
        {
            var err = await resp.Content.ReadAsStringAsync();
            throw new InvalidOperationException($"Failed to delete batch from Meilisearch index {indexUid} ({resp.StatusCode}): {err}");
        }

        var task = await resp.Content.ReadFromJsonAsync<MeiliTaskResponse>();
        if (waitForTask && task != null)
        {
            await WaitForTaskAsync(task.TaskUid);
        }

        return infohashes.Count;
    }

    public async Task SwapIndexesAsync(string indexA, string indexB, CancellationToken ct = default)
    {
        _logger.LogInformation("Executing atomic index swap between '{IndexA}' and '{IndexB}'...", indexA, indexB);
        var payload = new[] { new { indexes = new[] { indexA, indexB } } };
        var resp = await _httpClient.PostAsJsonAsync("/swap-indexes", payload, ct);
        if (!resp.IsSuccessStatusCode)
        {
            var err = await resp.Content.ReadAsStringAsync(ct);
            throw new InvalidOperationException($"Failed to swap indexes ({resp.StatusCode}): {err}");
        }

        var task = await resp.Content.ReadFromJsonAsync<MeiliTaskResponse>(cancellationToken: ct);
        if (task != null)
        {
            var ok = await WaitForTaskAsync(task.TaskUid);
            if (!ok) throw new InvalidOperationException($"Swap task {task.TaskUid} failed or timed out.");
        }
    }

    public async Task<int> GetPendingTaskCountAsync(string indexUid)
    {
        try
        {
            var resp = await _httpClient.GetAsync($"/tasks?statuses=enqueued,processing&indexUids={indexUid}&limit=1");
            if (resp.IsSuccessStatusCode)
            {
                using var doc = await JsonDocument.ParseAsync(await resp.Content.ReadAsStreamAsync());
                if (doc.RootElement.TryGetProperty("total", out var totalEl))
                {
                    return totalEl.GetInt32();
                }
            }
        }
        catch (Exception ex)
        {
            _logger.LogWarning(ex, "Error fetching pending task count for {IndexUid}", indexUid);
        }
        return 0;
    }

    public async Task<List<string>> GetFailedTasksAsync(string indexUid)
    {
        var errors = new List<string>();
        try
        {
            var resp = await _httpClient.GetAsync($"/tasks?statuses=failed&indexUids={indexUid}&limit=20");
            if (resp.IsSuccessStatusCode)
            {
                using var doc = await JsonDocument.ParseAsync(await resp.Content.ReadAsStreamAsync());
                if (doc.RootElement.TryGetProperty("results", out var resultsEl))
                {
                    foreach (var taskEl in resultsEl.EnumerateArray())
                    {
                        var taskType = taskEl.TryGetProperty("type", out var t) ? t.GetString() : string.Empty;
                        var errCode = taskEl.TryGetProperty("error", out var errObj) && errObj.TryGetProperty("code", out var c) ? c.GetString() : string.Empty;

                        // Benign: deleting a non-existent index before recreation is expected
                        if (taskType == "indexDeletion" && errCode == "index_not_found")
                        {
                            continue;
                        }

                        var uid = taskEl.TryGetProperty("uid", out var u) ? u.GetInt64() : 0;
                        var msg = errObj.ValueKind != JsonValueKind.Undefined && errObj.TryGetProperty("message", out var m) ? m.GetString() : "Unknown error";
                        errors.Add($"Task {uid} ({taskType}): {msg}");
                    }
                }
            }
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Failed to inspect task failure list for {IndexUid}", indexUid);
            errors.Add($"Error checking tasks: {ex.Message}");
        }
        return errors;
    }

    public async Task WaitForIndexIdleAsync(string indexUid, int maxWaitSeconds = 900)
    {
        var start = DateTime.UtcNow;
        while ((DateTime.UtcNow - start).TotalSeconds < maxWaitSeconds)
        {
            var resp = await _httpClient.GetAsync($"/tasks?statuses=enqueued,processing&indexUids={indexUid}");
            if (resp.IsSuccessStatusCode)
            {
                using var doc = await JsonDocument.ParseAsync(await resp.Content.ReadAsStreamAsync());
                if (doc.RootElement.TryGetProperty("results", out var results) && results.GetArrayLength() == 0)
                {
                    return;
                }
            }
            await Task.Delay(1000);
        }
        _logger.LogWarning("Timed out waiting for index {IndexUid} queue to become idle", indexUid);
    }

    public async Task<bool> WaitForTaskAsync(long taskUid, int maxWaitSeconds = 600)
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
            await Task.Delay(1000);
        }
        _logger.LogWarning("Meilisearch task {Uid} timed out after {Sec}s", taskUid, maxWaitSeconds);
        return false;
    }

    public async Task DeleteIndexAsync(string indexUid = "torrents")
    {
        var checkResp = await _httpClient.GetAsync($"/indexes/{indexUid}");
        if (checkResp.StatusCode == System.Net.HttpStatusCode.NotFound)
        {
            return;
        }

        var resp = await _httpClient.DeleteAsync($"/indexes/{indexUid}");
        if (resp.IsSuccessStatusCode)
        {
            var task = await resp.Content.ReadFromJsonAsync<MeiliTaskResponse>();
            if (task != null) await WaitForTaskAsync(task.TaskUid);
        }
    }

    public async Task<Dictionary<string, JsonElement>?> GetStatsAsync(string indexUid)
    {
        try
        {
            var resp = await _httpClient.GetAsync($"/indexes/{indexUid}/stats");
            if (resp.IsSuccessStatusCode)
            {
                return await resp.Content.ReadFromJsonAsync<Dictionary<string, JsonElement>>();
            }
        }
        catch (Exception ex)
        {
            _logger.LogWarning(ex, "Failed to fetch stats for index {IndexUid}", indexUid);
        }
        return null;
    }

    public async Task<JsonElement?> GetDocumentAsync(string indexUid, string docId)
    {
        try
        {
            var resp = await _httpClient.GetAsync($"/indexes/{indexUid}/documents/{docId}");
            if (resp.IsSuccessStatusCode)
            {
                using var doc = await JsonDocument.ParseAsync(await resp.Content.ReadAsStreamAsync());
                return doc.RootElement.Clone();
            }
        }
        catch
        {
            // not found or error
        }
        return null;
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
