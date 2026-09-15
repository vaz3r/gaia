using Dapper;
using Npgsql;

namespace Gaia.Sync;

public record LoopState(string LoopName, DateTime? CursorTs, string CursorHash, bool Completed, long RowsSynced);

public class SyncStateRepository
{
    private readonly string _connectionString;

    public SyncStateRepository(string connectionString)
    {
        _connectionString = connectionString;
    }

    public async Task<LoopState> GetStateAsync(string loopName)
    {
        await using var conn = new NpgsqlConnection(_connectionString);
        await conn.OpenAsync();

        var row = await conn.QuerySingleOrDefaultAsync<dynamic>(
            "SELECT loop_name, cursor_ts, cursor_hash, completed, rows_synced FROM portal_sync_state WHERE loop_name = @loopName",
            new { loopName });

        if (row == null)
        {
            return new LoopState(loopName, null, string.Empty, false, 0);
        }

        DateTime? cursorTs = row.cursor_ts != null ? (DateTime?)row.cursor_ts : null;
        string cursorHash = row.cursor_hash ?? string.Empty;
        bool completed = row.completed ?? false;
        long rowsSynced = row.rows_synced != null ? (long)row.rows_synced : 0;

        return new LoopState(loopName, cursorTs, cursorHash, completed, rowsSynced);
    }

    public async Task UpdateProgressAsync(string loopName, DateTime cursorTs, string cursorHash, int batchCount)
    {
        await using var conn = new NpgsqlConnection(_connectionString);
        await conn.OpenAsync();

        await conn.ExecuteAsync(@"
            UPDATE portal_sync_state
            SET cursor_ts = @cursorTs,
                cursor_hash = @cursorHash,
                rows_synced = rows_synced + @batchCount,
                last_run_at = now(),
                last_error = NULL
            WHERE loop_name = @loopName",
            new { loopName, cursorTs, cursorHash, batchCount });
    }

    public async Task MarkCompletedAsync(string loopName)
    {
        await using var conn = new NpgsqlConnection(_connectionString);
        await conn.OpenAsync();

        await conn.ExecuteAsync(@"
            UPDATE portal_sync_state
            SET completed = true,
                last_run_at = now()
            WHERE loop_name = @loopName",
            new { loopName });
    }

    public async Task RecordErrorAsync(string loopName, string error)
    {
        try
        {
            await using var conn = new NpgsqlConnection(_connectionString);
            await conn.OpenAsync();

            await conn.ExecuteAsync(@"
                UPDATE portal_sync_state
                SET last_error = @error,
                    last_run_at = now()
                WHERE loop_name = @loopName",
                new { loopName, error });
        }
        catch
        {
            // Suppress secondary failures during error logging
        }
    }

    public async Task TouchLoopAsync(string loopName)
    {
        try
        {
            await using var conn = new NpgsqlConnection(_connectionString);
            await conn.OpenAsync();

            await conn.ExecuteAsync(@"
                UPDATE portal_sync_state
                SET last_run_at = now()
                WHERE loop_name = @loopName",
                new { loopName });
        }
        catch
        {
            // Suppress secondary failures
        }
    }
}
