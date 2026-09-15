using Gaia.Api.Endpoints;
using Gaia.Api.Services;
using StackExchange.Redis;

var builder = WebApplication.CreateBuilder(args);

// ── CORS ──────────────────────────────────────────────────────────────────────
builder.Services.AddCors(options =>
{
    options.AddPolicy("AllowAll", policy =>
        policy.AllowAnyOrigin().AllowAnyMethod().AllowAnyHeader());
});

// ── Redis ─────────────────────────────────────────────────────────────────────
var redisUrl  = builder.Configuration["REDIS_URL"]
    ?? Environment.GetEnvironmentVariable("REDIS_URL")
    ?? "redis://127.0.0.1:6379";
var redisHost = redisUrl.Replace("redis://", "").TrimEnd('/');

builder.Services.AddSingleton<CacheService>(sp =>
{
    var logger = sp.GetRequiredService<ILogger<CacheService>>();
    IConnectionMultiplexer? muxer = null;
    try
    {
        var opts = ConfigurationOptions.Parse(redisHost);
        opts.AbortOnConnectFail   = false;
        opts.ConnectRetry         = 3;
        opts.ConnectTimeout       = 2000;
        opts.SyncTimeout          = 1000;
        opts.AsyncTimeout         = 1000;
        opts.ReconnectRetryPolicy = new ExponentialRetry(5000);
        muxer = ConnectionMultiplexer.Connect(opts);
    }
    catch (Exception ex)
    {
        logger.LogWarning(ex, "Failed to initialize Redis connection multiplexer for {Host}", redisHost);
    }
    return new CacheService(muxer, logger);
});

// ── Meilisearch HTTP Client ───────────────────────────────────────────────────
var meiliUrl = builder.Configuration["Meilisearch:Url"]
    ?? Environment.GetEnvironmentVariable("MEILI_URL")
    ?? "http://127.0.0.1:7700";
var meiliKey = builder.Configuration["Meilisearch:ApiKey"]
    ?? Environment.GetEnvironmentVariable("MEILI_API_KEY")
    ?? "";

builder.Services.AddHttpClient<MeilisearchClient>(client =>
{
    client.BaseAddress = new Uri(meiliUrl);
    client.Timeout     = TimeSpan.FromSeconds(10);
    if (!string.IsNullOrWhiteSpace(meiliKey))
        client.DefaultRequestHeaders.Add("Authorization", $"Bearer {meiliKey}");
}).ConfigurePrimaryHttpMessageHandler(() => new SocketsHttpHandler
{
    PooledConnectionLifetime    = TimeSpan.FromMinutes(10),
    PooledConnectionIdleTimeout = TimeSpan.FromMinutes(5),
    MaxConnectionsPerServer     = 10,
});

// ── Meilisearch Sync Background Service ──────────────────────────────────────
builder.Services.AddHostedService<MeilisearchSyncService>();

// ── Core Services ─────────────────────────────────────────────────────────────
builder.Services.AddMemoryCache();
builder.Services.AddSingleton<DatabaseService>();

// ── Output Cache (in-process, protects against spike traffic) ─────────────────
builder.Services.AddOutputCache(opts =>
{
    opts.AddBasePolicy(p => p.Expire(TimeSpan.FromSeconds(5)));
    opts.AddPolicy("stats", p => p.Expire(TimeSpan.FromSeconds(30)).Tag("stats"));
});

var app = builder.Build();

// Warm up the Npgsql connection pool on startup (eliminates cold-start P99 spikes)
var db = app.Services.GetRequiredService<DatabaseService>();
_ = Task.Run(async () =>
{
    await Task.Delay(500);
    try { await db.WarmUpAsync(); } catch { /* non-fatal */ }
});

app.UseCors("AllowAll");
app.UseOutputCache();

// ── Routes ────────────────────────────────────────────────────────────────────
app.MapTorrentEndpoints();
app.MapDashboardEndpoints();

app.MapGet("/health", () => Results.Ok(new
{
    status           = "healthy",
    runtime          = ".NET 10.0",
    meilisearch_ready = MeilisearchSyncService.IsReady
}));

app.MapGet("/", () => Results.Ok(new
{
    service = "GAIA V2 Core API",
    version = "2.0.0",
    runtime = ".NET 10.0",
    search  = "meilisearch",
    docs    = "/api/torrents"
}));

app.Run();
