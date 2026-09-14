FROM mcr.microsoft.com/dotnet/sdk:latest AS build
WORKDIR /src

COPY Gaia.Api.csproj .
RUN dotnet restore Gaia.Api.csproj

COPY . .
RUN dotnet publish Gaia.Api.csproj -c Release -o /app/publish /p:UseAppHost=false

FROM mcr.microsoft.com/dotnet/aspnet:latest AS runtime
WORKDIR /app
COPY --from=build /app/publish .

ENV ASPNETCORE_URLS=http://+:5000
EXPOSE 5000

ENTRYPOINT ["dotnet", "Gaia.Api.dll"]
