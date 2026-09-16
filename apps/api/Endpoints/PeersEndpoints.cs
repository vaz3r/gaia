using Gaia.Api.Services;
using Microsoft.AspNetCore.Mvc;

namespace Gaia.Api.Endpoints;

public static class PeersEndpoints
{
    public static void MapPeersEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/peers").WithTags("Peers");

        group.MapGet("/", async (
            [FromQuery] string? search,
            [FromQuery] string? sort,
            [FromQuery] string? order,
            [FromQuery] int? page,
            [FromQuery] int? limit,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            var result = await repo.GetStablePeersAsync(search, sort, order, page ?? 1, limit ?? 25, ct);
            return Results.Ok(result);
        });

        group.MapGet("/{ip}/{port:int}/torrents", async (
            string ip,
            int port,
            DashboardRepository repo,
            CancellationToken ct) =>
        {
            var torrents = await repo.GetPeerTorrentsAsync(ip, port, ct);
            return Results.Ok(torrents);
        });
    }
}
