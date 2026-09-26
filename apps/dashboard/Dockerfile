# Stage 1: build or use pre-built Vite React client
FROM node:20-slim AS client-builder
WORKDIR /build
COPY apps/dashboard/client/ ./
RUN if [ -f dist/index.html ]; then \
      echo "--> Using pre-built client dist"; \
    else \
      npm ci --prefer-offline --no-audit && NODE_OPTIONS="--max-old-space-size=1536" npm run build; \
    fi

# Stage 2: build Gaia.Api (.NET 10 Minimal API)
FROM mcr.microsoft.com/dotnet/sdk:latest AS api-builder
WORKDIR /src
COPY apps/api/Gaia.Api.csproj ./
RUN dotnet restore Gaia.Api.csproj
COPY apps/api/ ./
RUN dotnet publish Gaia.Api.csproj -c Release -o /app/publish /p:UseAppHost=false

# Stage 3: runtime container
FROM mcr.microsoft.com/dotnet/aspnet:latest AS runtime
WORKDIR /app
COPY --from=api-builder /app/publish .
COPY --from=client-builder /build/dist ./wwwroot

ENV ASPNETCORE_URLS=http://0.0.0.0:3000
ENV SEARCH_PROVIDER=PostgreSQL
EXPOSE 3000

ENTRYPOINT ["dotnet", "Gaia.Api.dll"]