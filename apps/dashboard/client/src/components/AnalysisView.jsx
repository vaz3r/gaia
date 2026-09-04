import React, { useState, useEffect } from 'react';
import {
  Flame,
  Zap,
  TrendingUp,
  DownloadCloud,
  Eye,
  RefreshCw,
  Clock,
  Radio,
  BarChart3,
  Layers,
  ArrowUpRight,
  Shield,
  Copy,
  Check
} from 'lucide-react';
import { api, magnetFrom } from '../api.js';
import { formatBytes, formatNum, formatTime } from '../utils.js';

export default function AnalysisView({ onInspectTorrent, copyToClipboard }) {
  const [data, setData] = useState(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [activeSubTab, setActiveSubTab] = useState('trending'); // 'trending' | 'velocity' | 'top_swarms'
  const [copiedIh, setCopiedIh] = useState(null);

  const fetchAnalysis = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await api('/api/analysis');
      setData(res);
    } catch (err) {
      console.error('Failed to load analysis:', err);
      setError(err.message);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchAnalysis();
    const timer = setInterval(fetchAnalysis, 30000);
    return () => clearInterval(timer);
  }, []);

  const handleCopyMagnet = (t) => {
    const magnet = magnetFrom(t.infohash, t.name);
    copyToClipboard(magnet, 'magnet');
    setCopiedIh(t.infohash);
    setTimeout(() => setCopiedIh(null), 2000);
  };

  const summary = data?.summary || {};
  const currentList =
    activeSubTab === 'trending'
      ? data?.trending || []
      : activeSubTab === 'velocity'
      ? data?.fastest_growing || []
      : data?.top_swarms || [];

  return (
    <div className="space-y-6">
      {/* Telemetry Summary Cards */}
      <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-3">
        <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-3.5">
          <div className="text-[10px] text-[#666] uppercase font-sans flex items-center gap-1.5">
            <Radio className="w-3 h-3 text-cyan-400" /> Active (24h)
          </div>
          <div className="text-lg font-bold font-mono text-white mt-1">
            {(summary.active_swarms_24h || 0).toLocaleString()}
          </div>
          <div className="text-[10px] text-[#555] font-mono mt-0.5">Sighted recently</div>
        </div>

        <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-3.5">
          <div className="text-[10px] text-[#666] uppercase font-sans flex items-center gap-1.5">
            <Zap className="w-3 h-3 text-amber-400" /> New Releases (48h)
          </div>
          <div className="text-lg font-bold font-mono text-amber-400 mt-1">
            {(summary.fresh_swarms_48h || 0).toLocaleString()}
          </div>
          <div className="text-[10px] text-[#555] font-mono mt-0.5">Fresh DHT swarms</div>
        </div>

        <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-3.5">
          <div className="text-[10px] text-[#666] uppercase font-sans flex items-center gap-1.5">
            <Flame className="w-3 h-3 text-rose-400" /> High Activity
          </div>
          <div className="text-lg font-bold font-mono text-rose-400 mt-1">
            {(summary.high_activity_swarms || 0).toLocaleString()}
          </div>
          <div className="text-[10px] text-[#555] font-mono mt-0.5">≥10 cumulative sightings</div>
        </div>

        <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-3.5">
          <div className="text-[10px] text-[#666] uppercase font-sans flex items-center gap-1.5">
            <BarChart3 className="w-3 h-3 text-emerald-400" /> Avg Sightings
          </div>
          <div className="text-lg font-bold font-mono text-white mt-1">
            {summary.avg_sightings || 0}
          </div>
          <div className="text-[10px] text-[#555] font-mono mt-0.5">Per verified torrent</div>
        </div>

        <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-3.5">
          <div className="text-[10px] text-[#666] uppercase font-sans flex items-center gap-1.5">
            <TrendingUp className="w-3 h-3 text-indigo-400" /> Peak Sighting
          </div>
          <div className="text-lg font-bold font-mono text-indigo-400 mt-1">
            {summary.max_sightings || 0} hits
          </div>
          <div className="text-[10px] text-[#555] font-mono mt-0.5">Max swarm sightings</div>
        </div>

        <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-3.5">
          <div className="text-[10px] text-[#666] uppercase font-sans flex items-center gap-1.5">
            <Layers className="w-3 h-3 text-violet-400" /> Total Swarms
          </div>
          <div className="text-lg font-bold font-mono text-white mt-1">
            {((summary.total_torrents || 0) / 1000000).toFixed(2)}M
          </div>
          <div className="text-[10px] text-[#555] font-mono mt-0.5">Tracked in database</div>
        </div>
      </div>

      {/* Main Analysis Section */}
      <div className="rounded-xl border border-[#1e1e1e] bg-[#0a0a0a] overflow-hidden">
        {/* Controls & Sub-Navigation */}
        <div className="p-4 border-b border-[#1c1c1c] flex flex-col sm:flex-row sm:items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <button
              onClick={() => setActiveSubTab('trending')}
              className={`px-3 py-1.5 rounded-lg text-xs font-medium transition-colors flex items-center gap-1.5 ${
                activeSubTab === 'trending'
                  ? 'bg-[#1f1f1f] text-white border border-[#383838]'
                  : 'text-[#888] hover:text-[#eee] hover:bg-[#141414]'
              }`}
            >
              <Flame className="w-3.5 h-3.5 text-rose-400" />
              <span>Trending Swarms</span>
            </button>

            <button
              onClick={() => setActiveSubTab('velocity')}
              className={`px-3 py-1.5 rounded-lg text-xs font-medium transition-colors flex items-center gap-1.5 ${
                activeSubTab === 'velocity'
                  ? 'bg-[#1f1f1f] text-white border border-[#383838]'
                  : 'text-[#888] hover:text-[#eee] hover:bg-[#141414]'
              }`}
            >
              <Zap className="w-3.5 h-3.5 text-amber-400" />
              <span>Rising Velocity (&lt;48h)</span>
            </button>

            <button
              onClick={() => setActiveSubTab('top_swarms')}
              className={`px-3 py-1.5 rounded-lg text-xs font-medium transition-colors flex items-center gap-1.5 ${
                activeSubTab === 'top_swarms'
                  ? 'bg-[#1f1f1f] text-white border border-[#383838]'
                  : 'text-[#888] hover:text-[#eee] hover:bg-[#141414]'
              }`}
            >
              <TrendingUp className="w-3.5 h-3.5 text-emerald-400" />
              <span>Top Swarms (All-Time)</span>
            </button>
          </div>

          <div className="flex items-center gap-2">
            <button
              onClick={fetchAnalysis}
              disabled={loading}
              className="px-2.5 py-1.5 rounded-lg border border-[#222] bg-[#111] text-[#888] hover:text-white text-xs flex items-center gap-1.5 disabled:opacity-50"
              title="Refresh telemetry"
            >
              <RefreshCw className={`w-3 h-3 ${loading ? 'animate-spin text-white' : ''}`} />
              <span>Refresh</span>
            </button>
          </div>
        </div>

        {/* Table Content */}
        <div className="overflow-x-auto">
          <table className="w-full text-left text-xs font-mono">
            <thead className="border-b border-[#1c1c1c] bg-[#070707] text-[#666]">
              <tr>
                <th className="py-2.5 px-4 font-normal">#</th>
                <th className="py-2.5 px-4 font-normal">Release Name</th>
                <th className="py-2.5 px-4 font-normal">Size</th>
                <th className="py-2.5 px-4 font-normal">Sightings</th>
                <th className="py-2.5 px-4 font-normal">
                  {activeSubTab === 'velocity' ? 'Velocity' : 'Trending Metric'}
                </th>
                <th className="py-2.5 px-4 font-normal">Discovered</th>
                <th className="py-2.5 px-4 font-normal text-right">Actions</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[#141414]">
              {loading && !data ? (
                <tr>
                  <td colSpan={7} className="py-12 text-center text-[#666]">
                    <div className="flex items-center justify-center gap-2">
                      <RefreshCw className="w-4 h-4 animate-spin" />
                      <span>Computing telemetry & swarm metrics...</span>
                    </div>
                  </td>
                </tr>
              ) : error ? (
                <tr>
                  <td colSpan={7} className="py-8 text-center text-rose-400">
                    {error}
                  </td>
                </tr>
              ) : currentList.length === 0 ? (
                <tr>
                  <td colSpan={7} className="py-8 text-center text-[#666]">
                    No torrents found in this category.
                  </td>
                </tr>
              ) : (
                currentList.map((t, idx) => {
                  const firstSeenDate = t.first_seen ? new Date(t.first_seen) : null;
                  const hoursAgo = firstSeenDate
                    ? Math.max(0.1, (Date.now() - firstSeenDate.getTime()) / 3600000).toFixed(1)
                    : null;

                  return (
                    <tr
                      key={t.infohash}
                      className="hover:bg-[#111] transition-colors cursor-pointer group"
                      onClick={() => onInspectTorrent(t)}
                    >
                      <td className="py-3 px-4 text-[#555] w-8">{idx + 1}</td>

                      <td className="py-3 px-4 max-w-xs sm:max-w-md truncate">
                        <div className="font-semibold text-white truncate group-hover:text-emerald-400 transition-colors">
                          {t.name || `payload-${t.infohash.slice(0, 10)}`}
                        </div>
                        <div className="text-[10px] text-[#555] font-mono mt-0.5 truncate">
                          {t.infohash}
                        </div>
                      </td>

                      <td className="py-3 px-4 text-[#888] whitespace-nowrap">
                        {formatBytes(t.total_size)}
                      </td>

                      <td className="py-3 px-4 whitespace-nowrap">
                        <span className="px-2 py-0.5 rounded bg-[#161616] text-[#ccc] border border-[#262626]">
                          {t.total_seen} seen
                        </span>
                      </td>

                      <td className="py-3 px-4 whitespace-nowrap">
                        {activeSubTab === 'velocity' ? (
                          <div className="flex items-center gap-1.5 text-amber-400">
                            <Zap className="w-3 h-3" />
                            <span>+{t.velocity} / hr</span>
                          </div>
                        ) : activeSubTab === 'trending' ? (
                          <div className="flex items-center gap-1.5 text-rose-400">
                            <Flame className="w-3 h-3" />
                            <span>Score {t.trend_score}</span>
                          </div>
                        ) : (
                          <div className="flex items-center gap-1.5 text-emerald-400">
                            <TrendingUp className="w-3 h-3" />
                            <span>{t.total_seen} total</span>
                          </div>
                        )}
                      </td>

                      <td className="py-3 px-4 text-[#777] whitespace-nowrap">
                        <div className="flex items-center gap-1">
                          <Clock className="w-3 h-3 text-[#555]" />
                          <span>{hoursAgo ? `${hoursAgo}h ago` : '—'}</span>
                        </div>
                      </td>

                      <td className="py-3 px-4 text-right" onClick={(e) => e.stopPropagation()}>
                        <div className="flex items-center justify-end gap-1.5">
                          <button
                            onClick={() => handleCopyMagnet(t)}
                            className="p-1.5 rounded-md bg-[#141414] border border-[#262626] text-[#888] hover:text-white hover:border-[#444] transition-colors"
                            title="Copy Magnet"
                          >
                            {copiedIh === t.infohash ? (
                              <Check className="w-3.5 h-3.5 text-emerald-400" />
                            ) : (
                              <DownloadCloud className="w-3.5 h-3.5" />
                            )}
                          </button>
                          <button
                            onClick={() => onInspectTorrent(t)}
                            className="p-1.5 rounded-md bg-[#141414] border border-[#262626] text-[#888] hover:text-white hover:border-[#444] transition-colors"
                            title="Inspect Details"
                          >
                            <Eye className="w-3.5 h-3.5" />
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
      </div>
    </div>
  );
}
