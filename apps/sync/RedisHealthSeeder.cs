using System.Data;
using Microsoft.Extensions.Logging;
using Npgsql;
using StackExchange.Redis;

namespace Gaia.Sync;

public class RedisHealthSeeder
{
    private readonly string _pgConnString;
    private readonly IConnectionMultiplexer _redis;
    private readonly ILogger<RedisHealthSeeder> _logger;

    public RedisHealthSeeder(string pgConnString, IConnectionMultiplexer redis, ILogger<RedisHealthSeeder> logger)
    {
        _pgConnString = pgConnString;
        _redis = redis;
        _logger = logger;
    }

    public async Task<int> SeedAsync(CancellationToken ct = default)
    {
        _logger.LogInformation("Starting Redis Health Seed into DB 1...");
        var redisDb1 = _redis.GetDatabase(1);

        await using var conn = new NpgsqlConnection(_pgConnString);
        await conn.OpenAsync(ct);

        var sql = @"
            SELECT encode(infohash, 'hex') AS h, 
                   health_score, popularity_score, swarm_peers, seed_confirmed,
                   EXTRACT(EPOCH FROM last_health_attempt)::bigint AS t
            FROM torrents 
            WHERE policy_action IS DISTINCT FROM 'SUPPRESS';";

        await using var cmd = new NpgsqlCommand(sql, conn) { CommandTimeout = 3600 };
        await using var reader = await cmd.ExecuteReaderAsync(CommandBehavior.SequentialAccess, ct);

        var batch = redisDb1.CreateBatch();
        var batchTasks = new List<Task>(5000);
        int totalCount = 0;
        var sw = System.Diagnostics.Stopwatch.StartNew();

        while (await reader.ReadAsync(ct))
        {
            string hash = reader.GetString(0);
            short h = reader.IsDBNull(1) ? (short)0 : reader.GetInt16(1);
            short p = reader.IsDBNull(2) ? (short)0 : reader.GetInt16(2);
            int s = reader.IsDBNull(3) ? 0 : reader.GetInt32(3);
            bool c = !reader.IsDBNull(4) && reader.GetBoolean(4);
            long? t = reader.IsDBNull(5) ? null : reader.GetInt64(5);

            var entries = t.HasValue
                ? new[] {
                    new HashEntry("h", (int)h),
                    new HashEntry("p", (int)p),
                    new HashEntry("s", s),
                    new HashEntry("c", c ? 1 : 0),
                    new HashEntry("t", t.Value)
                }
                : new[] {
                    new HashEntry("h", (int)h),
                    new HashEntry("p", (int)p),
                    new HashEntry("s", s),
                    new HashEntry("c", c ? 1 : 0)
                };

            var task = batch.HashSetAsync($"gaia:health:{hash}", entries);
            batchTasks.Add(task);
            totalCount++;

            if (totalCount % 5000 == 0)
            {
                batch.Execute();
                await Task.WhenAll(batchTasks);
                batchTasks.Clear();
                batch = redisDb1.CreateBatch();

                if (totalCount % 100000 == 0)
                {
                    var rate = totalCount / sw.Elapsed.TotalSeconds;
                    _logger.LogInformation("Seeded {Count} records into Redis DB 1 ({Rate:F0} docs/sec)...", totalCount, rate);
                }
            }
        }

        if (batchTasks.Count > 0)
        {
            batch.Execute();
            await Task.WhenAll(batchTasks);
        }

        sw.Stop();
        _logger.LogInformation("Redis Health Seed completed successfully! Seeded {Count} records in {Elapsed:F1}s.", totalCount, sw.Elapsed.TotalSeconds);
        return totalCount;
    }
}
