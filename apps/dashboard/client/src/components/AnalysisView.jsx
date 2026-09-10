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
  AlertTriangle,
  ChevronRight,
  Filter,
  Search,
  Sparkles,
  Globe
} from 'lucide-react';
import { api, magnetFrom } from '../api.js';
import { formatBytes, formatNum, formatTime } from '../utils.js';
import {
  ResponsiveContainer,
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
} from 'recharts';

const CATEGORY_THEMES = {
  Adult: { badge: 'bg-rose-500/10 text-rose-400 border-rose-500/30', bar: 'bg-rose-500', text: 'text-rose-400' },
  Anime: { badge: 'bg-pink-500/10 text-pink-400 border-pink-500/30', bar: 'bg-pink-500', text: 'text-pink-400' },
  Applications: { badge: 'bg-amber-500/10 text-amber-400 border-amber-500/30', bar: 'bg-amber-500', text: 'text-amber-400' },
  Audiobooks: { badge: 'bg-indigo-500/10 text-indigo-400 border-indigo-500/30', bar: 'bg-indigo-500', text: 'text-indigo-400' },
  'Books & Learning': { badge: 'bg-teal-500/10 text-teal-400 border-teal-500/30', bar: 'bg-teal-500', text: 'text-teal-400' },
  Documentaries: { badge: 'bg-emerald-500/10 text-emerald-400 border-emerald-500/30', bar: 'bg-emerald-500', text: 'text-emerald-400' },
  Games: { badge: 'bg-lime-500/10 text-lime-400 border-lime-500/30', bar: 'bg-lime-500', text: 'text-lime-400' },
  Movies: { badge: 'bg-blue-500/10 text-blue-400 border-blue-500/30', bar: 'bg-blue-500', text: 'text-blue-400' },
  Music: { badge: 'bg-cyan-500/10 text-cyan-400 border-cyan-500/30', bar: 'bg-cyan-500', text: 'text-cyan-400' },
  Television: { badge: 'bg-purple-500/10 text-purple-400 border-purple-500/30', bar: 'bg-purple-500', text: 'text-purple-400' },
  Other: { badge: 'bg-zinc-500/10 text-zinc-400 border-zinc-500/30', bar: 'bg-zinc-500', text: 'text-zinc-400' },
  Unclassified: { badge: 'bg-zinc-800/40 text-zinc-400 border-zinc-700/50', bar: 'bg-zinc-700', text: 'text-zinc-400' },
};

export default function AnalysisView({ onInspectTorrent, copyToClipboard }) {
  const [data, setData] = useState(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [activeSubTab, setActiveSubTab] = useState('trends'); // 'trends' | 'peer_geo' | 'survivability'
  const [selectedCategory, setSelectedCategory] = useState('All');
  const [copiedHash, setCopiedHash] = useState(null);

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
    const timer = setInterval(() => fetchAnalysis(selectedCategory), 60000);
    return () => clearInterval(timer);
  }, [selectedCategory]);

  const handleCategorySelect = (cat) => {
    setSelectedCategory(cat);
  };

  const handleCopy = (text, type = 'hash') => {
    if (copyToClipboard) {
      copyToClipboard(text, type);
    } else {
      navigator.clipboard.writeText(text);
    }
    setCopiedHash(text);
    setTimeout(() => setCopiedHash(null), 2000);
  };

  const summary = data?.summary || {};
  const categories = data?.categories || [];

  const classifiedPct = summary.total_torrents
    ? ((summary.classified_torrents / summary.total_torrents) * 100).toFixed(1)
    : '0.0';

  return (
    <div className="space-y-6">
      {/* Top Telemetry Header & Status Summary */}
      <section className="rounded-xl border border-[#222] bg-[#090909] p-4 flex flex-col md:flex-row md:items-center justify-between gap-4">
        <div className="flex items-start md:items-center gap-3">
          <div className="w-7 h-7 rounded-lg bg-[#141414] border border-[#262626] flex items-center justify-center shrink-0">
            <Sparkles className="w-4 h-4 text-cyan-400" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold text-white tracking-tight">Content & Swarm Intelligence Matrix</span>
              <span className="text-[11px] px-2 py-0.5 rounded-full bg-cyan-950/60 border border-cyan-800/60 text-cyan-400 font-mono">
                {selectedCategory === 'All' ? 'All 10 Categories' : selectedCategory}
              </span>
              {selectedCategory !== 'All' && (
                <button
                  onClick={() => handleCategorySelect('All')}
                  className="text-[11px] text-[#888] hover:text-white underline font-mono ml-1"
                >
                  Reset Filter
                </button>
              )}
            </div>
            <p className="text-xs text-[#888] mt-0.5 leading-relaxed">
              Real-time indexing telemetry across {summary.total_torrents?.toLocaleString() || '2.76M+'} torrents, {summary.total_size_tb ? `${(summary.total_size_tb / 1024).toFixed(2)} PB` : '16 PB'} verified payload storage, and DHT swarm retention.
            </p>
          </div>
        </div>

        <div className="flex items-center gap-3">
          <button
            onClick={() => fetchAnalysis(selectedCategory)}
            disabled={loading}
            className="px-2.5 py-1 rounded-md border border-[#222] bg-[#111] hover:bg-[#161616] text-[#888] hover:text-white text-xs font-mono transition-colors flex items-center gap-1.5 disabled:opacity-50"
          >
            <RefreshCw className={`w-3 h-3 ${loading ? 'animate-spin text-cyan-400' : ''}`} />
            <span>Refresh</span>
          </button>
        </div>
      </section>

      {/* SECTION 1: Telemetry KPI Pulse Cards */}
      <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-3">
        <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-3.5">
          <div className="text-[10px] text-[#666] uppercase font-mono flex items-center gap-1.5">
            <Radio className="w-3 h-3 text-cyan-400" /> Active (24h)
          </div>
          <div className="text-lg font-bold font-mono text-white mt-1">
            {(summary.active_swarms_24h || 0).toLocaleString()}
          </div>
          <div className="text-[10px] text-[#555] font-mono mt-0.5">Sighted recently</div>
        </div>

        <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-3.5">
          <div className="text-[10px] text-[#666] uppercase font-mono flex items-center gap-1.5">
            <Zap className="w-3 h-3 text-amber-400" /> New Releases (48h)
          </div>
          <div className="text-lg font-bold font-mono text-amber-400 mt-1">
            {(summary.fresh_swarms_48h || 0).toLocaleString()}
          </div>
          <div className="text-[10px] text-[#555] font-mono mt-0.5">Fresh DHT swarms</div>
        </div>

        <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-3.5">
          <div className="text-[10px] text-[#666] uppercase font-mono flex items-center gap-1.5">
            <Flame className="w-3 h-3 text-rose-400" /> High Activity
          </div>
          <div className="text-lg font-bold font-mono text-rose-400 mt-1">
            {(summary.high_activity_swarms || 0).toLocaleString()}
          </div>
          <div className="text-[10px] text-[#555] font-mono mt-0.5">≥10 cumulative sightings</div>
        </div>

        <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-3.5">
          <div className="text-[10px] text-[#666] uppercase font-mono flex items-center gap-1.5">
            <Tag className="w-3 h-3 text-emerald-400" /> ML Classified
          </div>
          <div className="text-lg font-bold font-mono text-emerald-400 mt-1">
            {formatNum(summary.classified_torrents || 0)}
          </div>
          <div className="text-[10px] text-[#555] font-mono mt-0.5">{classifiedPct}% of full catalog</div>
        </div>

        <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-3.5">
          <div className="text-[10px] text-[#666] uppercase font-mono flex items-center gap-1.5">
            <HardDrive className="w-3 h-3 text-indigo-400" /> Total Storage
          </div>
          <div className="text-lg font-bold font-mono text-indigo-400 mt-1">
            {summary.total_size_tb ? `${(summary.total_size_tb / 1024).toFixed(2)} PB` : '—'}
          </div>
          <div className="text-[10px] text-[#555] font-mono mt-0.5">
            {summary.total_size_tb ? `${Math.round(summary.total_size_tb).toLocaleString()} TB analyzed` : 'Indexed payload'}
          </div>
        </div>

        <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-3.5">
          <div className="text-[10px] text-[#666] uppercase font-mono flex items-center gap-1.5">
            <Layers className="w-3 h-3 text-violet-400" /> Total Swarms
          </div>
          <div className="text-lg font-bold font-mono text-white mt-1">
            {((summary.total_torrents || 0) / 1000000).toFixed(2)}M
          </div>
          <div className="text-[10px] text-[#555] font-mono mt-0.5">PostgreSQL database</div>
        </div>
      </div>

      {/* SECTION 2: Content Category Landscape & Storage Distribution */}
      {categories.length > 0 && (
        <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-5 space-y-4">
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 border-b border-[#181818] pb-3">
            <div>
              <h3 className="text-sm font-semibold text-white font-mono flex items-center gap-2">
                <PieChart className="w-4 h-4 text-cyan-400" />
                <span>Content Category Landscape & Catalog Distribution</span>
              </h3>
              <p className="text-[11px] text-[#777] mt-0.5 font-mono">
                Multimodal classification breakdown across {summary.classified_torrents?.toLocaleString()} verified payloads
              </p>
            </div>
            <div className="flex items-center gap-3 text-xs font-mono text-[#888]">
              <span>Review Needed: <strong className="text-amber-400 font-normal">{(summary.review_needed_torrents || 0).toLocaleString()}</strong></span>
              <span>·</span>
              <span>Categories: <strong className="text-white font-normal">{categories.length}</strong></span>
            </div>
          </div>

          {/* Interactive Stacked Distribution Spectrum Bar */}
          <div className="space-y-1.5">
            <div className="h-3.5 w-full bg-[#141414] rounded-md overflow-hidden flex ring-1 ring-[#222]">
              {categories.map((c) => {
                const theme = CATEGORY_THEMES[c.category] || CATEGORY_THEMES.Other;
                const isSelected = selectedCategory === c.category;
                return (
                  <div
                    key={c.category}
                    className={`h-full ${theme.bar} transition-all cursor-pointer hover:brightness-125 ${
                      selectedCategory !== 'All' && !isSelected ? 'opacity-30' : 'opacity-100'
                    }`}
                    style={{ width: `${c.pct}%` }}
                    title={`${c.category}: ${c.count.toLocaleString()} (${c.pct}%) · ${c.total_size_tb} TB`}
                    onClick={() => handleCategorySelect(isSelected ? 'All' : c.category)}
                  />
                );
              })}
            </div>
            <div className="flex items-center justify-between text-[11px] font-mono text-[#666]">
              <span>Proportional distribution across indexed catalog</span>
              <span>Click any category segment or card below to isolate telemetry</span>
            </div>
          </div>

          {/* Category Dossier Matrix Cards */}
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
                      ? 'border-cyan-500/80 bg-cyan-950/20 shadow-md ring-1 ring-cyan-500/40'
                      : 'border-[#1b1b1b] bg-[#070707] hover:border-[#333] hover:bg-[#0d0d0d]'
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

                  <div className="mt-2.5 space-y-1 text-[11px]">
                    <div className="flex justify-between text-[#777]">
                      <span>Volume:</span>
                      <span className="text-white font-medium">{c.count.toLocaleString()}</span>
                    </div>
                    <div className="flex justify-between text-[#777]">
                      <span>Storage:</span>
                      <span className="text-indigo-300 font-medium">{c.total_size_tb} TB</span>
                    </div>
                    <div className="flex justify-between text-[#777]">
                      <span>Avg Size:</span>
                      <span className="text-[#bbb]">{c.avg_size_gb} GB</span>
                    </div>
                    <div className="flex justify-between text-[#777] pt-1 border-t border-[#161616]">
                      <span>Swarm Peers:</span>
                      <span className="text-emerald-400">{c.avg_peers}p · {c.avg_health}%</span>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* SECTION 3: Deep Analytical Workspaces */}
      <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] overflow-hidden">
        {/* Sub-Navigation Bar */}
        <div className="p-3.5 border-b border-[#181818] flex flex-col sm:flex-row sm:items-center justify-between gap-3">
          <div className="flex items-center gap-1.5">
            <button
              onClick={() => setActiveSubTab('trends')}
              className={`px-3 py-1.5 rounded-md text-xs font-medium font-mono transition-colors flex items-center gap-1.5 ${
                activeSubTab === 'trends'
                  ? 'bg-[#1a1a1a] text-white border border-[#333]'
                  : 'text-[#888] hover:text-white hover:bg-[#111]'
              }`}
            >
              <BarChart3 className="w-3.5 h-3.5 text-cyan-400" />
              <span>Temporal Trends (7d)</span>
            </button>

            <button
              onClick={() => setActiveSubTab('peer_geo')}
              className={`px-3 py-1.5 rounded-md text-xs font-medium font-mono transition-colors flex items-center gap-1.5 ${
                activeSubTab === 'peer_geo'
                  ? 'bg-[#1a1a1a] text-white border border-[#333]'
                  : 'text-[#888] hover:text-white hover:bg-[#111]'
              }`}
            >
              <Users className="w-3.5 h-3.5 text-indigo-400" />
              <span>Peer Geography & ASNs</span>
            </button>

            <button
              onClick={() => setActiveSubTab('survivability')}
              className={`px-3 py-1.5 rounded-md text-xs font-medium font-mono transition-colors flex items-center gap-1.5 ${
                activeSubTab === 'survivability'
                  ? 'bg-[#1a1a1a] text-white border border-[#333]'
                  : 'text-[#888] hover:text-white hover:bg-[#111]'
              }`}
            >
              <Shield className="w-3.5 h-3.5 text-emerald-400" />
              <span>Swarm Survivability</span>
            </button>
          </div>

          <div className="text-xs font-mono text-[#666] hidden sm:block">
            {activeSubTab === 'trends' && 'Segmented Daily Ingestion Volumes'}
            {activeSubTab === 'peer_geo' && 'Autonomous Systems (BEP 9/10 Handshakes)'}
            {activeSubTab === 'survivability' && 'Seed Retention & Longevity Benchmark'}
          </div>
        </div>

        {/* WORKSPACE 1: TEMPORAL INGESTION TRENDS (7D) */}
        {activeSubTab === 'trends' && (
          <div className="p-5 space-y-4 font-mono">
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 pb-2 border-b border-[#181818]">
              <div>
                <h4 className="text-sm font-semibold text-white flex items-center gap-2">
                  <BarChart3 className="w-4 h-4 text-cyan-400" />
                  Category Ingestion Dynamics (Past 7 Days)
                </h4>
                <p className="text-xs text-[#777] mt-0.5">
                  Daily verified torrent volume partitioned across content categories.
                </p>
              </div>
            </div>

            {/* Ingestion Timeline Visualization */}
            {(() => {
              const trends = data?.trends_7d || [];
              const daysMap = {};
              const detectedCategories = new Set();
              let grandTotal = 0;

              trends.forEach((t) => {
                const dayKey = t.day ? new Date(t.day).toISOString().split('T')[0] : 'Unknown';
                if (!daysMap[dayKey]) daysMap[dayKey] = { day: dayKey, total: 0 };
                const cat = t.category || 'Other';
                daysMap[dayKey][cat] = (daysMap[dayKey][cat] || 0) + t.count;
                daysMap[dayKey].total += t.count;
                detectedCategories.add(cat);
                grandTotal += t.count;
              });

              const chartData = Object.values(daysMap).sort((a, b) => a.day.localeCompare(b.day));
              const catList = Array.from(detectedCategories);

              if (chartData.length === 0) {
                return (
                  <div className="p-8 text-center text-[#666] text-xs">
                    No ingestion trend data points recorded in the last 7 days.
                  </div>
                );
              }

              // Identify top category over past 7 days
              const catTotals = {};
              trends.forEach((t) => {
                const cat = t.category || 'Other';
                catTotals[cat] = (catTotals[cat] || 0) + t.count;
              });
              const topCategory = Object.entries(catTotals).sort((a, b) => b[1] - a[1])[0];
              const peakDay = [...chartData].sort((a, b) => b.total - a.total)[0];

              const PALETTE = {
                Adult: '#f43f5e',
                Television: '#a855f7',
                Movies: '#3b82f6',
                Music: '#06b6d4',
                Anime: '#ec4899',
                Games: '#84cc16',
                'Books & Learning': '#14b8a6',
                Applications: '#f59e0b',
                Audiobooks: '#6366f1',
                Documentaries: '#10b981',
                Other: '#71717a',
                Unclassified: '#52525b',
              };

              return (
                <div className="space-y-4">
                  {/* Summary Stat Pills */}
                  <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
                    <div className="p-3 rounded-lg border border-[#1a1a1a] bg-[#070707] flex items-center justify-between">
                      <span className="text-xs text-[#777]">7-Day Total Verified</span>
                      <span className="text-sm font-bold text-cyan-400">{grandTotal.toLocaleString()}</span>
                    </div>
                    <div className="p-3 rounded-lg border border-[#1a1a1a] bg-[#070707] flex items-center justify-between">
                      <span className="text-xs text-[#777]">Peak Ingestion Day</span>
                      <span className="text-sm font-bold text-white">{peakDay ? `${peakDay.day} (${peakDay.total.toLocaleString()})` : '—'}</span>
                    </div>
                    <div className="p-3 rounded-lg border border-[#1a1a1a] bg-[#070707] flex items-center justify-between">
                      <span className="text-xs text-[#777]">Dominant Category</span>
                      <span className="text-sm font-bold text-purple-400">
                        {topCategory ? `${topCategory[0]} (${Math.round((topCategory[1] / grandTotal) * 100)}%)` : '—'}
                      </span>
                    </div>
                  </div>

                  {/* Chart Container */}
                  <div className="p-4 rounded-xl border border-[#1a1a1a] bg-[#070707] h-80">
                    <ResponsiveContainer width="100%" height="100%">
                      <BarChart data={chartData} margin={{ top: 15, right: 15, bottom: 5, left: 0 }}>
                        <CartesianGrid stroke="#1a1a1a" strokeDasharray="3 3" vertical={false} />
                        <XAxis
                          dataKey="day"
                          stroke="#555"
                          fontSize={11}
                          tickLine={false}
                          tickFormatter={(d) => {
                            const p = d.split('-');
                            return p.length === 3 ? `${p[1]}/${p[2]}` : d;
                          }}
                        />
                        <YAxis
                          stroke="#555"
                          fontSize={11}
                          tickLine={false}
                          axisLine={false}
                          tickFormatter={(v) => (v >= 1000 ? `${(v / 1000).toFixed(1)}k` : v)}
                        />
                        <Tooltip
                          contentStyle={{
                            backgroundColor: '#0c0c0c',
                            borderColor: '#262626',
                            borderRadius: '8px',
                            fontSize: '11px',
                            fontFamily: 'monospace',
                            boxShadow: '0 8px 24px rgba(0,0,0,0.6)',
                          }}
                          itemStyle={{ padding: '1px 0' }}
                          formatter={(value, name) => [value.toLocaleString(), name]}
                          labelFormatter={(label) => `Date: ${label}`}
                        />
                        <Legend
                          wrapperStyle={{ fontSize: '11px', paddingTop: '10px' }}
                          iconType="circle"
                          iconSize={8}
                        />
                        {catList.map((cat) => (
                          <Bar
                            key={cat}
                            dataKey={cat}
                            name={cat}
                            stackId="a"
                            fill={PALETTE[cat] || '#888888'}
                            radius={[0, 0, 0, 0]}
                          />
                        ))}
                      </BarChart>
                    </ResponsiveContainer>
                  </div>
                </div>
              );
            })()}
          </div>
        )}

        {/* WORKSPACE 2: SWARM PEER GEOGRAPHY & ASNS */}
        {activeSubTab === 'peer_geo' && (
          <div className="p-5 space-y-4 font-mono text-xs">
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 pb-2 border-b border-[#181818]">
              <div>
                <h4 className="text-sm font-semibold text-white flex items-center gap-2">
                  <Globe className="w-4 h-4 text-indigo-400" />
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
              {(data?.peer_geography || []).map((peer) => {
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

                    <div className="text-[11px] text-[#aaa] truncate font-mono" title={peer.asn}>
                      {peer.asn}
                    </div>

                    <div className="space-y-1">
                      <div className="flex justify-between text-[10px] text-[#666]">
                        <span>Subnet Prefix: {peer.prefix}.0.0.0/8</span>
                        <span>{pct}% mesh share</span>
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

        {/* WORKSPACE 3: SWARM HALF-LIFE & SURVIVABILITY */}
        {activeSubTab === 'survivability' && (
          <div className="p-5 space-y-4 font-mono text-xs">
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 pb-2 border-b border-[#181818]">
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
                Retention Target: &gt; 80% Healthy
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
                        <span>Active Seeds:</span>
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

