import React, { useState, useEffect, useCallback, useRef } from 'react';
import {
  SlidersHorizontal,
  ShieldAlert,
  ShieldCheck,
  CheckCircle2,
  AlertTriangle,
  RefreshCw,
  Trash2,
  StopCircle,
  Clock,
  HardDrive,
  Database,
  Layers,
  ArrowRight,
  Sparkles,
  Ban,
  Activity,
  Zap,
  Check,
  X
} from 'lucide-react';
import { api } from '../api.js';
import { formatBytes, formatNum } from '../utils.js';

const CATEGORY_COLORS = {
  Adult: 'bg-rose-500/10 text-rose-400 border-rose-500/30',
  Movies: 'bg-blue-500/10 text-blue-400 border-blue-500/30',
  Television: 'bg-purple-500/10 text-purple-400 border-purple-500/30',
  Music: 'bg-cyan-500/10 text-cyan-400 border-cyan-500/30',
  'Books & Learning': 'bg-teal-500/10 text-teal-400 border-teal-500/30',
  Anime: 'bg-pink-500/10 text-pink-400 border-pink-500/30',
  Games: 'bg-lime-500/10 text-lime-400 border-lime-500/30',
  Applications: 'bg-amber-500/10 text-amber-400 border-amber-500/30',
  Audiobooks: 'bg-indigo-500/10 text-indigo-400 border-indigo-500/30',
  Documentaries: 'bg-emerald-500/10 text-emerald-400 border-emerald-500/30',
  Other: 'bg-zinc-500/10 text-zinc-400 border-zinc-500/30',
};

export default function CategoryPoliciesView() {
  const [categories, setCategories] = useState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [togglingCategory, setTogglingCategory] = useState(null);

  // Purge worker state
  const [purgeStatus, setPurgeStatus] = useState(null);
  const [purgeConfirmCategory, setPurgeConfirmCategory] = useState(null);
  const [launchingPurge, setLaunchingPurge] = useState(false);
  const [cancellingPurge, setCancellingPurge] = useState(false);
  const [actionSuccess, setActionSuccess] = useState(null);

  const pollTimerRef = useRef(null);

  const fetchCategories = useCallback(async () => {
    try {
      const data = await api('/api/admin/categories');
      setCategories(data || []);
      setError(null);
    } catch (err) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }, []);

  const fetchPurgeStatus = useCallback(async () => {
    try {
      const status = await api('/api/admin/categories/purge/status');
      setPurgeStatus(status);
      if (status?.isRunning) {
        // Continue fast polling while purge is active
        if (!pollTimerRef.current) {
          pollTimerRef.current = setInterval(fetchPurgeStatus, 1000);
        }
      } else if (pollTimerRef.current) {
        clearInterval(pollTimerRef.current);
        pollTimerRef.current = null;
        // Refresh categories once finished to update counts
        fetchCategories();
      }
    } catch (err) {
      console.warn('Failed to fetch purge status:', err.message);
    }
  }, [fetchCategories]);

  useEffect(() => {
    fetchCategories();
    fetchPurgeStatus();

    // Setup SSE stream for purge progress if supported
    let eventSource = null;
    try {
      eventSource = new EventSource('/api/admin/categories/purge/stream');
      eventSource.onmessage = (event) => {
        try {
          const progress = JSON.parse(event.data);
          setPurgeStatus(progress);
          if (!progress.isRunning) {
            fetchCategories();
          }
        } catch {
          // parse error
        }
      };
    } catch (err) {
      console.warn('SSE not available, falling back to polling:', err);
    }

    return () => {
      if (eventSource) eventSource.close();
      if (pollTimerRef.current) clearInterval(pollTimerRef.current);
    };
  }, [fetchCategories, fetchPurgeStatus]);

  const handleTogglePolicy = async (cat) => {
    const newEnabled = !cat.is_enabled;
    setTogglingCategory(cat.category);
    setActionSuccess(null);

    try {
      await api(`/api/admin/categories/${encodeURIComponent(cat.category)}/policy`, {
        method: 'PUT',
        body: JSON.stringify({ isEnabled: newEnabled })
      });

      setCategories((prev) =>
        prev.map((c) => (c.category === cat.category ? { ...c, is_enabled: newEnabled } : c))
      );

      setActionSuccess(
        newEnabled
          ? `Policy for '${cat.category}' updated: Future crawling enabled.`
          : `Policy for '${cat.category}' updated: Future crawling blocked.`
      );
      setTimeout(() => setActionSuccess(null), 4000);
    } catch (err) {
      alert(`Failed to update policy: ${err.message}`);
    } finally {
      setTogglingCategory(null);
    }
  };

  const handleStartPurge = async () => {
    if (!purgeConfirmCategory) return;
    setLaunchingPurge(true);
    setActionSuccess(null);

    try {
      const res = await api(`/api/admin/categories/${encodeURIComponent(purgeConfirmCategory.category)}/purge`, {
        method: 'POST',
        body: JSON.stringify({ chunkSize: 5000, delayMs: 50 })
      });

      setPurgeStatus(res);
      setPurgeConfirmCategory(null);
      setActionSuccess(`Background purge worker launched for '${purgeConfirmCategory.category}'.`);

      if (!pollTimerRef.current) {
        pollTimerRef.current = setInterval(fetchPurgeStatus, 1000);
      }
    } catch (err) {
      alert(`Purge failed to launch: ${err.message}`);
    } finally {
      setLaunchingPurge(false);
    }
  };

  const handleCancelPurge = async () => {
    setCancellingPurge(true);
    try {
      await api('/api/admin/categories/purge/cancel', { method: 'POST' });
      setActionSuccess('Cancellation requested for active purge worker.');
      setTimeout(fetchPurgeStatus, 500);
    } catch (err) {
      alert(`Failed to cancel purge: ${err.message}`);
    } finally {
      setCancellingPurge(false);
    }
  };

  const totalTorrents = categories.reduce((sum, c) => sum + (c.torrent_count || 0), 0);
  const totalTombstones = categories.reduce((sum, c) => sum + (c.tombstone_count || 0), 0);
  const disabledCount = categories.filter((c) => !c.is_enabled).length;
  const purgeableCount = categories
    .filter((c) => !c.is_enabled)
    .reduce((sum, c) => sum + (c.torrent_count || 0), 0);

  return (
    <div className="space-y-6">
      {/* Header & Overview Card */}
      <section className="rounded-xl border border-[#222] bg-[#0a0a0a] p-5">
        <div className="flex flex-col md:flex-row md:items-center justify-between gap-4">
          <div>
            <div className="flex items-center gap-2">
              <SlidersHorizontal className="w-5 h-5 text-amber-400" />
              <h2 className="text-base font-semibold text-white tracking-tight">
                Category Governance & Storage Reclamation
              </h2>
              <span className="text-[11px] px-2 py-0.5 rounded-full bg-zinc-800 text-zinc-300 font-mono">
                {categories.length} Categories
              </span>
            </div>
            <p className="text-xs text-[#888] mt-1 max-w-3xl leading-relaxed">
              Dynamically control which categories the crawler is permitted to harvest. Disabling a category prevents
              future BEP-9 wire fetches and drops announces immediately. Purging permanently removes existing torrents
              and tombstones their infohashes so they are never crawled again.
            </p>
          </div>

          <button
            onClick={fetchCategories}
            disabled={loading}
            className="px-3 py-1.5 rounded-lg border border-[#333] bg-[#141414] text-xs font-mono text-[#aaa] hover:text-white flex items-center gap-1.5 self-start shrink-0"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} />
            <span>Refresh</span>
          </button>
        </div>

        {/* Global Stats Row */}
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 mt-5 pt-4 border-t border-[#1a1a1a]">
          <div className="bg-[#111] rounded-lg p-3 border border-[#222]">
            <div className="text-[10px] uppercase font-mono text-[#666]">Stored Torrents</div>
            <div className="text-base font-bold font-mono text-white mt-0.5">
              {formatNum(totalTorrents)}
            </div>
            <div className="text-[10px] text-[#555] font-mono mt-0.5">Across all categories</div>
          </div>

          <div className="bg-[#111] rounded-lg p-3 border border-[#222]">
            <div className="text-[10px] uppercase font-mono text-[#666]">Tombstoned Hashes</div>
            <div className="text-base font-bold font-mono text-emerald-400 mt-0.5">
              {formatNum(totalTombstones)}
            </div>
            <div className="text-[10px] text-[#555] font-mono mt-0.5">Permanently blocked</div>
          </div>

          <div className="bg-[#111] rounded-lg p-3 border border-[#222]">
            <div className="text-[10px] uppercase font-mono text-[#666]">Disabled Policies</div>
            <div className="text-base font-bold font-mono text-amber-400 mt-0.5">
              {disabledCount} <span className="text-xs font-normal text-[#666]">/ {categories.length}</span>
            </div>
            <div className="text-[10px] text-[#555] font-mono mt-0.5">Future crawl blocked</div>
          </div>

          <div className="bg-[#111] rounded-lg p-3 border border-[#222]">
            <div className="text-[10px] uppercase font-mono text-[#666]">Purgeable in DB</div>
            <div className="text-base font-bold font-mono text-rose-400 mt-0.5">
              {formatNum(purgeableCount)}
            </div>
            <div className="text-[10px] text-[#555] font-mono mt-0.5">
              ~{formatBytes(purgeableCount * 28000)} reclaimable
            </div>
          </div>
        </div>
      </section>

      {/* Action Success Alert */}
      {actionSuccess && (
        <div className="rounded-lg bg-emerald-950/40 border border-emerald-800/60 p-3 text-xs text-emerald-300 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <CheckCircle2 className="w-4 h-4 text-emerald-400 shrink-0" />
            <span>{actionSuccess}</span>
          </div>
          <button onClick={() => setActionSuccess(null)} className="text-[#888] hover:text-white">
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      )}

      {/* Active Purge Banner */}
      {purgeStatus?.isRunning && (
        <section className="rounded-xl border border-amber-600/50 bg-amber-950/20 p-5 space-y-3">
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
            <div className="flex items-center gap-2.5">
              <Activity className="w-5 h-5 text-amber-400 animate-pulse" />
              <div>
                <div className="text-sm font-semibold text-white flex items-center gap-2">
                  <span>Purging category:</span>
                  <span className="px-2 py-0.5 rounded bg-amber-500/20 text-amber-300 font-mono text-xs">
                    {purgeStatus.category}
                  </span>
                </div>
                <div className="text-xs text-[#aaa] font-mono mt-0.5">
                  C# Background Worker executing 5,000/batch with 50ms lock pacing
                </div>
              </div>
            </div>

            <button
              onClick={handleCancelPurge}
              disabled={cancellingPurge}
              className="px-3 py-1.5 rounded-lg border border-rose-800 bg-rose-950/80 text-rose-300 hover:bg-rose-900 text-xs font-mono font-medium flex items-center gap-1.5 self-start shrink-0"
            >
              <StopCircle className="w-4 h-4 text-rose-400" />
              <span>{cancellingPurge ? 'Cancelling...' : 'Cancel Purge'}</span>
            </button>
          </div>

          {/* Progress Bar */}
          <div className="w-full bg-zinc-900 h-2.5 rounded-full overflow-hidden border border-zinc-800">
            <div
              className="bg-amber-500 h-full transition-all duration-300 rounded-full"
              style={{
                width: `${Math.min(100, Math.round((purgeStatus.processed / (purgeStatus.totalTarget || 1)) * 100))}%`
              }}
            />
          </div>

          {/* Progress Metrics Grid */}
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 text-xs font-mono pt-1">
            <div className="text-[#aaa]">
              <span className="text-[#666] block text-[10px] uppercase font-sans">Progress</span>
              <span className="text-white font-bold">
                {formatNum(purgeStatus.processed)}
              </span>{' '}
              / {formatNum(purgeStatus.totalTarget)} (
              {Math.round((purgeStatus.processed / (purgeStatus.totalTarget || 1)) * 100)}%)
            </div>

            <div className="text-[#aaa]">
              <span className="text-[#666] block text-[10px] uppercase font-sans">Velocity</span>
              <span className="text-emerald-400 font-bold">
                {formatNum(purgeStatus.itemsPerSecond)}
              </span>{' '}
              items/sec
            </div>

            <div className="text-[#aaa]">
              <span className="text-[#666] block text-[10px] uppercase font-sans">Storage Reclaimed</span>
              <span className="text-amber-400 font-bold">
                ~{formatBytes(purgeStatus.estimatedBytesReclaimed)}
              </span>
            </div>

            <div className="text-[#aaa]">
              <span className="text-[#666] block text-[10px] uppercase font-sans">Est. Remaining</span>
              <span className="text-[#ededed]">
                {purgeStatus.etaSeconds != null ? `${purgeStatus.etaSeconds}s` : 'Calculating...'}
              </span>
            </div>
          </div>
        </section>
      )}

      {/* Categories Table */}
      <section className="rounded-xl border border-[#222] bg-[#090909] overflow-hidden">
        <div className="p-4 border-b border-[#1c1c1c] flex items-center justify-between">
          <div className="text-xs font-semibold text-white font-mono uppercase tracking-wider">
            Category Policies & Storage
          </div>
          <div className="text-[11px] text-[#666] font-mono">
            Toggle switch controls future crawler ingestion
          </div>
        </div>

        <div className="overflow-x-auto">
          <table className="w-full text-left border-collapse">
            <thead>
              <tr className="border-b border-[#1a1a1a] bg-[#0d0d0d] text-[10px] font-mono uppercase text-[#666] tracking-wider">
                <th className="py-3 px-4">Category</th>
                <th className="py-3 px-4">Description</th>
                <th className="py-3 px-4">In Database</th>
                <th className="py-3 px-4">Tombstones</th>
                <th className="py-3 px-4">Future Crawl Policy</th>
                <th className="py-3 px-4 text-right">Data Purge Action</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[#151515] text-xs font-mono">
              {categories.map((cat) => {
                const badgeColor = CATEGORY_COLORS[cat.category] || 'bg-zinc-800 text-zinc-300 border-zinc-700';
                const isToggling = togglingCategory === cat.category;
                const isAdult = cat.category === 'Adult';
                const hasTorrents = (cat.torrent_count || 0) > 0;

                return (
                  <tr key={cat.category} className="hover:bg-[#111]/60 transition-colors">
                    {/* Category Name */}
                    <td className="py-3 px-4">
                      <span className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs font-medium border ${badgeColor}`}>
                        <span>{cat.category}</span>
                      </span>
                    </td>

                    {/* Description */}
                    <td className="py-3 px-4 text-[#888] font-sans max-w-xs text-xs truncate">
                      {cat.description || '—'}
                    </td>

                    {/* Torrents Count */}
                    <td className="py-3 px-4">
                      <div className="text-white font-medium">
                        {formatNum(cat.torrent_count || 0)}
                      </div>
                      <div className="text-[10px] text-[#555]">
                        ~{formatBytes((cat.torrent_count || 0) * 28000)}
                      </div>
                    </td>

                    {/* Tombstones Count */}
                    <td className="py-3 px-4 text-[#aaa]">
                      {formatNum(cat.tombstone_count || 0)}
                    </td>

                    {/* Toggle Switch */}
                    <td className="py-3 px-4">
                      <button
                        onClick={() => handleTogglePolicy(cat)}
                        disabled={isToggling || purgeStatus?.isRunning}
                        className={`inline-flex items-center gap-2 px-3 py-1 rounded-full text-xs font-medium transition-all ${
                          cat.is_enabled
                            ? 'bg-emerald-950/60 text-emerald-400 border border-emerald-800/60 hover:bg-emerald-900/60'
                            : 'bg-rose-950/60 text-rose-400 border border-rose-800/60 hover:bg-rose-900/60'
                        } ${isToggling ? 'opacity-50 cursor-not-allowed' : ''}`}
                      >
                        <span
                          className={`w-2 h-2 rounded-full ${
                            cat.is_enabled ? 'bg-emerald-400' : 'bg-rose-400'
                          }`}
                        />
                        <span>{cat.is_enabled ? 'CRAWL ALLOWED' : 'CRAWL BLOCKED'}</span>
                      </button>
                    </td>

                    {/* Purge Button */}
                    <td className="py-3 px-4 text-right">
                      {cat.is_enabled ? (
                        <span className="text-[11px] text-[#555] font-sans italic">
                          Disable policy to purge
                        </span>
                      ) : hasTorrents ? (
                        <button
                          onClick={() => setPurgeConfirmCategory(cat)}
                          disabled={purgeStatus?.isRunning}
                          className="px-3 py-1.5 rounded-lg border border-rose-800/80 bg-rose-950/70 text-rose-300 hover:bg-rose-900 text-xs font-mono font-medium flex items-center gap-1.5 ml-auto transition-colors"
                        >
                          <Trash2 className="w-3.5 h-3.5 text-rose-400" />
                          <span>Purge {formatNum(cat.torrent_count)} Torrents</span>
                        </button>
                      ) : (
                        <span className="inline-flex items-center gap-1 text-[11px] text-emerald-500 font-mono">
                          <Check className="w-3.5 h-3.5" />
                          <span>Clean (0 in DB)</span>
                        </span>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </section>

      {/* Confirmation Modal */}
      {purgeConfirmCategory && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/80 backdrop-blur-sm animate-in fade-in duration-200">
          <div className="w-full max-w-md bg-[#111] border border-[#333] rounded-xl p-6 shadow-2xl space-y-4">
            <div className="flex items-center gap-3 text-rose-400">
              <div className="w-9 h-9 rounded-lg bg-rose-950/60 border border-rose-800/60 flex items-center justify-center">
                <AlertTriangle className="w-5 h-5 text-rose-400" />
              </div>
              <div>
                <h3 className="text-base font-semibold text-white">
                  Purge {purgeConfirmCategory.category} Data?
                </h3>
                <p className="text-xs text-[#888]">Non-locking C# background worker</p>
              </div>
            </div>

            <p className="text-xs text-[#ccc] leading-relaxed">
              This action will permanently delete{' '}
              <strong className="text-white font-mono">
                {formatNum(purgeConfirmCategory.torrent_count)}
              </strong>{' '}
              records from PostgreSQL, cascade cleanup across peer probe logs, and record all infohashes into the
              permanent anti-recrawl tombstone table.
            </p>

            <div className="bg-[#181818] rounded-lg p-3 border border-[#262626] text-xs font-mono space-y-1">
              <div className="flex justify-between text-[#888]">
                <span>Category:</span>
                <span className="text-white">{purgeConfirmCategory.category}</span>
              </div>
              <div className="flex justify-between text-[#888]">
                <span>Torrents to delete:</span>
                <span className="text-white">{formatNum(purgeConfirmCategory.torrent_count)}</span>
              </div>
              <div className="flex justify-between text-[#888]">
                <span>Estimated space freed:</span>
                <span className="text-emerald-400">
                  ~{formatBytes(purgeConfirmCategory.torrent_count * 28000)}
                </span>
              </div>
              <div className="flex justify-between text-[#888]">
                <span>Batch execution:</span>
                <span className="text-[#aaa]">5,000 / batch (50ms yield)</span>
              </div>
            </div>

            <div className="flex items-center justify-end gap-3 pt-2">
              <button
                onClick={() => setPurgeConfirmCategory(null)}
                disabled={launchingPurge}
                className="px-4 py-2 rounded-lg border border-[#333] bg-[#1a1a1a] text-xs font-medium text-[#ccc] hover:text-white"
              >
                Cancel
              </button>
              <button
                onClick={handleStartPurge}
                disabled={launchingPurge}
                className="px-4 py-2 rounded-lg bg-rose-600 hover:bg-rose-500 text-xs font-medium text-white flex items-center gap-2"
              >
                <Trash2 className="w-3.5 h-3.5" />
                <span>{launchingPurge ? 'Starting Worker...' : 'Confirm & Launch Purge'}</span>
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
