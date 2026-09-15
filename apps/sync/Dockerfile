FROM mcr.microsoft.com/dotnet/sdk:latest AS build
WORKDIR /src

COPY Gaia.Sync.csproj .
RUN dotnet restore Gaia.Sync.csproj

COPY . .
RUN dotnet publish Gaia.Sync.csproj -c Release -o /app/publish /p:UseAppHost=false

FROM mcr.microsoft.com/dotnet/runtime:latest AS runtime
WORKDIR /app
COPY --from=build /app/publish .

ENTRYPOINT ["dotnet", "Gaia.Sync.dll"]
