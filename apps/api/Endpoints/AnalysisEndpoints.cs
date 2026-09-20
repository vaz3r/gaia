using Gaia.Api.Services;
using Microsoft.AspNetCore.Mvc;

namespace Gaia.Api.Endpoints;

public static class AnalysisEndpoints
{
    public static void MapAnalysisEndpoints(this IEndpointRouteBuilder app)
    {
        app.MapGet("/api/analysis", async (
            [FromQuery] string? category,
            DashboardRepository repo,
            ILoggerFactory loggerFactory,
            CancellationToken ct) =>
        {
            try
            {
                var data = await repo.GetAnalysisDataAsync(category, ct);
                return Results.Ok(data);
            }
            catch (Exception ex)
            {
                var logger = loggerFactory.CreateLogger("AnalysisEndpoints");
                logger.LogError(ex, "Transient error while computing analysis data");
                return Results.Json(new
                {
                    status = "warming",
                    message = "Analysis telemetry is compiling in the background. Please refresh in a moment.",
                    error = ex.Message
                }, statusCode: 200);
            }
        }).WithTags("Analysis");
    }
}
