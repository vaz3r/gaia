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
  Radio
} from 'lucide-react';

export default function SurveillanceRadar() {
  const [nodes, setNodes] = useState([]);
  const [stats, setStats] = useState(null);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState('');
  const [minScore, setMinScore] = useState(0);
  const [category, setCategory] = useState('all');
  const [blockedOnly, setBlockedOnly] = useState(false);
  const [page, setPage] = useState(1);
  const [pagination, setPagination] = useState({ page: 1, limit: 25, total: 0, totalPages: 1 });
  const [inspectingHashes, setInspectingHashes] = useState(null);
  const [togglingIp, setTogglingIp] = useState(null);

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

  const getCategoryBadge = (cat) => {
    switch (cat) {
      case 'Sybil Node Rotator':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-semibold bg-purple-500/15 text-purple-400 border border-purple-500/30">
            <Layers className="w-3 h-3" />
            Sybil Rotator
          </span>
        );
      case 'Passive Swarm Monitor':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-semibold bg-rose-500/15 text-rose-400 border border-rose-500/30">
            <Eye className="w-3 h-3" />
            Passive Monitor
          </span>
        );
      case 'DHT Table Scraper':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-semibold bg-amber-500/15 text-amber-400 border border-amber-500/30">
            <Radio className="w-3 h-3" />
            Table Scraper
          </span>
        );
      case 'Unreciprocating Leecher':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-semibold bg-orange-500/15 text-orange-400 border border-orange-500/30">
            <AlertTriangle className="w-3 h-3" />
            Leecher
          </span>
        );
      case 'Cryptographic Spoofing Node':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-semibold bg-red-500/15 text-red-400 border border-red-500/30">
            <Ban className="w-3 h-3" />
            Crypto Spoof
          </span>
        );
      case 'High-Rate Query Flooder':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-semibold bg-blue-500/15 text-blue-400 border border-blue-500/30">
            <Activity className="w-3 h-3" />
            Query Flooder
          </span>
        );
      default:
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-medium bg-[#222] text-[#aaa]">
            <ShieldAlert className="w-3 h-3" />
            {cat || 'Suspicious'}
          </span>
        );
    }
  };

  return (
    <div className="space-y-6">
      {/* Header section */}
      <div className="flex flex-col md:flex-row md:items-center justify-between gap-4 border-b border-[#222] pb-5">
        <div>
          <div className="flex items-center gap-2.5">
            <div className="p-2 rounded-lg bg-rose-500/10 border border-rose-500/20 text-rose-400">
              <ShieldAlert className="w-5 h-5" />
            </div>
            <div>
              <h2 className="text-lg font-bold text-white tracking-tight flex items-center gap-2">
                Universal DHT Abuse & Surveillance Radar
                <span className="text-xs px-2 py-0.5 rounded-full bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-mono">
                  ACTIVE INTERCEPTION
                </span>
              </h2>
              <p className="text-xs text-[#888] mt-0.5">
                Behavioral detection intercepting all non-contributing DHT abusers: Sybil rotators, passive swarm monitors, routing scrapers, and unreciprocating leeches.
              </p>
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={fetchNodes}
            disabled={loading}
            className="px-3 py-1.5 text-xs font-medium text-[#aaa] hover:text-white bg-[#141414] hover:bg-[#1f1f1f] border border-[#2a2a2a] rounded-lg transition-colors flex items-center gap-1.5"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} />
            Refresh
          </button>
          <a
            href="/api/surveillance/blocklist.txt"
            download="ipfilter.dat"
            className="px-3.5 py-1.5 text-xs font-semibold text-white bg-rose-600 hover:bg-rose-500 rounded-lg transition-colors flex items-center gap-2 shadow-lg shadow-rose-950/40"
            title="Download blocklist for qBittorrent, Transmission, or Deluge"
          >
            <DownloadCloud className="w-4 h-4" />
            Export ipfilter.dat
          </a>
        </div>
      </div>

      {/* Stat Cards */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3.5">
        <div className="p-4 rounded-xl bg-[#0e0e0e] border border-[#222]">
          <div className="flex items-center justify-between text-[#888] text-xs font-medium mb-1.5">
            <span>Blocked Abusers & Spies</span>
            <Ban className="w-4 h-4 text-rose-400" />
          </div>
          <div className="text-2xl font-bold font-mono text-white">
            {parseInt(stats?.blocked_nodes || '0', 10).toLocaleString()}
          </div>
          <div className="text-[11px] text-[#666] mt-1">
            of {parseInt(stats?.total_surveillance_nodes || '0', 10).toLocaleString()} detected entities
          </div>
        </div>

        <div className="p-4 rounded-xl bg-[#0e0e0e] border border-[#222]">
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

        <div className="p-4 rounded-xl bg-[#0e0e0e] border border-[#222]">
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

        <div className="p-4 rounded-xl bg-[#0e0e0e] border border-[#222]">
          <div className="flex items-center justify-between text-[#888] text-xs font-medium mb-1.5">
            <span>Routing Scrapers & Flooders</span>
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

      {/* Info / Countermeasure Banner */}
      <div className="p-4 rounded-xl bg-gradient-to-r from-rose-950/20 via-[#121212] to-[#121212] border border-rose-500/20">
        <div className="flex items-start gap-3">
          <Shield className="w-5 h-5 text-rose-400 mt-0.5 shrink-0" />
          <div className="space-y-1 text-xs text-[#aaa]">
            <p className="font-semibold text-white">
              Autonomous Honey-Pot Counter-Measures & Protocol Defense
            </p>
            <p>
              When an identified surveillance entity or non-contributing leech queries GAIA for torrent peers or routing table nodes, GAIA automatically feeds them 
              <span className="text-rose-300 font-mono"> RFC 5737 dummy documentation nodes (192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24)</span>. 
              This poisons external tracking databases, traps scrapers in dead-end routing loops, and protects legitimate BitTorrent users from IP harvesting.
            </p>
            <p className="text-[11px] text-[#777] pt-0.5">
              💡 Community Protection: Export <span className="text-white font-medium">ipfilter.dat</span> into your torrent client to prevent known monitors and malicious crawlers from ever discovering your downloads.
            </p>
          </div>
        </div>
      </div>

      {/* Filter and Search Bar */}
      <div className="flex flex-wrap items-center justify-between gap-3 bg-[#0d0d0d] p-3 rounded-xl border border-[#222]">
        <form onSubmit={handleSearchSubmit} className="flex items-center gap-2 flex-1 min-w-[240px]">
          <div className="relative flex-1">
            <Search className="w-3.5 h-3.5 text-[#666] absolute left-3 top-1/2 -translate-y-1/2" />
            <input
              type="text"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search by IP, Category, Organization, or Entity name..."
              className="w-full bg-[#161616] border border-[#282828] text-white text-xs pl-8 pr-3 py-1.5 rounded-lg focus:outline-none focus:border-rose-500/50"
            />
          </div>
          <button
            type="submit"
            className="px-3 py-1.5 text-xs font-medium text-white bg-[#222] hover:bg-[#2c2c2c] rounded-lg transition-colors"
          >
            Search
          </button>
        </form>

        <div className="flex flex-wrap items-center gap-3 text-xs">
          <div className="flex items-center gap-1.5">
            <span className="text-[#888]">Category:</span>
            <select
              value={category}
              onChange={(e) => {
                setCategory(e.target.value);
                setPage(1);
              }}
              className="bg-[#161616] border border-[#282828] text-white rounded-lg px-2 py-1 text-xs focus:outline-none"
            >
              <option value="all">All Abuse Types</option>
              <option value="Sybil Node Rotator">Sybil Node Rotator</option>
              <option value="Passive Swarm Monitor">Passive Swarm Monitor</option>
              <option value="DHT Table Scraper">DHT Table Scraper</option>
              <option value="Unreciprocating Leecher">Unreciprocating Leecher</option>
              <option value="Cryptographic Spoofing Node">Cryptographic Spoofing</option>
              <option value="High-Rate Query Flooder">Query Flooder</option>
            </select>
          </div>

          <div className="flex items-center gap-1.5">
            <span className="text-[#888]">Min Score:</span>
            <select
              value={minScore}
              onChange={(e) => {
                setMinScore(parseInt(e.target.value, 10));
                setPage(1);
              }}
              className="bg-[#161616] border border-[#282828] text-white rounded-lg px-2 py-1 text-xs focus:outline-none"
            >
              <option value="0">All Scores (0+)</option>
              <option value="50">Suspicious (50+)</option>
              <option value="75">Confirmed Abusers (75+)</option>
            </select>
          </div>

          <label className="flex items-center gap-1.5 text-[#aaa] cursor-pointer hover:text-white select-none">
            <input
              type="checkbox"
              checked={blockedOnly}
              onChange={(e) => {
                setBlockedOnly(e.target.checked);
                setPage(1);
              }}
              className="rounded border-[#333] bg-[#161616] text-rose-500 focus:ring-0 focus:ring-offset-0"
            />
            Blocked Only
          </label>
        </div>
      </div>

      {/* Nodes Table */}
      <div className="bg-[#0e0e0e] border border-[#222] rounded-xl overflow-hidden shadow-2xl">
        <div className="overflow-x-auto">
          <table className="w-full text-left text-xs border-collapse">
            <thead>
              <tr className="bg-[#121212] border-b border-[#222] text-[#888] font-medium">
                <th className="py-3 px-4">Abuse Category</th>
                <th className="py-3 px-4">IP Address</th>
                <th className="py-3 px-4">Suspected Entity / ASN</th>
                <th className="py-3 px-4 text-center">Threat Score</th>
                <th className="py-3 px-4 text-center">BEP 42 Crypto</th>
                <th className="py-3 px-4 text-center">Sybil Node IDs</th>
                <th className="py-3 px-4 text-right">Traffic Volume</th>
                <th className="py-3 px-4 text-center">Status</th>
                <th className="py-3 px-4 text-right">Action</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[#1b1b1b]">
              {loading && nodes.length === 0 ? (
                <tr>
                  <td colSpan="9" className="py-12 text-center text-[#666]">
                    <div className="flex items-center justify-center gap-2">
                      <RefreshCw className="w-4 h-4 animate-spin" />
                      Scanning DHT abuse & surveillance telemetry...
                    </div>
                  </td>
                </tr>
              ) : nodes.length === 0 ? (
                <tr>
                  <td colSpan="9" className="py-12 text-center text-[#666]">
                    No entities match the active filters.
                  </td>
                </tr>
              ) : (
                nodes.map((node) => {
                  const scorePct = Math.min(100, Math.max(0, node.score || 0));
                  const nodeIdsCount = parseInt(node.distinct_node_ids || '1', 10);
                  const getPeers = parseInt(node.get_peers_count || node.query_count || '0', 10);
                  const announces = parseInt(node.announce_peer_count || '0', 10);
                  const findNodes = parseInt(node.find_node_count || '0', 10);

                  return (
                    <tr key={node.ip} className="hover:bg-[#141414] transition-colors">
                      <td className="py-3 px-4">
                        {getCategoryBadge(node.abuse_category)}
                      </td>
                      <td className="py-3 px-4">
                        <div className="font-mono text-white font-medium">{node.ip}</div>
                        <div className="text-[10px] text-[#666]">
                          Last: {new Date(node.last_seen).toLocaleTimeString()}
                        </div>
                      </td>
                      <td className="py-3 px-4">
                        <div className="text-[#ccc] font-medium truncate max-w-[220px]" title={node.suspected_entity}>
                          {node.suspected_entity}
                        </div>
                        <div className="text-[11px] font-mono text-[#777]">
                          {node.asn || 'AS Pending'} {node.org ? `• ${node.org}` : ''}
                        </div>
                      </td>
                      <td className="py-3 px-4 text-center">
                        <div className="inline-flex flex-col items-center gap-1">
                          <span className={`font-mono font-bold text-xs ${scorePct >= 80 ? 'text-rose-400' : scorePct >= 50 ? 'text-amber-400' : 'text-emerald-400'}`}>
                            {scorePct}%
                          </span>
                          <div className="w-16 h-1.5 bg-[#222] rounded-full overflow-hidden">
                            <div
                              className={`h-full rounded-full ${scorePct >= 80 ? 'bg-rose-500' : scorePct >= 50 ? 'bg-amber-500' : 'bg-emerald-500'}`}
                              style={{ width: `${scorePct}%` }}
                            />
                          </div>
                        </div>
                      </td>
                      <td className="py-3 px-4 text-center">
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
                      <td className="py-3 px-4 text-center">
                        {nodeIdsCount > 1 ? (
                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-mono font-bold bg-purple-500/20 text-purple-300 border border-purple-500/30">
                            <Layers className="w-3 h-3" />
                            {nodeIdsCount} IDs (Sybil)
                          </span>
                        ) : (
                          <span className="font-mono text-[11px] text-[#888]">1 ID</span>
                        )}
                      </td>
                      <td className="py-3 px-4 text-right font-mono text-[#ccc]">
                        <div>{parseInt(node.query_count || '0', 10).toLocaleString()} queries</div>
                        <div className="text-[10px] text-[#666]">
                          {getPeers} GP • {announces} Ann • {findNodes} FN
                        </div>
                        {node.sample_hashes && node.sample_hashes.length > 0 && (
                          <button
                            onClick={() => setInspectingHashes({ ip: node.ip, hashes: node.sample_hashes })}
                            className="text-[10px] text-rose-400 hover:text-rose-300 underline mt-0.5 inline-flex items-center gap-0.5"
                          >
                            <Eye className="w-2.5 h-2.5" />
                            {node.sample_hashes.length} target hashes
                          </button>
                        )}
                      </td>
                      <td className="py-3 px-4 text-center">
                        {node.is_blocked ? (
                          <span className="px-2 py-0.5 rounded text-[10px] font-semibold bg-rose-500/20 text-rose-300 border border-rose-500/40">
                            POISONED
                          </span>
                        ) : (
                          <span className="px-2 py-0.5 rounded text-[10px] font-medium bg-[#222] text-[#888]">
                            MONITORED
                          </span>
                        )}
                      </td>
                      <td className="py-3 px-4 text-right">
                        <button
                          onClick={() => toggleBlock(node.ip)}
                          disabled={togglingIp === node.ip}
                          className={`px-2.5 py-1 rounded text-[11px] font-medium transition-colors ${
                            node.is_blocked
                              ? 'bg-[#1e1e1e] hover:bg-[#282828] text-[#ccc]'
                              : 'bg-rose-600/80 hover:bg-rose-500 text-white'
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

        {/* Pagination bar */}
        {pagination.totalPages > 1 && (
          <div className="flex items-center justify-between px-4 py-3 border-t border-[#222] bg-[#111] text-xs text-[#888]">
            <div>
              Showing <span className="text-white">{(pagination.page - 1) * pagination.limit + 1}</span> to{' '}
              <span className="text-white">
                {Math.min(pagination.page * pagination.limit, pagination.total)}
              </span>{' '}
              of <span className="text-white">{pagination.total.toLocaleString()}</span> entities
            </div>
            <div className="flex items-center gap-1">
              <button
                onClick={() => setPage((p) => Math.max(1, p - 1))}
                disabled={page <= 1}
                className="p-1 rounded bg-[#1c1c1c] text-[#aaa] hover:text-white disabled:opacity-40"
              >
                <ChevronLeft className="w-4 h-4" />
              </button>
              <span className="px-2 font-mono text-white">
                {pagination.page} / {pagination.totalPages}
              </span>
              <button
                onClick={() => setPage((p) => Math.min(pagination.totalPages, p + 1))}
                disabled={page >= pagination.totalPages}
                className="p-1 rounded bg-[#1c1c1c] text-[#aaa] hover:text-white disabled:opacity-40"
              >
                <ChevronRight className="w-4 h-4" />
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Target Infohashes Modal */}
      {inspectingHashes && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 backdrop-blur-sm p-4">
          <div className="bg-[#121212] border border-[#282828] rounded-2xl w-full max-w-lg overflow-hidden shadow-2xl animate-in fade-in zoom-in-95 duration-150">
            <div className="flex items-center justify-between px-5 py-4 border-b border-[#222]">
              <div>
                <h3 className="text-sm font-bold text-white flex items-center gap-2">
                  <ShieldAlert className="w-4 h-4 text-rose-400" />
                  Targeted Torrent Infohashes
                </h3>
                <p className="text-xs text-[#888] font-mono mt-0.5">
                  Node IP: {inspectingHashes.ip}
                </p>
              </div>
              <button
                onClick={() => setInspectingHashes(null)}
                className="text-[#888] hover:text-white p-1 rounded-lg hover:bg-[#222]"
              >
                ✕
              </button>
            </div>
            <div className="p-5 space-y-2 max-h-[360px] overflow-y-auto font-mono text-xs">
              <p className="text-[11px] text-[#777] mb-3">
                Torrents probed by this entity. Poison responses containing dummy peer IPs were returned.
              </p>
              {inspectingHashes.hashes.map((hash, idx) => (
                <div
                  key={idx}
                  className="flex items-center justify-between p-2.5 rounded-lg bg-[#181818] border border-[#222] text-[#ddd] select-all"
                >
                  <span className="truncate">{hash}</span>
                  <a
                    href={`/api/torrent/${hash}`}
                    target="_blank"
                    rel="noreferrer"
                    className="text-[#888] hover:text-white shrink-0 ml-2"
                    title="Inspect in GAIA database"
                  >
                    <ExternalLink className="w-3.5 h-3.5" />
                  </a>
                </div>
              ))}
            </div>
            <div className="px-5 py-3 bg-[#161616] border-t border-[#222] flex justify-end">
              <button
                onClick={() => setInspectingHashes(null)}
                className="px-4 py-1.5 text-xs font-semibold text-white bg-[#222] hover:bg-[#2c2c2c] rounded-lg"
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
