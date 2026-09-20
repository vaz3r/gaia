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

    public async Task<PurgeProgress> StartPurgeAsync(string category, int chunkSize = 250, int delayMs = 2000)
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
                new CommandDefinition(
                    @"SELECT count(*) FROM torrents 
                      WHERE category = @category 
                        AND (needs_review = false OR needs_review IS NULL) 
                        AND (category_confidence IS NULL OR category_confidence >= 0.85);",
                    new { category },
                    commandTimeout: 180));

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
                StartedAt = startedAt,
                LastError = null,
                Cancelled = false
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
                int chunkProcessed = 0;
                var batchSw = Stopwatch.StartNew();
                const int maxRetries = 10;
                int retryCount = 0;

                while (true)
                {
                    try
                    {
                        await using var conn = await _db.DataSource.OpenConnectionAsync(ct);
                        await using var tx = await conn.BeginTransactionAsync(ct);

                        // Fetch chunk of infohashes (only high-confidence verified records; ambiguous ones are protected)
                        // Deterministic ORDER BY infohash prevents lock circularity (deadlocks) with concurrent crawler/classifier workers
                        var hashes = (await conn.QueryAsync<byte[]>(
                            new CommandDefinition(
                                @"SELECT infohash FROM torrents 
                                  WHERE category = @category 
                                    AND (needs_review = false OR needs_review IS NULL)
                                    AND (category_confidence IS NULL OR category_confidence >= 0.85)
                                  ORDER BY infohash
                                  LIMIT @chunkSize 
                                  FOR UPDATE SKIP LOCKED;",
                                new { category, chunkSize },
                                transaction: tx,
                                commandTimeout: 180,
                                cancellationToken: ct))).ToList();

                        if (hashes.Count == 0)
                        {
                            await tx.RollbackAsync(ct);
                            chunkProcessed = 0;
                            break;
                        }

                        // 1. Tombstone them permanently so they are NEVER crawled again
                        await conn.ExecuteAsync(
                            new CommandDefinition(
                                @"INSERT INTO blocked_infohashes (infohash, category, reason, blocked_at)
                                  SELECT u, @category, 'category_purged', now()
                                  FROM unnest(@hashes) AS u
                                  ON CONFLICT (infohash) DO NOTHING;",
                                new { hashes, category },
                                transaction: tx,
                                commandTimeout: 180,
                                cancellationToken: ct));

                        // 2. Cascade delete from secondary and log tables
                        await conn.ExecuteAsync(new CommandDefinition("DELETE FROM fetch_peer_outcomes WHERE infohash = ANY(@hashes);", new { hashes }, transaction: tx, commandTimeout: 180, cancellationToken: ct));
                        await conn.ExecuteAsync(new CommandDefinition("DELETE FROM verification_jobs WHERE infohash = ANY(@hashes);", new { hashes }, transaction: tx, commandTimeout: 180, cancellationToken: ct));
                        await conn.ExecuteAsync(new CommandDefinition("DELETE FROM infohash_sightings WHERE infohash = ANY(@hashes);", new { hashes }, transaction: tx, commandTimeout: 180, cancellationToken: ct));
                        await conn.ExecuteAsync(new CommandDefinition("DELETE FROM peer_torrents WHERE infohash = ANY(@hashes);", new { hashes }, transaction: tx, commandTimeout: 180, cancellationToken: ct));
                        await conn.ExecuteAsync(new CommandDefinition("DELETE FROM torrent_score_history WHERE infohash = ANY(@hashes);", new { hashes }, transaction: tx, commandTimeout: 180, cancellationToken: ct));

                        // 3. Delete from primary torrents table
                        await conn.ExecuteAsync(new CommandDefinition("DELETE FROM torrents WHERE infohash = ANY(@hashes);", new { hashes }, transaction: tx, commandTimeout: 180, cancellationToken: ct));

                        await tx.CommitAsync(ct);
                        chunkProcessed = hashes.Count;
                        break;
                    }
                    catch (Exception ex) when (IsTransientException(ex) && retryCount < maxRetries && !ct.IsCancellationRequested)
                    {
                        retryCount++;
                        var backoffMs = Math.Min(15000, (int)(Math.Pow(2, retryCount) * 500)) + Random.Shared.Next(100, 500);
                        _logger.LogWarning(ex, "Transient database contention or timeout ({Message}) in purge worker for '{Category}'. Retrying batch ({Retry}/{Max}) in {Backoff}ms...",
                            ex.Message, category, retryCount, maxRetries, backoffMs);
                        await Task.Delay(backoffMs, ct);
                    }
                }

                if (chunkProcessed == 0)
                {
                    break;
                }

                processed += chunkProcessed;
                totalBytesReclaimed += chunkProcessed * estimatedBytesPerRow;

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

                batchSw.Stop();
                var batchDurationMs = batchSw.ElapsedMilliseconds;

                // Adaptive Backpressure:
                // If PostgreSQL took longer than 1,000ms to commit the batch, dynamically scale
                // the rest delay to 2x the execution time. This guarantees that autovacuum, WAL writer,
                // and concurrent crawler writes are given ample time to breathe without I/O starvation.
                var effectiveDelay = delayMs;
                if (batchDurationMs > 1000)
                {
                    effectiveDelay = Math.Max(delayMs, (int)batchDurationMs * 2);
                    _logger.LogInformation("Adaptive backpressure active for '{Category}': batch took {Duration}ms; pacing delay extended to {Delay}ms.",
                        category, batchDurationMs, effectiveDelay);
                }

                if (effectiveDelay > 0)
                {
                    await Task.Delay(effectiveDelay, ct);
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
                await vacuumConn.ExecuteAsync(new CommandDefinition("VACUUM ANALYZE torrents;", commandTimeout: 600));
                await vacuumConn.ExecuteAsync(new CommandDefinition("VACUUM ANALYZE fetch_peer_outcomes;", commandTimeout: 600));
                await vacuumConn.ExecuteAsync(new CommandDefinition("VACUUM ANALYZE infohash_sightings;", commandTimeout: 600));
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

    private static bool IsTransientException(Exception ex)
    {
        if (ex is TimeoutException || ex.InnerException is TimeoutException)
        {
            return true;
        }

        if (ex is PostgresException pex)
        {
            return pex.SqlState is PostgresErrorCodes.DeadlockDetected       // 40P01
                                or PostgresErrorCodes.SerializationFailure    // 40001
                                or PostgresErrorCodes.LockNotAvailable;       // 55P03
        }

        if (ex is NpgsqlException nex)
        {
            if (nex.InnerException is PostgresException innerPex)
            {
                return innerPex.SqlState is PostgresErrorCodes.DeadlockDetected
                                         or PostgresErrorCodes.SerializationFailure
                                         or PostgresErrorCodes.LockNotAvailable;
            }

            if (nex.InnerException is System.IO.IOException || nex.InnerException is System.Net.Sockets.SocketException)
            {
                return true;
            }
        }

        return ex.Message.Contains("40P01", StringComparison.OrdinalIgnoreCase)
            || ex.Message.Contains("deadlock", StringComparison.OrdinalIgnoreCase)
            || ex.Message.Contains("lock", StringComparison.OrdinalIgnoreCase)
            || ex.Message.Contains("timeout", StringComparison.OrdinalIgnoreCase)
            || ex.Message.Contains("timed out", StringComparison.OrdinalIgnoreCase);
    }
}
