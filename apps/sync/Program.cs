using Gaia.Sync;
using Microsoft.Extensions.Logging;
using StackExchange.Redis;

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
    var redisConfig = ConfigurationOptions.Parse(redisUrl);
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

var stateRepo = new SyncStateRepository(pgConnStr);
var syncEngineLogger = loggerFactory.CreateLogger<SyncEngine>();
var engine = new SyncEngine(pgConnStr, meiliClient, stateRepo, redis, syncEngineLogger);

using var cts = new CancellationTokenSource();
Console.CancelKeyPress += (_, e) =>
{
    e.Cancel = true;
    logger.LogInformation("Shutdown signal received. Cancelling worker...");
    cts.Cancel();
};

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
