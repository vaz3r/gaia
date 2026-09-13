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
  Filter
} from 'lucide-react';

export default function SurveillanceRadar() {
  const [nodes, setNodes] = useState([]);
  const [stats, setStats] = useState(null);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState('');
  const [minScore, setMinScore] = useState(0);
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
  }, [page, minScore, blockedOnly]);

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

  const getEntityBadge = (entity, score) => {
    const e = (entity || '').toLowerCase();
    if (e.includes('selectel') || e.includes('ikwyd') || e.includes('rightscorp') || e.includes('markmonitor')) {
      return (
        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-semibold bg-rose-500/15 text-rose-400 border border-rose-500/30">
          <ShieldAlert className="w-3 h-3" />
          {entity}
        </span>
      );
    }
    if (e.includes('datacamp') || e.includes('m247') || e.includes('crawler') || e.includes('probe')) {
      return (
        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-medium bg-amber-500/15 text-amber-400 border border-amber-500/30">
          <AlertTriangle className="w-3 h-3" />
          {entity}
        </span>
      );
    }
    return (
      <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-medium bg-purple-500/15 text-purple-400 border border-purple-500/30">
        <Zap className="w-3 h-3" />
        {entity || 'Suspicious Query Node'}
      </span>
    );
  };

  const getScoreColor = (score) => {
    if (score >= 80) return 'text-rose-400 bg-rose-500';
    if (score >= 50) return 'text-amber-400 bg-amber-500';
    return 'text-emerald-400 bg-emerald-500';
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
                DHT Surveillance Radar & Anti-Abuse Shield
                <span className="text-xs px-2 py-0.5 rounded-full bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-mono">
                  ACTIVE POISONING
                </span>
              </h2>
              <p className="text-xs text-[#888] mt-0.5">
                Real-time threat intelligence intercepting IKWYD (<span className="text-[#bbb]">iknowwhatyoudownload.com</span>), copyright monitoring firms, and Sybil scrapers.
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
            <span>Blocked Spy Bots</span>
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
            <span>IKWYD / Selectel Nodes</span>
            <ShieldAlert className="w-4 h-4 text-orange-400" />
          </div>
          <div className="text-2xl font-bold font-mono text-orange-400">
            {parseInt(stats?.ikwyd_nodes || '0', 10).toLocaleString()}
          </div>
          <div className="text-[11px] text-[#666] mt-1">
            Selectel AS49505 passive sniffers
          </div>
        </div>

        <div className="p-4 rounded-xl bg-[#0e0e0e] border border-[#222]">
          <div className="flex items-center justify-between text-[#888] text-xs font-medium mb-1.5">
            <span>BEP 42 Sybil Violations</span>
            <Zap className="w-4 h-4 text-purple-400" />
          </div>
          <div className="text-2xl font-bold font-mono text-purple-400">
            {parseInt(stats?.total_bep42_violations || '0', 10).toLocaleString()}
          </div>
          <div className="text-[11px] text-[#666] mt-1">
            cryptographically spoofed Node IDs
          </div>
        </div>

        <div className="p-4 rounded-xl bg-[#0e0e0e] border border-[#222]">
          <div className="flex items-center justify-between text-[#888] text-xs font-medium mb-1.5">
            <span>Intercepted Queries</span>
            <Activity className="w-4 h-4 text-blue-400" />
          </div>
          <div className="text-2xl font-bold font-mono text-blue-400">
            {parseInt(stats?.total_intercepted_queries || '0', 10).toLocaleString()}
          </div>
          <div className="text-[11px] text-[#666] mt-1">
            get_peers snooping requests
          </div>
        </div>
      </div>

      {/* Info / Countermeasure Banner */}
      <div className="p-4 rounded-xl bg-gradient-to-r from-rose-950/20 via-[#121212] to-[#121212] border border-rose-500/20">
        <div className="flex items-start gap-3">
          <Shield className="w-5 h-5 text-rose-400 mt-0.5 shrink-0" />
          <div className="space-y-1 text-xs text-[#aaa]">
            <p className="font-semibold text-white">
              Autonomous Honey-Pot Counter-Measures Active
            </p>
            <p>
              When an identified surveillance entity queries GAIA for torrent peers, GAIA automatically feeds them 
              <span className="text-rose-300 font-mono"> RFC 5737 dummy documentation nodes (192.0.2.0/24, 198.51.100.0/24)</span>. 
              This pollutes IKWYD's public database and copyright troll logs with unrouteable IPs, preventing accurate attribution of home downloaders.
            </p>
            <p className="text-[11px] text-[#777] pt-0.5">
              💡 Tip: Click <span className="text-white font-medium">Export ipfilter.dat</span> and drop it into qBittorrent (<span className="font-mono text-[#999]">Tools &gt; Options &gt; Connection &gt; IP Filtering</span>) to prevent surveillance scrapers from discovering your client.
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
              placeholder="Search by IP, ASN, Organization, or Entity name..."
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

        <div className="flex items-center gap-3 text-xs">
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
              <option value="75">Confirmed Spies (75+)</option>
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
                <th className="py-3 px-4">Suspected Entity</th>
                <th className="py-3 px-4">IP Address</th>
                <th className="py-3 px-4">ASN / Hosting Organization</th>
                <th className="py-3 px-4 text-center">Threat Score</th>
                <th className="py-3 px-4 text-center">BEP 42 Crypto</th>
                <th className="py-3 px-4 text-right">Queries</th>
                <th className="py-3 px-4 text-center">Honey-Pot Status</th>
                <th className="py-3 px-4 text-right">Action</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[#1b1b1b]">
              {loading && nodes.length === 0 ? (
                <tr>
                  <td colSpan="8" className="py-12 text-center text-[#666]">
                    <div className="flex items-center justify-center gap-2">
                      <RefreshCw className="w-4 h-4 animate-spin" />
                      Scanning DHT surveillance telemetry...
                    </div>
                  </td>
                </tr>
              ) : nodes.length === 0 ? (
                <tr>
                  <td colSpan="8" className="py-12 text-center text-[#666]">
                    No surveillance entities match the active filters.
                  </td>
                </tr>
              ) : (
                nodes.map((node) => {
                  const scorePct = Math.min(100, Math.max(0, node.score || 0));
                  return (
                    <tr key={node.ip} className="hover:bg-[#141414] transition-colors">
                      <td className="py-3 px-4">
                        {getEntityBadge(node.suspected_entity, node.score)}
                      </td>
                      <td className="py-3 px-4">
                        <div className="font-mono text-white font-medium">{node.ip}</div>
                        <div className="text-[10px] text-[#666]">
                          Last seen: {new Date(node.last_seen).toLocaleTimeString()}
                        </div>
                      </td>
                      <td className="py-3 px-4">
                        <div className="text-[#ccc] font-medium">{node.org || 'Resolving AS Org...'}</div>
                        <div className="text-[11px] font-mono text-[#777]">{node.asn || 'AS Pending'}</div>
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
                      <td className="py-3 px-4 text-right font-mono text-[#ccc]">
                        <div>{parseInt(node.query_count || '0', 10).toLocaleString()}</div>
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
              of <span className="text-white">{pagination.total.toLocaleString()}</span> surveillance entities
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

      {/* Sample Hashes Modal */}
      {inspectingHashes && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 backdrop-blur-sm p-4">
          <div className="bg-[#121212] border border-[#2a2a2a] rounded-xl max-w-lg w-full p-5 space-y-4 shadow-2xl">
            <div className="flex items-center justify-between border-b border-[#222] pb-3">
              <div>
                <h3 className="text-sm font-bold text-white flex items-center gap-2">
                  <ShieldAlert className="w-4 h-4 text-rose-400" />
                  Snooped Infohashes by {inspectingHashes.ip}
                </h3>
                <p className="text-[11px] text-[#777] mt-0.5">
                  Content hashes queried by this surveillance entity
                </p>
              </div>
              <button
                onClick={() => setInspectingHashes(null)}
                className="text-[#888] hover:text-white text-xs px-2 py-1 bg-[#1c1c1c] rounded"
              >
                Close
              </button>
            </div>

            <div className="space-y-2 max-h-64 overflow-y-auto">
              {inspectingHashes.hashes.map((h, i) => (
                <div
                  key={i}
                  className="p-2.5 rounded-lg bg-[#181818] border border-[#262626] font-mono text-xs text-[#ddd] flex items-center justify-between"
                >
                  <span className="truncate">{h}</span>
                  <a
                    href={`/api/torrents/${h}`}
                    target="_blank"
                    rel="noreferrer"
                    className="text-rose-400 hover:text-rose-300 text-[11px] shrink-0 ml-2 flex items-center gap-1"
                  >
                    Inspect
                    <ExternalLink className="w-3 h-3" />
                  </a>
                </div>
              ))}
            </div>

            <div className="text-[11px] text-[#666] border-t border-[#222] pt-3">
              GAIA replaces results for these hashes with RFC 5737 dummy peers when this entity queries.
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
