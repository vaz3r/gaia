using Gaia.Sync;
using Microsoft.Extensions.Logging;
using StackExchange.Redis;

Dapper.DefaultTypeMap.MatchNamesWithUnderscores = true;

var loggerFactory = LoggerFactory.Create(builder =>
{
    builder.AddSimpleConsole(options =>
    {
        options.IncludeScopes = false;
        options.SingleLine = true;
        options.TimestampFormat = "yyyy-MM-dd HH:mm:ss ";
    });
    builder.SetMinimumLevel(LogLevel.Information);
});

var logger = loggerFactory.CreateLogger("Gaia.Sync");
logger.LogInformation("Gaia.Sync worker initializing...");

var pgConnStr = Environment.GetEnvironmentVariable("DATABASE_URL")
    ?? "Host=192.168.10.10;Port=5432;Database=craw;Username=crawler;Password=83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b;Pooling=false;";

if (!pgConnStr.Contains("Command Timeout", StringComparison.OrdinalIgnoreCase))
{
    pgConnStr = pgConnStr.TrimEnd(';') + ";Command Timeout=180;Timeout=60;";
}

var meiliUrl = Environment.GetEnvironmentVariable("Meilisearch__Url")
    ?? "http://meilisearch:7700";

var meiliKey = Environment.GetEnvironmentVariable("Meilisearch__ApiKey")
    ?? Environment.GetEnvironmentVariable("MEILI_MASTER_KEY")
    ?? "meili_secure_master_key_v2_modern_scaling";

var redisUrl = Environment.GetEnvironmentVariable("REDIS_URL")
    ?? "redis:6379";

logger.LogInformation("Target Meilisearch: {Url}", meiliUrl);
logger.LogInformation("Target Redis: {Url}", redisUrl);

// Connect Redis
IConnectionMultiplexer redis;
try
{
    ConfigurationOptions redisConfig;
    if (Uri.TryCreate(redisUrl, UriKind.Absolute, out var uri) && (uri.Scheme == "redis" || uri.Scheme == "rediss"))
    {
        var host = uri.Host;
        var port = uri.Port > 0 ? uri.Port : 6379;
        var userInfo = uri.UserInfo;
        var password = userInfo.Contains(':') ? userInfo.Split(':')[1] : userInfo;
        redisConfig = new ConfigurationOptions
        {
            EndPoints = { { host, port } },
            Password = string.IsNullOrEmpty(password) ? null : password,
            Ssl = uri.Scheme == "rediss"
        };
    }
    else
    {
        redisConfig = ConfigurationOptions.Parse(redisUrl.Replace("redis://", "").TrimEnd('/'));
    }

    redisConfig.AbortOnConnectFail = false;
    redisConfig.ConnectRetry = 5;
    redis = await ConnectionMultiplexer.ConnectAsync(redisConfig);
    logger.LogInformation("Connected to Redis successfully.");
}
catch (Exception ex)
{
    logger.LogError(ex, "Failed to connect to Redis. Exiting.");
    return;
}

using var httpClient = new HttpClient();
var meiliClientLogger = loggerFactory.CreateLogger<MeiliClient>();
var meiliClient = new MeiliClient(httpClient, meiliUrl, meiliKey, meiliClientLogger);

using var cts = new CancellationTokenSource();
Console.CancelKeyPress += (_, e) =>
{
    e.Cancel = true;
    logger.LogInformation("Shutdown signal received. Cancelling worker...");
    cts.Cancel();
};

// 1. Reconciler CLI switches (isolated before constructing SyncEngine)
if (args.Contains("--test-alert") || args.Contains("--reconcile-once") || args.Contains("--heartbeat"))
{
    var reconcilerLogger = loggerFactory.CreateLogger<SearchReconciler>();
    var reconciler = new SearchReconciler(pgConnStr, meiliClient, redis, reconcilerLogger);

    if (args.Contains("--test-alert"))
    {
        logger.LogInformation("CLI flag --test-alert detected. Executing alert test harness...");
        await reconciler.RunTestAlertHarnessAsync(cts.Token);
        return;
    }

    if (args.Contains("--heartbeat"))
    {
        logger.LogInformation("CLI flag --heartbeat detected. Running single check with forced heartbeat...");
        var ok = await reconciler.ReconcileAsync(forceHeartbeat: true, updateHeartbeatSchedule: false, triggerAlert: true, ct: cts.Token);
        Environment.ExitCode = ok ? 0 : 1;
        return;
    }

    if (args.Contains("--reconcile-once"))
    {
        logger.LogInformation("CLI flag --reconcile-once detected. Running single reconciliation check...");
        var ok = await reconciler.ReconcileAsync(forceHeartbeat: false, updateHeartbeatSchedule: false, triggerAlert: true, ct: cts.Token);
        Environment.ExitCode = ok ? 0 : 1;
        return;
    }
}

// 2. Seeder / Swap CLI switches
if (args.Contains("--seed-redis-health"))
{
    logger.LogInformation("CLI flag --seed-redis-health detected. Executing Redis Health Seed into DB 1...");
    var seederLogger = loggerFactory.CreateLogger<RedisHealthSeeder>();
    var seeder = new RedisHealthSeeder(pgConnStr, redis, seederLogger);
    await seeder.SeedAsync(cts.Token);
    logger.LogInformation("Redis Health Seeding finished successfully.");
    return;
}

if (args.Contains("--test-swap"))
{
    logger.LogInformation("CLI flag --test-swap detected. Executing test swap on test_a / test_b...");
    await meiliClient.EnsureIndexAndSettingsAsync("test_a");
    await meiliClient.EnsureIndexAndSettingsAsync("test_b");
    await meiliClient.PushBatchToIndexAsync("test_a", new[] { new { infohash = "0000000000000000000000000000000000000001", name = "Test Index A" } }, waitForTask: true);
    await meiliClient.PushBatchToIndexAsync("test_b", new[] { new { infohash = "0000000000000000000000000000000000000002", name = "Test Index B" } }, waitForTask: true);
    await meiliClient.SwapIndexesAsync("test_a", "test_b", cts.Token);
    logger.LogInformation("Test swap succeeded! Cleaning up test indexes...");
    await meiliClient.DeleteIndexAsync("test_a");
    await meiliClient.DeleteIndexAsync("test_b");
    logger.LogInformation("Dry-run swap test passed completely.");
    return;
}

// 3. Continuous Sync Engine Execution
var stateRepo = new SyncStateRepository(pgConnStr);
var engineReconcilerLogger = loggerFactory.CreateLogger<SearchReconciler>();
var engineReconciler = new SearchReconciler(pgConnStr, meiliClient, redis, engineReconcilerLogger);
var syncEngineLogger = loggerFactory.CreateLogger<SyncEngine>();
var engine = new SyncEngine(pgConnStr, meiliClient, stateRepo, redis, engineReconciler, syncEngineLogger);

if (args.Contains("--rebuild-zero-downtime"))
{
    logger.LogInformation("CLI flag --rebuild-zero-downtime detected. Executing zero-downtime rebuild into torrents_v2...");
    await engine.RebuildTorrentsV2AndSwapAsync(cts.Token);
    logger.LogInformation("Zero-downtime rebuild finished successfully.");
    return;
}

try
{
    await engine.RunAsync(cts.Token);
}
catch (OperationCanceledException)
{
    logger.LogInformation("Gaia.Sync stopped gracefully.");
}
catch (Exception ex)
{
    logger.LogCritical(ex, "Gaia.Sync crashed unexpectedly.");
}
finally
{
    redis.Dispose();
}
