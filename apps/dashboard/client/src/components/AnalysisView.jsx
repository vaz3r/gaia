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
  Check,
  Tag,
  PieChart,
  HardDrive,
  Users,
  Activity,
  Filter,
  CheckCircle2,
  AlertTriangle
} from 'lucide-react';
import { api, magnetFrom } from '../api.js';
import { formatBytes, formatNum, formatTime } from '../utils.js';

const CATEGORY_THEMES = {
  Adult: { badge: 'bg-rose-500/10 text-rose-400 border-rose-500/30', bar: 'bg-rose-500' },
  Anime: { badge: 'bg-pink-500/10 text-pink-400 border-pink-500/30', bar: 'bg-pink-500' },
  Applications: { badge: 'bg-amber-500/10 text-amber-400 border-amber-500/30', bar: 'bg-amber-500' },
  Audiobooks: { badge: 'bg-indigo-500/10 text-indigo-400 border-indigo-500/30', bar: 'bg-indigo-500' },
  'Books & Learning': { badge: 'bg-teal-500/10 text-teal-400 border-teal-500/30', bar: 'bg-teal-500' },
  Documentaries: { badge: 'bg-emerald-500/10 text-emerald-400 border-emerald-500/30', bar: 'bg-emerald-500' },
  Games: { badge: 'bg-lime-500/10 text-lime-400 border-lime-500/30', bar: 'bg-lime-500' },
  Movies: { badge: 'bg-blue-500/10 text-blue-400 border-blue-500/30', bar: 'bg-blue-500' },
  Music: { badge: 'bg-cyan-500/10 text-cyan-400 border-cyan-500/30', bar: 'bg-cyan-500' },
  Television: { badge: 'bg-purple-500/10 text-purple-400 border-purple-500/30', bar: 'bg-purple-500' },
  Other: { badge: 'bg-zinc-500/10 text-zinc-400 border-zinc-500/30', bar: 'bg-zinc-500' },
  Unclassified: { badge: 'bg-zinc-800/40 text-zinc-400 border-zinc-700/50', bar: 'bg-zinc-700' },
};

export default function AnalysisView({ onInspectTorrent, copyToClipboard }) {
  const [data, setData] = useState(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [activeSubTab, setActiveSubTab] = useState('trending'); // 'trending' | 'velocity' | 'top_swarms'
  const [selectedCategory, setSelectedCategory] = useState('All');
  const [copiedIh, setCopiedIh] = useState(null);

  const fetchAnalysis = async (category = selectedCategory) => {
    setLoading(true);
    setError(null);
    try {
      const catParam = category && category !== 'All' ? `?category=${encodeURIComponent(category)}` : '';
      const res = await api(`/api/analysis${catParam}`);
      setData(res);
    } catch (err) {
      console.error('Failed to load analysis:', err);
      setError(err.message);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchAnalysis(selectedCategory);
    const timer = setInterval(() => fetchAnalysis(selectedCategory), 30000);
    return () => clearInterval(timer);
  }, [selectedCategory]);

  const handleCategorySelect = (cat) => {
    setSelectedCategory(cat);
  };

  const handleCopyMagnet = (t) => {
    const magnet = magnetFrom(t.infohash, t.name);
    copyToClipboard(magnet, 'magnet');
    setCopiedIh(t.infohash);
    setTimeout(() => setCopiedIh(null), 2000);
  };

  const summary = data?.summary || {};
  const categories = data?.categories || [];
  const currentList =
    activeSubTab === 'trending'
      ? data?.trending || []
      : activeSubTab === 'velocity'
      ? data?.fastest_growing || []
      : data?.top_swarms || [];

  const classifiedPct = summary.total_torrents
    ? ((summary.classified_torrents / summary.total_torrents) * 100).toFixed(1)
    : '0.0';

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
            <Tag className="w-3 h-3 text-emerald-400" /> ML Classified
          </div>
          <div className="text-lg font-bold font-mono text-emerald-400 mt-1">
            {formatNum(summary.classified_torrents || 0)}
          </div>
          <div className="text-[10px] text-[#555] font-mono mt-0.5">{classifiedPct}% of full catalog</div>
        </div>

        <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-3.5">
          <div className="text-[10px] text-[#666] uppercase font-sans flex items-center gap-1.5">
            <HardDrive className="w-3 h-3 text-indigo-400" /> Total Footprint
          </div>
          <div className="text-lg font-bold font-mono text-indigo-400 mt-1">
            {summary.total_size_tb ? `${(summary.total_size_tb).toFixed(1)} TB` : '—'}
          </div>
          <div className="text-[10px] text-[#555] font-mono mt-0.5">Analyzed storage</div>
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

      {/* Content Landscape & Category Distribution Matrix */}
      {categories.length > 0 && (
        <div className="rounded-xl border border-[#1e1e1e] bg-[#0a0a0a] p-5 space-y-4">
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 border-b border-[#161616] pb-3">
            <div>
              <h3 className="text-sm font-semibold text-white font-mono flex items-center gap-2">
                <PieChart className="w-4 h-4 text-cyan-400" />
                <span>Content Category Landscape & Catalog Distribution</span>
              </h3>
              <p className="text-[11px] text-[#777] mt-0.5 font-mono">
                Multimodal classification breakdown across {summary.classified_torrents?.toLocaleString()} verified releases
              </p>
            </div>
            <div className="flex items-center gap-3 text-xs font-mono text-[#888]">
              <span>Review Queue: <strong className="text-amber-400 font-normal">{(summary.review_needed_torrents || 0).toLocaleString()}</strong></span>
              <span>·</span>
              <span>Categories: <strong className="text-white font-normal">{categories.length}</strong></span>
            </div>
          </div>

          {/* Stacked Distribution Proportional Bar */}
          <div className="space-y-1.5">
            <div className="h-3 w-full bg-[#141414] rounded-full overflow-hidden flex">
              {categories.map((c) => {
                const theme = CATEGORY_THEMES[c.category] || CATEGORY_THEMES.Other;
                return (
                  <div
                    key={c.category}
                    className={`h-full ${theme.bar} transition-all cursor-pointer hover:opacity-80`}
                    style={{ width: `${c.pct}%` }}
                    title={`${c.category}: ${c.count.toLocaleString()} (${c.pct}%)`}
                    onClick={() => handleCategorySelect(selectedCategory === c.category ? 'All' : c.category)}
                  />
                );
              })}
            </div>
            <div className="flex items-center justify-between text-[10px] font-mono text-[#555]">
              <span>Distribution across verified torrents</span>
              <span>Click category to isolate discovery below</span>
            </div>
          </div>

          {/* Category Cards Matrix */}
          <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 gap-2.5 pt-1">
            {categories.map((c) => {
              const theme = CATEGORY_THEMES[c.category] || CATEGORY_THEMES.Other;
              const isSelected = selectedCategory === c.category;

              return (
                <div
                  key={c.category}
                  onClick={() => handleCategorySelect(isSelected ? 'All' : c.category)}
                  className={`p-3 rounded-lg border transition-all cursor-pointer select-none font-mono ${
                    isSelected
                      ? 'border-cyan-500 bg-cyan-950/30 shadow-md ring-1 ring-cyan-500/50'
                      : 'border-[#1b1b1b] bg-[#070707] hover:border-[#2a2a2a] hover:bg-[#0c0c0c]'
                  }`}
                >
                  <div className="flex items-center justify-between gap-1">
                    <span className={`px-1.5 py-0.5 rounded text-[10px] font-semibold border ${theme.badge} truncate`}>
                      {c.category}
                    </span>
                    <span className="text-xs font-bold text-white">
                      {c.pct}%
                    </span>
                  </div>

                  <div className="mt-2.5 space-y-1 text-[10px]">
                    <div className="flex justify-between text-[#888]">
                      <span>Volume:</span>
                      <span className="text-white font-medium">{c.count.toLocaleString()}</span>
                    </div>
                    <div className="flex justify-between text-[#888]">
                      <span>Storage:</span>
                      <span className="text-indigo-300">{c.total_size_tb} TB</span>
                    </div>
                    <div className="flex justify-between text-[#888]">
                      <span>Avg Size:</span>
                      <span className="text-[#aaa]">{c.avg_size_gb} GB</span>
                    </div>
                    <div className="flex justify-between text-[#888] pt-1 border-t border-[#161616]">
                      <span>Avg Peers / Health:</span>
                      <span className="text-emerald-400">{c.avg_peers}p · {c.avg_health}%</span>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* Main Analysis Section */}
      <div className="rounded-xl border border-[#1e1e1e] bg-[#0a0a0a] overflow-hidden">
        {/* Controls & Sub-Navigation */}
        <div className="p-4 border-b border-[#1c1c1c] flex flex-col gap-3">
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
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
                onClick={() => fetchAnalysis(selectedCategory)}
                disabled={loading}
                className="px-2.5 py-1.5 rounded-lg border border-[#222] bg-[#111] text-[#888] hover:text-white text-xs flex items-center gap-1.5 disabled:opacity-50"
                title="Refresh telemetry"
              >
                <RefreshCw className={`w-3 h-3 ${loading ? 'animate-spin text-white' : ''}`} />
                <span>Refresh</span>
              </button>
            </div>
          </div>

          {/* Category Filter Pills */}
          <div className="flex items-center gap-1.5 flex-wrap pt-2 border-t border-[#141414]">
            <div className="flex items-center gap-1 text-[11px] font-mono text-[#666] mr-1">
              <Filter className="w-3 h-3" />
              <span>Filter:</span>
            </div>
            <button
              onClick={() => handleCategorySelect('All')}
              className={`px-2.5 py-1 rounded text-xs font-mono transition-colors ${
                selectedCategory === 'All'
                  ? 'bg-cyan-600 text-white font-semibold'
                  : 'bg-[#121212] text-[#888] hover:text-white border border-[#222]'
              }`}
            >
              All Categories
            </button>
            {categories.map((c) => {
              const isSelected = selectedCategory === c.category;
              return (
                <button
                  key={c.category}
                  onClick={() => handleCategorySelect(c.category)}
                  className={`px-2 py-1 rounded text-xs font-mono transition-colors flex items-center gap-1.5 ${
                    isSelected
                      ? 'bg-cyan-600 text-white font-semibold'
                      : 'bg-[#121212] text-[#888] hover:text-[#ccc] border border-[#202020]'
                  }`}
                >
                  <span>{c.category}</span>
                  <span className={`text-[10px] ${isSelected ? 'text-cyan-200' : 'text-[#555]'}`}>
                    {c.pct}%
                  </span>
                </button>
              );
            })}
          </div>
        </div>

        {/* Table Content */}
        <div className="overflow-x-auto">
          <table className="w-full text-left text-xs font-mono">
            <thead className="border-b border-[#1c1c1c] bg-[#070707] text-[#666]">
              <tr>
                <th className="py-2.5 px-4 font-normal">#</th>
                <th className="py-2.5 px-4 font-normal">Release Name</th>
                <th className="py-2.5 px-4 font-normal">Category</th>
                <th className="py-2.5 px-4 font-normal">Size</th>
                <th className="py-2.5 px-4 font-normal">Sightings</th>
                <th className="py-2.5 px-4 font-normal">
                  {activeSubTab === 'velocity' ? 'Velocity' : activeSubTab === 'trending' ? 'Trend Score' : 'Total Seen'}
                </th>
                <th className="py-2.5 px-4 font-normal">Swarm Health</th>
                <th className="py-2.5 px-4 font-normal">Discovered</th>
                <th className="py-2.5 px-4 font-normal text-right">Actions</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[#141414]">
              {loading && !data ? (
                <tr>
                  <td colSpan={9} className="py-12 text-center text-[#666]">
                    <div className="flex items-center justify-center gap-2">
                      <RefreshCw className="w-4 h-4 animate-spin" />
                      <span>Computing telemetry & swarm metrics...</span>
                    </div>
                  </td>
                </tr>
              ) : error ? (
                <tr>
                  <td colSpan={9} className="py-8 text-center text-rose-400">
                    {error}
                  </td>
                </tr>
              ) : currentList.length === 0 ? (
                <tr>
                  <td colSpan={9} className="py-8 text-center text-[#666]">
                    No torrents found in this category.
                  </td>
                </tr>
              ) : (
                currentList.map((t, idx) => {
                  const dateVal = t.verified_at || t.first_seen;
                  const dateObj = dateVal ? new Date(dateVal) : null;
                  const hoursAgo = dateObj
                    ? Math.max(0.1, (Date.now() - dateObj.getTime()) / 3600000).toFixed(1)
                    : null;
                  const theme = CATEGORY_THEMES[t.category] || CATEGORY_THEMES.Unclassified;

                  return (
                    <tr
                      key={t.infohash}
                      className="hover:bg-[#111] transition-colors cursor-pointer group"
                      onClick={() => onInspectTorrent(t)}
                    >
                      <td className="py-3 px-4 text-[#555] w-8">{idx + 1}</td>

                      <td className="py-3 px-4 max-w-xs sm:max-w-md truncate">
                        <div className="font-semibold text-white truncate group-hover:text-cyan-400 transition-colors">
                          {t.name || `payload-${t.infohash.slice(0, 10)}`}
                        </div>
                        <div className="text-[10px] text-[#555] font-mono mt-0.5 truncate">
                          {t.infohash}
                        </div>
                      </td>

                      <td className="py-3 px-4 whitespace-nowrap">
                        {t.category ? (
                          <span className={`px-2 py-0.5 rounded text-[11px] font-semibold border ${theme.badge} inline-flex items-center gap-1`}>
                            {t.category}
                            {t.category_confidence && (
                              <span className="text-[9px] opacity-75 font-normal">
                                {Math.round(t.category_confidence * 100)}%
                              </span>
                            )}
                          </span>
                        ) : (
                          <span className="px-2 py-0.5 rounded text-[11px] bg-zinc-900 text-zinc-500 border border-zinc-800">
                            Unclassified
                          </span>
                        )}
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
                          <div className="flex items-center gap-1.5 text-amber-400 font-medium">
                            <Zap className="w-3.5 h-3.5" />
                            <span>+{t.velocity} / hr</span>
                          </div>
                        ) : activeSubTab === 'trending' ? (
                          <div className="flex items-center gap-1.5 text-rose-400 font-medium">
                            <Flame className="w-3.5 h-3.5" />
                            <span>Score {t.trend_score}</span>
                          </div>
                        ) : (
                          <div className="flex items-center gap-1.5 text-emerald-400 font-medium">
                            <TrendingUp className="w-3.5 h-3.5" />
                            <span>{t.total_seen} total</span>
                          </div>
                        )}
                      </td>

                      <td className="py-3 px-4 whitespace-nowrap">
                        {t.health_score > 0 ? (
                          <span
                            className={`px-2 py-0.5 rounded text-[11px] font-medium border ${
                              t.health_score >= 70
                                ? 'bg-emerald-950/40 text-emerald-400 border-emerald-800/50'
                                : t.health_score >= 40
                                ? 'bg-amber-950/40 text-amber-400 border-amber-800/50'
                                : 'bg-rose-950/40 text-rose-400 border-rose-800/50'
                            }`}
                            title={`Health: ${t.health_score}%, Popularity: ${t.popularity_score || 0}%`}
                          >
                            {t.health_score}% ({t.swarm_peers || 0}p)
                          </span>
                        ) : (
                          <span className="text-[#555] text-[11px]">Unscraped</span>
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
