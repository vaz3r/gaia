using System.Diagnostics;
using System.Threading.Channels;
using Dapper;
using Npgsql;

namespace Gaia.Api.Services;

public record PurgeProgress
{
    public bool IsRunning { get; init; }
    public string? Category { get; init; }
    public long TotalTarget { get; init; }
    public long Processed { get; init; }
    public long EstimatedBytesReclaimed { get; init; }
    public double ItemsPerSecond { get; init; }
    public double? EtaSeconds { get; init; }
    public DateTime? StartedAt { get; init; }
    public DateTime? CompletedAt { get; init; }
    public string? LastError { get; init; }
    public bool Cancelled { get; init; }
}

public class CategoryPurgeService
{
    private readonly DatabaseService _db;
    private readonly CacheService _redis;
    private readonly ILogger<CategoryPurgeService> _logger;
    private readonly SemaphoreSlim _lock = new(1, 1);
    private CancellationTokenSource? _cts;
    private PurgeProgress _status = new() { IsRunning = false };
    private readonly Channel<PurgeProgress> _progressChannel = Channel.CreateBounded<PurgeProgress>(new BoundedChannelOptions(100)
    {
        FullMode = BoundedChannelFullMode.DropOldest
    });

    public CategoryPurgeService(DatabaseService db, CacheService redis, ILogger<CategoryPurgeService> logger)
    {
        _db = db;
        _redis = redis;
        _logger = logger;
    }

    public PurgeProgress GetStatus() => _status;

    public ChannelReader<PurgeProgress> GetProgressReader() => _progressChannel.Reader;

    public async Task<PurgeProgress> StartPurgeAsync(string category, int chunkSize = 5000, int delayMs = 50)
    {
        if (string.IsNullOrWhiteSpace(category))
            throw new ArgumentException("Category cannot be empty", nameof(category));

        if (!await _lock.WaitAsync(0))
            throw new InvalidOperationException("A category purge task is already running.");

        try
        {
            await using var conn = await _db.DataSource.OpenConnectionAsync();

            // Verification: Ensure category is disabled in policies before allowing purge
            var policy = await conn.QueryFirstOrDefaultAsync<dynamic>(
                "SELECT is_enabled FROM category_policies WHERE category = @category;",
                new { category });

            if (policy == null)
            {
                throw new InvalidOperationException($"Category '{category}' does not exist in category_policies.");
            }

            if ((bool)policy.is_enabled)
            {
                throw new InvalidOperationException(
                    $"Category '{category}' is currently enabled. You must disable future crawling for this category before purging stored data.");
            }

            // Confidence guard: only purge verified high-confidence records.
            // Ambiguous or low-confidence records (needs_review = true or conf < 0.85) are preserved
            // to ensure false positives (e.g. movies, games mislabeled as Adult) are never purged or tombstoned.
            var totalCount = await conn.ExecuteScalarAsync<long>(
                @"SELECT count(*) FROM torrents 
                  WHERE category = @category 
                    AND (needs_review = false OR needs_review IS NULL) 
                    AND (category_confidence IS NULL OR category_confidence >= 0.85);",
                new { category });

            if (totalCount == 0)
            {
                _lock.Release();
                return new PurgeProgress
                {
                    IsRunning = false,
                    Category = category,
                    TotalTarget = 0,
                    Processed = 0,
                    CompletedAt = DateTime.UtcNow
                };
            }

            _cts = new CancellationTokenSource();
            var startedAt = DateTime.UtcNow;

            _status = new PurgeProgress
            {
                IsRunning = true,
                Category = category,
                TotalTarget = totalCount,
                Processed = 0,
                EstimatedBytesReclaimed = 0,
                ItemsPerSecond = 0,
                StartedAt = startedAt
            };
            _progressChannel.Writer.TryWrite(_status);

            var ct = _cts.Token;
            _ = Task.Run(async () =>
            {
                try
                {
                    await ExecutePurgeLoopAsync(category, totalCount, chunkSize, delayMs, startedAt, ct);
                }
                finally
                {
                    _lock.Release();
                }
            }, CancellationToken.None);

            return _status;
        }
        catch
        {
            _lock.Release();
            throw;
        }
    }

    public bool Cancel()
    {
        if (_cts != null && !_cts.IsCancellationRequested && _status.IsRunning)
        {
            _logger.LogInformation("Cancellation requested for category purge: {Category}", _status.Category);
            _cts.Cancel();
            return true;
        }
        return false;
    }

    private async Task ExecutePurgeLoopAsync(
        string category,
        long totalCount,
        int chunkSize,
        int delayMs,
        DateTime startedAt,
        CancellationToken ct)
    {
        var sw = Stopwatch.StartNew();
        long processed = 0;
        long totalBytesReclaimed = 0;
        const long estimatedBytesPerRow = 28000; // ~28KB across torrents, peer outcomes, sightings, scores

        _logger.LogInformation("Starting background purge of {Count:N0} torrents for category '{Category}'...", totalCount, category);

        try
        {
            while (!ct.IsCancellationRequested)
            {
                await using var conn = await _db.DataSource.OpenConnectionAsync(ct);
                await using var tx = await conn.BeginTransactionAsync(ct);

                // Fetch chunk of infohashes (only high-confidence verified records; ambiguous ones are protected)
                var hashes = (await conn.QueryAsync<byte[]>(
                    @"SELECT infohash FROM torrents 
                      WHERE category = @category 
                        AND (needs_review = false OR needs_review IS NULL)
                        AND (category_confidence IS NULL OR category_confidence >= 0.85)
                      LIMIT @chunkSize 
                      FOR UPDATE SKIP LOCKED;",
                    new { category, chunkSize }, tx)).ToList();

                if (hashes.Count == 0)
                {
                    await tx.RollbackAsync(ct);
                    break;
                }

                // 1. Tombstone them permanently so they are NEVER crawled again
                await conn.ExecuteAsync(
                    @"INSERT INTO blocked_infohashes (infohash, category, reason, blocked_at)
                      SELECT u, @category, 'category_purged', now()
                      FROM unnest(@hashes) AS u
                      ON CONFLICT (infohash) DO NOTHING;",
                    new { hashes, category }, tx);

                // 2. Cascade delete from secondary and log tables
                await conn.ExecuteAsync("DELETE FROM fetch_peer_outcomes WHERE infohash = ANY(@hashes);", new { hashes }, tx);
                await conn.ExecuteAsync("DELETE FROM verification_jobs WHERE infohash = ANY(@hashes);", new { hashes }, tx);
                await conn.ExecuteAsync("DELETE FROM infohash_sightings WHERE infohash = ANY(@hashes);", new { hashes }, tx);
                await conn.ExecuteAsync("DELETE FROM peer_torrents WHERE infohash = ANY(@hashes);", new { hashes }, tx);
                await conn.ExecuteAsync("DELETE FROM torrent_score_history WHERE infohash = ANY(@hashes);", new { hashes }, tx);

                // 3. Delete from primary torrents table
                await conn.ExecuteAsync("DELETE FROM torrents WHERE infohash = ANY(@hashes);", new { hashes }, tx);

                await tx.CommitAsync(ct);

                processed += hashes.Count;
                totalBytesReclaimed += hashes.Count * estimatedBytesPerRow;

                var elapsed = sw.Elapsed.TotalSeconds;
                var rate = elapsed > 0 ? processed / elapsed : 0;
                var remaining = Math.Max(0, totalCount - processed);
                var eta = rate > 0 ? remaining / rate : (double?)null;

                _status = new PurgeProgress
                {
                    IsRunning = true,
                    Category = category,
                    TotalTarget = totalCount,
                    Processed = processed,
                    EstimatedBytesReclaimed = totalBytesReclaimed,
                    ItemsPerSecond = Math.Round(rate, 1),
                    EtaSeconds = eta.HasValue ? Math.Round(eta.Value, 0) : null,
                    StartedAt = startedAt
                };
                _progressChannel.Writer.TryWrite(_status);

                if (delayMs > 0)
                {
                    await Task.Delay(delayMs, ct);
                }
            }

            if (ct.IsCancellationRequested)
            {
                _logger.LogWarning("Category purge for '{Category}' cancelled by operator at {Processed:N0}/{Total:N0}.", category, processed, totalCount);
                _status = _status with
                {
                    IsRunning = false,
                    Cancelled = true,
                    CompletedAt = DateTime.UtcNow
                };
                _progressChannel.Writer.TryWrite(_status);
                return;
            }

            // Post-purge cleanup: execute VACUUM ANALYZE to reclaim disk space to OS / freespace map
            _logger.LogInformation("Purge loop completed for '{Category}'. Executing VACUUM ANALYZE...", category);
            try
            {
                await using var vacuumConn = await _db.DataSource.OpenConnectionAsync(CancellationToken.None);
                await vacuumConn.ExecuteAsync("VACUUM ANALYZE torrents;");
                await vacuumConn.ExecuteAsync("VACUUM ANALYZE fetch_peer_outcomes;");
                await vacuumConn.ExecuteAsync("VACUUM ANALYZE infohash_sightings;");
            }
            catch (Exception ex)
            {
                _logger.LogWarning(ex, "Non-fatal error running VACUUM ANALYZE after purge.");
            }

            _status = new PurgeProgress
            {
                IsRunning = false,
                Category = category,
                TotalTarget = totalCount,
                Processed = processed,
                EstimatedBytesReclaimed = totalBytesReclaimed,
                ItemsPerSecond = 0,
                EtaSeconds = 0,
                StartedAt = startedAt,
                CompletedAt = DateTime.UtcNow
            };
            _progressChannel.Writer.TryWrite(_status);

            _logger.LogInformation("Purge finished successfully for '{Category}': {Processed:N0} records cleaned, ~{Gb:F2} GB freed.",
                category, processed, totalBytesReclaimed / (1024.0 * 1024 * 1024));
        }
        catch (OperationCanceledException)
        {
            _status = _status with
            {
                IsRunning = false,
                Cancelled = true,
                CompletedAt = DateTime.UtcNow
            };
            _progressChannel.Writer.TryWrite(_status);
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Unexpected error in purge worker for '{Category}'", category);
            _status = _status with
            {
                IsRunning = false,
                LastError = ex.Message,
                CompletedAt = DateTime.UtcNow
            };
            _progressChannel.Writer.TryWrite(_status);
        }
    }
}
