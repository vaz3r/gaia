using Microsoft.Extensions.Caching.Memory;

namespace Gaia.Api.Endpoints;

public static class ClassifierEndpoints
{
    public static void MapClassifierEndpoints(this IEndpointRouteBuilder app)
    {
        var methods = new[] { "GET", "POST", "PUT", "DELETE", "PATCH" };

        app.MapMethods("/api/classifier/{**slug}", methods, ForwardClassifierRequest);
        app.MapMethods("/api/classifier", methods, ForwardClassifierRequest);
    }

    private static async Task ForwardClassifierRequest(
        string? slug,
        HttpContext ctx,
        IHttpClientFactory clientFactory,
        IMemoryCache cache,
        ILoggerFactory loggerFactory)
    {
        var logger = loggerFactory.CreateLogger("Gaia.Api.Endpoints.ClassifierEndpoints");
        var client = clientFactory.CreateClient("Classifier");

        var path = (slug ?? string.Empty).TrimStart('/');
        var targetPath = $"api/{path}";
        var queryString = ctx.Request.QueryString.Value ?? string.Empty;
        var fullTargetUrl = $"{targetPath}{queryString}";

        // Fast in-memory cache for status and metrics on GET to reduce ML daemon proxy hammering
        var isCacheable = HttpMethods.IsGet(ctx.Request.Method) && (path == "status" || path == "metrics" || path == "reclassify/status");
        var cacheKey = $"classifier:proxy:{path}";

        if (isCacheable && cache.TryGetValue(cacheKey, out (string ContentType, byte[] Data) cached))
        {
            ctx.Response.StatusCode = StatusCodes.Status200OK;
            ctx.Response.ContentType = cached.ContentType;
            await ctx.Response.Body.WriteAsync(cached.Data, ctx.RequestAborted);
            return;
        }

        using var proxyRequest = new HttpRequestMessage(new HttpMethod(ctx.Request.Method), fullTargetUrl);

        if (HttpMethods.IsPost(ctx.Request.Method) ||
            HttpMethods.IsPut(ctx.Request.Method) ||
            HttpMethods.IsPatch(ctx.Request.Method))
        {
            proxyRequest.Content = new StreamContent(ctx.Request.Body);
            if (!string.IsNullOrEmpty(ctx.Request.ContentType))
            {
                proxyRequest.Content.Headers.ContentType =
                    System.Net.Http.Headers.MediaTypeHeaderValue.Parse(ctx.Request.ContentType);
            }
        }

        // Forward headers
        if (ctx.Request.Headers.TryGetValue("Accept", out var accept))
        {
            proxyRequest.Headers.TryAddWithoutValidation("Accept", (IEnumerable<string?>)accept);
        }
        if (ctx.Request.Headers.TryGetValue("X-Gaia-Admin-Token", out var token))
        {
            proxyRequest.Headers.TryAddWithoutValidation("X-Gaia-Admin-Token", (IEnumerable<string?>)token);
        }
        if (ctx.Request.Headers.TryGetValue("Authorization", out var auth))
        {
            proxyRequest.Headers.TryAddWithoutValidation("Authorization", (IEnumerable<string?>)auth);
        }

        try
        {
            using var response = await client.SendAsync(proxyRequest, HttpCompletionOption.ResponseHeadersRead, ctx.RequestAborted);

            ctx.Response.StatusCode = (int)response.StatusCode;
            if (response.Content.Headers.ContentType != null)
            {
                ctx.Response.ContentType = response.Content.Headers.ContentType.ToString();
            }

            if (isCacheable && response.IsSuccessStatusCode)
            {
                var bodyBytes = await response.Content.ReadAsByteArrayAsync(ctx.RequestAborted);
                var cType = response.Content.Headers.ContentType?.ToString() ?? "application/json";
                cache.Set(cacheKey, (cType, bodyBytes), TimeSpan.FromSeconds(10));
                await ctx.Response.Body.WriteAsync(bodyBytes, ctx.RequestAborted);
            }
            else
            {
                await response.Content.CopyToAsync(ctx.Response.Body, ctx.RequestAborted);
            }
        }
        catch (HttpRequestException ex)
        {
            logger.LogWarning(ex, "Failed to proxy request to Classifier API at {Url}", fullTargetUrl);
            ctx.Response.StatusCode = StatusCodes.Status502BadGateway;
            await ctx.Response.WriteAsJsonAsync(new
            {
                error = "Classifier API daemon unreachable",
                details = ex.Message,
                target = fullTargetUrl
            }, ctx.RequestAborted);
        }
    }
}
