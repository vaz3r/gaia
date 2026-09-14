using Gaia.Api.Endpoints;
using Gaia.Api.Services;

var builder = WebApplication.CreateBuilder(args);

// CORS configuration for public portal and internal dashboard
builder.Services.AddCors(options =>
{
    options.AddPolicy("AllowAll", policy =>
    {
        policy.AllowAnyOrigin()
              .AllowAnyMethod()
              .AllowAnyHeader();
    });
});

// Configure Quickwit Client with persistent connection pooling
builder.Services.AddHttpClient<QuickwitClient>(client =>
{
    var quickwitUrl = builder.Configuration["QUICKWIT_URL"] 
        ?? Environment.GetEnvironmentVariable("QUICKWIT_URL") 
        ?? "http://127.0.0.1:7280";

    client.BaseAddress = new Uri(quickwitUrl);
    client.Timeout = TimeSpan.FromSeconds(5);
});

// Register Singleton Services
builder.Services.AddSingleton<DatabaseService>();

var app = builder.Build();

app.UseCors("AllowAll");

// Map Route Groups
app.MapTorrentEndpoints();
app.MapDashboardEndpoints();

// Root greeting
app.MapGet("/", () => Results.Ok(new
{
    service = "GAIA V2 Core API",
    version = "1.0.0",
    runtime = ".NET 10.0",
    docs = "/api/torrents"
}));

app.Run();
