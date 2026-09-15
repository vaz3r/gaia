import React, { useState, useEffect } from 'react';
import {
  ShieldAlert,
  Shield,
  AlertTriangle,
  Ban,
  DownloadCloud,
  Activity,
  RefreshCw,
  Search,
  CheckCircle2,
  Zap,
  Eye,
  ExternalLink,
  Layers,
  ChevronLeft,
  ChevronRight,
  Filter,
  Radio,
  Copy,
  Check,
  Film,
  Music,
  FolderArchive,
  BookOpen,
  FileCode,
  Sparkles,
  Link2,
  Database
} from 'lucide-react';

function formatBytes(bytes) {
  if (!bytes || bytes <= 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`;
}

export default function SurveillanceRadar({ onInspectTorrent }) {
  const [nodes, setNodes] = useState([]);
  const [stats, setStats] = useState(null);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState('');
  const [minScore, setMinScore] = useState(0);
  const [category, setCategory] = useState('all');
  const [blockedOnly, setBlockedOnly] = useState(false);
  const [page, setPage] = useState(1);
  const [pagination, setPagination] = useState({ page: 1, limit: 25, total: 0, totalPages: 1 });
  const [togglingIp, setTogglingIp] = useState(null);

  // Target Torrent Hashes Inspection
  const [inspectingNode, setInspectingNode] = useState(null);
  const [targetTorrents, setTargetTorrents] = useState({});
  const [loadingTorrents, setLoadingTorrents] = useState(false);
  const [copiedKey, setCopiedKey] = useState(null);

  const fetchNodes = async () => {
    setLoading(true);
    try {
      const params = new URLSearchParams({
        page: page.toString(),
        limit: '25',
        min_score: minScore.toString(),
        blocked_only: blockedOnly ? 'true' : 'false',
        search: search.trim()
      });
      if (category && category !== 'all') {
        params.append('category', category);
      }
      const res = await fetch(`/api/surveillance/nodes?${params.toString()}`);
      if (res.ok) {
        const data = await res.json();
        setNodes(data.nodes || []);
        setPagination(data.pagination || { page: 1, limit: 25, total: 0, totalPages: 1 });
        if (data.stats) {
          setStats(data.stats);
        }
      }
    } catch (err) {
      console.error('Failed to fetch surveillance nodes:', err);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchNodes();
  }, [page, minScore, category, blockedOnly]);

  const handleSearchSubmit = (e) => {
    e.preventDefault();
    setPage(1);
    fetchNodes();
  };

  const toggleBlock = async (ip) => {
    setTogglingIp(ip);
    try {
      const res = await fetch(`/api/surveillance/nodes/${encodeURIComponent(ip)}/toggle`, {
        method: 'POST'
      });
      if (res.ok) {
        const data = await res.json();
        setNodes((prev) =>
          prev.map((n) => (n.ip === ip ? { ...n, is_blocked: data.node.is_blocked } : n))
        );
        if (stats) {
          setStats((prev) => ({
            ...prev,
            blocked_nodes: data.node.is_blocked
              ? parseInt(prev.blocked_nodes || '0', 10) + 1
              : Math.max(0, parseInt(prev.blocked_nodes || '0', 10) - 1)
          }));
        }
      }
    } catch (err) {
      console.error('Failed to toggle block:', err);
    } finally {
      setTogglingIp(null);
    }
  };

  const handleOpenHashes = async (node) => {
    setInspectingNode(node);
    const hashes = node.sample_hashes || [];
    if (hashes.length > 0) {
      setLoadingTorrents(true);
      try {
        const res = await fetch('/api/torrents/batch-lookup', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ hashes })
        });
        if (res.ok) {
          const data = await res.json();
          setTargetTorrents(data.torrents || {});
        }
      } catch (err) {
        console.error('Failed to resolve target torrent info:', err);
      } finally {
        setLoadingTorrents(false);
      }
    } else {
      setTargetTorrents({});
    }
  };

  const copyText = (text, key) => {
    if (!text) return;
    navigator.clipboard.writeText(text);
    setCopiedKey(key);
    setTimeout(() => setCopiedKey(null), 2000);
  };

  const getCategoryBadge = (cat) => {
    switch (cat) {
      case 'Sybil Node Rotator':
        return (
          <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-[11px] font-semibold bg-purple-500/10 text-purple-400 border border-purple-500/25">
            <Layers className="w-3 h-3 text-purple-400" />
            Sybil Rotator
          </span>
        );
      case 'Passive Swarm Monitor':
        return (
          <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-[11px] font-semibold bg-rose-500/10 text-rose-400 border border-rose-500/25">
            <Eye className="w-3 h-3 text-rose-400" />
            Passive Monitor
          </span>
        );
      case 'DHT Table Scraper':
        return (
          <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-[11px] font-semibold bg-amber-500/10 text-amber-400 border border-amber-500/25">
            <Radio className="w-3 h-3 text-amber-400" />
            Table Scraper
          </span>
        );
      case 'Unreciprocating Leecher':
        return (
          <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-[11px] font-semibold bg-orange-500/10 text-orange-400 border border-orange-500/25">
            <AlertTriangle className="w-3 h-3 text-orange-400" />
            Leecher
          </span>
        );
      case 'Cryptographic Spoofing Node':
        return (
          <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-[11px] font-semibold bg-red-500/10 text-red-400 border border-red-500/25">
            <Ban className="w-3 h-3 text-red-400" />
            Crypto Spoof
          </span>
        );
      case 'High-Rate Query Flooder':
        return (
          <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-[11px] font-semibold bg-blue-500/10 text-blue-400 border border-blue-500/25">
            <Activity className="w-3 h-3 text-blue-400" />
            Query Flooder
          </span>
        );
      case 'Legitimate Peer':
        return (
          <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-[11px] font-semibold bg-emerald-500/10 text-emerald-400 border border-emerald-500/25">
            <CheckCircle2 className="w-3 h-3 text-emerald-400" />
            Legit Peer / Seedbox
          </span>
        );
      default:
        return (
          <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-[11px] font-medium bg-[#1c1c1c] text-[#aaa] border border-[#2a2a2a]">
            <ShieldAlert className="w-3 h-3" />
            {cat || 'Suspicious'}
          </span>
        );
    }
  };

  const getTorrentCategoryIcon = (cat) => {
    switch ((cat || '').toLowerCase()) {
      case 'video':
      case 'movies':
      case 'tv':
        return <Film className="w-3.5 h-3.5 text-blue-400" />;
      case 'audio':
      case 'music':
        return <Music className="w-3.5 h-3.5 text-purple-400" />;
      case 'applications':
      case 'software':
        return <FileCode className="w-3.5 h-3.5 text-emerald-400" />;
      case 'books':
      case 'documents':
        return <BookOpen className="w-3.5 h-3.5 text-amber-400" />;
      default:
        return <FolderArchive className="w-3.5 h-3.5 text-neutral-400" />;
    }
  };

  return (
    <div className="space-y-6">
      {/* Header section with live radar activity pulse */}
      <div className="flex flex-col md:flex-row md:items-center justify-between gap-4 border-b border-[#222] pb-5">
        <div>
          <div className="flex items-center gap-3">
            <div className="p-2.5 rounded-xl bg-rose-500/10 border border-rose-500/20 text-rose-400 shadow-inner shadow-rose-500/20">
              <ShieldAlert className="w-6 h-6" />
            </div>
            <div>
              <div className="flex items-center gap-2.5">
                <h2 className="text-xl font-bold text-white tracking-tight">
                  DHT Surveillance Radar
                </h2>
                <span className="inline-flex items-center gap-1.5 text-[11px] px-2.5 py-0.5 rounded-full bg-blue-500/10 text-blue-400 border border-blue-500/20 font-medium">
                  <span className="relative flex h-2 w-2">
                    <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-blue-400 opacity-75"></span>
                    <span className="relative inline-flex rounded-full h-2 w-2 bg-blue-500"></span>
                  </span>
                  Manual Blocking Mode
                </span>
              </div>
              <p className="text-xs text-[#888] mt-1">
                Observing and scoring all DHT nodes. No automatic blocking — all nodes receive legitimate responses unless you explicitly block them below.
              </p>
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2.5">
          <button
            onClick={fetchNodes}
            disabled={loading}
            className="px-3.5 py-1.5 text-xs font-medium text-[#ccc] hover:text-white bg-[#141414] hover:bg-[#1f1f1f] border border-[#2a2a2a] rounded-lg transition-all flex items-center gap-1.5 shadow-sm"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin text-rose-400' : ''}`} />
            Refresh
          </button>
          <a
            href="/api/surveillance/blocklist.txt"
            download="ipfilter.dat"
            className="px-3.5 py-1.5 text-xs font-semibold text-white bg-rose-600 hover:bg-rose-500 rounded-lg transition-all flex items-center gap-2 shadow-lg shadow-rose-950/40 hover:shadow-rose-900/50"
            title="Download community blocklist for qBittorrent, Transmission, or Deluge"
          >
            <DownloadCloud className="w-4 h-4" />
            Export ipfilter.dat
          </a>
        </div>
      </div>

      {/* Stat Cards */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3.5">
        <div className="p-4 rounded-xl bg-gradient-to-b from-[#131313] to-[#0c0c0c] border border-[#242424] hover:border-[#333] transition-colors">
          <div className="flex items-center justify-between text-[#888] text-xs font-medium mb-1.5">
            <span>Blocked Threats & Spies</span>
            <Ban className="w-4 h-4 text-rose-400" />
          </div>
          <div className="text-2xl font-bold font-mono text-white">
            {parseInt(stats?.blocked_nodes || '0', 10).toLocaleString()}
          </div>
          <div className="text-[11px] text-[#666] mt-1">
            of {parseInt(stats?.total_surveillance_nodes || '0', 10).toLocaleString()} tracked entities
          </div>
        </div>

        <div className="p-4 rounded-xl bg-gradient-to-b from-[#131313] to-[#0c0c0c] border border-[#242424] hover:border-[#333] transition-colors">
          <div className="flex items-center justify-between text-[#888] text-xs font-medium mb-1.5">
            <span>Sybil Attacks Neutralized</span>
            <Layers className="w-4 h-4 text-purple-400" />
          </div>
          <div className="text-2xl font-bold font-mono text-purple-400">
            {parseInt(stats?.sybil_nodes || '0', 10).toLocaleString()}
          </div>
          <div className="text-[11px] text-[#666] mt-1">
            rotating multiple Node IDs per IP
          </div>
        </div>

        <div className="p-4 rounded-xl bg-gradient-to-b from-[#131313] to-[#0c0c0c] border border-[#242424] hover:border-[#333] transition-colors">
          <div className="flex items-center justify-between text-[#888] text-xs font-medium mb-1.5">
            <span>Passive Swarm Monitors</span>
            <Eye className="w-4 h-4 text-orange-400" />
          </div>
          <div className="text-2xl font-bold font-mono text-orange-400">
            {parseInt(stats?.passive_monitors || '0', 10).toLocaleString()}
          </div>
          <div className="text-[11px] text-[#666] mt-1">
            zero-announce sniffing nodes
          </div>
        </div>

        <div className="p-4 rounded-xl bg-gradient-to-b from-[#131313] to-[#0c0c0c] border border-[#242424] hover:border-[#333] transition-colors">
          <div className="flex items-center justify-between text-[#888] text-xs font-medium mb-1.5">
            <span>DHT Table Scrapers</span>
            <Radio className="w-4 h-4 text-blue-400" />
          </div>
          <div className="text-2xl font-bold font-mono text-blue-400">
            {parseInt(stats?.dht_scrapers || '0', 10).toLocaleString()}
          </div>
          <div className="text-[11px] text-[#666] mt-1">
            excessive find_node harvesters
          </div>
        </div>
      </div>

      {/* Counter-Measure Explainer Banner */}
      <div className="p-4 rounded-xl bg-gradient-to-r from-blue-950/25 via-[#111] to-[#111] border border-blue-500/20">
        <div className="flex items-start gap-3">
          <Shield className="w-5 h-5 text-blue-400 mt-0.5 shrink-0" />
          <div className="space-y-1 text-xs text-[#aaa]">
            <p className="font-semibold text-white">
              Manual Honey-Pot Blocking
            </p>
            <p>
              When you manually block a confirmed spy using the toggle below, GAIA responds to its DHT queries with{' '}
              <span className="text-blue-300 font-mono font-medium">RFC 5737 dummy documentation addresses (192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24)</span>.
              This pollutes their tracking logs and derails their scrapers. All other nodes receive legitimate responses.
            </p>
          </div>
        </div>
      </div>

      {/* Search & Filter Bar */}
      <div className="space-y-3 bg-[#0d0d0d] p-3.5 rounded-xl border border-[#222]">
        <div className="flex flex-col md:flex-row items-stretch md:items-center justify-between gap-3">
          <form onSubmit={handleSearchSubmit} className="flex items-center gap-2 flex-1">
            <div className="relative flex-1">
              <Search className="w-3.5 h-3.5 text-[#666] absolute left-3 top-1/2 -translate-y-1/2" />
              <input
                type="text"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder="Search by IP address, Category, ASN, Organization..."
                className="w-full bg-[#161616] border border-[#282828] text-white text-xs pl-8 pr-3 py-2 rounded-lg focus:outline-none focus:border-rose-500/60 transition-colors"
              />
            </div>
            <button
              type="submit"
              className="px-3.5 py-2 text-xs font-medium text-white bg-[#222] hover:bg-[#2b2b2b] rounded-lg transition-colors shrink-0"
            >
              Search
            </button>
          </form>

          {/* Quick Threat Level Filters */}
          <div className="flex items-center gap-1.5 self-start md:self-auto bg-[#141414] p-1 rounded-lg border border-[#242424]">
            {[
              { label: 'All Scores', value: 0 },
              { label: 'Threats (≥60)', value: 60 },
              { label: 'Critical (≥80)', value: 80 }
            ].map((lvl) => (
              <button
                key={lvl.value}
                onClick={() => {
                  setMinScore(lvl.value);
                  setPage(1);
                }}
                className={`px-2.5 py-1 text-xs rounded-md font-medium transition-colors ${
                  minScore === lvl.value
                    ? 'bg-[#222] text-white shadow-sm'
                    : 'text-[#888] hover:text-[#ddd]'
                }`}
              >
                {lvl.label}
              </button>
            ))}
            <div className="h-3.5 w-[1px] bg-[#2a2a2a] mx-1" />
            <button
              onClick={() => {
                setBlockedOnly((prev) => !prev);
                setPage(1);
              }}
              className={`px-2.5 py-1 text-xs rounded-md font-medium transition-colors flex items-center gap-1 ${
                blockedOnly
                  ? 'bg-rose-950/40 text-rose-300 border border-rose-500/40'
                  : 'text-[#888] hover:text-[#ddd]'
              }`}
            >
              <Ban className="w-3 h-3" />
              Blocked Only
            </button>
          </div>
        </div>

        {/* Category Pill Buttons */}
        <div className="flex items-center gap-1.5 overflow-x-auto pb-1 text-xs pt-1 border-t border-[#1a1a1a]">
          <span className="text-[#666] text-[11px] font-medium mr-1 uppercase tracking-wider shrink-0">
            Category:
          </span>
          {[
            { id: 'all', label: 'All Types' },
            { id: 'Sybil Node Rotator', label: 'Sybil Rotators' },
            { id: 'Passive Swarm Monitor', label: 'Passive Monitors' },
            { id: 'DHT Table Scraper', label: 'Table Scrapers' },
            { id: 'Unreciprocating Leecher', label: 'Leechers' },
            { id: 'Cryptographic Spoofing Node', label: 'Crypto Spoofing' },
            { id: 'High-Rate Query Flooder', label: 'Query Flooders' },
            { id: 'Legitimate Peer', label: 'Legit Peers / Seedboxes' }
          ].map((cat) => (
            <button
              key={cat.id}
              onClick={() => {
                setCategory(cat.id);
                setPage(1);
              }}
              className={`px-2.5 py-1 rounded-md text-[11px] font-medium whitespace-nowrap transition-colors ${
                category === cat.id
                  ? 'bg-rose-500/20 text-rose-300 border border-rose-500/40 font-semibold'
                  : 'bg-[#151515] text-[#888] hover:text-white border border-[#222]'
              }`}
            >
              {cat.label}
            </button>
          ))}
        </div>
      </div>

      {/* Nodes Table */}
      <div className="bg-[#0e0e0e] border border-[#222] rounded-xl overflow-hidden shadow-2xl">
        <div className="overflow-x-auto">
          <table className="w-full text-left text-xs border-collapse">
            <thead>
              <tr className="bg-[#121212] border-b border-[#222] text-[#888] font-medium">
                <th className="py-3 px-4">Abuse Signature</th>
                <th className="py-3 px-4">Node Address</th>
                <th className="py-3 px-4">Suspected Entity / ASN</th>
                <th className="py-3 px-4 text-center">Threat Score</th>
                <th className="py-3 px-4 text-center">BEP 42 Crypto</th>
                <th className="py-3 px-4 text-center">Sybil Node IDs</th>
                <th className="py-3 px-4 text-right">Traffic Volume & Targets</th>
                <th className="py-3 px-4 text-center">Status</th>
                <th className="py-3 px-4 text-right">Action</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[#181818]">
              {loading && nodes.length === 0 ? (
                <tr>
                  <td colSpan="9" className="py-14 text-center text-[#666]">
                    <div className="flex items-center justify-center gap-2.5 text-xs">
                      <RefreshCw className="w-4 h-4 animate-spin text-rose-400" />
                      Scanning DHT abuse & surveillance telemetry...
                    </div>
                  </td>
                </tr>
              ) : nodes.length === 0 ? (
                <tr>
                  <td colSpan="9" className="py-14 text-center text-[#666]">
                    No entities match the active filters.
                  </td>
                </tr>
              ) : (
                nodes.map((node) => {
                  const scorePct = Math.min(100, Math.max(0, node.score || 0));
                  const nodeIdsCount = parseInt(node.distinct_node_ids || '1', 10);
                  const getPeers = parseInt(node.get_peers_count || '0', 10);
                  const announces = parseInt(node.announce_peer_count || '0', 10);
                  const findNodes = parseInt(node.find_node_count || '0', 10);
                  const totalQueries = parseInt(node.query_count || '0', 10);
                  const sampleHashes = node.sample_hashes || [];

                  return (
                    <tr key={node.ip} className="hover:bg-[#131313] transition-colors group">
                      <td className="py-3.5 px-4">
                        {getCategoryBadge(node.abuse_category)}
                      </td>
                      <td className="py-3.5 px-4">
                        <div className="font-mono text-white font-semibold text-xs tracking-tight">
                          {node.ip}
                        </div>
                        <div className="text-[10px] text-[#666] font-mono mt-0.5">
                          Last seen: {new Date(node.last_seen).toLocaleTimeString()}
                        </div>
                      </td>
                      <td className="py-3.5 px-4">
                        <div className="text-[#eee] font-medium truncate max-w-[220px]" title={node.suspected_entity}>
                          {node.suspected_entity}
                        </div>
                        <div className="text-[11px] font-mono text-[#777] truncate max-w-[220px]">
                          {node.asn || 'AS Pending'} {node.org ? `• ${node.org}` : ''}
                        </div>
                      </td>
                      <td className="py-3.5 px-4 text-center">
                        <div className="inline-flex flex-col items-center gap-1">
                          <span
                            className={`font-mono font-bold text-xs ${
                              scorePct >= 80 ? 'text-rose-400' : scorePct >= 50 ? 'text-amber-400' : 'text-emerald-400'
                            }`}
                          >
                            {scorePct}%
                          </span>
                          <div className="w-14 h-1.5 bg-[#222] rounded-full overflow-hidden">
                            <div
                              className={`h-full rounded-full ${
                                scorePct >= 80 ? 'bg-rose-500' : scorePct >= 50 ? 'bg-amber-500' : 'bg-emerald-500'
                              }`}
                              style={{ width: `${scorePct}%` }}
                            />
                          </div>
                        </div>
                      </td>
                      <td className="py-3.5 px-4 text-center">
                        {node.bep42_violations > 0 ? (
                          <span className="inline-flex items-center gap-1 text-[11px] text-rose-400 font-mono">
                            <Ban className="w-3 h-3 text-rose-500" />
                            Spoofed ({node.bep42_violations})
                          </span>
                        ) : (
                          <span className="inline-flex items-center gap-1 text-[11px] text-emerald-400 font-mono">
                            <CheckCircle2 className="w-3 h-3 text-emerald-500" />
                            Valid
                          </span>
                        )}
                      </td>
                      <td className="py-3.5 px-4 text-center">
                        {nodeIdsCount > 1 ? (
                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-mono font-bold bg-purple-500/15 text-purple-300 border border-purple-500/30">
                            <Layers className="w-3 h-3" />
                            {nodeIdsCount} IDs (Sybil)
                          </span>
                        ) : (
                          <span className="font-mono text-[11px] text-[#777]">1 ID</span>
                        )}
                      </td>
                      <td className="py-3.5 px-4 text-right font-mono text-[#ccc]">
                        <div className="font-medium">{totalQueries.toLocaleString()} queries</div>
                        <div className="text-[10px] text-[#666]">
                          {getPeers} GP • {announces} Ann • {findNodes} FN
                        </div>
                        {sampleHashes.length > 0 && (
                          <button
                            onClick={() => handleOpenHashes(node)}
                            className="mt-1 px-2 py-0.5 rounded text-[10px] font-medium bg-[#1e1e1e] hover:bg-[#282828] text-rose-300 border border-rose-500/30 hover:border-rose-500/50 transition-colors inline-flex items-center gap-1 shadow-sm"
                            title="Inspect torrents probed by this entity"
                          >
                            <Eye className="w-3 h-3 text-rose-400" />
                            {sampleHashes.length} Target {sampleHashes.length === 1 ? 'Hash' : 'Hashes'}
                          </button>
                        )}
                      </td>
                      <td className="py-3.5 px-4 text-center">
                        {node.is_blocked ? (
                          <span className="px-2 py-0.5 rounded text-[10px] font-semibold bg-rose-500/15 text-rose-300 border border-rose-500/30 tracking-wide">
                            POISONED
                          </span>
                        ) : (
                          <span className="px-2 py-0.5 rounded text-[10px] font-medium bg-[#1c1c1c] text-[#888] border border-[#2a2a2a]">
                            MONITORED
                          </span>
                        )}
                      </td>
                      <td className="py-3.5 px-4 text-right">
                        <button
                          onClick={() => toggleBlock(node.ip)}
                          disabled={togglingIp === node.ip}
                          className={`px-2.5 py-1 rounded-md text-[11px] font-medium transition-colors ${
                            node.is_blocked
                              ? 'bg-[#1a1a1a] hover:bg-[#252525] text-[#ccc] border border-[#333]'
                              : 'bg-rose-600 hover:bg-rose-500 text-white shadow-sm'
                          }`}
                        >
                          {togglingIp === node.ip ? (
                            <RefreshCw className="w-3 h-3 animate-spin inline" />
                          ) : node.is_blocked ? (
                            'Unblock'
                          ) : (
                            'Block & Poison'
                          )}
                        </button>
                      </td>
                    </tr>
                  );
                })
              )}
            </tbody>
          </table>
        </div>

        {/* Pagination Bar */}
        {pagination.totalPages > 1 && (
          <div className="flex items-center justify-between px-4 py-3 border-t border-[#222] bg-[#111] text-xs text-[#888]">
            <div>
              Showing <span className="text-white font-mono">{(pagination.page - 1) * pagination.limit + 1}</span> to{' '}
              <span className="text-white font-mono">
                {Math.min(pagination.page * pagination.limit, pagination.total)}
              </span>{' '}
              of <span className="text-white font-mono">{pagination.total.toLocaleString()}</span> entities
            </div>
            <div className="flex items-center gap-1.5">
              <button
                onClick={() => setPage((p) => Math.max(1, p - 1))}
                disabled={page <= 1}
                className="p-1 rounded-md bg-[#1c1c1c] text-[#aaa] hover:text-white disabled:opacity-30 transition-colors"
              >
                <ChevronLeft className="w-4 h-4" />
              </button>
              <span className="px-2 font-mono text-white text-xs">
                {pagination.page} / {pagination.totalPages}
              </span>
              <button
                onClick={() => setPage((p) => Math.min(pagination.totalPages, p + 1))}
                disabled={page >= pagination.totalPages}
                className="p-1 rounded-md bg-[#1c1c1c] text-[#aaa] hover:text-white disabled:opacity-30 transition-colors"
              >
                <ChevronRight className="w-4 h-4" />
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Target Torrent Infohashes Modal with Real Metadata & Drawer Inspection */}
      {inspectingNode && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 backdrop-blur-sm p-4 animate-in fade-in duration-150">
          <div
            className="bg-[#111] border border-[#282828] rounded-2xl w-full max-w-2xl overflow-hidden shadow-2xl flex flex-col max-h-[85vh]"
            onClick={(e) => e.stopPropagation()}
          >
            {/* Modal Header */}
            <div className="flex items-center justify-between px-6 py-4 border-b border-[#222] bg-[#141414]">
              <div>
                <h3 className="text-sm font-bold text-white flex items-center gap-2">
                  <ShieldAlert className="w-4 h-4 text-rose-400" />
                  Targeted Torrent Infohashes & Probed Swarms
                </h3>
                <p className="text-xs text-[#888] font-mono mt-0.5">
                  Probing Entity: <span className="text-white">{inspectingNode.ip}</span> ({inspectingNode.suspected_entity || 'Abuse Node'})
                </p>
              </div>
              <button
                onClick={() => setInspectingNode(null)}
                className="text-[#888] hover:text-white p-1.5 rounded-lg hover:bg-[#222] transition-colors"
              >
                ✕
              </button>
            </div>

            {/* Modal Body */}
            <div className="p-6 space-y-3 overflow-y-auto flex-1">
              <div className="flex items-center justify-between text-xs text-[#888] pb-2 border-b border-[#1f1f1f]">
                <span>
                  Showing {inspectingNode.sample_hashes?.length || 0} sample torrent swarms queried by this node:
                </span>
                {loadingTorrents && (
                  <span className="inline-flex items-center gap-1.5 text-rose-400 font-mono text-[11px]">
                    <RefreshCw className="w-3 h-3 animate-spin" />
                    Resolving catalog metadata...
                  </span>
                )}
              </div>

              {(inspectingNode.sample_hashes || []).map((hash) => {
                const torrent = targetTorrents[hash];
                const isCopiedHash = copiedKey === `hash-${hash}`;
                const isCopiedMagnet = copiedKey === `magnet-${hash}`;
                const magnetLink = `magnet:?xt=urn:btih:${hash}${torrent?.name ? `&dn=${encodeURIComponent(torrent.name)}` : ''}`;

                return (
                  <div
                    key={hash}
                    className="p-3.5 rounded-xl bg-[#161616] border border-[#262626] hover:border-[#333] transition-colors space-y-2.5"
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="space-y-1 flex-1 min-w-0">
                        {torrent ? (
                          <div className="flex items-center gap-2">
                            {getTorrentCategoryIcon(torrent.category)}
                            <h4
                              onClick={() => {
                                if (onInspectTorrent) {
                                  onInspectTorrent({ ...torrent, hash });
                                }
                              }}
                              className="text-sm font-semibold text-white hover:text-rose-300 cursor-pointer truncate"
                              title={torrent.name}
                            >
                              {torrent.name}
                            </h4>
                          </div>
                        ) : (
                          <div className="flex items-center gap-2">
                            <Database className="w-3.5 h-3.5 text-[#666]" />
                            <h4 className="text-xs font-mono text-[#aaa] truncate">
                              Target Swarm: {hash}
                            </h4>
                          </div>
                        )}

                        {/* Metadata Pills */}
                        <div className="flex flex-wrap items-center gap-2 text-[11px] font-mono text-[#888]">
                          {torrent ? (
                            <>
                              <span className="px-1.5 py-0.5 rounded bg-[#202020] text-[#ccc]">
                                {torrent.category || 'Other'}
                              </span>
                              <span className="px-1.5 py-0.5 rounded bg-[#202020] text-emerald-400 font-medium">
                                {formatBytes(torrent.total_size)}
                              </span>
                              <span className="text-[#666]">
                                {torrent.file_count || 1} {torrent.file_count === 1 ? 'file' : 'files'}
                              </span>
                              {torrent.health_score != null && (
                                <span className="text-[#666]">
                                  Health: {torrent.health_score}/100
                                </span>
                              )}
                            </>
                          ) : (
                            <span className="px-1.5 py-0.5 rounded bg-[#222] text-[#888]">
                              Unindexed Swarm (Probed in DHT)
                            </span>
                          )}
                        </div>
                      </div>

                      {/* Action buttons */}
                      <div className="flex items-center gap-1.5 shrink-0">
                        {torrent && onInspectTorrent && (
                          <button
                            onClick={() => {
                              onInspectTorrent({ ...torrent, hash });
                            }}
                            className="px-2.5 py-1 text-xs font-medium text-white bg-rose-600 hover:bg-rose-500 rounded-lg transition-colors flex items-center gap-1 shadow-sm"
                            title="Inspect in GAIA Details Drawer"
                          >
                            <ExternalLink className="w-3 h-3" />
                            Inspect
                          </button>
                        )}
                        <button
                          onClick={() => copyText(magnetLink, `magnet-${hash}`)}
                          className="p-1.5 rounded-lg bg-[#222] hover:bg-[#2c2c2c] text-[#aaa] hover:text-white transition-colors"
                          title="Copy Magnet Link"
                        >
                          {isCopiedMagnet ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Link2 className="w-3.5 h-3.5" />}
                        </button>
                        <button
                          onClick={() => copyText(hash, `hash-${hash}`)}
                          className="p-1.5 rounded-lg bg-[#222] hover:bg-[#2c2c2c] text-[#aaa] hover:text-white transition-colors"
                          title="Copy Infohash"
                        >
                          {isCopiedHash ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
                        </button>
                      </div>
                    </div>

                    {/* Monospace Hash Footer */}
                    <div className="flex items-center justify-between text-[11px] font-mono text-[#666] bg-[#111] px-2.5 py-1 rounded-md border border-[#1e1e1e]">
                      <span className="truncate select-all text-[#888]">{hash}</span>
                      <span className="text-[10px] text-[#555] shrink-0 ml-2">Poisoned with RFC 5737 dummy peers</span>
                    </div>
                  </div>
                );
              })}
            </div>

            {/* Modal Footer */}
            <div className="px-6 py-3.5 bg-[#141414] border-t border-[#222] flex items-center justify-between text-xs text-[#888]">
              <span>Dummy peer IPs were returned to protect user anonymity.</span>
              <button
                onClick={() => setInspectingNode(null)}
                className="px-4 py-1.5 text-xs font-semibold text-white bg-[#222] hover:bg-[#2a2a2a] rounded-lg transition-colors"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
