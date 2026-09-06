import React, { useState, useEffect, useCallback } from 'react';
import {
  Tag,
  CheckCircle2,
  AlertTriangle,
  RotateCcw,
  RefreshCw,
  Search,
  Layers,
  ChevronRight,
  Database,
  Cpu,
  Sliders,
  Check,
  X,
  Sparkles,
  ExternalLink,
  Flame,
  FileCode,
  HardDrive,
  Clock,
  ArrowRight,
  Activity,
  AlertCircle
} from 'lucide-react';
import { api, magnetFrom } from '../api.js';
import { formatBytes, formatNum, formatTime } from '../utils.js';

const CATEGORY_COLORS = {
  Movies: 'bg-blue-500/10 text-blue-400 border-blue-500/30',
  TV: 'bg-purple-500/10 text-purple-400 border-purple-500/30',
  Anime: 'bg-pink-500/10 text-pink-400 border-pink-500/30',
  Games: 'bg-emerald-500/10 text-emerald-400 border-emerald-500/30',
  Software: 'bg-amber-500/10 text-amber-400 border-amber-500/30',
  Music: 'bg-cyan-500/10 text-cyan-400 border-cyan-500/30',
  Books: 'bg-teal-500/10 text-teal-400 border-teal-500/30',
  Adult: 'bg-rose-500/10 text-rose-400 border-rose-500/30',
  Other: 'bg-zinc-500/10 text-zinc-400 border-zinc-500/30',
};

const ALL_CATEGORIES = [
  'Movies',
  'TV',
  'Anime',
  'Games',
  'Software',
  'Music',
  'Books',
  'Adult',
  'Other'
];

export default function ClassifierView({ onInspectTorrent, copyToClipboard }) {
  // Metrics & Status
  const [metrics, setMetrics] = useState(null);
  const [status, setStatus] = useState(null);
  const [models, setModels] = useState(null);

  // Queue state
  const [queueTab, setQueueTab] = useState('review'); // 'review' | 'all'
  const [torrents, setTorrents] = useState([]);
  const [totalTorrents, setTotalTorrents] = useState(0);
  const [loading, setLoading] = useState(true);
  const [searchQuery, setSearchQuery] = useState('');
  const [searchInput, setSearchInput] = useState('');
  const [page, setPage] = useState(1);
  const limit = 20;

  // Selected item & Live Classification Explainability
  const [selectedTorrent, setSelectedTorrent] = useState(null);
  const [explainData, setExplainData] = useState(null);
  const [explainLoading, setExplainLoading] = useState(false);

  // Human Relabeling Action State
  const [selectedCategory, setSelectedCategory] = useState('');
  const [submittingLabel, setSubmittingLabel] = useState(false);
  const [actionSuccess, setActionSuccess] = useState(null);

  // Retrain / Model Modal
  const [showModelModal, setShowModelModal] = useState(false);
  const [retrainStatus, setRetrainStatus] = useState(null);
  const [retrainingTriggered, setRetrainingTriggered] = useState(false);

  // Fetch telemetry & status
  const fetchStatusAndMetrics = useCallback(async () => {
    try {
      const [m, s] = await Promise.all([
        api('/api/classifier/metrics'),
        api('/api/classifier/status')
      ]);
      setMetrics(m);
      setStatus(s);
    } catch (err) {
      console.warn('Failed to load classifier metrics/status:', err.message);
    }
  }, []);

  // Fetch queue items
  const fetchQueueTorrents = useCallback(async () => {
    setLoading(true);
    try {
      const offset = (page - 1) * limit;
      const params = new URLSearchParams({
        offset: String(offset),
        limit: String(limit)
      });
      if (queueTab === 'review') {
        params.set('needs_review', 'true');
      }
      if (searchQuery.trim()) {
        params.set('search', searchQuery.trim());
      }

      const res = await api(`/api/classifier/torrents?${params.toString()}`);
      setTorrents(res.items || []);
      setTotalTorrents(res.total || 0);

      // Auto-select first item if none selected or not in current items
      if (res.items && res.items.length > 0) {
        setSelectedTorrent((prev) => {
          if (!prev) return res.items[0];
          const exists = res.items.some((it) => it.infohash === prev.infohash);
          return exists ? prev : res.items[0];
        });
      } else {
        setSelectedTorrent(null);
      }
    } catch (err) {
      console.error('Failed to load classifier torrents:', err);
    } finally {
      setLoading(false);
    }
  }, [page, limit, queueTab, searchQuery]);

  // Initial load & polling
  useEffect(() => {
    fetchStatusAndMetrics();
    const interval = setInterval(fetchStatusAndMetrics, 15000);
    return () => clearInterval(interval);
  }, [fetchStatusAndMetrics]);

  useEffect(() => {
    fetchQueueTorrents();
  }, [fetchQueueTorrents]);

  // Fetch explainability / probabilities when selectedTorrent changes
  useEffect(() => {
    if (!selectedTorrent) {
      setExplainData(null);
      return;
    }

    let isMounted = true;
    const fetchExplain = async () => {
      setExplainLoading(true);
      try {
        const payload = {
          infohash: selectedTorrent.infohash,
          name: selectedTorrent.name,
          total_size: selectedTorrent.total_size,
          file_count: selectedTorrent.file_count,
          files: selectedTorrent.files || []
        };
        const res = await fetch('/api/classifier/classify', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload)
        });
        if (res.ok && isMounted) {
          const json = await res.json();
          setExplainData(json.classification);
          setSelectedCategory(json.classification?.predicted_category || selectedTorrent.category || 'Movies');
        }
      } catch (err) {
        console.warn('Explainability fetch failed:', err);
      } finally {
        if (isMounted) setExplainLoading(false);
      }
    };

    fetchExplain();
    return () => {
      isMounted = false;
    };
  }, [selectedTorrent]);

  // Handle Search submit
  const handleSearchSubmit = (e) => {
    e.preventDefault();
    setSearchQuery(searchInput);
    setPage(1);
  };

  const handleClearSearch = () => {
    setSearchInput('');
    setSearchQuery('');
    setPage(1);
  };

  // Submit human verified label / correction
  const handleApplyLabel = async (targetCategory, customReason = null) => {
    if (!selectedTorrent || !targetCategory) return;
    setSubmittingLabel(true);
    setActionSuccess(null);

    try {
      const payload = {
        infohash: selectedTorrent.infohash,
        category: targetCategory,
        reason: customReason || `Human verified via GAIA Dashboard`
      };

      const res = await fetch('/api/classifier/labels', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload)
      });

      if (!res.ok) {
        const errJson = await res.json();
        throw new Error(errJson.detail || 'Failed to submit label');
      }

      setActionSuccess(`Saved: Classified as "${targetCategory}" & removed from review queue.`);
      fetchStatusAndMetrics();

      // Refresh list or remove item locally from review list
      setTorrents((prev) => prev.filter((t) => t.infohash !== selectedTorrent.infohash));
      setTotalTorrents((prev) => Math.max(0, prev - 1));

      setTimeout(() => setActionSuccess(null), 4000);
    } catch (err) {
      alert(`Error saving label: ${err.message}`);
    } finally {
      setSubmittingLabel(false);
    }
  };

  // Model Management Handlers
  const openModelManager = async () => {
    setShowModelModal(true);
    try {
      const [modRes, retRes] = await Promise.all([
        api('/api/classifier/models'),
        api('/api/classifier/retrain/status')
      ]);
      setModels(modRes);
      setRetrainStatus(retRes);
    } catch (e) {
      console.warn('Failed loading models info:', e);
    }
  };

  const triggerRetraining = async () => {
    setRetrainingTriggered(true);
    try {
      const res = await fetch('/api/classifier/retrain', { method: 'POST' });
      const json = await res.json();
      if (!res.ok) {
        throw new Error(json.detail || 'Retraining start failed');
      }
      alert('Direct DB Retraining pipeline started in background! Tracking progress...');
      const pollTimer = setInterval(async () => {
        const st = await api('/api/classifier/retrain/status');
        setRetrainStatus(st);
        if (!st.is_training) {
          clearInterval(pollTimer);
          fetchStatusAndMetrics();
        }
      }, 4000);
    } catch (err) {
      alert(`Failed to trigger retrain: ${err.message}`);
    } finally {
      setRetrainingTriggered(false);
    }
  };

  const handleRollback = async (version) => {
    if (!confirm(`Are you sure you want to rollback active model to version: ${version}?`)) return;
    try {
      const res = await fetch('/api/classifier/models/rollback', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ version })
      });
      if (!res.ok) {
        const j = await res.json();
        throw new Error(j.detail || 'Rollback failed');
      }
      alert(`Successfully rolled back to version ${version}`);
      openModelManager();
      fetchStatusAndMetrics();
    } catch (err) {
      alert(`Rollback failed: ${err.message}`);
    }
  };

  const totalPages = Math.max(1, Math.ceil(totalTorrents / limit));

  return (
    <div className="space-y-6">
      {/* Top Metrics Row */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
        {/* Review Queue Depth */}
        <div className="rounded-xl border border-[#222] bg-[#0c0c0c] p-4 flex items-center justify-between">
          <div>
            <div className="text-[11px] text-[#888] uppercase tracking-wider font-semibold">
              Review Queue Depth
            </div>
            <div className="text-2xl font-bold font-mono text-white mt-1 flex items-center gap-2">
              <span>{metrics?.review_queue_depth != null ? metrics.review_queue_depth.toLocaleString() : '—'}</span>
              {(metrics?.review_queue_depth || 0) > 0 && (
                <span className="text-xs px-2 py-0.5 rounded-full bg-amber-500/20 text-amber-400 border border-amber-500/30">
                  Needs Attention
                </span>
              )}
            </div>
            <div className="text-xs text-[#666] mt-1">Ambiguous or low confidence</div>
          </div>
          <div className="w-10 h-10 rounded-lg bg-amber-500/10 border border-amber-500/20 flex items-center justify-center text-amber-400">
            <AlertTriangle className="w-5 h-5" />
          </div>
        </div>

        {/* Unclassified Backlog */}
        <div className="rounded-xl border border-[#222] bg-[#0c0c0c] p-4 flex items-center justify-between">
          <div>
            <div className="text-[11px] text-[#888] uppercase tracking-wider font-semibold">
              Unclassified Backlog
            </div>
            <div className="text-2xl font-bold font-mono text-white mt-1">
              {metrics?.unclassified_torrents != null ? metrics.unclassified_torrents.toLocaleString() : '—'}
            </div>
            <div className="text-xs text-emerald-400 mt-1 flex items-center gap-1">
              <Activity className="w-3 h-3" /> Continuous worker ingesting
            </div>
          </div>
          <div className="w-10 h-10 rounded-lg bg-blue-500/10 border border-blue-500/20 flex items-center justify-center text-blue-400">
            <Database className="w-5 h-5" />
          </div>
        </div>

        {/* Total Labeled Ground Truth */}
        <div className="rounded-xl border border-[#222] bg-[#0c0c0c] p-4 flex items-center justify-between">
          <div>
            <div className="text-[11px] text-[#888] uppercase tracking-wider font-semibold">
              Training Corpus Ground Truth
            </div>
            <div className="text-2xl font-bold font-mono text-white mt-1">
              {metrics?.total_labeled_results != null ? metrics.total_labeled_results.toLocaleString() : '—'}
            </div>
            <div className="text-xs text-[#666] mt-1">Direct DB training samples</div>
          </div>
          <div className="w-10 h-10 rounded-lg bg-emerald-500/10 border border-emerald-500/20 flex items-center justify-center text-emerald-400">
            <CheckCircle2 className="w-5 h-5" />
          </div>
        </div>

        {/* Model Version & Actions */}
        <div className="rounded-xl border border-[#222] bg-[#0c0c0c] p-4 flex items-center justify-between">
          <div>
            <div className="text-[11px] text-[#888] uppercase tracking-wider font-semibold">
              Active Model Architecture
            </div>
            <div className="text-lg font-bold font-mono text-white mt-1 flex items-center gap-2">
              <span className="text-cyan-400">{status?.model_version || 'LightGBM v2'}</span>
            </div>
            <button
              onClick={openModelManager}
              className="text-xs text-[#aaa] hover:text-white mt-1 flex items-center gap-1 underline underline-offset-2 transition-colors"
            >
              <Cpu className="w-3 h-3 text-cyan-400" /> Manage Models & Retraining
            </button>
          </div>
          <div className="w-10 h-10 rounded-lg bg-cyan-500/10 border border-cyan-500/20 flex items-center justify-center text-cyan-400">
            <Sliders className="w-5 h-5" />
          </div>
        </div>
      </div>

      {/* Main Split Layout: Queue on Left, Inspector & Active Learning on Right */}
      <div className="grid grid-cols-1 lg:grid-cols-12 gap-6 min-h-[600px]">
        {/* Left Column: Torrent Queue (5 cols) */}
        <div className="lg:col-span-5 flex flex-col rounded-xl border border-[#1e1e1e] bg-[#090909] overflow-hidden">
          {/* Header Controls */}
          <div className="p-3 border-b border-[#181818] bg-[#0d0d0d] space-y-2.5">
            <div className="flex items-center justify-between">
              {/* Tabs: Needs Review vs All */}
              <div className="flex items-center p-0.5 rounded-lg bg-[#141414] border border-[#222]">
                <button
                  onClick={() => {
                    setQueueTab('review');
                    setPage(1);
                  }}
                  className={`px-3 py-1 text-xs rounded-md font-medium transition-colors flex items-center gap-1.5 ${
                    queueTab === 'review'
                      ? 'bg-amber-500/20 text-amber-300 border border-amber-500/30'
                      : 'text-[#888] hover:text-white'
                  }`}
                >
                  <AlertTriangle className="w-3 h-3" />
                  <span>Review Queue</span>
                  {(metrics?.review_queue_depth || 0) > 0 && (
                    <span className="text-[10px] px-1.5 py-0.2 rounded-full bg-amber-500/40 text-amber-200">
                      {metrics.review_queue_depth.toLocaleString()}
                    </span>
                  )}
                </button>
                <button
                  onClick={() => {
                    setQueueTab('all');
                    setPage(1);
                  }}
                  className={`px-3 py-1 text-xs rounded-md font-medium transition-colors ${
                    queueTab === 'all'
                      ? 'bg-[#222] text-white'
                      : 'text-[#888] hover:text-white'
                  }`}
                >
                  All Classified
                </button>
              </div>

              <span className="text-xs font-mono text-[#666]">
                {totalTorrents.toLocaleString()} torrents
              </span>
            </div>

            {/* Search Box */}
            <form onSubmit={handleSearchSubmit} className="relative">
              <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-[#555]" />
              <input
                type="text"
                placeholder="Search by title or hash in queue..."
                value={searchInput}
                onChange={(e) => setSearchInput(e.target.value)}
                className="w-full bg-[#050505] border border-[#222] rounded-lg pl-8 pr-7 py-1.5 text-xs text-[#ededed] placeholder-[#555] focus:outline-none focus:border-[#444] font-mono"
              />
              {searchInput && (
                <button
                  type="button"
                  onClick={handleClearSearch}
                  className="absolute right-2.5 top-1/2 -translate-y-1/2 text-[#555] hover:text-[#bbb]"
                >
                  <X className="w-3 h-3" />
                </button>
              )}
            </form>
          </div>

          {/* Torrents List */}
          <div className="flex-1 overflow-y-auto divide-y divide-[#141414] max-h-[680px]">
            {loading ? (
              <div className="p-12 text-center text-[#666] flex flex-col items-center justify-center gap-2">
                <RefreshCw className="w-5 h-5 animate-spin text-amber-400" />
                <span className="text-xs font-mono">Loading classifier queue...</span>
              </div>
            ) : torrents.length === 0 ? (
              <div className="p-12 text-center text-[#666] space-y-2">
                <CheckCircle2 className="w-8 h-8 text-emerald-500 mx-auto opacity-70" />
                <div className="text-sm font-semibold text-white">Review Queue Clean!</div>
                <p className="text-xs text-[#777]">
                  {queueTab === 'review'
                    ? 'No torrents currently flag needs_review=true.'
                    : 'No torrents match the specified search.'}
                </p>
              </div>
            ) : (
              torrents.map((t) => {
                const isSelected = selectedTorrent?.infohash === t.infohash;
                const cat = t.category || 'Other';
                const colorClass = CATEGORY_COLORS[cat] || CATEGORY_COLORS.Other;
                const confPct = t.category_confidence ? Math.round(t.category_confidence * 100) : null;

                return (
                  <div
                    key={t.infohash}
                    onClick={() => setSelectedTorrent(t)}
                    className={`p-3 cursor-pointer transition-colors ${
                      isSelected
                        ? 'bg-[#181818] border-l-2 border-l-amber-400'
                        : 'hover:bg-[#111]'
                    }`}
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="font-medium text-xs text-[#ededed] line-clamp-2 leading-relaxed">
                        {t.name || 'Unnamed Infohash'}
                      </div>
                      <ChevronRight
                        className={`w-3.5 h-3.5 shrink-0 mt-0.5 ${
                          isSelected ? 'text-amber-400' : 'text-[#444]'
                        }`}
                      />
                    </div>

                    <div className="flex items-center gap-2 mt-2 font-mono text-[10px]">
                      <span className={`px-2 py-0.5 rounded border text-[10px] font-semibold ${colorClass}`}>
                        {cat}
                      </span>
                      {confPct !== null && (
                        <span className="text-[#888]">
                          {confPct}% conf
                        </span>
                      )}
                      <span className="text-[#555]">·</span>
                      <span className="text-[#777]">
                        {formatBytes(t.total_size)}
                      </span>
                      <span className="text-[#555]">·</span>
                      <span className="text-[#666] truncate max-w-[80px]">
                        {t.infohash.slice(0, 8)}...
                      </span>
                    </div>
                  </div>
                );
              })
            )}
          </div>

          {/* Pagination Footer */}
          <div className="p-3 border-t border-[#181818] bg-[#0c0c0c] flex items-center justify-between text-xs font-mono text-[#777]">
            <span>
              Page {page} of {totalPages}
            </span>
            <div className="flex items-center gap-1.5">
              <button
                disabled={page <= 1 || loading}
                onClick={() => setPage((p) => Math.max(1, p - 1))}
                className="px-2.5 py-1 rounded bg-[#141414] border border-[#222] text-[#888] hover:text-white disabled:opacity-30 disabled:pointer-events-none"
              >
                Prev
              </button>
              <button
                disabled={page >= totalPages || loading}
                onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                className="px-2.5 py-1 rounded bg-[#141414] border border-[#222] text-[#888] hover:text-white disabled:opacity-30 disabled:pointer-events-none"
              >
                Next
              </button>
            </div>
          </div>
        </div>

        {/* Right Column: Deep Inspector & Human Relabeling Action Bar (7 cols) */}
        <div className="lg:col-span-7 flex flex-col rounded-xl border border-[#1e1e1e] bg-[#090909] overflow-hidden">
          {selectedTorrent ? (
            <div className="flex-1 flex flex-col overflow-y-auto max-h-[800px]">
              {/* Header */}
              <div className="p-5 border-b border-[#181818] bg-[#0c0c0c] space-y-3">
                <div className="flex items-start justify-between gap-4">
                  <div className="space-y-1">
                    <h2 className="text-base font-semibold text-white leading-snug">
                      {selectedTorrent.name || 'Unnamed Torrent'}
                    </h2>
                    <div className="flex items-center gap-2 text-xs font-mono text-[#777]">
                      <span>Hash: {selectedTorrent.infohash}</span>
                      <button
                        onClick={() => copyToClipboard(selectedTorrent.infohash, 'infohash')}
                        className="text-[#999] hover:text-white"
                        title="Copy infohash"
                      >
                        [copy]
                      </button>
                    </div>
                  </div>

                  <button
                    onClick={() => onInspectTorrent && onInspectTorrent(selectedTorrent)}
                    className="px-3 py-1.5 rounded-lg bg-[#181818] border border-[#282828] text-xs text-[#aaa] hover:text-white flex items-center gap-1 shrink-0"
                  >
                    <span>Open in Browser</span>
                    <ExternalLink className="w-3 h-3" />
                  </button>
                </div>

                {/* Metadata Pills */}
                <div className="flex flex-wrap items-center gap-3 pt-1 text-xs font-mono">
                  <div className="bg-[#141414] border border-[#222] px-2.5 py-1 rounded text-[#bbb]">
                    Size: <span className="text-white font-bold">{formatBytes(selectedTorrent.total_size)}</span>
                  </div>
                  <div className="bg-[#141414] border border-[#222] px-2.5 py-1 rounded text-[#bbb]">
                    Files: <span className="text-white font-bold">{selectedTorrent.file_count || 1}</span>
                  </div>
                  <div className="bg-[#141414] border border-[#222] px-2.5 py-1 rounded text-[#bbb]">
                    Swarm: <span className="text-emerald-400 font-bold">{selectedTorrent.swarm_peers || 0} peers</span>
                  </div>
                  {selectedTorrent.needs_review && (
                    <span className="px-2.5 py-1 rounded border border-amber-500/40 bg-amber-500/10 text-amber-300 font-medium">
                      Needs Review
                    </span>
                  )}
                </div>
              </div>

              {/* Action Success Alert */}
              {actionSuccess && (
                <div className="m-4 p-3 rounded-lg border border-emerald-800/40 bg-emerald-950/20 text-emerald-400 text-xs font-mono flex items-center gap-2">
                  <CheckCircle2 className="w-4 h-4 text-emerald-400 shrink-0" />
                  <span>{actionSuccess}</span>
                </div>
              )}

              {/* Main Inspector Body */}
              <div className="p-5 space-y-6 flex-1">
                {/* 1. Interactive Human Relabeling Bar (Active Learning Feedback Loop) */}
                <div className="rounded-xl border border-[#262626] bg-[#0f0f0f] p-4 space-y-3">
                  <div className="flex items-center justify-between">
                    <span className="text-xs font-bold text-white uppercase tracking-wider flex items-center gap-1.5">
                      <Sparkles className="w-3.5 h-3.5 text-amber-400" />
                      Active Learning Labeling Console
                    </span>
                    <span className="text-[11px] text-[#666] font-mono">Anti-bias verified</span>
                  </div>

                  <p className="text-xs text-[#888]">
                    Verify the predicted classification or assign a correction. Updates PostgreSQL <code className="text-white font-mono">torrents</code> and saves to ground-truth training set.
                  </p>

                  <div className="flex flex-wrap items-center gap-2.5 pt-2">
                    {/* Quick Confirm button for predicted category */}
                    {explainData?.predicted_category && (
                      <button
                        disabled={submittingLabel}
                        onClick={() => handleApplyLabel(explainData.predicted_category, 'One-click confirmation')}
                        className="px-4 py-2 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white font-semibold text-xs transition-colors flex items-center gap-1.5 disabled:opacity-50"
                      >
                        <Check className="w-3.5 h-3.5" />
                        <span>Confirm: {explainData.predicted_category}</span>
                      </button>
                    )}

                    {/* Category Dropdown Selector */}
                    <div className="flex items-center gap-1.5">
                      <select
                        value={selectedCategory}
                        onChange={(e) => setSelectedCategory(e.target.value)}
                        className="bg-[#181818] border border-[#333] rounded-lg px-3 py-2 text-xs text-white focus:outline-none cursor-pointer font-mono"
                      >
                        {ALL_CATEGORIES.map((cat) => (
                          <option key={cat} value={cat}>
                            {cat}
                          </option>
                        ))}
                      </select>

                      <button
                        disabled={submittingLabel || !selectedCategory}
                        onClick={() => handleApplyLabel(selectedCategory, 'Manual category correction')}
                        className="px-3.5 py-2 rounded-lg bg-[#222] hover:bg-[#2a2a2a] text-[#ededed] text-xs font-medium border border-[#333] transition-colors disabled:opacity-50"
                      >
                        Apply Label
                      </button>
                    </div>

                    {/* Junk / Other fast action */}
                    <button
                      disabled={submittingLabel}
                      onClick={() => handleApplyLabel('Other', 'Classified as Junk / Other')}
                      className="px-3 py-2 rounded-lg bg-rose-950/30 hover:bg-rose-900/50 text-rose-300 text-xs border border-rose-800/40 transition-colors ml-auto disabled:opacity-50"
                    >
                      Mark as Other / Junk
                    </button>
                  </div>
                </div>

                {/* 2. Model Explainability & Category Probability Distribution */}
                <div className="space-y-3">
                  <div className="flex items-center justify-between">
                    <span className="text-xs font-bold text-white uppercase tracking-wider flex items-center gap-1.5">
                      <Cpu className="w-3.5 h-3.5 text-cyan-400" />
                      Model Confidence & Class Probabilities
                    </span>
                    {explainLoading && (
                      <span className="text-[11px] font-mono text-cyan-400 flex items-center gap-1">
                        <RefreshCw className="w-3 h-3 animate-spin" /> Evaluating features...
                      </span>
                    )}
                  </div>

                  {explainData?.class_probabilities ? (
                    <div className="rounded-xl border border-[#1a1a1a] bg-[#000] p-4 space-y-2.5">
                      {Object.entries(explainData.class_probabilities)
                        .sort((a, b) => b[1] - a[1])
                        .map(([category, prob]) => {
                          const pct = Math.round(prob * 100);
                          const isTop = category === explainData.predicted_category;
                          const colorClass = CATEGORY_COLORS[category] || CATEGORY_COLORS.Other;

                          return (
                            <div key={category} className="space-y-1">
                              <div className="flex items-center justify-between text-xs font-mono">
                                <span className="flex items-center gap-2">
                                  <span className={`px-1.5 py-0.2 rounded border text-[10px] font-semibold ${colorClass}`}>
                                    {category}
                                  </span>
                                  {isTop && (
                                    <span className="text-[10px] text-amber-400">
                                      ★ top prediction
                                    </span>
                                  )}
                                </span>
                                <span className={`font-bold ${isTop ? 'text-white' : 'text-[#777]'}`}>
                                  {pct}%
                                </span>
                              </div>
                              <div className="w-full bg-[#161616] rounded-full h-1.5 overflow-hidden">
                                <div
                                  className={`h-full rounded-full transition-all ${
                                    isTop ? 'bg-amber-400' : 'bg-[#444]'
                                  }`}
                                  style={{ width: `${Math.max(2, pct)}%` }}
                                />
                              </div>
                            </div>
                          );
                        })}
                    </div>
                  ) : (
                    <div className="p-6 text-center text-[#555] text-xs font-mono border border-[#1a1a1a] rounded-xl">
                      Select an item to run feature evaluation
                    </div>
                  )}
                </div>

                {/* 3. Feature Signals & File Structure Breakdown */}
                {selectedTorrent.files && Array.isArray(selectedTorrent.files) && selectedTorrent.files.length > 0 && (
                  <div className="space-y-2.5">
                    <span className="text-xs font-bold text-white uppercase tracking-wider flex items-center gap-1.5">
                      <FileCode className="w-3.5 h-3.5 text-blue-400" />
                      Payload File Tree ({selectedTorrent.files.length} items)
                    </span>
                    <div className="rounded-xl border border-[#1a1a1a] bg-[#050505] p-3 max-h-48 overflow-y-auto space-y-1 font-mono text-xs text-[#999]">
                      {selectedTorrent.files.slice(0, 30).map((file, idx) => {
                        const path = typeof file === 'string' ? file : file.path || file.name || 'unnamed';
                        const sz = typeof file === 'object' && file.length ? formatBytes(file.length) : '';
                        return (
                          <div key={idx} className="flex items-center justify-between py-0.5 hover:text-white">
                            <span className="truncate pr-4">• {path}</span>
                            {sz && <span className="text-[#555] text-[11px] shrink-0">{sz}</span>}
                          </div>
                        );
                      })}
                      {selectedTorrent.files.length > 30 && (
                        <div className="text-[11px] text-[#555] pt-1 italic">
                          + {selectedTorrent.files.length - 30} more files
                        </div>
                      )}
                    </div>
                  </div>
                )}
              </div>
            </div>
          ) : (
            <div className="p-16 text-center text-[#666] flex flex-col items-center justify-center flex-1 space-y-3">
              <Tag className="w-12 h-12 text-[#333]" />
              <div className="text-sm font-medium text-[#888]">No Torrent Selected</div>
              <p className="text-xs text-[#555] max-w-sm">
                Pick a release from the left review queue to inspect model explanations and submit active learning corrections.
              </p>
            </div>
          )}
        </div>
      </div>

      {/* Model Retraining & Version History Modal */}
      {showModelModal && (
        <div
          className="fixed inset-0 z-50 bg-black/80 backdrop-blur-sm flex items-center justify-center p-4"
          onClick={() => setShowModelModal(false)}
        >
          <div
            className="bg-[#0e0e0e] border border-[#222] rounded-xl w-full max-w-2xl max-h-[85vh] flex flex-col shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-150"
            onClick={(e) => e.stopPropagation()}
          >
            {/* Modal Header */}
            <div className="px-5 py-4 border-b border-[#1c1c1c] flex items-center justify-between">
              <div className="flex items-center gap-3">
                <div className="w-8 h-8 rounded-lg bg-cyan-950/50 border border-cyan-800/40 flex items-center justify-center text-cyan-400">
                  <Cpu className="w-4 h-4" />
                </div>
                <div>
                  <h3 className="text-sm font-semibold text-white font-mono">
                    Model Management & Retraining Pipeline
                  </h3>
                  <p className="text-[11px] text-[#777] mt-0.5">
                    Continuous ML lifecycle: direct database training from PostgreSQL ground truth
                  </p>
                </div>
              </div>
              <button
                onClick={() => setShowModelModal(false)}
                className="p-1 rounded-md text-[#666] hover:text-white hover:bg-[#1a1a1a]"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            {/* Modal Body */}
            <div className="p-5 overflow-y-auto space-y-6 flex-1 text-xs font-mono">
              {/* Trigger Direct DB Retraining Box */}
              <div className="rounded-lg border border-[#222] bg-[#080808] p-4 space-y-3">
                <div className="flex items-center justify-between">
                  <span className="font-semibold text-white">Direct DB Training Pipeline</span>
                  <button
                    disabled={retrainingTriggered || retrainStatus?.is_training}
                    onClick={triggerRetraining}
                    className="px-3.5 py-1.5 rounded-lg bg-cyan-600 hover:bg-cyan-500 text-white font-semibold text-xs transition-colors flex items-center gap-1.5 disabled:opacity-50"
                  >
                    {retrainStatus?.is_training ? (
                      <>
                        <RefreshCw className="w-3.5 h-3.5 animate-spin" />
                        <span>Training in progress...</span>
                      </>
                    ) : (
                      <>
                        <Sparkles className="w-3.5 h-3.5" />
                        <span>Trigger Retraining</span>
                      </>
                    )}
                  </button>
                </div>
                <p className="text-[#888] leading-relaxed">
                  Extracts 31,700+ verified samples directly from PostgreSQL, tunes a candidate LightGBM model, evaluates against active model on a holdout test set, and updates <code className="text-white">active_model.json</code> with zero downtime.
                </p>

                {retrainStatus?.last_run && (
                  <div className="mt-3 p-3 rounded bg-[#020202] border border-[#1a1a1a] text-[11px] space-y-1 text-[#aaa]">
                    <div>Last Pipeline Exit Code: <span className={retrainStatus.last_run.exit_code === 0 ? 'text-emerald-400' : 'text-rose-400'}>{retrainStatus.last_run.exit_code}</span></div>
                    {retrainStatus.last_run.stdout && (
                      <pre className="text-[10px] text-[#777] max-h-24 overflow-y-auto font-mono whitespace-pre-wrap">
                        {retrainStatus.last_run.stdout.slice(-500)}
                      </pre>
                    )}
                  </div>
                )}
              </div>

              {/* Available Model Artifacts List */}
              <div className="space-y-3">
                <span className="font-semibold text-white uppercase text-[11px] tracking-wider">
                  Available Version Artifacts
                </span>
                <div className="divide-y divide-[#181818] border border-[#1c1c1c] rounded-lg overflow-hidden bg-[#0a0a0a]">
                  {models?.available && models.available.length > 0 ? (
                    models.available.map((m) => {
                      const isActive = models.active?.version === m.version;
                      return (
                        <div key={m.version} className="p-3 flex items-center justify-between">
                          <div>
                            <div className="flex items-center gap-2">
                              <span className="font-bold text-white">{m.version}</span>
                              {isActive && (
                                <span className="px-1.5 py-0.2 rounded bg-cyan-950/60 border border-cyan-800/40 text-cyan-300 text-[10px]">
                                  Active Model
                                </span>
                              )}
                            </div>
                            <div className="text-[11px] text-[#777] mt-0.5">
                              Accuracy: {m.val_accuracy ? `${(m.val_accuracy * 100).toFixed(2)}%` : '—'} · {m.samples_count?.toLocaleString() || '31k'} samples · {m.created_at ? formatTime(m.created_at) : 'Active'}
                            </div>
                          </div>

                          {!isActive && (
                            <button
                              onClick={() => handleRollback(m.version)}
                              className="px-2.5 py-1 rounded bg-[#181818] border border-[#2a2a2a] text-[#aaa] hover:text-white text-xs flex items-center gap-1"
                            >
                              <RotateCcw className="w-3 h-3" />
                              <span>Rollback</span>
                            </button>
                          )}
                        </div>
                      );
                    })
                  ) : (
                    <div className="p-4 text-center text-[#666]">
                      No previous model versions found
                    </div>
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
