using System.Text.Json;
using Dapper;

namespace Gaia.Api.Services;

/// <summary>
/// Background service that keeps Meilisearch in sync with PostgreSQL.
///
/// Phase 1 (startup): Checks if index is populated. If < 500k docs,
///   bulk-imports all 3.4M torrents in batches of 10,000.
///   Falls back to PostgreSQL trigram search during import.
///
/// Phase 2 (steady state): Polls every 60s for recently verified/updated
///   records and upserts them into Meilisearch.
/// </summary>
public class MeilisearchSyncService : BackgroundService
{
    private readonly IServiceProvider _services;
    private readonly ILogger<MeilisearchSyncService> _logger;
    private readonly string _meiliUrl;
    private readonly string _meiliKey;

    // Exposed so TorrentEndpoints can check readiness before using Meilisearch
    public static volatile bool IsReady = false;

    public MeilisearchSyncService(
        IServiceProvider services,
        IConfiguration config,
        ILogger<MeilisearchSyncService> logger)
    {
        _services  = services;
        _logger    = logger;
        _meiliUrl  = config["Meilisearch:Url"] ?? "http://127.0.0.1:7700";
        _meiliKey  = config["Meilisearch:ApiKey"] ?? "";
    }

    protected override async Task ExecuteAsync(CancellationToken ct)
    {
        // Brief delay so the app fully starts before we touch PG
        await Task.Delay(TimeSpan.FromSeconds(3), ct);

        using var http = new HttpClient();
        http.BaseAddress = new Uri(_meiliUrl);
        if (!string.IsNullOrWhiteSpace(_meiliKey))
            http.DefaultRequestHeaders.Add("Authorization", $"Bearer {_meiliKey}");

        // Ensure index exists with correct settings
        await EnsureIndexConfiguredAsync(http, ct);

        // Phase 1: Bulk import if needed
        var docCount = await GetDocCountAsync(http, ct);
        _logger.LogInformation("Meilisearch index has {Count:N0} documents", docCount);

        if (docCount < 500_000)
        {
            _logger.LogInformation("Starting bulk import from PostgreSQL → Meilisearch...");
            await BulkImportAsync(http, ct);
        }
        else
        {
            _logger.LogInformation("Meilisearch index already populated — skipping bulk import");
        }

        IsReady = true;
        _logger.LogInformation("Meilisearch sync service ready. Starting incremental sync every 60s.");

        // Phase 2: Incremental sync
        var lastSync = DateTime.UtcNow.AddMinutes(-2); // overlap to catch anything missed
        while (!ct.IsCancellationRequested)
        {
            try
            {
                await Task.Delay(TimeSpan.FromSeconds(60), ct);
                var syncFrom = lastSync;
                lastSync = DateTime.UtcNow;
                await IncrementalSyncAsync(http, syncFrom, ct);
            }
            catch (OperationCanceledException) { break; }
            catch (Exception ex)
            {
                _logger.LogError(ex, "Incremental Meilisearch sync failed — will retry in 60s");
            }
        }
    }

    private async Task EnsureIndexConfiguredAsync(HttpClient http, CancellationToken ct)
    {
        try
        {
            // Create index (idempotent — returns 202 if already exists)
            await http.PostAsJsonAsync("/indexes", new { uid = "torrents", primaryKey = "infohash" }, ct);
            await Task.Delay(1000, ct); // let Meilisearch process

            // Configure settings
            var settings = new
            {
                searchableAttributes = new[] { "name", "infohash" },
                filterableAttributes = new[] { "category", "policy_action", "risk_tier" },
                sortableAttributes   = new[] { "verified_at", "total_size", "health_score", "popularity_score" },
                typoTolerance = new
                {
                    enabled = true,
                    minWordSizeForTypos = new { oneTypo = 4, twoTypos = 8 }
                },
                pagination = new { maxTotalHits = 100000 }
            };

            var resp = await http.PatchAsJsonAsync("/indexes/torrents/settings", settings, ct);
            _logger.LogInformation("Meilisearch index settings applied: {Status}", resp.StatusCode);
        }
        catch (Exception ex)
        {
            _logger.LogWarning(ex, "Failed to configure Meilisearch index settings");
        }
    }

    private async Task<long> GetDocCountAsync(HttpClient http, CancellationToken ct)
    {
        try
        {
            var resp = await http.GetFromJsonAsync<JsonDocument>("/indexes/torrents/stats", ct);
            return resp?.RootElement.GetProperty("numberOfDocuments").GetInt64() ?? 0;
        }
        catch { return 0; }
    }

    private async Task BulkImportAsync(HttpClient http, CancellationToken ct)
    {
        using var scope = _services.CreateScope();
        var db = scope.ServiceProvider.GetRequiredService<DatabaseService>();

        const int batchSize = 10_000;
        long offset = 0;
        long total  = 0;

        const string sql = """
            SELECT
                encode(infohash, 'hex')  AS infohash,
                name,
                category,
                total_size,
                file_count,
                EXTRACT(EPOCH FROM verified_at)::bigint AS verified_at,
                health_score,
                popularity_score,
                swarm_peers,
                seed_confirmed,
                risk_tier,
                policy_action
            FROM torrents
            WHERE policy_action IS DISTINCT FROM 'SUPPRESS'
            ORDER BY verified_at DESC
            LIMIT @lim OFFSET @off
        """;

        while (!ct.IsCancellationRequested)
        {
            await using var conn = await db.OpenConnectionAsync(ct);
            var batch = (await conn.QueryAsync(sql, new { lim = batchSize, off = offset })).AsList();
            if (batch.Count == 0) break;

            var docs = batch.Select(r => (IDictionary<string, object?>)r).ToList();
            await IndexDocumentsAsync(http, docs, ct);

            total  += batch.Count;
            offset += batchSize;
            _logger.LogInformation("Bulk import progress: {Total:N0} documents indexed", total);

            // Small pause to avoid overwhelming Meilisearch during indexing
            await Task.Delay(200, ct);
        }

        _logger.LogInformation("Bulk import complete: {Total:N0} total documents", total);
    }

    private async Task IncrementalSyncAsync(HttpClient http, DateTime since, CancellationToken ct)
    {
        using var scope = _services.CreateScope();
        var db = scope.ServiceProvider.GetRequiredService<DatabaseService>();

        const string sql = """
            SELECT
                encode(infohash, 'hex')  AS infohash,
                name,
                category,
                total_size,
                file_count,
                EXTRACT(EPOCH FROM verified_at)::bigint AS verified_at,
                health_score,
                popularity_score,
                swarm_peers,
                seed_confirmed,
                risk_tier,
                policy_action
            FROM torrents
            WHERE verified_at >= @since
              AND policy_action IS DISTINCT FROM 'SUPPRESS'
            LIMIT 50000
        """;

        await using var conn = await db.OpenConnectionAsync(ct);
        var rows = (await conn.QueryAsync(sql, new { since })).AsList();
        if (rows.Count == 0) return;

        var docs = rows.Select(r => (IDictionary<string, object?>)r).ToList();
        await IndexDocumentsAsync(http, docs, ct);
        _logger.LogInformation("Incremental sync: upserted {Count:N0} records", docs.Count);
    }

    private async Task IndexDocumentsAsync(HttpClient http, List<IDictionary<string, object?>> docs, CancellationToken ct)
    {
        // Batch into 5,000-doc chunks for Meilisearch's optimal ingestion
        const int chunkSize = 5_000;
        for (int i = 0; i < docs.Count; i += chunkSize)
        {
            var chunk = docs.Skip(i).Take(chunkSize).ToList();
            var resp  = await http.PostAsJsonAsync("/indexes/torrents/documents", chunk, ct);
            if (!resp.IsSuccessStatusCode)
            {
                var err = await resp.Content.ReadAsStringAsync(ct);
                _logger.LogWarning("Meilisearch index error: {Status} {Error}", resp.StatusCode, err[..Math.Min(200, err.Length)]);
            }
        }
    }
}
