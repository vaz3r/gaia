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
  const [activeSubTab, setActiveSubTab] = useState('trends'); // 'trends' | 'peer_geo' | 'survivability'
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
    const timer = setInterval(() => fetchAnalysis(selectedCategory), 120000); // 2 minutes
    return () => clearInterval(timer);
  }, [selectedCategory]);

  const handleCategorySelect = (cat) => {
    setSelectedCategory(cat);
  };

  const summary = data?.summary || {};
  const categories = data?.categories || [];

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
                onClick={() => setActiveSubTab('trends')}
                className={`px-3 py-1.5 rounded-lg text-xs font-medium transition-colors flex items-center gap-1.5 ${
                  activeSubTab === 'trends'
                    ? 'bg-[#1f1f1f] text-white border border-[#383838]'
                    : 'text-[#888] hover:text-[#eee] hover:bg-[#141414]'
                }`}
              >
                <BarChart3 className="w-3.5 h-3.5 text-cyan-400" />
                <span>Temporal Trends (7d)</span>
              </button>

              <button
                onClick={() => setActiveSubTab('peer_geo')}
                className={`px-3 py-1.5 rounded-lg text-xs font-medium transition-colors flex items-center gap-1.5 ${
                  activeSubTab === 'peer_geo'
                    ? 'bg-[#1f1f1f] text-white border border-[#383838]'
                    : 'text-[#888] hover:text-[#eee] hover:bg-[#141414]'
                }`}
              >
                <Users className="w-3.5 h-3.5 text-indigo-400" />
                <span>Peer Geography</span>
              </button>

              <button
                onClick={() => setActiveSubTab('survivability')}
                className={`px-3 py-1.5 rounded-lg text-xs font-medium transition-colors flex items-center gap-1.5 ${
                  activeSubTab === 'survivability'
                    ? 'bg-[#1f1f1f] text-white border border-[#383838]'
                    : 'text-[#888] hover:text-[#eee] hover:bg-[#141414]'
                }`}
              >
                <Shield className="w-3.5 h-3.5 text-emerald-400" />
                <span>Swarm Survivability</span>
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
        </div>

        {/* SUBTAB: TEMPORAL INGESTION TRENDS (7D) */}
        {activeSubTab === 'trends' && (
          <div className="p-5 space-y-4">
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 pb-2 border-b border-[#1c1c1c]">
              <div>
                <h4 className="text-sm font-semibold text-white flex items-center gap-2">
                  <BarChart3 className="w-4 h-4 text-cyan-400" />
                  Category Ingestion Velocity (Past 7 Days)
                </h4>
                <p className="text-xs text-[#777] mt-0.5">
                  Daily indexed torrent volume segmented across media categories.
                </p>
              </div>
              <div className="flex items-center gap-3 text-[11px] font-mono text-[#888]">
                <span className="flex items-center gap-1.5"><span className="w-2.5 h-2.5 rounded-xs bg-rose-500" /> Adult</span>
                <span className="flex items-center gap-1.5"><span className="w-2.5 h-2.5 rounded-xs bg-purple-500" /> Television</span>
                <span className="flex items-center gap-1.5"><span className="w-2.5 h-2.5 rounded-xs bg-blue-500" /> Movies</span>
                <span className="flex items-center gap-1.5"><span className="w-2.5 h-2.5 rounded-xs bg-cyan-500" /> Music</span>
                <span className="flex items-center gap-1.5"><span className="w-2.5 h-2.5 rounded-xs bg-pink-500" /> Anime</span>
              </div>
            </div>

            {/* Ingestion Timeline Chart Representation */}
            {(() => {
              const trends = data?.trends_7d || [];
              // Group by day
              const daysMap = {};
              trends.forEach((t) => {
                const dayKey = t.day ? new Date(t.day).toISOString().split('T')[0] : 'Unknown';
                if (!daysMap[dayKey]) daysMap[dayKey] = {};
                daysMap[dayKey][t.category] = t.count;
              });

              const days = Object.keys(daysMap).sort();
              if (days.length === 0) {
                return (
                  <div className="p-8 text-center text-[#666] font-mono text-xs">
                    No ingestion trend points recorded in the last 7 days.
                  </div>
                );
              }

              // Compute max daily count
              let maxDayTotal = 1;
              days.forEach(d => {
                const total = Object.values(daysMap[d]).reduce((a, b) => a + b, 0);
                if (total > maxDayTotal) maxDayTotal = total;
              });

              return (
                <div className="space-y-3 font-mono text-xs">
                  {days.map((d) => {
                    const catObj = daysMap[d];
                    const dayTotal = Object.values(catObj).reduce((a, b) => a + b, 0);

                    return (
                      <div key={d} className="p-3 rounded-lg border border-[#181818] bg-[#070707] space-y-2">
                        <div className="flex items-center justify-between text-xs">
                          <span className="text-white font-semibold flex items-center gap-2">
                            <Clock className="w-3.5 h-3.5 text-cyan-400" />
                            {d}
                          </span>
                          <span className="text-[#aaa] font-bold">
                            {dayTotal.toLocaleString()} releases verified
                          </span>
                        </div>

                        {/* Stacked Proportional Bar */}
                        <div className="h-3 w-full bg-[#141414] rounded overflow-hidden flex">
                          {Object.entries(catObj).map(([cat, cnt]) => {
                            const pct = ((cnt / dayTotal) * 100).toFixed(1);
                            const theme = CATEGORY_THEMES[cat] || CATEGORY_THEMES.Other;
                            return (
                              <div
                                key={cat}
                                className={`h-full ${theme.bar} transition-all hover:opacity-80`}
                                style={{ width: `${pct}%` }}
                                title={`${cat}: ${cnt.toLocaleString()} (${pct}%)`}
                              />
                            );
                          })}
                        </div>

                        {/* Category Badges for Day */}
                        <div className="flex flex-wrap gap-2 text-[10px] pt-1">
                          {Object.entries(catObj)
                            .sort((a, b) => b[1] - a[1])
                            .slice(0, 6)
                            .map(([cat, cnt]) => {
                              const theme = CATEGORY_THEMES[cat] || CATEGORY_THEMES.Other;
                              return (
                                <span key={cat} className={`px-2 py-0.5 rounded border ${theme.badge} flex items-center gap-1`}>
                                  <span>{cat}:</span>
                                  <span className="font-semibold text-white">{cnt.toLocaleString()}</span>
                                </span>
                              );
                            })}
                        </div>
                      </div>
                    );
                  })}
                </div>
              );
            })()}
          </div>
        )}

        {/* SUBTAB: SWARM PEER GEOGRAPHY & ASNS */}
        {activeSubTab === 'peer_geo' && (
          <div className="p-5 space-y-4 font-mono text-xs">
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 pb-2 border-b border-[#1c1c1c]">
              <div>
                <h4 className="text-sm font-semibold text-white flex items-center gap-2">
                  <Users className="w-4 h-4 text-indigo-400" />
                  Swarm Peer Topology & Autonomous Systems (ASNs)
                </h4>
                <p className="text-xs text-[#777] mt-0.5">
                  Top seeding clusters and datacenter networks verified through BitTorrent BEP 9/10 metadata handshakes.
                </p>
              </div>
              <div className="text-[11px] text-[#888]">
                Cluster source: <code className="text-cyan-400">stable_peers</code>
              </div>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
              {(data?.peer_geography || []).map((peer, idx) => {
                const totalPeers = (data?.peer_geography || []).reduce((a, b) => a + b.peer_count, 0) || 1;
                const pct = ((peer.peer_count / totalPeers) * 100).toFixed(1);

                return (
                  <div key={peer.prefix} className="p-3.5 rounded-lg border border-[#1c1c1c] bg-[#070707] space-y-2 hover:border-[#282828] transition-colors">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-2">
                        <span className="text-base">{peer.flag}</span>
                        <div>
                          <span className="font-bold text-white text-xs">{peer.country}</span>
                          <span className="text-[10px] text-[#666] ml-1.5 font-mono">({peer.country_code})</span>
                        </div>
                      </div>
                      <div className="text-right">
                        <span className="font-bold text-emerald-400 text-xs">{peer.peer_count.toLocaleString()}</span>
                        <span className="text-[10px] text-[#777] ml-1">peers</span>
                      </div>
                    </div>

                    <div className="text-[10px] text-[#aaa] truncate" title={peer.asn}>
                      {peer.asn}
                    </div>

                    <div className="space-y-1">
                      <div className="flex justify-between text-[10px] text-[#666]">
                        <span>Subnet Prefix: {peer.prefix}.0.0.0/8</span>
                        <span>{pct}% share</span>
                      </div>
                      <div className="h-1.5 w-full bg-[#161616] rounded-full overflow-hidden">
                        <div
                          className="h-full bg-indigo-500 rounded-full"
                          style={{ width: `${pct}%` }}
                        />
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {/* SUBTAB: SWARM HALF-LIFE & SURVIVABILITY */}
        {activeSubTab === 'survivability' && (
          <div className="p-5 space-y-4 font-mono text-xs">
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 pb-2 border-b border-[#1c1c1c]">
              <div>
                <h4 className="text-sm font-semibold text-white flex items-center gap-2">
                  <Shield className="w-4 h-4 text-emerald-400" />
                  Category Swarm Half-Life & Seed Retention
                </h4>
                <p className="text-xs text-[#777] mt-0.5">
                  Percentage of torrent releases retaining active seeding nodes across catalog longevity.
                </p>
              </div>
              <div className="text-[11px] text-emerald-400 bg-emerald-950/40 px-2.5 py-1 rounded border border-emerald-800/40">
                Retention Benchmark: &gt; 80% Healthy
              </div>
            </div>

            <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
              {(data?.survivability || []).map((s) => {
                const theme = CATEGORY_THEMES[s.category] || CATEGORY_THEMES.Other;
                return (
                  <div key={s.category} className="p-3.5 rounded-lg border border-[#1b1b1b] bg-[#070707] space-y-2.5">
                    <div className="flex items-center justify-between">
                      <span className={`px-2 py-0.5 rounded text-[10px] font-semibold border ${theme.badge}`}>
                        {s.category}
                      </span>
                      <span className="text-sm font-bold text-emerald-400">
                        {s.survivability_pct}% Active
                      </span>
                    </div>

                    <div className="h-1.5 w-full bg-[#181818] rounded-full overflow-hidden">
                      <div
                        className="h-full bg-emerald-500 rounded-full"
                        style={{ width: `${s.survivability_pct}%` }}
                      />
                    </div>

                    <div className="grid grid-cols-2 gap-2 text-[10px] pt-1 text-[#888] border-t border-[#141414]">
                      <div>
                        <span>Seeded Torrents:</span>
                        <div className="font-semibold text-white mt-0.5">{s.active_seed_torrents.toLocaleString()}</div>
                      </div>
                      <div>
                        <span>Total Catalog:</span>
                        <div className="font-semibold text-white mt-0.5">{s.total_torrents.toLocaleString()}</div>
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        )}

      </div>
    </div>
  );
}
