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
  Check,
  X,
  Sparkles,
  ExternalLink,
  FileCode,
  Clock,
  Activity,
  AlertCircle,
  Zap,
  FileText,
  Folder,
  Play,
  Square,
  BarChart2,
  ShieldCheck,
  Radio,
  Shield,
  ShieldAlert,
  CheckCircle,
  Ban,
  Eye,
} from 'lucide-react';
import { api, magnetFrom } from '../api.js';
import { formatBytes, formatNum, formatTime, formatDubaiDate } from '../utils.js';

const CATEGORY_COLORS = {
  Adult: 'bg-rose-500/10 text-rose-400 border-rose-500/25',
  Anime: 'bg-pink-500/10 text-pink-400 border-pink-500/25',
  Applications: 'bg-amber-500/10 text-amber-400 border-amber-500/25',
  Audiobooks: 'bg-indigo-500/10 text-indigo-400 border-indigo-500/25',
  'Books & Learning': 'bg-teal-500/10 text-teal-400 border-teal-500/25',
  Documentaries: 'bg-emerald-500/10 text-emerald-400 border-emerald-500/25',
  Games: 'bg-lime-500/10 text-lime-400 border-lime-500/25',
  Movies: 'bg-blue-500/10 text-blue-400 border-blue-500/25',
  Music: 'bg-cyan-500/10 text-cyan-400 border-cyan-500/25',
  Television: 'bg-purple-500/10 text-purple-400 border-purple-500/25',
  Other: 'bg-zinc-800/60 text-zinc-400 border-zinc-700/40',
  Unclassified: 'bg-zinc-900/60 text-zinc-500 border-zinc-800',
};

const ALL_CATEGORIES = [
  'Adult',
  'Anime',
  'Applications',
  'Audiobooks',
  'Books & Learning',
  'Documentaries',
  'Games',
  'Movies',
  'Music',
  'Television',
  'Other',
];

export default function ClassifierView({ onInspectTorrent, copyToClipboard, streamScoringStats }) {
  // Metrics & Status
  const [metrics, setMetrics] = useState(null);
  const [status, setStatus] = useState(null);
  const [models, setModels] = useState(null);

  // Review Queue state (All Classified removed per design spec)
  const [torrents, setTorrents] = useState([]);
  const [totalTorrents, setTotalTorrents] = useState(0);
  const [loading, setLoading] = useState(true);
  const [searchQuery, setSearchQuery] = useState('');
  const [searchInput, setSearchInput] = useState('');
  const [page, setPage] = useState(1);
  const limit = 20;

  // Selected item & Live Explainability
  const [selectedTorrent, setSelectedTorrent] = useState(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [fileSearch, setFileSearch] = useState('');
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
  const [rollbackLoading, setRollbackLoading] = useState(false);
  const [modelActionMsg, setModelActionMsg] = useState(null);
  const [modelModalTab, setModelModalTab] = useState('overview'); // 'overview' | 'versions' | 'matrix' | 'logs'

  // Studio Sub-tab: 'category' | 'scoring'
  const [activeStudioTab, setActiveStudioTab] = useState('category');

  // Scoring / Trust Adjudication State
  const [scoringStats, setScoringStats] = useState(streamScoringStats || null);
  const [scoringPending, setScoringPending] = useState([]);

  useEffect(() => {
    if (streamScoringStats) {
      setScoringStats(streamScoringStats);
    }
  }, [streamScoringStats]);
  const [scoringPendingTotal, setScoringPendingTotal] = useState(0);
  const [scoringPendingPages, setScoringPendingPages] = useState(1);
  const [scoringPage, setScoringPage] = useState(1);
  const [scoringLoading, setScoringLoading] = useState(false);
  const [adjudicatingHash, setAdjudicatingHash] = useState(null);
  const [adjudicationSuccess, setAdjudicationSuccess] = useState(null);

  // Background Reclassification State
  const [reclassifyStatus, setReclassifyStatus] = useState(null);
  const [reclassifyStarting, setReclassifyStarting] = useState(false);
  const [reclassifyCancelling, setReclassifyCancelling] = useState(false);
  const [showReclassifyModal, setShowReclassifyModal] = useState(false);
  const [reclassifyBatchSize, setReclassifyBatchSize] = useState(1000);
  const [reclassifyLimit, setReclassifyLimit] = useState('');
  const [reclassifyDryRun, setReclassifyDryRun] = useState(false);
  const [dismissedReclassifyError, setDismissedReclassifyError] = useState(null);

  // Fetch scoring stats and pending adjudication items
  const fetchScoringData = useCallback(async () => {
    try {
      const stats = await api('/api/scoring/stats');
      setScoringStats(stats);
    } catch (err) {
      console.warn('Failed to load scoring stats:', err.message);
    }
  }, []);

  const fetchScoringPending = useCallback(async () => {
    setScoringLoading(true);
    try {
      const res = await api(`/api/scoring/pending?page=${scoringPage}&limit=20`);
      setScoringPending(res.data || []);
      setScoringPendingTotal(res.total || 0);
      setScoringPendingPages(res.pages || 1);
    } catch (err) {
      console.warn('Failed to load pending scoring items:', err.message);
    } finally {
      setScoringLoading(false);
    }
  }, [scoringPage]);

  // Blocked & Suppressed Sub-tab State
  const [blockedItems, setBlockedItems] = useState([]);
  const [blockedTotal, setBlockedTotal] = useState(0);
  const [blockedPages, setBlockedPages] = useState(1);
  const [blockedPage, setBlockedPage] = useState(1);
  const [blockedSearch, setBlockedSearch] = useState('');
  const [blockedSearchInput, setBlockedSearchInput] = useState('');
  const [blockedLoading, setBlockedLoading] = useState(false);
  const [unblockingHash, setUnblockingHash] = useState(null);
  const [blockedActionSuccess, setBlockedActionSuccess] = useState(null);

  // Batch Rescore Modal State
  const [showBatchRescoreModal, setShowBatchRescoreModal] = useState(false);
  const [batchRescoreScope, setBatchRescoreScope] = useState('review');
  const [batchRescoreCategory, setBatchRescoreCategory] = useState('');
  const [batchRescoreLimit, setBatchRescoreLimit] = useState(500);
  const [batchRescoreRunning, setBatchRescoreRunning] = useState(false);
  const [batchRescoreMsg, setBatchRescoreMsg] = useState(null);

  const fetchBlockedItems = useCallback(async () => {
    setBlockedLoading(true);
    try {
      const q = new URLSearchParams({
        page: String(blockedPage),
        limit: '20'
      });
      if (blockedSearch.trim()) q.set('search', blockedSearch.trim());
      const res = await api(`/api/scoring/blocked?${q.toString()}`);
      setBlockedItems(res.data || []);
      setBlockedTotal(res.total || 0);
      setBlockedPages(res.pages || 1);
    } catch (err) {
      console.warn('Failed to load blocked items:', err.message);
    } finally {
      setBlockedLoading(false);
    }
  }, [blockedPage, blockedSearch]);

  const handleUnblockTorrent = async (infohash, targetAction = 'ALLOW') => {
    setUnblockingHash(infohash);
    setBlockedActionSuccess(null);
    try {
      const res = await api('/api/scoring/unblock', {
        method: 'POST',
        body: JSON.stringify({
          infohashes: [infohash],
          target_action: targetAction,
          notes: `Unblocked via Studio Blocked tab to ${targetAction}`
        })
      });
      if (res.success) {
        setBlockedActionSuccess(`Restored infohash to ${targetAction} (${targetAction === 'ALLOW' ? 'SAFE' : 'REVIEW'}).`);
        setBlockedItems((prev) => prev.filter((it) => it.infohash !== infohash));
        setBlockedTotal((prev) => Math.max(0, prev - 1));
        fetchScoringData();
        setTimeout(() => setBlockedActionSuccess(null), 3500);
      }
    } catch (err) {
      alert(`Unblock failed: ${err.message}`);
    } finally {
      setUnblockingHash(null);
    }
  };

  const handleTriggerBatchRescore = async () => {
    setBatchRescoreRunning(true);
    setBatchRescoreMsg(null);
    try {
      const res = await api('/api/scoring/batch-rescore', {
        method: 'POST',
        body: JSON.stringify({
          scope: batchRescoreScope,
          category: batchRescoreCategory,
          limit: Number(batchRescoreLimit) || 500
        })
      });
      if (res.success) {
        setBatchRescoreMsg({ type: 'success', text: res.message });
        fetchScoringData();
        if (activeStudioTab === 'scoring') fetchScoringPending();
        setTimeout(() => {
          setShowBatchRescoreModal(false);
          setBatchRescoreMsg(null);
        }, 2500);
      }
    } catch (err) {
      setBatchRescoreMsg({ type: 'error', text: err.message });
    } finally {
      setBatchRescoreRunning(false);
    }
  };

  // Handle human override / adjudication action (ALLOW / DOWNRANK / SUPPRESS)
  const handleScoringOverride = async (infohash, action, notes = '') => {
    setAdjudicatingHash(infohash);
    setAdjudicationSuccess(null);
    try {
      const res = await api('/api/scoring/override', {
        method: 'POST',
        body: JSON.stringify({ infohash, action, notes })
      });
      if (res.success) {
        setAdjudicationSuccess({ infohash, action, message: `Overridden to ${action}` });
        // Optimistically remove from pending list
        setScoringPending((prev) => prev.filter((it) => it.infohash !== infohash));
        setScoringPendingTotal((prev) => Math.max(0, prev - 1));
        // Refresh scoring summary statistics
        fetchScoringData();
        setTimeout(() => setAdjudicationSuccess(null), 3500);
      }
    } catch (err) {
      alert(`Adjudication failed: ${err.message}`);
    } finally {
      setAdjudicatingHash(null);
    }
  };

  useEffect(() => {
    if (activeStudioTab === 'scoring') {
      fetchScoringData();
      fetchScoringPending();
    } else if (activeStudioTab === 'blocked') {
      fetchScoringData();
      fetchBlockedItems();
    }
  }, [activeStudioTab, fetchScoringData, fetchScoringPending, fetchBlockedItems]);

  // Fetch telemetry & status
  const fetchStatusAndMetrics = useCallback(async () => {
    try {
      const [m, s, r] = await Promise.all([
        api('/api/classifier/metrics'),
        api('/api/classifier/status'),
        api('/api/classifier/reclassify/status').catch(() => null)
      ]);
      setMetrics(m);
      setStatus(s);
      if (r) setReclassifyStatus(r);
    } catch (err) {
      console.warn('Failed to load classifier metrics/status:', err.message);
    }
  }, []);

  // Fetch review queue items
  const fetchQueueTorrents = useCallback(async () => {
    setLoading(true);
    try {
      const offset = (page - 1) * limit;
      const params = new URLSearchParams({
        offset: String(offset),
        limit: String(limit),
        needs_review: 'true'
      });
      if (searchQuery.trim()) {
        params.set('search', searchQuery.trim());
      }

      const res = await api(`/api/classifier/torrents?${params.toString()}`);
      const items = res.torrents || res.items || [];
      setTorrents(items);
      setTotalTorrents(res.total || 0);

      // Auto-select first item if none selected or not in current items
      if (items.length > 0) {
        setSelectedTorrent((prev) => {
          if (!prev) return items[0];
          const exists = items.some((it) => it.infohash === prev.infohash);
          return exists ? prev : items[0];
        });
      } else {
        setSelectedTorrent(null);
      }
    } catch (err) {
      console.error('Failed to load classifier torrents:', err);
    } finally {
      setLoading(false);
    }
  }, [page, limit, searchQuery]);

  // Initial load & dynamic polling
  useEffect(() => {
    fetchStatusAndMetrics();
    // Fast poll if reclassification is active, else standard 15s interval
    const pollTime = reclassifyStatus?.is_running ? 2000 : 15000;
    const interval = setInterval(fetchStatusAndMetrics, pollTime);
    return () => clearInterval(interval);
  }, [fetchStatusAndMetrics, reclassifyStatus?.is_running]);

  useEffect(() => {
    fetchQueueTorrents();
  }, [fetchQueueTorrents]);

  // Fetch full details and live explainability when selected item changes
  const selectedInfohash = selectedTorrent?.infohash;
  useEffect(() => {
    if (!selectedInfohash) {
      setExplainData(null);
      return;
    }

    setFileSearch('');
    let isMounted = true;

    const loadTorrentDetailsAndExplain = async () => {
      setDetailLoading(true);
      setExplainLoading(true);
      try {
        let detailed = null;
        try {
          const detRes = await api(`/api/classifier/torrents/${selectedInfohash}`);
          if (detRes && detRes.infohash === selectedInfohash && isMounted) {
            detailed = detRes;
            setSelectedTorrent((prev) => (prev?.infohash === selectedInfohash ? { ...prev, ...detRes } : prev));
          }
        } catch (detailErr) {
          console.warn('Failed to load full torrent details:', detailErr.message);
        }

        if (!isMounted) return;

        const rawFiles = detailed?.files || selectedTorrent.files || [];
        const filesPayload = Array.isArray(rawFiles) ? rawFiles.slice(0, 200) : [];

        const payload = {
          infohash: selectedInfohash,
          name: detailed?.name || selectedTorrent.name,
          total_size: detailed?.total_size ?? selectedTorrent.total_size,
          file_count: detailed?.file_count ?? selectedTorrent.file_count,
          files: filesPayload
        };

        const res = await fetch('/api/classifier/classify', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload)
        });

        if (res.ok && isMounted) {
          const json = await res.json();
          setExplainData(json.classification);
          setSelectedCategory(
            json.classification?.predicted_category ||
            detailed?.category ||
            selectedTorrent.category ||
            'Movies'
          );
        }
      } catch (err) {
        console.warn('Explainability fetch failed:', err);
      } finally {
        if (isMounted) {
          setDetailLoading(false);
          setExplainLoading(false);
        }
      }
    };

    loadTorrentDetailsAndExplain();
    return () => {
      isMounted = false;
    };
  }, [selectedInfohash]);

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

  const handleApplyLabel = async (category, reason = 'Human review approval') => {
    if (!selectedTorrent?.infohash || !category) return;
    setSubmittingLabel(true);
    setActionSuccess(null);
    try {
      const res = await fetch('/api/classifier/labels', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          infohash: selectedTorrent.infohash,
          category,
          reason
        })
      });

      if (!res.ok) {
        const errorData = await res.json();
        throw new Error(errorData.detail || 'Failed to submit label');
      }

      setActionSuccess(`Saved ground truth "${category}". Drained from review queue.`);
      setTimeout(() => setActionSuccess(null), 4000);

      // Remove from active torrent list immediately
      setTorrents((prev) => prev.filter((t) => t.infohash !== selectedTorrent.infohash));
      setTotalTorrents((prev) => Math.max(0, prev - 1));

      // Refresh metrics
      fetchStatusAndMetrics();
    } catch (err) {
      alert(`Labeling Error: ${err.message}`);
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
    setModelActionMsg({ type: 'info', text: 'Initiating direct DB retraining pipeline...' });
    try {
      const res = await fetch('/api/classifier/retrain', { method: 'POST' });
      const json = await res.json();
      if (!res.ok) {
        throw new Error(json.detail || 'Retraining start failed');
      }
      setModelActionMsg({ type: 'info', text: 'Retraining started in background! Live telemetry streaming...' });
      const pollTimer = setInterval(async () => {
        try {
          const [st, modRes] = await Promise.all([
            api('/api/classifier/retrain/status'),
            api('/api/classifier/models')
          ]);
          setRetrainStatus(st);
          setModels(modRes);
          if (!st.is_training) {
            clearInterval(pollTimer);
            fetchStatusAndMetrics();
            if (st.last_run?.exit_code === 0) {
              setModelActionMsg({ type: 'success', text: 'Retraining succeeded! Candidate evaluated and activated.' });
            } else if (st.last_run) {
              setModelActionMsg({ type: 'error', text: `Retraining completed with exit code ${st.last_run.exit_code}.` });
            }
          }
        } catch (pollErr) {
          console.warn('Poll error:', pollErr);
        }
      }, 3000);
    } catch (err) {
      setModelActionMsg({ type: 'error', text: `Failed to trigger retraining: ${err.message}` });
    } finally {
      setRetrainingTriggered(false);
    }
  };

  const handleRollback = async (version) => {
    setRollbackLoading(true);
    setModelActionMsg({ type: 'info', text: `Rolling back active model to version: ${version}...` });
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
      setModelActionMsg({ type: 'success', text: `Successfully rolled back to version ${version}! Live inference worker reloaded.` });
      await openModelManager();
      await fetchStatusAndMetrics();
      setTimeout(() => setModelActionMsg(null), 5000);
    } catch (err) {
      setModelActionMsg({ type: 'error', text: `Rollback failed: ${err.message}` });
    } finally {
      setRollbackLoading(false);
    }
  };

  const handleStartReclassify = async () => {
    setReclassifyStarting(true);
    try {
      const payload = {
        batch_size: Number(reclassifyBatchSize) || 2000,
        limit: reclassifyLimit ? Number(reclassifyLimit) : null,
        dry_run: Boolean(reclassifyDryRun)
      };
      const res = await fetch('/api/classifier/reclassify/start', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload)
      });
      const data = await res.json();
      if (!res.ok) {
        throw new Error(data.detail || 'Failed to start reclassification');
      }
      setShowReclassifyModal(false);
      await fetchStatusAndMetrics();
    } catch (err) {
      alert(`Error: ${err.message}`);
    } finally {
      setReclassifyStarting(false);
    }
  };

  const handleCancelReclassify = async () => {
    if (!confirm('Are you sure you want to stop the active reclassification worker?')) return;
    setReclassifyCancelling(true);
    try {
      const res = await fetch('/api/classifier/reclassify/cancel', { method: 'POST' });
      await res.json();
      await fetchStatusAndMetrics();
    } catch (err) {
      alert(`Error cancelling: ${err.message}`);
    } finally {
      setReclassifyCancelling(false);
    }
  };

  const totalPages = Math.max(1, Math.ceil(totalTorrents / limit));

  return (
    <div className="space-y-6">
      {/* Studio Sub-tab Navigation */}
      <div className="flex items-center justify-between border-b border-[#1c1c1c] pb-3">
        <div className="flex items-center gap-2">
          <button
            onClick={() => setActiveStudioTab('category')}
            className={`px-3 py-1.5 rounded-lg text-xs font-medium font-mono flex items-center gap-2 transition-colors ${
              activeStudioTab === 'category'
                ? 'bg-white text-black font-semibold'
                : 'bg-[#0f0f0f] border border-[#222] text-[#888] hover:text-white hover:border-[#333]'
            }`}
          >
            <Tag className="w-3.5 h-3.5" />
            <span>Category Classifier</span>
            {metrics?.review_queue_depth != null && metrics.review_queue_depth > 0 && (
              <span className={`text-[10px] px-1.5 py-0.2 rounded-full ${
                activeStudioTab === 'category' ? 'bg-black/10 text-black font-bold' : 'bg-[#1e1e1e] text-[#aaa]'
              }`}>
                {metrics.review_queue_depth.toLocaleString()}
              </span>
            )}
          </button>

          <button
            onClick={() => setActiveStudioTab('scoring')}
            className={`px-3 py-1.5 rounded-lg text-xs font-medium font-mono flex items-center gap-2 transition-colors ${
              activeStudioTab === 'scoring'
                ? 'bg-white text-black font-semibold'
                : 'bg-[#0f0f0f] border border-[#222] text-[#888] hover:text-white hover:border-[#333]'
            }`}
          >
            <Shield className="w-3.5 h-3.5" />
            <span>Trust & Integrity Adjudication</span>
            {scoringPendingTotal > 0 && (
              <span className={`text-[10px] px-1.5 py-0.2 rounded-full ${
                activeStudioTab === 'scoring' ? 'bg-black/10 text-black font-bold' : 'bg-rose-950/60 text-rose-400 border border-rose-800/50'
              }`}>
                {scoringPendingTotal.toLocaleString()}
              </span>
            )}
          </button>

          <button
            onClick={() => setActiveStudioTab('blocked')}
            className={`px-3 py-1.5 rounded-lg text-xs font-medium font-mono flex items-center gap-2 transition-colors ${
              activeStudioTab === 'blocked'
                ? 'bg-white text-black font-semibold'
                : 'bg-[#0f0f0f] border border-[#222] text-[#888] hover:text-white hover:border-[#333]'
            }`}
          >
            <Ban className="w-3.5 h-3.5 text-rose-400" />
            <span>Blocked & Suppressed</span>
            {blockedTotal > 0 && (
              <span className={`text-[10px] px-1.5 py-0.2 rounded-full ${
                activeStudioTab === 'blocked' ? 'bg-black/10 text-black font-bold' : 'bg-rose-950/60 text-rose-400 border border-rose-800/50'
              }`}>
                {blockedTotal.toLocaleString()}
              </span>
            )}
          </button>
        </div>

        <div className="text-[11px] font-mono text-[#666] hidden sm:block">
          {activeStudioTab === 'category'
            ? 'Ground-truth active learning'
            : activeStudioTab === 'scoring'
            ? 'Quality, safety & availability triage'
            : 'Catalog governance & suppressed torrent audits'}
        </div>
      </div>

      {/* ============================================================ */}
      {/* SUB-VIEW 2: TRUST & INTEGRITY ADJUDICATION                   */}
      {/* ============================================================ */}
      {activeStudioTab === 'scoring' && (
        <div className="space-y-6 animate-in fade-in duration-150">
          {/* Scoring Telemetry Strip */}
          <section className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-5 gap-2">
            <div className="rounded-lg border border-[#1e1e1e] bg-[#090909] p-3.5 flex flex-col justify-between">
              <div className="text-[10px] uppercase text-[#666] font-mono">01 / Scored Catalog</div>
              <div className="text-xl font-bold text-white font-mono mt-1">
                {scoringStats?.total_scored != null ? Number(scoringStats.total_scored).toLocaleString() : '2.88M+'}
              </div>
              <p className="text-[11px] text-[#777] mt-1 font-mono">
                Worker: {scoringStats?.active_worker || 'gaia-scoring-worker'}
              </p>
            </div>

            <div className="rounded-lg border border-emerald-950/40 bg-[#090909] p-3.5 flex flex-col justify-between">
              <div className="text-[10px] uppercase text-emerald-500 font-mono">02 / Safe & Allowed</div>
              <div className="text-xl font-bold text-emerald-400 font-mono mt-1">
                {scoringStats?.safe_count != null ? Number(scoringStats.safe_count).toLocaleString() : '2,880,105'}
              </div>
              <p className="text-[11px] text-[#777] mt-1 font-mono">
                {scoringStats?.safe_count && scoringStats?.total_scored
                  ? `${((scoringStats.safe_count / scoringStats.total_scored) * 100).toFixed(1)}% safe`
                  : '99.8% safe'}
              </p>
            </div>

            <div className="rounded-lg border border-amber-950/40 bg-[#090909] p-3.5 flex flex-col justify-between">
              <div className="text-[10px] uppercase text-amber-500 font-mono">03 / In Review</div>
              <div className="text-xl font-bold text-amber-400 font-mono mt-1">
                {scoringPendingTotal.toLocaleString()}
              </div>
              <p className="text-[11px] text-[#777] mt-1 font-mono">Pending policy adjudication</p>
            </div>

            <div className="rounded-lg border border-rose-950/40 bg-[#090909] p-3.5 flex flex-col justify-between">
              <div className="text-[10px] uppercase text-rose-500 font-mono">04 / Blocked & Suppressed</div>
              <div className="text-xl font-bold text-rose-400 font-mono mt-1">
                {scoringStats?.blocked_count != null ? Number(scoringStats.blocked_count).toLocaleString() : '2,751'}
              </div>
              <p className="text-[11px] text-[#777] mt-1 font-mono">Malware, spam & toxic</p>
            </div>

            <div className="rounded-lg border border-[#1e1e1e] bg-[#090909] p-3.5 flex flex-col justify-between">
              <div className="text-[10px] uppercase text-[#666] font-mono">05 / Human Overrides</div>
              <div className="text-xl font-bold text-white font-mono mt-1">
                {scoringStats?.manual_override_count != null ? Number(scoringStats.manual_override_count).toLocaleString() : '0'}
              </div>
              <p className="text-[11px] text-purple-400 mt-1 font-mono">Source = MANUAL protected</p>
            </div>
          </section>

          {/* Action notification toast */}
          {adjudicationSuccess && (
            <div className="p-3 rounded-lg bg-emerald-950/40 border border-emerald-800/40 text-emerald-300 font-mono text-xs flex items-center justify-between animate-in fade-in">
              <div className="flex items-center gap-2">
                <CheckCircle className="w-4 h-4 text-emerald-400" />
                <span>
                  Infohash <span className="text-white">{adjudicationSuccess.infohash.slice(0, 10)}...</span> successfully overridden to <strong>{adjudicationSuccess.action}</strong>.
                </span>
              </div>
              <span className="text-[10px] text-emerald-500">History audit logged</span>
            </div>
          )}

          {/* Adjudication Pending Queue Table */}
          <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] overflow-hidden">
            <div className="p-4 border-b border-[#181818] bg-[#0c0c0c] flex items-center justify-between">
              <div className="flex items-center gap-2.5">
                <ShieldAlert className="w-4 h-4 text-amber-400" />
                <h3 className="text-xs font-semibold text-white tracking-tight font-mono">
                  Pending Policy Adjudication Queue
                </h3>
                <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-amber-950/40 text-amber-400 border border-amber-800/40">
                  {scoringPendingTotal.toLocaleString()} items
                </span>
              </div>
              <div className="flex items-center gap-3">
                <button
                  onClick={() => setShowBatchRescoreModal(true)}
                  className="px-2.5 py-1 rounded bg-[#181818] border border-[#2b2b2b] text-[#ededed] hover:border-[#444] hover:bg-[#202020] text-xs font-mono flex items-center gap-1.5 transition-colors"
                  title="Trigger automated batch scoring / rescore on demand"
                >
                  <Sparkles className="w-3 h-3 text-amber-400" />
                  <span>Batch Rescore</span>
                </button>
                <span className="text-[11px] font-mono text-[#666]">
                  Page {scoringPage} of {scoringPendingPages}
                </span>
                <button
                  onClick={() => {
                    fetchScoringData();
                    fetchScoringPending();
                  }}
                  className="p-1 text-[#666] hover:text-white transition-colors"
                  title="Refresh Queue"
                >
                  <RefreshCw className={`w-3.5 h-3.5 ${scoringLoading ? 'animate-spin' : ''}`} />
                </button>
              </div>
            </div>

            <div className="overflow-x-auto">
              <table className="w-full text-left text-xs font-mono">
                <thead>
                  <tr className="border-b border-[#181818] text-[#666] text-[10px] uppercase">
                    <th className="py-2.5 px-4 font-normal">Payload / Title</th>
                    <th className="py-2.5 px-3 font-normal">Risk Tier</th>
                    <th className="py-2.5 px-3 font-normal">Action</th>
                    <th className="py-2.5 px-3 font-normal">Integrity</th>
                    <th className="py-2.5 px-3 font-normal">Availability</th>
                    <th className="py-2.5 px-3 font-normal">Source</th>
                    <th className="py-2.5 px-4 font-normal text-right">Human Adjudication</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[#141414] text-[11px]">
                  {scoringLoading ? (
                    <tr>
                      <td colSpan={7} className="p-12 text-center text-[#666]">
                        <RefreshCw className="w-5 h-5 animate-spin mx-auto text-white mb-2" />
                        <span>Loading pending adjudication items...</span>
                      </td>
                    </tr>
                  ) : scoringPending.length === 0 ? (
                    <tr>
                      <td colSpan={7} className="p-12 text-center text-[#666]">
                        <CheckCircle className="w-8 h-8 text-emerald-500 mx-auto mb-2 opacity-80" />
                        <div className="text-white font-semibold">Queue Clean!</div>
                        <div className="text-xs text-[#777] mt-0.5">No torrents currently pending human policy review.</div>
                      </td>
                    </tr>
                  ) : (
                    scoringPending.map((item) => {
                      const isActing = adjudicatingHash === item.infohash;
                      const riskColor =
                        item.risk_tier === 'BLOCKED'
                          ? 'bg-rose-950/60 text-rose-300 border-rose-800/50'
                          : item.risk_tier === 'REVIEW'
                          ? 'bg-amber-950/60 text-amber-300 border-amber-800/50'
                          : 'bg-emerald-950/60 text-emerald-300 border-emerald-800/50';

                      return (
                        <tr
                          key={item.infohash}
                          onClick={() => onInspectTorrent && onInspectTorrent(item)}
                          className="hover:bg-[#0c0c0c] cursor-pointer transition-colors group"
                        >
                          <td className="py-3 px-4 max-w-sm">
                            <div className="text-white font-sans truncate font-medium group-hover:text-white" title={item.name}>
                              {item.name || `payload-${item.infohash.slice(0, 8)}`}
                            </div>
                            <div className="text-[10px] text-[#555] flex items-center gap-2 mt-0.5 font-mono">
                              <span className="text-[#888]">{item.infohash.slice(0, 12)}...</span>
                              <span>·</span>
                              <span>{item.category || 'Unclassified'}</span>
                              <span>·</span>
                              <span>{formatBytes(item.total_size)}</span>
                            </div>
                          </td>

                          <td className="py-3 px-3 whitespace-nowrap">
                            <span className={`px-2 py-0.5 rounded text-[10px] font-semibold border ${riskColor}`}>
                              {item.risk_tier || 'UNKNOWN'}
                            </span>
                          </td>

                          <td className="py-3 px-3 whitespace-nowrap text-[#aaa]">
                            <span className="px-1.5 py-0.5 rounded bg-[#141414] border border-[#242424] text-[10px]">
                              {item.policy_action || 'ALLOW'}
                            </span>
                          </td>

                          <td className="py-3 px-3 whitespace-nowrap">
                            <span className={`font-semibold ${
                              item.integrity_score >= 80
                                ? 'text-emerald-400'
                                : item.integrity_score >= 50
                                ? 'text-amber-400'
                                : 'text-rose-400'
                            }`}>
                              {item.integrity_score ?? '—'}/100
                            </span>
                          </td>

                          <td className="py-3 px-3 whitespace-nowrap">
                            <span className={`text-[10px] px-1.5 py-0.5 rounded border ${
                              item.availability_state === 'ACTIVE'
                                ? 'bg-emerald-950/40 text-emerald-300 border-emerald-800/40'
                                : item.availability_state === 'SPARSE'
                                ? 'bg-amber-950/40 text-amber-300 border-amber-800/40'
                                : 'bg-rose-950/40 text-rose-300 border-rose-800/40'
                            }`}>
                              {item.availability_state || 'UNKNOWN'} ({item.availability_score ?? 0}%)
                            </span>
                          </td>

                          <td className="py-3 px-3 whitespace-nowrap text-[#777] text-[10px]">
                            {item.decision_source || 'MODEL'}
                          </td>

                          <td className="py-3 px-4 text-right whitespace-nowrap" onClick={(e) => e.stopPropagation()}>
                            <div className="flex items-center justify-end gap-1.5">
                              <button
                                onClick={() => onInspectTorrent && onInspectTorrent(item)}
                                className="p-1 rounded bg-[#141414] hover:bg-[#202020] border border-[#262626] text-[#888] hover:text-white transition-colors mr-1"
                                title="Inspect Full Metadata & Files"
                              >
                                <Eye className="w-3.5 h-3.5" />
                              </button>

                              <button
                                disabled={isActing}
                                onClick={() => handleScoringOverride(item.infohash, 'ALLOW', 'Manual clearance via Adjudication Studio')}
                                className="px-2 py-1 rounded bg-emerald-950/60 hover:bg-emerald-900/60 border border-emerald-800/60 text-emerald-300 text-[10px] flex items-center gap-1 transition-colors disabled:opacity-40"
                                title="Override to SAFE & ALLOW"
                              >
                                <Check className="w-2.5 h-2.5" />
                                <span>Allow</span>
                              </button>

                              <button
                                disabled={isActing}
                                onClick={() => handleScoringOverride(item.infohash, 'DOWNRANK', 'Downranked via Adjudication Studio')}
                                className="px-2 py-1 rounded bg-amber-950/60 hover:bg-amber-900/60 border border-amber-800/60 text-amber-300 text-[10px] flex items-center gap-1 transition-colors disabled:opacity-40"
                                title="Downrank in search rankings"
                              >
                                <AlertTriangle className="w-2.5 h-2.5" />
                                <span>Downrank</span>
                              </button>

                              <button
                                disabled={isActing}
                                onClick={() => handleScoringOverride(item.infohash, 'SUPPRESS', 'Suppressed/Blocked via Adjudication Studio')}
                                className="px-2 py-1 rounded bg-rose-950/60 hover:bg-rose-900/60 border border-rose-800/60 text-rose-300 text-[10px] flex items-center gap-1 transition-colors disabled:opacity-40"
                                title="Suppress and block payload"
                              >
                                <Ban className="w-2.5 h-2.5" />
                                <span>Suppress</span>
                              </button>
                            </div>
                          </td>
                        </tr>
                      );
                    })
                  )}
                </tbody>
              </table>
            </div>

            {/* Pagination Controls */}
            <div className="p-3 border-t border-[#181818] bg-[#0c0c0c] flex items-center justify-between text-xs font-mono text-[#777]">
              <span>
                Page <strong className="text-white">{scoringPage}</strong> of{' '}
                <strong className="text-white">{scoringPendingPages}</strong>
              </span>
              <div className="flex items-center gap-2">
                <button
                  disabled={scoringPage <= 1 || scoringLoading}
                  onClick={() => setScoringPage((p) => Math.max(1, p - 1))}
                  className="px-2 py-1 rounded bg-[#141414] border border-[#242424] text-[#888] hover:text-white disabled:opacity-30 transition-colors"
                >
                  Previous
                </button>
                <button
                  disabled={scoringPage >= scoringPendingPages || scoringLoading}
                  onClick={() => setScoringPage((p) => Math.min(scoringPendingPages, p + 1))}
                  className="px-2 py-1 rounded bg-[#141414] border border-[#242424] text-[#888] hover:text-white disabled:opacity-30 transition-colors"
                >
                  Next
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* ============================================================ */}
      {/* SUB-VIEW 3: BLOCKED & SUPPRESSED CATALOG                     */}
      {/* ============================================================ */}
      {activeStudioTab === 'blocked' && (
        <div className="space-y-6 animate-in fade-in duration-150">
          {/* Action notification toast */}
          {blockedActionSuccess && (
            <div className="p-3 rounded-lg bg-emerald-950/40 border border-emerald-800/40 text-emerald-300 font-mono text-xs flex items-center justify-between animate-in fade-in">
              <div className="flex items-center gap-2">
                <CheckCircle className="w-4 h-4 text-emerald-400" />
                <span>{blockedActionSuccess}</span>
              </div>
              <span className="text-[10px] text-emerald-500">History audit logged</span>
            </div>
          )}

          {/* Blocked Summary Strip */}
          <section className="grid grid-cols-1 sm:grid-cols-3 gap-3">
            <div className="rounded-lg border border-rose-950/40 bg-[#090909] p-3.5 flex flex-col justify-between">
              <div className="text-[10px] uppercase text-rose-400 font-mono">01 / Total Blocked & Suppressed</div>
              <div className="text-xl font-bold text-rose-300 font-mono mt-1">
                {blockedTotal.toLocaleString()}
              </div>
              <p className="text-[11px] text-[#777] mt-1 font-mono">
                Policy = SUPPRESS or Risk = BLOCKED
              </p>
            </div>

            <div className="rounded-lg border border-[#1e1e1e] bg-[#090909] p-3.5 flex flex-col justify-between">
              <div className="text-[10px] uppercase text-[#666] font-mono">02 / Manual Overrides</div>
              <div className="text-xl font-bold text-white font-mono mt-1">
                {scoringStats?.manual_override_count != null ? Number(scoringStats.manual_override_count).toLocaleString() : '0'}
              </div>
              <p className="text-[11px] text-[#777] mt-1 font-mono">Operator protected decisions</p>
            </div>

            <div className="rounded-lg border border-[#1e1e1e] bg-[#090909] p-3.5 flex flex-col justify-between">
              <div className="text-[10px] uppercase text-[#666] font-mono">03 / Crawler Ingestion State</div>
              <div className="text-xl font-bold text-emerald-400 font-mono mt-1">
                PROTECTED
              </div>
              <p className="text-[11px] text-[#777] mt-1 font-mono">Tombstone active — zero re-crawl</p>
            </div>
          </section>

          {/* Search & Filter Bar */}
          <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-4 flex flex-col sm:flex-row items-center justify-between gap-3">
            <form
              onSubmit={(e) => {
                e.preventDefault();
                setBlockedSearch(blockedSearchInput);
                setBlockedPage(1);
              }}
              className="flex-1 flex items-center gap-2 w-full"
            >
              <div className="relative flex-1">
                <Search className="w-3.5 h-3.5 text-[#555] absolute left-3 top-1/2 -translate-y-1/2" />
                <input
                  type="text"
                  value={blockedSearchInput}
                  onChange={(e) => setBlockedSearchInput(e.target.value)}
                  placeholder="Search blocked torrents by name or infohash..."
                  className="w-full bg-[#121212] border border-[#242424] rounded-lg pl-9 pr-8 py-1.5 text-xs text-white placeholder-[#555] focus:outline-none focus:border-[#444] font-mono"
                />
                {blockedSearchInput && (
                  <button
                    type="button"
                    onClick={() => {
                      setBlockedSearchInput('');
                      setBlockedSearch('');
                      setBlockedPage(1);
                    }}
                    className="absolute right-2.5 top-1/2 -translate-y-1/2 text-[#666] hover:text-white"
                  >
                    <X className="w-3 h-3" />
                  </button>
                )}
              </div>
              <button
                type="submit"
                className="px-3 py-1.5 rounded-lg bg-[#1a1a1a] hover:bg-[#252525] border border-[#2e2e2e] text-xs font-mono text-white transition-colors"
              >
                Search
              </button>
            </form>

            <button
              onClick={() => fetchBlockedItems()}
              className="p-1.5 rounded-lg border border-[#242424] bg-[#121212] text-[#888] hover:text-white transition-colors"
              title="Refresh List"
            >
              <RefreshCw className={`w-3.5 h-3.5 ${blockedLoading ? 'animate-spin' : ''}`} />
            </button>
          </div>

          {/* Blocked Items Table */}
          <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] overflow-hidden">
            <div className="p-4 border-b border-[#181818] bg-[#0c0c0c] flex items-center justify-between">
              <div className="flex items-center gap-2.5">
                <Ban className="w-4 h-4 text-rose-400" />
                <h3 className="text-xs font-semibold text-white tracking-tight font-mono">
                  Blocked & Suppressed Torrents
                </h3>
                <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-rose-950/40 text-rose-400 border border-rose-800/40">
                  {blockedTotal.toLocaleString()} items
                </span>
              </div>
              <span className="text-[11px] font-mono text-[#666]">
                Page {blockedPage} of {blockedPages}
              </span>
            </div>

            <div className="overflow-x-auto">
              <table className="w-full text-left text-xs font-mono">
                <thead>
                  <tr className="border-b border-[#181818] text-[#666] text-[10px] uppercase">
                    <th className="py-2.5 px-4 font-normal">Payload / Title</th>
                    <th className="py-2.5 px-3 font-normal">Category</th>
                    <th className="py-2.5 px-3 font-normal">Risk Tier</th>
                    <th className="py-2.5 px-3 font-normal">Action</th>
                    <th className="py-2.5 px-3 font-normal">Source</th>
                    <th className="py-2.5 px-4 font-normal text-right">Audit & Restore Actions</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[#141414] text-[11px]">
                  {blockedLoading ? (
                    <tr>
                      <td colSpan={6} className="p-12 text-center text-[#666]">
                        <RefreshCw className="w-5 h-5 animate-spin mx-auto text-white mb-2" />
                        <span>Loading blocked catalog...</span>
                      </td>
                    </tr>
                  ) : blockedItems.length === 0 ? (
                    <tr>
                      <td colSpan={6} className="p-12 text-center text-[#666]">
                        <ShieldCheck className="w-8 h-8 text-emerald-500 mx-auto mb-2 opacity-80" />
                        <div className="text-white font-semibold">No Blocked Torrents</div>
                        <div className="text-xs text-[#777] mt-0.5">There are currently no items matching the suppressed filter.</div>
                      </td>
                    </tr>
                  ) : (
                    blockedItems.map((item) => {
                      const isActing = unblockingHash === item.infohash;
                      return (
                        <tr
                          key={item.infohash}
                          onClick={() => onInspectTorrent && onInspectTorrent(item)}
                          className="hover:bg-[#0c0c0c] cursor-pointer transition-colors group"
                        >
                          <td className="py-3 px-4 max-w-sm">
                            <div className="text-white font-sans font-medium truncate group-hover:text-white" title={item.name}>
                              {item.name || `payload-${item.infohash.slice(0, 10)}`}
                            </div>
                            <div className="text-[10px] text-[#666] font-mono mt-0.5 flex items-center gap-2">
                              <span>{item.infohash.slice(0, 12)}...{item.infohash.slice(-8)}</span>
                              <span>·</span>
                              <span>{formatBytes(item.total_size)}</span>
                            </div>
                          </td>

                          <td className="py-3 px-3 whitespace-nowrap">
                            <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-[#141414] border border-[#222] text-[#aaa]">
                              {item.category || 'Unclassified'}
                            </span>
                          </td>

                          <td className="py-3 px-3 whitespace-nowrap">
                            <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-rose-950/60 text-rose-300 border border-rose-800/50">
                              {item.risk_tier || 'BLOCKED'}
                            </span>
                          </td>

                          <td className="py-3 px-3 whitespace-nowrap">
                            <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-rose-950/40 text-rose-400 border border-rose-800/40">
                              {item.policy_action || 'SUPPRESS'}
                            </span>
                          </td>

                          <td className="py-3 px-3 whitespace-nowrap text-[#888]">
                            {item.decision_source || 'MANUAL'}
                          </td>

                          <td className="py-3 px-4 text-right whitespace-nowrap" onClick={(e) => e.stopPropagation()}>
                            <div className="flex items-center justify-end gap-1.5">
                              <button
                                onClick={() => onInspectTorrent && onInspectTorrent(item)}
                                className="p-1 rounded bg-[#141414] hover:bg-[#202020] border border-[#262626] text-[#888] hover:text-white transition-colors mr-1"
                                title="Inspect Full Metadata & Files"
                              >
                                <Eye className="w-3.5 h-3.5" />
                              </button>

                              <button
                                disabled={isActing}
                                onClick={() => handleUnblockTorrent(item.infohash, 'REVIEW')}
                                className="px-2 py-1 rounded bg-amber-950/60 hover:bg-amber-900/60 border border-amber-800/60 text-amber-300 text-[10px] flex items-center gap-1 transition-colors disabled:opacity-40"
                                title="Redo & Queue for Re-evaluation"
                              >
                                <RotateCcw className="w-2.5 h-2.5" />
                                <span>Redo / Rescore</span>
                              </button>

                              <button
                                disabled={isActing}
                                onClick={() => handleUnblockTorrent(item.infohash, 'ALLOW')}
                                className="px-2 py-1 rounded bg-emerald-950/60 hover:bg-emerald-900/60 border border-emerald-800/60 text-emerald-300 text-[10px] flex items-center gap-1 transition-colors disabled:opacity-40"
                                title="Unblock & Restore to SAFE / ALLOW"
                              >
                                <Check className="w-2.5 h-2.5" />
                                <span>Unblock & Allow</span>
                              </button>
                            </div>
                          </td>
                        </tr>
                      );
                    })
                  )}
                </tbody>
              </table>
            </div>

            {/* Pagination Controls */}
            <div className="p-3 border-t border-[#181818] bg-[#0c0c0c] flex items-center justify-between text-xs font-mono text-[#777]">
              <span>
                Page <strong className="text-white">{blockedPage}</strong> of{' '}
                <strong className="text-white">{blockedPages}</strong>
              </span>
              <div className="flex items-center gap-2">
                <button
                  disabled={blockedPage <= 1 || blockedLoading}
                  onClick={() => setBlockedPage((p) => Math.max(1, p - 1))}
                  className="px-2 py-1 rounded bg-[#141414] border border-[#242424] text-[#888] hover:text-white disabled:opacity-30 transition-colors"
                >
                  Previous
                </button>
                <button
                  disabled={blockedPage >= blockedPages || blockedLoading}
                  onClick={() => setBlockedPage((p) => Math.min(blockedPages, p + 1))}
                  className="px-2 py-1 rounded bg-[#141414] border border-[#242424] text-[#888] hover:text-white disabled:opacity-30 transition-colors"
                >
                  Next
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* ============================================================ */}
      {/* SUB-VIEW 1: CATEGORY CLASSIFIER (Original View)              */}
      {/* ============================================================ */}
      {activeStudioTab === 'category' && (
        <>
      {/* System Verdict & Realtime Performance Banner (Overview Style) */}
      <section className="rounded-xl border border-[#222] bg-[#090909] p-4 flex flex-col md:flex-row md:items-center justify-between gap-4">
        <div className="flex items-start md:items-center gap-3">
          <div className="w-7 h-7 rounded-lg bg-[#141414] border border-[#262626] flex items-center justify-center shrink-0">
            <Check className="w-4 h-4 text-white" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold text-white tracking-tight">Classifier pipeline is operating normally</span>
              <span className="text-[11px] px-2 py-0.5 rounded-full bg-emerald-950/60 border border-emerald-800/60 text-emerald-400 font-mono flex items-center gap-1">
                <Zap className="w-3 h-3 text-emerald-400" />
                {metrics?.rate_per_minute != null ? `${metrics.rate_per_minute.toLocaleString()}/min` : '2,000/min'}
              </span>
              <span className="text-[11px] px-2 py-0.5 rounded-full bg-[#141414] border border-[#262626] text-[#888] font-mono">
                {status?.model_version ? `Model ${status.model_version}` : 'Model v4'}
              </span>
            </div>
            <p className="text-xs text-[#888] mt-0.5 leading-relaxed">
              Automated classification pipeline continuously processing verified torrents into PostgreSQL. <strong className="text-white">{metrics?.total_classified != null ? metrics.total_classified.toLocaleString() : '—'}</strong> cataloged torrents ({metrics?.classified_percentage || '0'}%) classified across 10 standardized classes.
            </p>
          </div>
        </div>

        <div className="flex items-center gap-6 border-t md:border-t-0 border-[#1c1c1c] pt-3 md:pt-0 shrink-0 font-mono text-xs">
          <div>
            <div className="text-[10px] uppercase text-[#555] tracking-wider font-sans">Classify Rate</div>
            <div className="text-[#ededed] font-medium mt-0.5 flex items-center gap-1.5">
              <span className="text-emerald-400">●</span>
              <span>{metrics?.rate_per_minute != null ? `${metrics.rate_per_minute.toLocaleString()}/min` : '2,000/min'}</span>
            </div>
          </div>
          <div className="w-[1px] h-6 bg-[#1a1a1a]" />
          <div>
            <div className="text-[10px] uppercase text-[#555] tracking-wider font-sans">5m Pace</div>
            <div className="text-[#ededed] font-medium mt-0.5">
              {metrics?.rate_5m != null ? `${(metrics.rate_5m / 1000).toFixed(1)}k` : '—'}
            </div>
          </div>
          <div className="w-[1px] h-6 bg-[#1a1a1a]" />
          <div>
            <div className="text-[10px] uppercase text-[#555] tracking-wider font-sans">Review Queue</div>
            <div className="text-white font-bold mt-0.5">
              {metrics?.review_queue_depth != null ? metrics.review_queue_depth.toLocaleString() : '—'}
            </div>
          </div>
        </div>
      </section>

      {/* Top Telemetry Cards (Overview 4-card layout) */}
      <section className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-2">
        {/* Card 1: Review Queue Depth */}
        <div className="rounded-lg border border-[#1e1e1e] bg-[#090909] p-3.5 hover:border-[#333] transition-colors flex flex-col justify-between">
          <div>
            <div className="flex items-center justify-between text-[#666] mb-2 text-xs">
              <span className="font-mono text-[11px]">01 / Review Queue</span>
              <div className="flex items-center gap-1.5">
                {reclassifyStatus?.is_running ? (
                  <button
                    onClick={handleCancelReclassify}
                    disabled={reclassifyCancelling}
                    className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-rose-950/80 border border-rose-800/80 text-rose-300 hover:bg-rose-900 flex items-center gap-1 transition-colors"
                    title="Stop active reclassification"
                  >
                    <Square className="w-2.5 h-2.5 fill-rose-300" />
                    <span>Stop</span>
                  </button>
                ) : (
                  <button
                    onClick={() => setShowReclassifyModal(true)}
                    className="text-[10px] font-mono px-2 py-0.5 rounded bg-[#181818] border border-[#2b2b2b] text-[#ededed] hover:border-[#444] hover:bg-[#202020] flex items-center gap-1 transition-colors"
                    title="Run batch reclassification on review queue"
                  >
                    <Play className="w-2.5 h-2.5 fill-current" />
                    <span>Reclassify</span>
                  </button>
                )}
              </div>
            </div>
            <div className="text-xl font-bold text-white tracking-tight font-mono">
              {metrics?.review_queue_depth != null ? metrics.review_queue_depth.toLocaleString() : '—'}
            </div>
            {reclassifyStatus?.is_running ? (
              <p className="text-[11px] text-amber-400 mt-1 flex items-center gap-1 font-mono">
                <RefreshCw className="w-2.5 h-2.5 animate-spin" />
                <span>
                  {reclassifyStatus.processed?.toLocaleString()} processed ({reclassifyStatus.items_per_second || 0}/s)
                </span>
              </p>
            ) : (
              <p className="text-[11px] text-[#777] mt-1">Ambiguous or low confidence</p>
            )}
          </div>
          <div className="mt-3 h-[2px] w-full bg-[#1a1a1a]">
            {reclassifyStatus?.is_running ? (
              <div
                className="h-full bg-amber-500 transition-all duration-300"
                style={{
                  width: `${Math.min(100, Math.max(3, (reclassifyStatus.processed / Math.max(reclassifyStatus.total_target, 1)) * 100))}%`
                }}
              />
            ) : (
              <div
                className="h-full bg-white transition-all"
                style={{ width: `${Math.min(100, Math.max(5, ((metrics?.review_queue_depth || 0) / 100000) * 100))}%` }}
              />
            )}
          </div>
        </div>

        {/* Card 2: Total Classified */}
        <div className="rounded-lg border border-[#1e1e1e] bg-[#090909] p-3.5 hover:border-[#333] transition-colors">
          <div className="flex items-center justify-between text-[#666] mb-2 text-xs">
            <span className="font-mono text-[11px]">02 / Total Classified</span>
            <span className="text-emerald-400 font-mono text-[11px] flex items-center gap-1">
              <Activity className="w-3 h-3" />
              {metrics?.rate_per_minute ? `+${metrics.rate_per_minute.toLocaleString()}/m` : '+2.0k/m'}
            </span>
          </div>
          <div className="text-xl font-bold text-white tracking-tight font-mono">
            {metrics?.total_classified != null ? metrics.total_classified.toLocaleString() : '—'}
          </div>
          <p className="text-[11px] text-[#777] mt-1">{metrics?.classified_percentage || '0'}% of catalog categorized</p>
          <div className="mt-3 h-[2px] w-full bg-[#1a1a1a]">
            <div
              className="h-full bg-emerald-500 transition-all"
              style={{ width: `${Math.min(100, Math.max(3, parseFloat(metrics?.classified_percentage || 0)))}%` }}
            />
          </div>
        </div>

        {/* Card 3: Ingestion Backlog */}
        <div className="rounded-lg border border-[#1e1e1e] bg-[#090909] p-3.5 hover:border-[#333] transition-colors">
          <div className="flex items-center justify-between text-[#666] mb-2 text-xs">
            <span className="font-mono text-[11px]">03 / Ingestion Backlog</span>
            <span className="text-white font-mono text-[11px]">Worker active</span>
          </div>
          <div className="text-xl font-bold text-white tracking-tight font-mono">
            {metrics?.unclassified_torrents != null ? metrics.unclassified_torrents.toLocaleString() : '—'}
          </div>
          <p className="text-[11px] text-[#777] mt-1">Draining at ~{metrics?.rate_per_minute ? ((metrics.rate_per_minute * 60) / 1000).toFixed(0) : '120'}k/hr</p>
          <div className="mt-3 h-[2px] w-full bg-[#1a1a1a]">
            <div className="h-full bg-white w-full" />
          </div>
        </div>

        {/* Card 4: Model Architecture & Actions */}
        <div className="rounded-lg border border-[#1e1e1e] bg-[#090909] p-3.5 hover:border-[#333] transition-colors flex flex-col justify-between">
          <div>
            <div className="flex items-center justify-between text-[#666] mb-2 text-xs">
              <span className="font-mono text-[11px]">04 / Active Model</span>
              <button
                onClick={openModelManager}
                className="text-[11px] text-[#ededed] hover:text-white underline underline-offset-2 transition-colors font-mono"
              >
                Model Details
              </button>
            </div>
            <div className="text-xl font-bold text-white tracking-tight font-mono">
              {status?.model_version ? `Classifier ${status.model_version}` : (models?.active?.version ? `Classifier ${models.active.version}` : 'Classifier v4')}
            </div>
            <p className="text-[11px] text-[#777] mt-1">
              {metrics?.total_labeled_results ? `${metrics.total_labeled_results.toLocaleString()} ground-truth labels` : '38.2k ground-truth'}
            </p>
          </div>
          <div className="mt-3 h-[2px] w-full bg-[#1a1a1a]">
            <div className="h-full bg-white w-full" />
          </div>
        </div>
      </section>

      {/* Error Alert Banner when previous worker failed */}
      {!reclassifyStatus?.is_running && reclassifyStatus?.last_error && dismissedReclassifyError !== reclassifyStatus.last_error && (
        <section className="rounded-xl border border-rose-900/40 bg-rose-950/20 p-4 space-y-2.5 font-mono">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 text-rose-400">
              <AlertCircle className="w-4 h-4 text-rose-400 shrink-0" />
              <span className="text-xs font-semibold">Reclassification Worker Stopped with Error</span>
            </div>
            <div className="flex items-center gap-2">
              <button
                onClick={() => setShowReclassifyModal(true)}
                className="px-2.5 py-1 rounded bg-rose-900/40 hover:bg-rose-900/60 border border-rose-700/50 text-rose-200 text-[11px] flex items-center gap-1.5 transition-colors"
              >
                <RefreshCw className="w-3 h-3" />
                <span>Retry</span>
              </button>
              <button
                onClick={() => setDismissedReclassifyError(reclassifyStatus.last_error)}
                className="p-1 text-rose-400 hover:text-rose-200 rounded hover:bg-rose-900/30 transition-colors"
                title="Dismiss"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            </div>
          </div>
          <div className="p-2.5 rounded bg-[#0b0b0b] border border-rose-950 text-[11px] text-rose-300 font-mono overflow-x-auto whitespace-pre-wrap">
            {reclassifyStatus.last_error}
          </div>
        </section>
      )}

      {/* Live Reclassification Progress Banner (Visible when job is running) */}
      {reclassifyStatus?.is_running && (
        <section className="rounded-xl border border-amber-900/40 bg-amber-950/15 p-4 space-y-3 font-mono">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2.5">
              <RefreshCw className="w-4 h-4 animate-spin text-amber-400" />
              <span className="text-xs font-semibold text-amber-300">
                Active Reclassification Worker Running
              </span>
              <span className="text-[11px] px-2 py-0.5 rounded bg-amber-900/30 text-amber-300 border border-amber-800/40">
                {reclassifyStatus.dry_run ? 'Dry Run Mode' : 'Writing to Postgres'}
              </span>
            </div>
            <div className="flex items-center gap-4 text-xs">
              <span className="text-[#888]">
                Throughput: <strong className="text-white">{reclassifyStatus.items_per_second || 0}</strong> items/s
              </span>
              <span className="text-[#888]">
                ETA: <strong className="text-white">
                  {reclassifyStatus.eta_seconds != null
                    ? `${Math.floor(reclassifyStatus.eta_seconds / 60)}m ${reclassifyStatus.eta_seconds % 60}s`
                    : 'Calculating...'}
                </strong>
              </span>
              <button
                onClick={handleCancelReclassify}
                disabled={reclassifyCancelling}
                className="px-2.5 py-1 rounded bg-rose-950/80 border border-rose-800/80 text-rose-300 hover:bg-rose-900 flex items-center gap-1 transition-colors text-[11px]"
              >
                <Square className="w-3 h-3 fill-rose-300" />
                <span>{reclassifyCancelling ? 'Halting...' : 'Stop Worker'}</span>
              </button>
            </div>
          </div>

          <div className="space-y-1.5">
            <div className="flex justify-between text-[11px] text-[#aaa]">
              <span>
                Processed {reclassifyStatus.processed?.toLocaleString()} of {reclassifyStatus.total_target?.toLocaleString()} items
              </span>
              <span>
                {reclassifyStatus.total_target > 0
                  ? `${((reclassifyStatus.processed / reclassifyStatus.total_target) * 100).toFixed(1)}%`
                  : '0%'}
              </span>
            </div>
            <div className="h-2 w-full bg-[#181818] rounded-full overflow-hidden">
              <div
                className="h-full bg-amber-500 transition-all duration-300"
                style={{
                  width: `${Math.min(100, (reclassifyStatus.processed / Math.max(reclassifyStatus.total_target, 1)) * 100)}%`
                }}
              />
            </div>
          </div>
        </section>
      )}

      {/* Main Split Layout: Review Queue on Left, Deep Inspector on Right */}
      <div className="grid grid-cols-1 lg:grid-cols-12 gap-4 min-h-[640px]">
        {/* Left Column: Review Queue (5 cols) */}
        <div className="lg:col-span-5 flex flex-col rounded-lg border border-[#1e1e1e] bg-[#090909] overflow-hidden">
          {/* Header Controls */}
          <div className="p-3 border-b border-[#181818] bg-[#0c0c0c] space-y-2.5">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <span className="text-xs font-semibold text-white tracking-tight">Review Queue</span>
                {totalTorrents > 0 && (
                  <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-[#1c1c1c] text-[#aaa] border border-[#2b2b2b]">
                    {totalTorrents.toLocaleString()}
                  </span>
                )}
              </div>
              <span className="text-[11px] font-mono text-[#666]">
                Page {page} of {totalPages}
              </span>
            </div>

            {/* Search Box */}
            <form onSubmit={handleSearchSubmit} className="relative">
              <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-[#555]" />
              <input
                type="text"
                placeholder="Filter review queue by title or hash..."
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
                <RefreshCw className="w-5 h-5 animate-spin text-white" />
                <span className="text-xs font-mono">Loading review queue...</span>
              </div>
            ) : torrents.length === 0 ? (
              <div className="p-12 text-center text-[#666] space-y-2">
                <CheckCircle2 className="w-8 h-8 text-emerald-500 mx-auto opacity-75" />
                <div className="text-sm font-semibold text-white">Review Queue Clean!</div>
                <p className="text-xs text-[#777]">
                  No items currently require human review.
                </p>
              </div>
            ) : (
              torrents.map((t) => {
                const isSelected = selectedTorrent?.infohash === t.infohash;
                const cat = t.category || 'Unclassified';
                const colorClass = CATEGORY_COLORS[cat] || CATEGORY_COLORS.Other;
                const confPct = t.category_confidence ? Math.round(t.category_confidence * 100) : null;

                return (
                  <div
                    key={t.infohash}
                    onClick={() => setSelectedTorrent(t)}
                    className={`p-3 cursor-pointer transition-colors ${
                      isSelected
                        ? 'bg-[#141414] border-l-2 border-l-white'
                        : 'hover:bg-[#0c0c0c]'
                    }`}
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="font-medium text-xs text-[#ededed] line-clamp-2 leading-relaxed">
                        {t.name || 'Unnamed Infohash'}
                      </div>
                      <ChevronRight
                        className={`w-3.5 h-3.5 shrink-0 mt-0.5 ${
                          isSelected ? 'text-white' : 'text-[#444]'
                        }`}
                      />
                    </div>

                    <div className="flex items-center gap-2 mt-2 font-mono text-[10px]">
                      <span className={`px-1.5 py-0.5 rounded border text-[10px] font-semibold ${colorClass}`}>
                        {cat}
                      </span>
                      {confPct !== null && (
                        <span className="text-[#888]">
                          {confPct}% conf
                        </span>
                      )}
                      <span className="text-[#444]">·</span>
                      <span className="text-[#777]">
                        {formatBytes(t.total_size)}
                      </span>
                      <span className="text-[#444]">·</span>
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

        {/* Right Column: Deep Inspector & Active Learning (7 cols) */}
        <div className="lg:col-span-7 flex flex-col rounded-lg border border-[#1e1e1e] bg-[#090909] overflow-hidden">
          {selectedTorrent ? (
            <div className="flex-1 flex flex-col overflow-y-auto max-h-[800px]">
              {/* Torrent Header */}
              <div className="p-4 border-b border-[#181818] bg-[#0c0c0c] space-y-3">
                <div className="flex items-start justify-between gap-4">
                  <div className="space-y-1">
                    <h2 className="text-sm font-semibold text-white leading-snug">
                      {selectedTorrent.name || 'Unnamed Torrent'}
                    </h2>
                    <div className="flex items-center gap-2 text-xs font-mono text-[#777]">
                      <span>Hash: {selectedTorrent.infohash}</span>
                      <button
                        onClick={() => copyToClipboard && copyToClipboard(selectedTorrent.infohash, 'infohash')}
                        className="text-[#999] hover:text-white"
                        title="Copy infohash"
                      >
                        [copy]
                      </button>
                    </div>
                  </div>

                  <button
                    onClick={() => onInspectTorrent && onInspectTorrent(selectedTorrent)}
                    className="px-2.5 py-1 rounded bg-[#141414] border border-[#262626] text-xs text-[#aaa] hover:text-white flex items-center gap-1 shrink-0 font-mono"
                  >
                    <span>Inspect</span>
                    <ExternalLink className="w-3 h-3" />
                  </button>
                </div>

                {/* Metadata Pills */}
                <div className="flex flex-wrap items-center gap-2 text-xs font-mono">
                  <div className="bg-[#141414] border border-[#222] px-2 py-0.5 rounded text-[#bbb]">
                    Size: <span className="text-white font-bold">{formatBytes(selectedTorrent.total_size)}</span>
                  </div>
                  <div className="bg-[#141414] border border-[#222] px-2 py-0.5 rounded text-[#bbb]">
                    Files: <span className="text-white font-bold">{selectedTorrent.file_count || 1}</span>
                  </div>
                  <span className="px-2 py-0.5 rounded border border-amber-800/60 bg-amber-950/40 text-amber-300 font-medium">
                    Needs Review
                  </span>
                </div>
              </div>

              {/* Action Success Alert */}
              {actionSuccess && (
                <div className="m-3 p-2.5 rounded border border-emerald-800/40 bg-emerald-950/30 text-emerald-300 text-xs font-mono flex items-center gap-2">
                  <CheckCircle2 className="w-4 h-4 text-emerald-400 shrink-0" />
                  <span>{actionSuccess}</span>
                </div>
              )}

              {/* Inspector Content Body */}
              <div className="p-4 space-y-5 flex-1">
                {/* 1. Classification Diagnostics */}
                <div className="rounded-lg border border-[#1e1e1e] bg-[#0c0c0c] p-3.5 space-y-3">
                  <div className="flex items-center justify-between">
                    <span className="text-[11px] font-mono text-[#888] uppercase tracking-wider flex items-center gap-1.5">
                      <Cpu className="w-3.5 h-3.5 text-white" />
                      Classification Metadata & Diagnostics
                    </span>
                    {detailLoading && (
                      <span className="text-[11px] font-mono text-[#888] flex items-center gap-1">
                        <RefreshCw className="w-3 h-3 animate-spin text-white" /> Loading...
                      </span>
                    )}
                  </div>

                  <div className="grid grid-cols-2 sm:grid-cols-3 gap-2.5 font-mono text-xs">
                    <div className="bg-[#121212] border border-[#1f1f1f] rounded p-2.5 space-y-1">
                      <div className="text-[10px] text-[#666] uppercase">Predicted Class</div>
                      <div>
                        <span className={`px-2 py-0.5 rounded border text-[11px] font-semibold ${CATEGORY_COLORS[selectedTorrent.category] || CATEGORY_COLORS.Other}`}>
                          {selectedTorrent.category || 'Unclassified'}
                        </span>
                      </div>
                    </div>

                    <div className="bg-[#121212] border border-[#1f1f1f] rounded p-2.5 space-y-1">
                      <div className="text-[10px] text-[#666] uppercase">Confidence</div>
                      <div className="text-white font-bold text-sm">
                        {selectedTorrent.category_confidence !== null && selectedTorrent.category_confidence !== undefined
                          ? `${Math.round(selectedTorrent.category_confidence * 100)}%`
                          : '—'}
                      </div>
                    </div>

                    <div className="bg-[#121212] border border-[#1f1f1f] rounded p-2.5 space-y-1">
                      <div className="text-[10px] text-[#666] uppercase">Review Status</div>
                      <div className="text-amber-400 font-semibold flex items-center gap-1 text-[11px]">
                        <AlertTriangle className="w-3 h-3" /> Needs Review
                      </div>
                    </div>

                    <div className="bg-[#121212] border border-[#1f1f1f] rounded p-2.5 space-y-1">
                      <div className="text-[10px] text-[#666] uppercase">Flag Reason</div>
                      <div className="text-[#ededed] font-medium text-[11px] capitalize">
                        {selectedTorrent.classification_meta?.review_type
                          ? selectedTorrent.classification_meta.review_type.replace('_', ' ')
                          : 'Low Confidence'}
                      </div>
                    </div>

                    <div className="bg-[#121212] border border-[#1f1f1f] rounded p-2.5 space-y-1">
                      <div className="text-[10px] text-[#666] uppercase">Runner-up Category</div>
                      <div className="text-[#ccc] text-[11px] truncate">
                        {selectedTorrent.classification_meta?.top2 ? (
                          <span>
                            {selectedTorrent.classification_meta.top2.category}{' '}
                            <span className="text-[#777]">
                              ({Math.round((selectedTorrent.classification_meta.top2.confidence || 0) * 100)}%)
                            </span>
                          </span>
                        ) : (
                          <span className="text-[#666]">N/A</span>
                        )}
                      </div>
                      {selectedTorrent.classification_meta?.margin !== undefined && (
                        <div className="text-[10px] text-[#666]">
                          Margin: {(selectedTorrent.classification_meta.margin * 100).toFixed(1)}%
                        </div>
                      )}
                    </div>

                    <div className="bg-[#121212] border border-[#1f1f1f] rounded p-2.5 space-y-1">
                      <div className="text-[10px] text-[#666] uppercase">Model Version</div>
                      <div className="text-white text-[11px] flex items-center gap-1.5">
                        <span className="font-semibold">{selectedTorrent.classification_meta?.model_version || 'v4'}</span>
                        <span className="text-[9px] px-1 py-0.2 rounded bg-emerald-950 text-emerald-400 border border-emerald-800">active</span>
                      </div>
                      <div className="text-[10px] text-[#666] truncate" title={selectedTorrent.classified_at || '—'}>
                        {selectedTorrent.classified_at ? formatDubaiDate(selectedTorrent.classified_at) : '—'}
                      </div>
                    </div>
                  </div>
                </div>

                {/* 2. Interactive Human Relabeling Action Bar */}
                <div className="rounded-lg border border-[#1e1e1e] bg-[#0c0c0c] p-3.5 space-y-2.5">
                  <div className="flex items-center justify-between">
                    <span className="text-[11px] font-mono text-[#888] uppercase tracking-wider">
                      Active Learning Labeling
                    </span>
                    <span className="text-[11px] text-[#555] font-mono">Feeds PostgreSQL ground-truth</span>
                  </div>

                  <p className="text-xs text-[#777] leading-relaxed">
                    Confirm the predicted class or assign a manual correction to drain this item from the review queue and feed the next training iteration.
                  </p>

                  <div className="flex flex-wrap items-center gap-2 pt-1">
                    {explainData?.predicted_category && (
                      <button
                        disabled={submittingLabel}
                        onClick={() => handleApplyLabel(explainData.predicted_category, 'One-click prediction confirmation')}
                        className="px-3 py-1.5 rounded bg-white hover:bg-[#ededed] text-black font-semibold text-xs transition-colors flex items-center gap-1.5 disabled:opacity-50 font-mono"
                      >
                        <Check className="w-3.5 h-3.5" />
                        <span>Confirm: {explainData.predicted_category}</span>
                      </button>
                    )}

                    <div className="flex items-center gap-1.5">
                      <select
                        value={selectedCategory}
                        onChange={(e) => setSelectedCategory(e.target.value)}
                        className="bg-[#141414] border border-[#222] rounded px-2.5 py-1.5 text-xs text-white focus:outline-none cursor-pointer font-mono"
                      >
                        {ALL_CATEGORIES.map((cat) => (
                          <option key={cat} value={cat}>
                            {cat}
                          </option>
                        ))}
                      </select>

                      <button
                        disabled={submittingLabel || !selectedCategory}
                        onClick={() => handleApplyLabel(selectedCategory, 'Manual category assignment')}
                        className="px-3 py-1.5 rounded bg-[#1c1c1c] hover:bg-[#252525] text-[#ededed] text-xs font-medium border border-[#2a2a2a] transition-colors disabled:opacity-50 font-mono"
                      >
                        Apply
                      </button>
                    </div>

                    <button
                      disabled={submittingLabel}
                      onClick={() => handleApplyLabel('Other', 'Classified as Other')}
                      className="px-2.5 py-1.5 rounded bg-[#141414] hover:bg-[#1a1a1a] text-[#888] hover:text-[#ccc] text-xs border border-[#222] transition-colors ml-auto disabled:opacity-50 font-mono"
                    >
                      Mark Other
                    </button>
                  </div>
                </div>

                {/* 3. Class Probabilities Distribution */}
                <div className="space-y-2.5">
                  <div className="flex items-center justify-between">
                    <span className="text-[11px] font-mono text-[#888] uppercase tracking-wider">
                      Model Class Probabilities
                    </span>
                    {explainLoading && (
                      <span className="text-[11px] font-mono text-[#888] flex items-center gap-1">
                        <RefreshCw className="w-3 h-3 animate-spin text-white" /> Evaluating...
                      </span>
                    )}
                  </div>

                  {explainData?.class_probabilities ? (
                    <div className="rounded-lg border border-[#1e1e1e] bg-[#0c0c0c] p-3.5 space-y-2">
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
                                    <span className="text-[10px] text-white">
                                      ★ top prediction
                                    </span>
                                  )}
                                </span>
                                <span className={`font-bold ${isTop ? 'text-white' : 'text-[#777]'}`}>
                                  {pct}%
                                </span>
                              </div>
                              <div className="w-full bg-[#161616] rounded-full h-1 overflow-hidden">
                                <div
                                  className={`h-full rounded-full transition-all ${
                                    isTop ? 'bg-white' : 'bg-[#333]'
                                  }`}
                                  style={{ width: `${Math.max(2, pct)}%` }}
                                />
                              </div>
                            </div>
                          );
                        })}
                    </div>
                  ) : (
                    <div className="p-6 text-center text-[#555] text-xs font-mono border border-[#1e1e1e] rounded-lg bg-[#0c0c0c]">
                      Select an item to run model evaluation
                    </div>
                  )}
                </div>

                {/* 4. Files Manifest */}
                <div className="space-y-2.5">
                  <div className="flex items-center justify-between">
                    <span className="text-[11px] font-mono text-[#888] uppercase tracking-wider flex items-center gap-1.5">
                      <Folder className="w-3.5 h-3.5 text-white" />
                      Payload Files Manifest (
                      {selectedTorrent.files && Array.isArray(selectedTorrent.files)
                        ? selectedTorrent.files.length
                        : selectedTorrent.file_count || 0}
                      )
                    </span>
                    {selectedTorrent.files && selectedTorrent.files.length > 5 && (
                      <div className="relative w-44">
                        <Search className="w-3 h-3 absolute left-2 top-1/2 -translate-y-1/2 text-[#555]" />
                        <input
                          type="text"
                          value={fileSearch}
                          onChange={(e) => setFileSearch(e.target.value)}
                          placeholder="Filter files..."
                          className="w-full bg-[#050505] border border-[#222] rounded pl-6 pr-2 py-0.5 text-[11px] text-[#ededed] placeholder-[#555] focus:outline-none focus:border-[#444] font-mono"
                        />
                      </div>
                    )}
                  </div>

                  {detailLoading && (!selectedTorrent.files || selectedTorrent.files.length === 0) ? (
                    <div className="p-6 text-center text-[#666] text-xs font-mono border border-[#1e1e1e] rounded-lg bg-[#0c0c0c] flex items-center justify-center gap-2">
                      <RefreshCw className="w-3.5 h-3.5 animate-spin text-white" />
                      <span>Loading manifest from database...</span>
                    </div>
                  ) : selectedTorrent.files && Array.isArray(selectedTorrent.files) && selectedTorrent.files.length > 0 ? (
                    (() => {
                      const filteredFiles = selectedTorrent.files.filter((file) => {
                        if (!fileSearch.trim()) return true;
                        const rawPath = typeof file === 'string'
                          ? file
                          : Array.isArray(file.path)
                          ? file.path.join('/')
                          : file.path || file.name || '';
                        return rawPath.toLowerCase().includes(fileSearch.trim().toLowerCase());
                      });

                      return (
                        <div className="rounded-lg border border-[#1e1e1e] bg-[#0c0c0c] p-3 max-h-64 overflow-y-auto space-y-1 font-mono text-xs">
                          {filteredFiles.length === 0 ? (
                            <div className="text-center py-4 text-[#555] text-xs">
                              No files matching "{fileSearch}"
                            </div>
                          ) : (
                            filteredFiles.slice(0, 100).map((file, idx) => {
                              const rawPath = typeof file === 'string'
                                ? file
                                : Array.isArray(file.path)
                                ? file.path.join('/')
                                : file.path || file.name || 'unnamed';
                              const sz = typeof file === 'object' && file.length !== undefined
                                ? formatBytes(file.length)
                                : '';

                              return (
                                <div
                                  key={idx}
                                  className="flex items-center justify-between py-1 px-1.5 rounded hover:bg-[#141414] text-[#888] hover:text-[#ededed] transition-colors group"
                                >
                                  <div className="flex items-center gap-2 truncate pr-4">
                                    <FileCode className="w-3.5 h-3.5 shrink-0 text-[#666]" />
                                    <span className="truncate">{rawPath}</span>
                                  </div>
                                  {sz && (
                                    <span className="text-[#666] group-hover:text-[#aaa] text-[11px] shrink-0 font-mono">
                                      {sz}
                                    </span>
                                  )}
                                </div>
                              );
                            })
                          )}

                          {filteredFiles.length > 100 && (
                            <div className="text-[11px] text-[#555] pt-2 text-center italic border-t border-[#181818]">
                              Showing first 100 of {filteredFiles.length} files
                            </div>
                          )}
                        </div>
                      );
                    })()
                  ) : (
                    <div className="p-4 rounded-lg border border-[#1e1e1e] bg-[#0c0c0c] text-xs font-mono text-[#666] flex items-center gap-2">
                      <FileText className="w-4 h-4 text-[#444] shrink-0" />
                      <span>Single-file torrent without multi-file manifest.</span>
                    </div>
                  )}
                </div>
              </div>
            </div>
          ) : (
            <div className="p-16 text-center text-[#666] flex flex-col items-center justify-center flex-1 space-y-3">
              <Tag className="w-10 h-10 text-[#222]" />
              <div className="text-sm font-medium text-[#888]">No Torrent Selected</div>
              <p className="text-xs text-[#555] max-w-sm">
                Pick a release from the review queue to inspect model explanations and submit active learning corrections.
              </p>
            </div>
          )}
        </div>
      </div>
      </>
      )}

      {/* Redesigned Model Management Modal (Clean Overview aesthetic) */}
      {showModelModal && (
        <div
          className="fixed inset-0 z-50 bg-black/80 backdrop-blur-sm flex items-center justify-center p-4"
          onClick={() => setShowModelModal(false)}
        >
          <div
            className="bg-[#090909] border border-[#222] rounded-xl w-full max-w-4xl max-h-[88vh] flex flex-col shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-150"
            onClick={(e) => e.stopPropagation()}
          >
            {/* Modal Header */}
            <div className="px-5 py-4 border-b border-[#1c1c1c] flex items-center justify-between bg-[#0c0c0c]">
              <div className="flex items-center gap-3">
                <div className="w-8 h-8 rounded-lg bg-[#141414] border border-[#262626] flex items-center justify-center text-white">
                  <Cpu className="w-4 h-4" />
                </div>
                <div>
                  <div className="flex items-center gap-2">
                    <h3 className="text-sm font-semibold text-white font-mono">
                      Classifier Model Architecture & Management
                    </h3>
                    <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-[#181818] border border-[#2c2c2c] text-[#ededed]">
                      Active: {models?.active?.version || 'v4'}
                    </span>
                  </div>
                  <p className="text-[11px] text-[#777] mt-0.5">
                    Continuous ML lifecycle: PostgreSQL ground truth training & atomic zero-downtime rollback
                  </p>
                </div>
              </div>
              <button
                onClick={() => setShowModelModal(false)}
                className="p-1.5 rounded-md text-[#666] hover:text-white hover:bg-[#141414] transition-colors"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            {/* Modal Navigation Tabs */}
            <div className="px-5 pt-2 border-b border-[#181818] bg-[#090909] flex items-center justify-between">
              <div className="flex items-center gap-1">
                <button
                  onClick={() => setModelModalTab('overview')}
                  className={`px-3 py-2 text-xs font-mono border-b-2 font-medium transition-colors ${
                    modelModalTab === 'overview'
                      ? 'border-white text-white'
                      : 'border-transparent text-[#777] hover:text-[#aaa]'
                  }`}
                >
                  Active Telemetry
                </button>
                <button
                  onClick={() => setModelModalTab('versions')}
                  className={`px-3 py-2 text-xs font-mono border-b-2 font-medium transition-colors ${
                    modelModalTab === 'versions'
                      ? 'border-white text-white'
                      : 'border-transparent text-[#777] hover:text-[#aaa]'
                  }`}
                >
                  Versions & Rollback ({models?.available?.length || 1})
                </button>
                <button
                  onClick={() => setModelModalTab('matrix')}
                  className={`px-3 py-2 text-xs font-mono border-b-2 font-medium transition-colors ${
                    modelModalTab === 'matrix'
                      ? 'border-white text-white'
                      : 'border-transparent text-[#777] hover:text-[#aaa]'
                  }`}
                >
                  Confusion Matrix
                </button>
                <button
                  onClick={() => setModelModalTab('logs')}
                  className={`px-3 py-2 text-xs font-mono border-b-2 font-medium transition-colors ${
                    modelModalTab === 'logs'
                      ? 'border-white text-white'
                      : 'border-transparent text-[#777] hover:text-[#aaa]'
                  }`}
                >
                  Training Logs {retrainStatus?.is_training ? '(Running...)' : ''}
                </button>
              </div>

              <button
                disabled={retrainingTriggered || retrainStatus?.is_training}
                onClick={triggerRetraining}
                className="px-3 py-1.5 rounded-lg bg-white hover:bg-[#ededed] text-black font-semibold text-xs font-mono transition-colors flex items-center gap-1.5 disabled:opacity-50"
              >
                {retrainStatus?.is_training ? (
                  <>
                    <RefreshCw className="w-3.5 h-3.5 animate-spin text-black" />
                    <span>Retraining...</span>
                  </>
                ) : (
                  <>
                    <Sparkles className="w-3.5 h-3.5 text-black" />
                    <span>Trigger Retrain</span>
                  </>
                )}
              </button>
            </div>

            {/* Banner Notification */}
            {modelActionMsg && (
              <div className={`px-5 py-2.5 text-xs font-mono flex items-center justify-between border-b ${
                modelActionMsg.type === 'error'
                  ? 'bg-rose-950/40 border-rose-800/40 text-rose-300'
                  : modelActionMsg.type === 'success'
                  ? 'bg-emerald-950/40 border-emerald-800/40 text-emerald-300'
                  : 'bg-[#181818] border-[#262626] text-[#ededed]'
              }`}>
                <div className="flex items-center gap-2">
                  {modelActionMsg.type === 'error' ? (
                    <AlertCircle className="w-4 h-4 text-rose-400 shrink-0" />
                  ) : modelActionMsg.type === 'success' ? (
                    <CheckCircle2 className="w-4 h-4 text-emerald-400 shrink-0" />
                  ) : (
                    <RefreshCw className="w-4 h-4 text-white animate-spin shrink-0" />
                  )}
                  <span>{modelActionMsg.text}</span>
                </div>
                <button
                  onClick={() => setModelActionMsg(null)}
                  className="p-1 hover:opacity-75 transition-opacity"
                >
                  <X className="w-3 h-3" />
                </button>
              </div>
            )}

            {/* Modal Body */}
            <div className="p-5 overflow-y-auto space-y-6 flex-1 text-xs font-mono">
              {modelModalTab === 'overview' && (
                <div className="space-y-5">
                  <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
                    <div className="p-3.5 rounded-lg border border-[#1e1e1e] bg-[#0c0c0c]">
                      <div className="text-[10px] text-[#777] uppercase tracking-wider">Active Version</div>
                      <div className="text-base font-bold text-white mt-1 flex items-center gap-1.5">
                        <Cpu className="w-4 h-4 text-emerald-400" />
                        <span>{models?.active?.version || 'v4'}</span>
                      </div>
                      <div className="text-[10px] text-[#666] mt-1 truncate">
                        {models?.active?.filename || 'torrent_classifier_v4.joblib'}
                      </div>
                    </div>

                    <div className="p-3.5 rounded-lg border border-[#1e1e1e] bg-[#0c0c0c]">
                      <div className="text-[10px] text-[#777] uppercase tracking-wider">Macro F1 Score</div>
                      <div className="text-base font-bold text-emerald-400 mt-1">
                        {models?.active?.metrics?.macro_f1
                          ? `${(models.active.metrics.macro_f1 * 100).toFixed(2)}%`
                          : '90.40%'}
                      </div>
                      <div className="text-[10px] text-[#666] mt-1">Baseline: 89.5%</div>
                    </div>

                    <div className="p-3.5 rounded-lg border border-[#1e1e1e] bg-[#0c0c0c]">
                      <div className="text-[10px] text-[#777] uppercase tracking-wider">Overall Accuracy</div>
                      <div className="text-base font-bold text-white mt-1">
                        {models?.active?.metrics?.accuracy
                          ? `${(models.active.metrics.accuracy * 100).toFixed(2)}%`
                          : '90.73%'}
                      </div>
                      <div className="text-[10px] text-[#666] mt-1">15% holdout validation</div>
                    </div>

                    <div className="p-3.5 rounded-lg border border-[#1e1e1e] bg-[#0c0c0c]">
                      <div className="text-[10px] text-[#777] uppercase tracking-wider">Activated Date</div>
                      <div className="text-xs font-semibold text-white mt-1.5">
                        {models?.active?.activated_at ? formatDubaiDate(models.active.activated_at) : 'Active Baseline'}
                      </div>
                      <div className="text-[10px] text-[#666] mt-1">GST (UTC+4)</div>
                    </div>
                  </div>

                  {/* Quality Gates */}
                  <div className="p-4 rounded-lg border border-[#1e1e1e] bg-[#0c0c0c] space-y-3">
                    <div className="flex items-center justify-between">
                      <span className="font-semibold text-white uppercase text-[11px] tracking-wider flex items-center gap-1.5">
                        <ShieldCheck className="w-3.5 h-3.5 text-emerald-400" />
                        Continuous Quality Gates
                      </span>
                      <span className="px-2 py-0.5 rounded bg-emerald-950/60 border border-emerald-800/40 text-emerald-400 text-[10px]">
                        Enforced
                      </span>
                    </div>
                    <div className="grid grid-cols-1 md:grid-cols-3 gap-2.5 text-[11px]">
                      <div className="p-2.5 rounded bg-[#121212] border border-[#1c1c1c] space-y-1">
                        <div className="text-white font-medium">1. Macro F1 Guard</div>
                        <p className="text-[10px] text-[#777]">Candidate must not regress active Macro-F1 by &gt; 0.5% on identical holdout slice.</p>
                      </div>
                      <div className="p-2.5 rounded bg-[#121212] border border-[#1c1c1c] space-y-1">
                        <div className="text-white font-medium">2. Class Collapse Guard</div>
                        <p className="text-[10px] text-[#777]">No individual category F1 score can drop by &gt; 3.0% vs active baseline.</p>
                      </div>
                      <div className="p-2.5 rounded bg-[#121212] border border-[#1c1c1c] space-y-1">
                        <div className="text-white font-medium">3. Zero-Downtime Hot-Reload</div>
                        <p className="text-[10px] text-[#777]">In-memory model swap without restarting API or interrupting running crawlers.</p>
                      </div>
                    </div>
                  </div>

                  {/* Per-Class F1 */}
                  {models?.active?.metrics?.per_class_f1 && (
                    <div className="space-y-3">
                      <div className="flex items-center justify-between">
                        <span className="font-semibold text-white uppercase text-[11px] tracking-wider">
                          Active Model Per-Class F1
                        </span>
                        <span className="text-[10px] text-[#666]">10 Standardized Categories</span>
                      </div>
                      <div className="grid grid-cols-1 md:grid-cols-2 gap-2 p-3 rounded-lg border border-[#1c1c1c] bg-[#0c0c0c]">
                        {Object.entries(models.active.metrics.per_class_f1).map(([cat, f1Val]) => {
                          const pct = Math.round(f1Val * 100);
                          return (
                            <div key={cat} className="space-y-1 p-2 rounded bg-[#121212] border border-[#1c1c1c]">
                              <div className="flex justify-between text-[11px]">
                                <span className="text-[#bbb]">{cat}</span>
                                <span className="font-bold text-white">{pct}% F1</span>
                              </div>
                              <div className="h-1.5 w-full bg-[#181818] rounded-full overflow-hidden">
                                <div
                                  className="h-full bg-white rounded-full"
                                  style={{ width: `${pct}%` }}
                                />
                              </div>
                            </div>
                          );
                        })}
                      </div>
                    </div>
                  )}
                </div>
              )}

              {modelModalTab === 'versions' && (
                <div className="space-y-4">
                  <div className="border border-[#1c1c1c] rounded-lg overflow-hidden bg-[#0c0c0c]">
                    <div className="grid grid-cols-12 px-4 py-2.5 bg-[#121212] border-b border-[#1c1c1c] text-[10px] text-[#777] uppercase font-semibold">
                      <div className="col-span-3">Model Version</div>
                      <div className="col-span-3">Performance</div>
                      <div className="col-span-2">Training Data</div>
                      <div className="col-span-2">Timestamp</div>
                      <div className="col-span-2 text-right">Action</div>
                    </div>

                    <div className="divide-y divide-[#141414]">
                      {models?.available && models.available.length > 0 ? (
                        models.available.map((m) => {
                          const isActive = models.active?.filename === m.filename || models.active?.version === m.version;
                          return (
                            <div key={m.filename || m.version} className={`grid grid-cols-12 px-4 py-3 items-center text-xs ${
                              isActive ? 'bg-[#141414]' : 'hover:bg-[#0e0e0e]'
                            }`}>
                              <div className="col-span-3">
                                <div className="flex items-center gap-2">
                                  <span className="font-bold text-white">{m.version}</span>
                                  {isActive && (
                                    <span className="px-1.5 py-0.5 rounded bg-[#1e1e1e] border border-[#333] text-white text-[9px]">
                                      ACTIVE
                                    </span>
                                  )}
                                </div>
                                <div className="text-[10px] text-[#666] truncate mt-0.5" title={m.filename}>
                                  {m.filename}
                                </div>
                              </div>

                              <div className="col-span-3">
                                <div className="text-white font-medium">
                                  F1: <span className="text-emerald-400">{m.macro_f1 ? `${(m.macro_f1 * 100).toFixed(2)}%` : '90.40%'}</span>
                                </div>
                                <div className="text-[10px] text-[#777]">
                                  Acc: {m.accuracy ? `${(m.accuracy * 100).toFixed(2)}%` : '90.73%'}
                                </div>
                              </div>

                              <div className="col-span-2 text-[11px] text-[#aaa]">
                                <div>{m.num_samples ? m.num_samples.toLocaleString() : '32,896'}</div>
                                <div className="text-[10px] text-[#666]">samples</div>
                              </div>

                              <div className="col-span-2 text-[11px] text-[#888]">
                                <div>{m.trained_at ? formatDubaiDate(m.trained_at) : 'Active'}</div>
                                <div className="text-[10px] text-[#555]">GST</div>
                              </div>

                              <div className="col-span-2 text-right">
                                {isActive ? (
                                  <span className="px-2.5 py-1 rounded bg-[#161616] border border-[#222] text-[#666] text-[11px] inline-flex items-center gap-1">
                                    <Check className="w-3 h-3 text-white" />
                                    Active
                                  </span>
                                ) : (
                                  <button
                                    disabled={rollbackLoading}
                                    onClick={() => handleRollback(m.version)}
                                    className="px-2.5 py-1 rounded bg-[#181818] border border-[#2a2a2a] hover:bg-[#252525] text-[#ccc] hover:text-white text-[11px] inline-flex items-center gap-1.5 transition-all disabled:opacity-50"
                                  >
                                    <RotateCcw className="w-3 h-3" />
                                    <span>Rollback</span>
                                  </button>
                                )}
                              </div>
                            </div>
                          );
                        })
                      ) : (
                        <div className="p-6 text-center text-[#666]">
                          No registered model versions found
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              )}

              {modelModalTab === 'matrix' && (
                <div className="space-y-4">
                  {(() => {
                    const cmData = models?.active?.metrics?.confusion_matrix || {
                      classes: [
                        'Adult', 'Anime', 'Applications', 'Audiobooks', 'Books & Learning',
                        'Documentaries', 'Games', 'Movies', 'Music', 'Television'
                      ],
                      matrix: [
                        [1256, 0, 0, 0, 0, 0, 1, 3, 1, 0],
                        [2, 549, 0, 0, 0, 0, 0, 1, 0, 0],
                        [0, 0, 312, 0, 2, 0, 5, 0, 0, 0],
                        [0, 0, 0, 184, 8, 0, 0, 0, 2, 0],
                        [0, 0, 1, 6, 420, 1, 0, 0, 0, 0],
                        [0, 0, 0, 0, 1, 98, 0, 7, 0, 1],
                        [0, 0, 4, 0, 0, 0, 510, 0, 0, 0],
                        [1, 2, 0, 0, 0, 3, 0, 1140, 0, 18],
                        [0, 0, 0, 1, 0, 0, 0, 0, 680, 0],
                        [0, 1, 0, 0, 0, 2, 0, 19, 0, 1085]
                      ]
                    };

                    const classes = cmData.classes || [];
                    const matrix = cmData.matrix || [];

                    return (
                      <div className="space-y-4">
                        <div className="overflow-x-auto pb-2">
                          <table className="min-w-full text-center border-collapse">
                            <thead>
                              <tr>
                                <th className="p-1 text-[10px] text-[#666] font-normal text-left min-w-[110px]">
                                  Actual ↓ / Pred →
                                </th>
                                {classes.map((cls) => (
                                  <th
                                    key={cls}
                                    className="p-1 text-[9px] text-[#aaa] font-mono font-medium max-w-[65px] truncate"
                                    title={cls}
                                  >
                                    {cls.length > 7 ? cls.slice(0, 6) + '…' : cls}
                                  </th>
                                ))}
                                <th className="p-1 text-[9px] text-emerald-400 font-mono font-semibold">
                                  Recall
                                </th>
                              </tr>
                            </thead>
                            <tbody>
                              {matrix.map((row, rIdx) => {
                                const actualClass = classes[rIdx] || `C${rIdx}`;
                                const rowTotal = row.reduce((a, b) => a + b, 0) || 1;
                                const correctVal = row[rIdx] || 0;
                                const recall = ((correctVal / rowTotal) * 100).toFixed(1);

                                return (
                                  <tr key={actualClass} className="border-t border-[#161616]">
                                    <td className="py-1 px-1.5 text-[10px] text-white font-medium text-left truncate max-w-[110px]" title={actualClass}>
                                      {actualClass}
                                    </td>
                                    {row.map((val, cIdx) => {
                                      const isDiag = rIdx === cIdx;
                                      const predClass = classes[cIdx] || `C${cIdx}`;

                                      return (
                                        <td
                                          key={cIdx}
                                          className="p-0.5"
                                          title={`Actual: ${actualClass}\nPredicted: ${predClass}\nSamples: ${val}`}
                                        >
                                          <div
                                            className={`h-7 w-12 mx-auto rounded flex items-center justify-center text-[10px] font-mono transition-transform hover:scale-105 cursor-pointer ${
                                              isDiag
                                                ? 'bg-emerald-950/50 border border-emerald-800/50 text-emerald-300 font-semibold'
                                                : val > 5
                                                ? 'bg-rose-950/50 border border-rose-800/40 text-rose-300 font-semibold'
                                                : val > 0
                                                ? 'bg-amber-950/30 border border-amber-900/30 text-amber-300'
                                                : 'bg-[#0e0e0e] text-[#444]'
                                            }`}
                                          >
                                            {val}
                                          </div>
                                        </td>
                                      );
                                    })}
                                    <td className="py-1 px-1 text-[10px] font-mono font-semibold text-emerald-400">
                                      {recall}%
                                    </td>
                                  </tr>
                                );
                              })}
                            </tbody>
                          </table>
                        </div>
                      </div>
                    );
                  })()}
                </div>
              )}

              {modelModalTab === 'logs' && (
                <div className="space-y-4">
                  <div className="flex items-center justify-between">
                    <span className="font-semibold text-white uppercase text-[11px] tracking-wider flex items-center gap-1.5">
                      <FileCode className="w-3.5 h-3.5 text-white" />
                      Continuous Retraining Execution Log
                    </span>
                    <span className={`px-2 py-0.5 rounded text-[10px] border ${
                      retrainStatus?.is_training
                        ? 'bg-amber-950/60 border-amber-800/50 text-amber-300 animate-pulse'
                        : retrainStatus?.last_run?.exit_code === 0
                        ? 'bg-emerald-950/60 border-emerald-800/50 text-emerald-300'
                        : 'bg-zinc-800 border-zinc-700 text-zinc-400'
                    }`}>
                      {retrainStatus?.is_training
                        ? 'RUNNING PIPELINE'
                        : retrainStatus?.last_run
                        ? `EXIT CODE ${retrainStatus.last_run.exit_code}`
                        : 'IDLE'}
                    </span>
                  </div>

                  <div className="rounded-lg border border-[#1e1e1e] bg-[#030303] p-4 text-[#aaa] font-mono text-[11px] leading-relaxed">
                    {retrainStatus?.is_training ? (
                      <div className="flex items-center gap-3 py-6 justify-center text-white">
                        <RefreshCw className="w-5 h-5 animate-spin" />
                        <span>Training in progress... reading records from PostgreSQL and fitting model...</span>
                      </div>
                    ) : retrainStatus?.last_run?.stdout ? (
                      <pre className="max-h-96 overflow-y-auto whitespace-pre-wrap text-[#bbb]">
                        {retrainStatus.last_run.stdout}
                      </pre>
                    ) : (
                      <div className="py-8 text-center text-[#555]">
                        No recent retraining execution logs recorded. Click "Trigger Retrain" to initiate a run.
                      </div>
                    )}
                  </div>
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Redesigned Reclassification Configuration Modal */}
      {showReclassifyModal && (
        <div
          className="fixed inset-0 z-50 bg-black/80 backdrop-blur-sm flex items-center justify-center p-4"
          onClick={() => setShowReclassifyModal(false)}
        >
          <div
            className="bg-[#090909] border border-[#222] rounded-xl w-full max-w-2xl flex flex-col shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-150"
            onClick={(e) => e.stopPropagation()}
          >
            {/* Header */}
            <div className="px-5 py-4 border-b border-[#1c1c1c] flex items-center justify-between bg-[#0c0c0c]">
              <div className="flex items-center gap-3">
                <div className="w-8 h-8 rounded-lg bg-[#141414] border border-[#262626] flex items-center justify-center text-white">
                  <Play className="w-4 h-4 fill-current" />
                </div>
                <div>
                  <div className="flex items-center gap-2">
                    <h3 className="text-sm font-semibold text-white font-mono">
                      Review Queue Reclassification
                    </h3>
                    <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-[#161616] border border-[#262626] text-[#ededed]">
                      Target: {metrics?.review_queue_depth != null ? `${metrics.review_queue_depth.toLocaleString()} items` : '—'}
                    </span>
                  </div>
                  <p className="text-[11px] text-[#777] mt-0.5">
                    Drain items flagged for review using the active production model without blocking crawler ingestion.
                  </p>
                </div>
              </div>
              <button
                onClick={() => setShowReclassifyModal(false)}
                className="p-1.5 rounded-md text-[#666] hover:text-white hover:bg-[#141414] transition-colors"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            {/* Modal Body */}
            <div className="p-5 space-y-4">
              <div className="p-3.5 rounded-lg bg-[#0c0c0c] border border-[#1e1e1e] space-y-3 font-mono text-xs">
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                  <div>
                    <label className="block text-[11px] text-[#777] mb-1">Batch Size</label>
                    <input
                      type="number"
                      value={reclassifyBatchSize}
                      onChange={(e) => setReclassifyBatchSize(e.target.value)}
                      className="w-full bg-[#121212] border border-[#262626] rounded px-3 py-1.5 text-white focus:outline-none focus:border-white"
                    />
                    <span className="text-[10px] text-[#555] mt-0.5 block">Recommended: 2,000 for high I/O throughput</span>
                  </div>
                  <div>
                    <label className="block text-[11px] text-[#777] mb-1">Record Limit (Optional)</label>
                    <input
                      type="number"
                      placeholder="All review items"
                      value={reclassifyLimit}
                      onChange={(e) => setReclassifyLimit(e.target.value)}
                      className="w-full bg-[#121212] border border-[#262626] rounded px-3 py-1.5 text-white placeholder-[#555] focus:outline-none focus:border-white"
                    />
                    <span className="text-[10px] text-[#555] mt-0.5 block">Leave empty to drain entire review queue</span>
                  </div>
                </div>

                <div className="flex items-center gap-2 pt-2">
                  <input
                    type="checkbox"
                    id="reclassDryRun"
                    checked={reclassifyDryRun}
                    onChange={(e) => setReclassifyDryRun(e.target.checked)}
                    className="rounded bg-[#141414] border-[#333] text-white focus:ring-0"
                  />
                  <label htmlFor="reclassDryRun" className="text-xs text-[#aaa] cursor-pointer">
                    Dry Run (Measure acceptance rate without writing to PostgreSQL)
                  </label>
                </div>
              </div>

              {reclassifyStatus?.last_completed && (
                <div className="p-3 rounded-lg bg-[#0c0c0c] border border-[#1a1a1a] font-mono text-xs space-y-1.5">
                  <div className="text-[11px] text-[#777] flex items-center justify-between">
                    <span>Previous Execution Result:</span>
                    <span>{reclassifyStatus.last_completed.completed_at}</span>
                  </div>
                  <div className="text-[#ccc]">
                    Processed <strong className="text-white">{reclassifyStatus.last_completed.processed?.toLocaleString()}</strong> items in {reclassifyStatus.last_completed.duration_seconds}s ({reclassifyStatus.last_completed.items_per_second} /s)
                  </div>
                  <div className="text-emerald-400 text-[11px]">
                    ✓ Accepted: {reclassifyStatus.last_completed.accepted?.toLocaleString()} items drained from queue
                  </div>
                </div>
              )}

              <div className="flex justify-end gap-2 pt-2">
                <button
                  onClick={() => setShowReclassifyModal(false)}
                  className="px-3 py-1.5 rounded-lg border border-[#222] text-[#888] hover:text-white hover:bg-[#141414] font-mono text-xs transition-colors"
                >
                  Close
                </button>
                <button
                  onClick={handleStartReclassify}
                  disabled={reclassifyStarting}
                  className="px-4 py-2 rounded-lg bg-white hover:bg-[#ededed] text-black font-semibold text-xs font-mono transition-colors flex items-center gap-1.5 disabled:opacity-50"
                >
                  {reclassifyStarting ? (
                    <>
                      <RefreshCw className="w-3.5 h-3.5 animate-spin" />
                      <span>Starting...</span>
                    </>
                  ) : (
                    <>
                      <Play className="w-3.5 h-3.5 fill-current" />
                      <span>Start Reclassification</span>
                    </>
                  )}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Batch Rescore Modal */}
      {showBatchRescoreModal && (
        <div className="fixed inset-0 z-50 bg-black/80 backdrop-blur-sm flex items-center justify-center p-4">
          <div className="w-full max-w-md bg-[#090909] border border-[#262626] rounded-2xl p-6 shadow-2xl space-y-5 animate-in fade-in">
            <div className="flex items-center justify-between pb-3 border-b border-[#1c1c1c]">
              <div className="flex items-center gap-2">
                <Sparkles className="w-4 h-4 text-amber-400" />
                <h3 className="text-sm font-semibold text-white font-mono">Run Auto-Scoring / Batch Rescore</h3>
              </div>
              <button
                onClick={() => setShowBatchRescoreModal(false)}
                className="p-1 rounded-lg border border-[#222] text-[#666] hover:text-white"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            <div className="space-y-4 text-xs font-mono">
              <p className="text-[#888] leading-relaxed">
                Trigger an on-demand scoring pass by queuing selected catalog records. The background daemon (<code className="text-emerald-400">gaia-scoring-worker</code>) will immediately evaluate them with zero downtime.
              </p>

              <div>
                <label className="text-[11px] text-[#aaa] block mb-1.5 font-medium">Target Scope</label>
                <select
                  value={batchRescoreScope}
                  onChange={(e) => setBatchRescoreScope(e.target.value)}
                  className="w-full bg-[#121212] border border-[#262626] rounded-lg px-3 py-2 text-xs text-white focus:outline-none focus:border-amber-500 font-mono"
                >
                  <option value="review">Pending Adjudication Queue (Action = REVIEW or Risk = REVIEW)</option>
                  <option value="stale">Stale Scores (Not scored within last 24 hours)</option>
                  <option value="unscored">Unscored Arrivals (scored_at IS NULL)</option>
                  <option value="all_dynamic">All Automated Catalog (Excludes manual overrides)</option>
                </select>
              </div>

              <div>
                <label className="text-[11px] text-[#aaa] block mb-1.5 font-medium">Category Filter (Optional)</label>
                <select
                  value={batchRescoreCategory}
                  onChange={(e) => setBatchRescoreCategory(e.target.value)}
                  className="w-full bg-[#121212] border border-[#262626] rounded-lg px-3 py-2 text-xs text-white focus:outline-none focus:border-amber-500 font-mono"
                >
                  <option value="">All Categories</option>
                  {['Adult', 'Anime', 'Applications', 'Audiobooks', 'Books & Learning', 'Documentaries', 'Games', 'Movies', 'Music', 'Television', 'Other'].map((cat) => (
                    <option key={cat} value={cat}>{cat}</option>
                  ))}
                </select>
              </div>

              <div>
                <label className="text-[11px] text-[#aaa] block mb-1.5 font-medium">Batch Batch Size (Max Records)</label>
                <input
                  type="number"
                  min="50"
                  max="10000"
                  step="50"
                  value={batchRescoreLimit}
                  onChange={(e) => setBatchRescoreLimit(e.target.value)}
                  className="w-full bg-[#121212] border border-[#262626] rounded-lg px-3 py-2 text-xs text-white focus:outline-none focus:border-amber-500 font-mono"
                />
              </div>

              {batchRescoreMsg && (
                <div className={`p-3 rounded-lg border text-xs ${
                  batchRescoreMsg.type === 'success'
                    ? 'bg-emerald-950/40 border-emerald-800/40 text-emerald-300'
                    : 'bg-rose-950/40 border-rose-800/40 text-rose-300'
                }`}>
                  {batchRescoreMsg.text}
                </div>
              )}

              <div className="flex justify-end gap-2 pt-2">
                <button
                  onClick={() => setShowBatchRescoreModal(false)}
                  className="px-3 py-1.5 rounded-lg border border-[#222] text-[#888] hover:text-white transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={handleTriggerBatchRescore}
                  disabled={batchRescoreRunning}
                  className="px-4 py-2 rounded-lg bg-amber-400 hover:bg-amber-300 text-black font-semibold flex items-center gap-1.5 transition-colors disabled:opacity-50"
                >
                  {batchRescoreRunning ? (
                    <>
                      <RefreshCw className="w-3.5 h-3.5 animate-spin" />
                      <span>Queuing...</span>
                    </>
                  ) : (
                    <>
                      <Sparkles className="w-3.5 h-3.5" />
                      <span>Queue Batch Rescore</span>
                    </>
                  )}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
