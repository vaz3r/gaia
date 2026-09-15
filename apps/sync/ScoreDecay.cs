namespace Gaia.Sync;

public static class ScoreDecay
{
    /// <summary>
    /// Computes decayed health score based on last_health_attempt without modifying PostgreSQL.
    /// - If swarm_peers == 0 and not seed_confirmed: health is 0 (incoherent swarm)
    /// - Days <= 7: 100% health
    /// - Days 7..30: Linear decay from rawHealth to 0
    /// - Days >= 30: 0
    /// </summary>
    public static short ComputeDecayedHealth(short rawHealth, DateTime? lastAttempt, int swarmPeers = -1, bool seedConfirmed = true)
    {
        if (rawHealth <= 0) return 0;
        if (swarmPeers == 0 && !seedConfirmed) return 0;
        if (!lastAttempt.HasValue) return 0;

        var days = (DateTime.UtcNow - lastAttempt.Value).TotalDays;
        if (days <= 7.0) return rawHealth;
        if (days >= 30.0) return 0;

        double ratio = 1.0 - ((days - 7.0) / 23.0);
        return (short)Math.Max(0, (int)Math.Round(rawHealth * ratio));
    }
}
