namespace Gaia.Api.Infrastructure;

public class AdminAuthFilter : IEndpointFilter
{
    private readonly string? _adminToken;

    public AdminAuthFilter(IConfiguration config)
    {
        _adminToken = config["GAIA_ADMIN_TOKEN"] 
            ?? Environment.GetEnvironmentVariable("GAIA_ADMIN_TOKEN");
    }

    public async ValueTask<object?> InvokeAsync(EndpointFilterInvocationContext context, EndpointFilterDelegate next)
    {
        // If no token is configured, skip enforcement (development mode)
        if (string.IsNullOrWhiteSpace(_adminToken))
        {
            return await next(context);
        }

        var httpCtx = context.HttpContext;
        if (httpCtx.Request.Headers.TryGetValue("X-Gaia-Admin-Token", out var token) && token == _adminToken)
        {
            return await next(context);
        }

        if (httpCtx.Request.Headers.TryGetValue("Authorization", out var authHeader) &&
            authHeader.ToString().StartsWith("Bearer ", StringComparison.OrdinalIgnoreCase) &&
            authHeader.ToString()[7..].Trim() == _adminToken)
        {
            return await next(context);
        }

        return Results.Json(new { error = "Unauthorized: Invalid or missing X-Gaia-Admin-Token" }, statusCode: StatusCodes.Status401Unauthorized);
    }
}
