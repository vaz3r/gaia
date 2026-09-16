using System.Data;
using System.Text;
using System.Text.Json;
using Dapper;
using Microsoft.Extensions.Logging;
using Npgsql;
using StackExchange.Redis;

namespace Gaia.Sync;

public class SearchReconciler
{
    private readonly string _pgConnString;
    private readonly MeiliClient _meili;
    private readonly IConnectionMultiplexer _redis;
    private readonly ILogger<SearchReconciler> _logger;
    private readonly HttpClient _alertHttp;

    private readonly string? _ntfyTopic;
    private readonly string? _ntfyAuthToken;
    private readonly string _ntfyServer;
    private readonly string? _alertWebhookUrl;
    private readonly string? _healthchecksUrl;

    private readonly double _meiliParityThreshold;
    private readonly double _redisParityThreshold;

    public SearchReconciler(
        string pgConnString,
        MeiliClient meili,
        IConnectionMultiplexer redis,
        ILogger<SearchReconciler> logger)
    {
        _pgConnString = pgConnString;
        _meili = meili;
        _redis = redis;
        _logger = logger;

        _alertHttp = new HttpClient { Timeout = TimeSpan.FromSeconds(5) };

        _ntfyTopic = Environment.GetEnvironmentVariable("NTFY_TOPIC");
        _ntfyAuthToken = Environment.GetEnvironmentVariable("NTFY_AUTH_TOKEN");
        _ntfyServer = Environment.GetEnvironmentVariable("NTFY_SERVER") ?? "https://ntfy.sh";
        _alertWebhookUrl = Environment.GetEnvironmentVariable("ALERT_WEBHOOK_URL");
        _healthchecksUrl = Environment.GetEnvironmentVariable("HEALTHCHECKS_URL");

        _meiliParityThreshold = double.TryParse(Environment.GetEnvironmentVariable("MEILI_PARITY_THRESHOLD"), out var mt) ? mt : 0.001;
        _redisParityThreshold = double.TryParse(Environment.GetEnvironmentVariable("REDIS_PARITY_THRESHOLD"), out var rt) ? rt : 0.005;

        if (string.IsNullOrEmpty(_healthchecksUrl))
        {
            _logger.LogWarning("⚠️ HEALTHCHECKS_URL is not configured! The search reconciler has NO external dead-man's switch. If Gaia.Sync crashes, no external notifications will trigger.");
        }
    }

    public async Task EnsureSchemaAsync(CancellationToken ct = default)
    {
        await using var conn = new NpgsqlConnection(_pgConnString);
        await conn.OpenAsync(ct);

        const string sql = @"
            CREATE TABLE IF NOT EXISTS search_reconcile_history (
                id BIGSERIAL PRIMARY KEY,
                checked_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                pg_active BIGINT NOT NULL,
                meili_docs BIGINT NOT NULL,
                redis_docs BIGINT NOT NULL,
                meili_delta BIGINT NOT NULL,
                redis_delta BIGINT NOT NULL,
                redis_div_pct DOUBLE PRECISION NOT NULL,
                is_heartbeat BOOLEAN NOT NULL DEFAULT false
            );
            CREATE INDEX IF NOT EXISTS idx_search_reconcile_history_checked_at 
            ON search_reconcile_history(checked_at DESC);";

        await conn.ExecuteAsync(sql);
    }

    public async Task RunHourlyLoopAsync(CancellationToken ct)
    {
        _logger.LogInformation("Starting Search Reconciler background loop (Hourly Cadence)...");
        await EnsureSchemaAsync(ct);

        // Allow 60s warm-up after container startup
        try
        {
            await Task.Delay(TimeSpan.FromSeconds(60), ct);
        }
        catch (OperationCanceledException)
        {
            return;
        }

        while (!ct.IsCancellationRequested)
        {
            try
            {
                _logger.LogInformation("Executing scheduled hourly search reconciliation check...");
                await ReconcileAsync(forceHeartbeat: false, updateHeartbeatSchedule: true, triggerAlert: true, ct: ct);
            }
            catch (OperationCanceledException) when (ct.IsCancellationRequested)
            {
                break;
            }
            catch (Exception ex)
            {
                _logger.LogError(ex, "Unexpected exception during hourly search reconciliation check.");
                await DispatchAlertAsync(
                    "🚨 GAIA Search Reconciler Crash",
                    $"Unexpected exception during reconciliation check:\n{ex.Message}",
                    priority: "urgent",
                    tags: "warning,skull");
            }

            try
            {
                await Task.Delay(TimeSpan.FromHours(1), ct);
            }
            catch (OperationCanceledException)
            {
                break;
            }
        }

        _logger.LogInformation("Search Reconciler background loop stopped.");
    }

    public async Task<bool> ReconcileAsync(
        bool forceHeartbeat = false,
        bool updateHeartbeatSchedule = true,
        bool triggerAlert = true,
        CancellationToken ct = default)
    {
        await EnsureSchemaAsync(ct);
        var errors = new List<string>();
        var now = DateTime.UtcNow;

        await using var conn = new NpgsqlConnection(_pgConnString);
        await conn.OpenAsync(ct);

        // 1. PostgreSQL Active Count
        long pgActive = 0;
        try
        {
            pgActive = await conn.ExecuteScalarAsync<long>(
                "SELECT count(*) FROM torrents WHERE policy_action IS DISTINCT FROM 'SUPPRESS';");
            _logger.LogInformation("PostgreSQL active (non-suppressed) count: {Count:N0}", pgActive);
        }
        catch (Exception ex)
        {
            var err = $"PostgreSQL count query failed: {ex.Message}";
            _logger.LogError(ex, "{Error}", err);
            errors.Add(err);
        }

        // 2. Meilisearch Document Count
        long meiliDocs = 0;
        try
        {
            var stats = await _meili.GetStatsAsync("torrents");
            if (stats != null && stats.TryGetValue("numberOfDocuments", out var docEl) && docEl.TryGetInt64(out var d))
            {
                meiliDocs = d;
            }
            _logger.LogInformation("Meilisearch 'torrents' document count: {Count:N0}", meiliDocs);
        }
        catch (Exception ex)
        {
            var err = $"Meilisearch stats query failed: {ex.Message}";
            _logger.LogError(ex, "{Error}", err);
            errors.Add(err);
        }

        // 3. Redis DB 1 Key Count
        long redisDocs = 0;
        try
        {
            var db1 = _redis.GetDatabase(1);
            var rawDbsize = await db1.ExecuteAsync("DBSIZE");
            if (rawDbsize != null && long.TryParse(rawDbsize.ToString(), out var rSize))
            {
                redisDocs = rSize;
            }
            _logger.LogInformation("Redis DB 1 'gaia:health:*' key count: {Count:N0}", redisDocs);
        }
        catch (Exception ex)
        {
            var err = $"Redis DB 1 DBSIZE query failed: {ex.Message}";
            _logger.LogError(ex, "{Error}", err);
            errors.Add(err);
        }

        // 4. Strict Parity Calculations
        long meiliDelta = 0;
        double meiliDiv = 0;
        long redisDelta = 0;
        double redisDiv = 0;

        if (pgActive > 0)
        {
            meiliDelta = Math.Abs(meiliDocs - pgActive);
            meiliDiv = meiliDelta / (double)pgActive;
            var meiliPct = (meiliDocs / (double)pgActive) * 100.0;
            _logger.LogInformation("Meilisearch Parity: {Pct:F4}% | Delta: {Delta:N0} | Divergence: {Div:F4}", meiliPct, meiliDelta, meiliDiv);

            if (meiliDiv > _meiliParityThreshold)
            {
                var err = $"Meilisearch divergence {meiliDiv:F4} ({meiliDiv * 100:F2}%) exceeds {_meiliParityThreshold * 100:F2}% threshold (Delta: {meiliDelta:N0})";
                _logger.LogError("❌ FAILED: {Error}", err);
                errors.Add(err);
            }
            else
            {
                _logger.LogInformation("✅ PASSED: Meilisearch parity within {Thresh:F2}% threshold.", _meiliParityThreshold * 100.0);
            }

            redisDelta = Math.Abs(redisDocs - pgActive);
            redisDiv = redisDelta / (double)pgActive;
            var redisPct = (redisDocs / (double)pgActive) * 100.0;
            _logger.LogInformation("Redis DB 1 Parity:   {Pct:F4}% | Delta: {Delta:N0} | Divergence: {Div:F4}", redisPct, redisDelta, redisDiv);

            if (redisDiv > _redisParityThreshold)
            {
                var err = $"Redis DB 1 divergence {redisDiv:F4} ({redisDiv * 100:F2}%) exceeds {_redisParityThreshold * 100:F2}% threshold (Delta: {redisDelta:N0})";
                _logger.LogError("❌ FAILED: {Error}", err);
                errors.Add(err);
            }
            else
            {
                _logger.LogInformation("✅ PASSED: Redis DB 1 parity within {Thresh:F2}% threshold.", _redisParityThreshold * 100.0);
            }
        }

        // 5. Watermark Liveness Check (strictly fast and suppression loops)
        try
        {
            var syncStates = await conn.QueryAsync<(string loop_name, DateTime? last_run_at, bool completed, long rows_synced)>(@"
                SELECT loop_name, last_run_at, completed, rows_synced 
                FROM portal_sync_state 
                WHERE loop_name IN ('fast', 'suppression');");

            foreach (var state in syncStates)
            {
                var lag = state.last_run_at.HasValue ? now - state.last_run_at.Value : TimeSpan.MaxValue;
                var lagStr = state.last_run_at.HasValue ? $"{(int)lag.TotalSeconds}s ago" : "never";

                if (lag > TimeSpan.FromMinutes(5))
                {
                    var err = $"Watermark liveness breached on [{state.loop_name}]: last run {lagStr} exceeds 5m threshold.";
                    _logger.LogError("❌ FAILED: {Error}", err);
                    errors.Add(err);
                }
                else
                {
                    _logger.LogInformation("  ✅ [{LoopName}] completed={Completed}, rows_synced={Rows:N0}, last_run={Lag}",
                        state.loop_name, state.completed, state.rows_synced, lagStr);
                }
            }
        }
        catch (Exception ex)
        {
            var err = $"Watermark query failed: {ex.Message}";
            _logger.LogError(ex, "{Error}", err);
            errors.Add(err);
        }

        // 6. Suppressed Leak Verification (Sample 500, Strict Zero-Tolerance)
        try
        {
            var suppressed = (await conn.QueryAsync<string>(
                "SELECT encode(infohash, 'hex') FROM torrents WHERE policy_action = 'SUPPRESS' LIMIT 500;")).ToList();

            var meiliLeaks = 0;
            var redisLeaks = 0;

            if (suppressed.Count > 0)
            {
                // Check first 50 in Meilisearch
                foreach (var ih in suppressed.Take(50))
                {
                    var doc = await _meili.GetDocumentAsync("torrents", ih);
                    if (doc != null)
                    {
                        _logger.LogCritical("🚨 ALERT: Suppressed infohash {Hash} leaked into Meilisearch!", ih);
                        meiliLeaks++;
                    }
                }

                // Check all 500 in Redis DB 1 in a pipeline
                var db1 = _redis.GetDatabase(1);
                var tasks = suppressed.Select(ih => db1.KeyExistsAsync($"gaia:health:{ih}")).ToArray();
                var results = await Task.WhenAll(tasks);

                for (var i = 0; i < results.Length; i++)
                {
                    if (results[i])
                    {
                        _logger.LogCritical("🚨 ALERT: Suppressed infohash {Hash} leaked into Redis DB 1!", suppressed[i]);
                        redisLeaks++;
                    }
                }

                if (meiliLeaks == 0 && redisLeaks == 0)
                {
                    _logger.LogInformation("✅ PASSED: Suppressed Leak Check (0/{Count} leaked).", suppressed.Count);
                }
                else
                {
                    var err = $"Suppressed Leak Check: {meiliLeaks} Meili leaks, {redisLeaks} Redis leaks.";
                    _logger.LogError("❌ FAILED: {Error}", err);
                    errors.Add(err);
                }
            }
        }
        catch (Exception ex)
        {
            var err = $"Suppressed leak query failed: {ex.Message}";
            _logger.LogError(ex, "{Error}", err);
            errors.Add(err);
        }

        // 7. Sample Integrity Check (Sample 100, 100% presence)
        try
        {
            var sample = (await conn.QueryAsync<string>(@"
                SELECT encode(infohash, 'hex') 
                FROM torrents 
                WHERE policy_action IS DISTINCT FROM 'SUPPRESS' 
                ORDER BY verified_at DESC LIMIT 100;")).ToList();

            var meiliMissing = 0;
            var redisMissing = 0;

            if (sample.Count > 0)
            {
                foreach (var ih in sample)
                {
                    var doc = await _meili.GetDocumentAsync("torrents", ih);
                    if (doc == null) meiliMissing++;
                }

                var db1 = _redis.GetDatabase(1);
                var tasks = sample.Select(ih => db1.KeyExistsAsync($"gaia:health:{ih}")).ToArray();
                var results = await Task.WhenAll(tasks);
                redisMissing = results.Count(ex => !ex);

                if (meiliMissing == 0 && redisMissing == 0)
                {
                    _logger.LogInformation("✅ PASSED: Sample Integrity Check (100% of sample present in Meilisearch & Redis DB 1).");
                }
                else
                {
                    var err = $"Sample Integrity Check: {meiliMissing}/100 missing in Meili, {redisMissing}/100 missing in Redis DB 1.";
                    _logger.LogError("❌ FAILED: {Error}", err);
                    errors.Add(err);
                }
            }
        }
        catch (Exception ex)
        {
            var err = $"Sample integrity query failed: {ex.Message}";
            _logger.LogError(ex, "{Error}", err);
            errors.Add(err);
        }

        // 8. Determine if Heartbeat is Due
        var isHeartbeatDue = forceHeartbeat;
        if (!isHeartbeatDue)
        {
            var lastHeartbeat = await conn.ExecuteScalarAsync<DateTime?>(
                "SELECT MAX(checked_at) FROM search_reconcile_history WHERE is_heartbeat = true;");

            if (!lastHeartbeat.HasValue || (now - lastHeartbeat.Value) >= TimeSpan.FromHours(24))
            {
                isHeartbeatDue = true;
            }
        }

        var willMarkHeartbeat = isHeartbeatDue && updateHeartbeatSchedule && errors.Count == 0;

        // 9. Persist Drift Snapshot & Enforce 90-Day Retention
        try
        {
            await conn.ExecuteAsync(@"
                INSERT INTO search_reconcile_history 
                    (checked_at, pg_active, meili_docs, redis_docs, meili_delta, redis_delta, redis_div_pct, is_heartbeat)
                VALUES 
                    (@now, @pgActive, @meiliDocs, @redisDocs, @meiliDelta, @redisDelta, @divPct, @isHeartbeat);
                
                DELETE FROM search_reconcile_history 
                WHERE checked_at < NOW() - INTERVAL '90 days';",
                new
                {
                    now,
                    pgActive,
                    meiliDocs,
                    redisDocs,
                    meiliDelta,
                    redisDelta,
                    divPct = redisDiv * 100.0,
                    isHeartbeat = willMarkHeartbeat
                });

            // If Redis divergence > 0.05%, compute slope against oldest record in last 24h
            if (redisDiv > 0.0005)
            {
                var oldest24h = await conn.QueryFirstOrDefaultAsync<(DateTime checked_at, long redis_delta)>(@"
                    SELECT checked_at, redis_delta 
                    FROM search_reconcile_history 
                    WHERE checked_at >= NOW() - INTERVAL '24 hours' 
                    ORDER BY checked_at ASC LIMIT 1;");

                if (oldest24h.checked_at != default)
                {
                    var hours = (now - oldest24h.checked_at).TotalHours;
                    if (hours > 0.1)
                    {
                        var deltaDiff = redisDelta - oldest24h.redis_delta;
                        var dailySlope = deltaDiff / (hours / 24.0);
                        _logger.LogWarning(
                            "⚠️ ELEVATED DRIFT: Redis divergence {Div:F3}% (Delta: {Delta:N0}). 24h drift velocity: {Slope:+0;-0} docs/day over {Hours:F1} hours.",
                            redisDiv * 100.0, redisDelta, dailySlope, hours);
                    }
                }
            }
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Failed to persist drift snapshot to PostgreSQL.");
        }

        // 10. Alerting or Heartbeat Dispatch
        var passed = errors.Count == 0;

        if (!passed)
        {
            _logger.LogError("=== RECONCILIATION RESULT: FAILED ({Count} errors) ===", errors.Count);
            if (triggerAlert)
            {
                var details = string.Join("\n", errors.Select(e => $"• {e}"));
                await DispatchAlertAsync(
                    "🚨 GAIA Search Reconciler Alert: Invariants Breached",
                    details,
                    priority: "urgent",
                    tags: "warning,rotating_light");
            }
            // Strict dead-man's switch policy: NEVER ping on failure
            return false;
        }

        _logger.LogInformation("=== RECONCILIATION RESULT: ALL INVARIANTS SATISFIED ===");

        // Dispatch Proof-of-Life Heartbeat if due
        if (isHeartbeatDue)
        {
            var summary = $"• PostgreSQL Active: {pgActive:N0}\n" +
                          $"• Meilisearch Index: {meiliDocs:N0} (div: {meiliDiv * 100:F3}%)\n" +
                          $"• Redis DB 1 Keys:   {redisDocs:N0} (div: {redisDiv * 100:F3}%)\n" +
                          $"• Fast Loop Cadence: Healthy (<5m)\n" +
                          $"• Suppression Sweep: 0 Leaks\n" +
                          $"• All Invariants Satisfied (Status: OK)";

            await DispatchAlertAsync(
                "💚 GAIA Search Monitoring Heartbeat",
                summary,
                priority: "low",
                tags: "white_check_mark,green_heart");
        }

        // Ping Dead-Man's Switch ONLY on 100% clean success
        if (!string.IsNullOrEmpty(_healthchecksUrl))
        {
            try
            {
                using var resp = await _alertHttp.GetAsync(_healthchecksUrl, ct);
                _logger.LogInformation("Pinging Dead-Man's Switch at {Url}: {StatusCode}", _healthchecksUrl, resp.StatusCode);
            }
            catch (Exception ex)
            {
                _logger.LogError(ex, "Failed to ping Dead-Man's Switch at {Url}", _healthchecksUrl);
            }
        }

        return true;
    }

    public async Task RunTestAlertHarnessAsync(CancellationToken ct = default)
    {
        _logger.LogInformation("===============================================================");
        _logger.LogInformation("EXECUTING RECONCILER ALERT & RECOVERY TEST HARNESS");
        _logger.LogInformation("===============================================================");

        if (string.IsNullOrEmpty(_ntfyTopic) && string.IsNullOrEmpty(_alertWebhookUrl))
        {
            throw new InvalidOperationException("Cannot run test alert harness: Neither NTFY_TOPIC nor ALERT_WEBHOOK_URL is configured.");
        }

        // Step 1: Test Notification Dispatch Pipeline
        _logger.LogInformation("[Step 1/6] Testing Notification Dispatch Pipeline...");
        await DispatchAlertAsync(
            "🧪 GAIA Reconciler Test Alert",
            "Verification of containerized reconciler alert pipeline. Non-destructive test.",
            priority: "default",
            tags: "test_tube,bell");

        // Step 2: Fetch 5 sample hashes
        _logger.LogInformation("[Step 2/6] Selecting 5 sample keys for non-destructive perturbation...");
        await using var conn = new NpgsqlConnection(_pgConnString);
        await conn.OpenAsync(ct);

        var testHashes = (await conn.QueryAsync<string>(@"
            SELECT encode(infohash, 'hex') 
            FROM torrents 
            WHERE policy_action IS DISTINCT FROM 'SUPPRESS' 
            ORDER BY verified_at DESC LIMIT 5;")).ToList();

        if (testHashes.Count < 5)
        {
            throw new InvalidOperationException("Less than 5 sample hashes available for test.");
        }

        foreach (var h in testHashes)
        {
            _logger.LogInformation("  - gaia:health:{Hash}", h);
        }

        // Step 3: Stash keys from Redis DB 1 into memory
        var db1 = _redis.GetDatabase(1);
        var stashed = new Dictionary<string, HashEntry[]>();
        foreach (var h in testHashes)
        {
            stashed[h] = await db1.HashGetAllAsync($"gaia:health:{h}");
        }
        _logger.LogInformation("✅ Stashed test key values in memory.");

        // Step 4: Temporarily delete keys to simulate corruption
        _logger.LogInformation("[Step 3/6] Simulating key deletion in Redis DB 1...");
        foreach (var h in testHashes)
        {
            await db1.KeyDeleteAsync($"gaia:health:{h}");
        }
        _logger.LogInformation("🔥 Deleted 5 test keys from Redis DB 1.");

        // Step 5: Run check — MUST FAIL
        _logger.LogInformation("[Step 4/6] Running reconciliation check (Expecting Failure)...");
        var passedDuringPerturbation = await ReconcileAsync(forceHeartbeat: false, updateHeartbeatSchedule: false, triggerAlert: false, ct: ct);

        if (passedDuringPerturbation)
        {
            // Restore before throwing
            foreach (var kv in stashed)
            {
                if (kv.Value.Length > 0) await db1.HashSetAsync($"gaia:health:{kv.Key}", kv.Value);
            }
            throw new InvalidOperationException("CRITICAL: Reconciler passed when it should have failed!");
        }
        _logger.LogInformation("🚨 ALERT CONFIRMED: Reconciler raised alarm and failed loudly as expected!");

        // Step 6: Restore keys
        _logger.LogInformation("[Step 5/6] Restoring stashed test keys back to Redis DB 1...");
        foreach (var kv in stashed)
        {
            if (kv.Value.Length > 0)
            {
                await db1.HashSetAsync($"gaia:health:{kv.Key}", kv.Value);
            }
        }
        _logger.LogInformation("✅ Restored all 5 test keys to Redis DB 1.");

        // Step 7: Run check again — MUST PASS
        _logger.LogInformation("[Step 6/6] Running reconciliation check (Expecting 100% Clean Pass)...");
        var recovered = await ReconcileAsync(forceHeartbeat: false, updateHeartbeatSchedule: false, triggerAlert: true, ct: ct);
        if (!recovered)
        {
            throw new InvalidOperationException("Reconciler did not return to clean pass after restoration!");
        }

        _logger.LogInformation("===============================================================");
        _logger.LogInformation("✅ RECONCILER ALERT & RECOVERY TEST: ALL TESTS PASSED!");
        _logger.LogInformation("===============================================================");
    }

    private async Task DispatchAlertAsync(
        string subject,
        string details,
        string priority = "urgent",
        string tags = "warning,rotating_light")
    {
        _logger.LogInformation("📢 Dispatching Alert: {Subject}", subject);

        // 1. NTFY dispatch
        if (!string.IsNullOrEmpty(_ntfyTopic))
        {
            try
            {
                var url = $"{_ntfyServer.TrimEnd('/')}/{_ntfyTopic}";
                var body = $"{subject}\n\n{details}";
                using var msg = new HttpRequestMessage(HttpMethod.Post, url)
                {
                    Content = new StringContent(body, Encoding.UTF8, "text/plain")
                };

                // Sanitize header to ASCII to comply with HTTP header standards
                var asciiSubject = Encoding.ASCII.GetString(Encoding.ASCII.GetBytes(subject)).Replace("?", "").Trim();
                msg.Headers.Add("Title", string.IsNullOrEmpty(asciiSubject) ? "GAIA Reconciler Alert" : asciiSubject);
                msg.Headers.Add("Priority", priority);
                msg.Headers.Add("Tags", tags);

                if (!string.IsNullOrEmpty(_ntfyAuthToken))
                {
                    msg.Headers.Add("Authorization", $"Bearer {_ntfyAuthToken}");
                }

                using var resp = await _alertHttp.SendAsync(msg);
                if (resp.IsSuccessStatusCode)
                {
                    _logger.LogInformation("  ✅ Alert dispatched to ntfy: {Url} ({StatusCode})", url, resp.StatusCode);
                }
                else
                {
                    _logger.LogWarning("  ⚠️ ntfy dispatch returned non-success: {StatusCode}", resp.StatusCode);
                }
            }
            catch (Exception ex)
            {
                _logger.LogWarning("  ⚠️ Failed to dispatch alert to ntfy: {Message}", ex.Message);
            }
        }

        // 2. Webhook dispatch
        if (!string.IsNullOrEmpty(_alertWebhookUrl))
        {
            try
            {
                var payload = JsonSerializer.Serialize(new { text = $"*{subject}*\n```{details}```" });
                using var content = new StringContent(payload, Encoding.UTF8, "application/json");
                using var resp = await _alertHttp.PostAsync(_alertWebhookUrl, content);
                _logger.LogInformation("  ✅ Alert dispatched to webhook: {Url} ({StatusCode})", _alertWebhookUrl, resp.StatusCode);
            }
            catch (Exception ex)
            {
                _logger.LogWarning("  ⚠️ Failed to dispatch alert to webhook: {Message}", ex.Message);
            }
        }
    }
}
