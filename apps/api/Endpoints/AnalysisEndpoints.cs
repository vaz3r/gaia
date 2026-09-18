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
            CancellationToken ct) =>
        {
            var data = await repo.GetAnalysisDataAsync(category, ct);
            return Results.Ok(data);
        }).WithTags("Analysis");
    }
}
