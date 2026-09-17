using Gaia.Api.Endpoints;
using Gaia.Api.Infrastructure;
using Gaia.Api.Services;
using Npgsql;
using StackExchange.Redis;

Dapper.DefaultTypeMap.MatchNamesWithUnderscores = true;

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

builder.Services.AddSingleton<CacheService>(sp =>
{
    var logger = sp.GetRequiredService<ILogger<CacheService>>();
    IConnectionMultiplexer? muxer = null;
    try
    {
        ConfigurationOptions opts;
        if (Uri.TryCreate(redisUrl, UriKind.Absolute, out var uri) && (uri.Scheme == "redis" || uri.Scheme == "rediss"))
        {
            var host = uri.Host;
            var port = uri.Port > 0 ? uri.Port : 6379;
            var userInfo = uri.UserInfo;
            var password = userInfo.Contains(':') ? userInfo.Split(':')[1] : userInfo;
            opts = new ConfigurationOptions
            {
                EndPoints = { { host, port } },
                Password = string.IsNullOrEmpty(password) ? null : password,
                Ssl = uri.Scheme == "rediss"
            };
        }
        else
        {
            opts = ConfigurationOptions.Parse(redisUrl.Replace("redis://", "").TrimEnd('/'));
        }

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
        logger.LogWarning(ex, "Failed to initialize Redis connection multiplexer for {Url}", redisUrl);
    }
    return new CacheService(muxer, logger);
});

// ── Search Providers ──────────────────────────────────────────────────────────
var searchProviderSetting = builder.Configuration["SEARCH_PROVIDER"]
    ?? Environment.GetEnvironmentVariable("SEARCH_PROVIDER")
    ?? "Meilisearch";

var meiliUrl = builder.Configuration["Meilisearch:Url"]
    ?? Environment.GetEnvironmentVariable("MEILI_URL")
    ?? "http://127.0.0.1:7700";
var meiliKey = builder.Configuration["Meilisearch:ApiKey"]
    ?? Environment.GetEnvironmentVariable("MEILI_API_KEY")
    ?? "";

builder.Services.AddSingleton<PostgresTrigramSearchProvider>();
builder.Services.AddSingleton<CircuitBreaker>();
builder.Services.AddHttpClient();

if (string.Equals(searchProviderSetting, "PostgreSQL", StringComparison.OrdinalIgnoreCase))
{
    // Dedicated Operator Dashboard instance: direct Postgres search provider only, zero Meilisearch
    builder.Services.AddSingleton<ISearchProvider>(sp => sp.GetRequiredService<PostgresTrigramSearchProvider>());
}
else
{
    builder.Services.AddHttpClient<ISearchProvider, MeilisearchSearchProvider>(client =>
    {
        client.BaseAddress = new Uri(meiliUrl);
        client.Timeout     = TimeSpan.FromSeconds(5);
        if (!string.IsNullOrWhiteSpace(meiliKey))
            client.DefaultRequestHeaders.Add("Authorization", $"Bearer {meiliKey}");
    }).ConfigurePrimaryHttpMessageHandler(() => new SocketsHttpHandler
    {
        PooledConnectionLifetime    = TimeSpan.FromMinutes(10),
        PooledConnectionIdleTimeout = TimeSpan.FromMinutes(5),
        MaxConnectionsPerServer     = 20,
    });

    builder.Services.AddHttpClient("Meilisearch", client =>
    {
        client.BaseAddress = new Uri(meiliUrl);
        client.Timeout     = TimeSpan.FromSeconds(5);
        if (!string.IsNullOrWhiteSpace(meiliKey))
            client.DefaultRequestHeaders.Add("Authorization", $"Bearer {meiliKey}");
    });
}

// ── Core Services & Repositories ──────────────────────────────────────────────
builder.Services.AddMemoryCache();
builder.Services.AddSingleton<DatabaseService>();
builder.Services.AddSingleton<NpgsqlDataSource>(sp => sp.GetRequiredService<DatabaseService>().DataSource);
builder.Services.AddSingleton<DashboardRepository>();
builder.Services.AddSingleton<WireMetadataFetcher>();
builder.Services.AddSingleton<AdminAuthFilter>();

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

// ── Static Files (Serves Dashboard SPA if wwwroot exists) ─────────────────────
var wwwrootPath = Path.Combine(app.Environment.ContentRootPath, "wwwroot");
if (Directory.Exists(wwwrootPath))
{
    app.UseDefaultFiles();
    app.UseStaticFiles();
}

// ── Route Groups ──────────────────────────────────────────────────────────────
app.MapTorrentEndpoints();
app.MapDashboardEndpoints();
app.MapDashboardTorrentEndpoints();
app.MapPeersEndpoints();
app.MapMetricsEndpoints();
app.MapSurveillanceEndpoints();
app.MapAlertsEndpoints();
app.MapScoringEndpoints();

app.MapGet("/health", () => Results.Ok(new
{
    status          = "healthy",
    runtime         = ".NET 10.0",
    search_provider = searchProviderSetting
}));

app.MapGet("/api/info", () => Results.Ok(new
{
    service = "GAIA V2 Core API",
    version = "2.0.0",
    runtime = ".NET 10.0",
    search  = searchProviderSetting,
    docs    = "/api/torrents"
}));

if (Directory.Exists(wwwrootPath))
{
    app.MapFallbackToFile("index.html");
}
else
{
    app.MapGet("/", () => Results.Ok(new
    {
        service = "GAIA V2 Core API",
        version = "2.0.0",
        runtime = ".NET 10.0",
        search  = searchProviderSetting,
        docs    = "/api/torrents"
    }));
}

app.Run();
