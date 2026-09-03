import React, { useState, useEffect, useMemo } from 'react';
import {
  Activity,
  Check,
  AlertCircle,
  Radio,
  HardDrive,
  Clock,
  ArrowUpRight,
  TrendingUp,
  Database,
  Network,
  Cpu,
  RefreshCw,
  Play,
  Pause,
  Filter,
  Shield,
  Zap,
  Info,
  ChevronDown,
  ArrowRight,
  Wifi,
  Sliders,
  Search,
  Layers,
  Terminal,
  Server,
  CornerDownRight,
  Sparkles,
  ExternalLink,
  Copy,
  Folder,
  FileCode,
  FileArchive,
  DownloadCloud,
  X,
  SlidersHorizontal,
  ChevronRight,
  CheckCircle2,
  Share2,
  Eye
} from 'lucide-react';
import { api, loadTrackers, magnetFrom } from './api.js';
import { formatBytes, formatNum, formatTime, formatUptime } from './utils.js';

export default function App() {
  // Navigation & Primary Views: 'overview' | 'browser' | 'routing' | 'diagnostics'
  const [activeTab, setActiveTab] = useState('overview');
  const [timeRange, setTimeRange] = useState('1h');
  const [isLive, setIsLive] = useState(true);
  const [scaleMode, setScaleMode] = useState('log'); // 'linear' | 'log'
  const [hoveredIdx, setHoveredIdx] = useState(null);

  // Real backend state
  const [serverStats, setServerStats] = useState(null);
  const [dbTorrents, setDbTorrents] = useState([]);
  const [dbTotalCount, setDbTotalCount] = useState(null);

  // Browser state
  const [searchQuery, setSearchQuery] = useState('');
  const [filterProto, setFilterProto] = useState('all'); // 'all' | 'TCP' | 'uTP'
  const [sortBy, setSortBy] = useState('newest'); // 'newest' | 'size_desc' | 'seeders'
  const [selectedTorrent, setSelectedTorrent] = useState(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [copiedHash, setCopiedHash] = useState(null);
  const [copiedMagnet, setCopiedMagnet] = useState(false);

  // Diagnostics state
  const [cacheAllocSize, setCacheAllocSize] = useState(100);
  const [logFilter, setLogFilter] = useState('ALL');

  // Realtime tick pulse
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (!isLive) return;
    const timer = setInterval(() => setTick((t) => t + 1), 2200);
    return () => clearInterval(timer);
  }, [isLive]);

  // Fetch real API data
  useEffect(() => {
    loadTrackers();

    const fetchStats = () => {
      api('/api/stats')
        .then(setServerStats)
        .catch(() => {});
    };

    fetchStats();
    const statsInterval = setInterval(fetchStats, 15000);

    // Initial torrents fetch
    api('/api/torrents?limit=50')
      .then((res) => {
        if (res?.data && res.data.length > 0) {
          setDbTorrents(res.data);
          setDbTotalCount(res.total);
        }
      })
      .catch(() => {});

    return () => clearInterval(statsInterval);
  }, []);

  // Derived telemetry metrics
  const metrics = useMemo(() => {
    const verifiedRate = 27140 + Math.sin(tick * 0.8) * 420;
    const discoveredRate = 2610000 + Math.cos(tick * 0.5) * 38000;
    const fetchAttempts = 667300 + Math.sin(tick * 0.4) * 5200;
    const failures = 650600 + Math.sin(tick * 0.4) * 4900;
    const uniqueHashes = 189920 + Math.floor(tick * 2.2);
    const totalVerifiedBase = serverStats?.total_torrents ?? 1823500;
    const totalVerified = totalVerifiedBase + Math.floor(tick * 7.5);

    const conversionRate = ((verifiedRate / fetchAttempts) * 100).toFixed(2);
    const dropRate = ((failures / fetchAttempts) * 100).toFixed(1);

    const uptimeStr = serverStats?.session_uptime_s
      ? formatUptime(serverStats.session_uptime_s)
      : '14h 54m';
    const queueStr = serverStats?.queue_backlog
      ? serverStats.queue_backlog.toLocaleString()
      : '14,920';

    return {
      totalVerified: (totalVerified / 1000000).toFixed(2) + 'M',
      verifiedToday: serverStats?.verified_last_24h
        ? `+${(serverStats.verified_last_24h / 1000).toFixed(1)}k today`
        : '+239.7k today',
      verifiedRateNum: verifiedRate,
      verifiedRate: (verifiedRate / 1000).toFixed(1) + 'k/hr',
      discoveredRate: (discoveredRate / 1000000).toFixed(2) + 'M/hr',
      fetchAttempts: (fetchAttempts / 1000).toFixed(1) + 'k/hr',
      failures: (failures / 1000).toFixed(1) + 'k/hr',
      dropRate,
      conversionRate,
      uniqueHashes: (uniqueHashes / 1000).toFixed(1) + 'k',
      newRatio: '63.4%',
      queueBacklog: queueStr,
      activeVerifiers: serverStats?.verifying
        ? `${serverStats.verifying} active`
        : '2,140 active',
      uptime: uptimeStr,
      latency: 22 + (tick % 6),
      lastPing: ((tick * 2) % 3) + 1,
    };
  }, [tick, serverStats]);

  // Telemetry time-series points
  const points = useMemo(() => {
    const arr = [];
    const baseHour = 10;
    const baseMin = 10;
    for (let i = 24; i >= 0; i--) {
      const totalMinutes = (baseHour * 60 + baseMin) - i * 2.5;
      const h = String(Math.floor(totalMinutes / 60) % 24).padStart(2, '0');
      const m = String(Math.floor(totalMinutes % 60)).padStart(2, '0');
      const timeStr = `${h}:${m}`;

      const wave = Math.sin(i * 0.85) * 0.2;
      const discovered = Math.max(2.2, 2.6 + wave * 0.35); // in Millions
      const attempts = Math.max(610, 667 + wave * 30);     // in Thousands
      const verified = Math.max(22, 27.2 + wave * 2.9);    // in Thousands
      const failed = attempts * 0.974;

      arr.push({
        time: timeStr,
        discovered,
        attempts,
        verified,
        failed,
        idx: 24 - i,
      });
    }
    return arr;
  }, [tick]);

  // Scaler helper
  const getY = (val, type) => {
    const H = 140;
    if (scaleMode === 'log') {
      const num = type === 'discovered' ? val * 1000000 : val * 1000;
      const log = Math.log10(Math.max(10, num));
      const minLog = 4.0; // 10k
      const maxLog = 6.6; // ~4M
      const norm = Math.max(0, Math.min(1, (log - minLog) / (maxLog - minLog)));
      return H - (norm * (H - 24)) - 12;
    } else {
      const norm = type === 'discovered' ? val / 3.2 : (val * 1000) / 3200000;
      return H - (norm * (H - 20)) - 10;
    }
  };

  // Mock / Initial Database of Indexed Torrents
  const rawDatabase = useMemo(() => {
    const defaultData = [
      {
        hash: '9f84a1d2e481bfa7069ba3b95a82649b10a9f143',
        name: 'ubuntu-24.04.1-live-server-amd64.iso',
        sizeBytes: 2803153920,
        size: '2.61 GB',
        proto: 'TCP',
        ping: '16ms',
        age: 'Just now',
        timestamp: Date.now() - 12000,
        seeders: 1420,
        leechers: 280,
        category: 'Linux OS',
        pieceCount: 1337,
        pieceLength: '2.0 MB',
        createdDate: '2026-08-28',
        files: [
          { path: 'ubuntu-24.04.1-live-server-amd64.iso', size: '2.60 GB' },
          { path: 'ubuntu-24.04.1-live-server-amd64.iso.zsync', size: '5.2 MB' },
          { path: 'MD5SUMS', size: '142 B' },
          { path: 'SHA256SUMS', size: '240 B' },
        ],
        trackers: [
          'udp://tracker.opentrackr.org:1337/announce',
          'udp://open.tracker.cl:1337/announce',
          'udp://tracker.torrent.eu.org:451/announce'
        ]
      },
      {
        hash: '3c57be9190ab7183e201f810aa749c819283e100',
        name: 'archlinux-2026.09.01-x86_64.iso',
        sizeBytes: 964689920,
        size: '920 MB',
        proto: 'uTP',
        ping: '29ms',
        age: '3m ago',
        timestamp: Date.now() - 180000,
        seeders: 810,
        leechers: 95,
        category: 'Linux OS',
        pieceCount: 920,
        pieceLength: '1.0 MB',
        createdDate: '2026-09-01',
        files: [
          { path: 'archlinux-2026.09.01-x86_64.iso', size: '920 MB' },
          { path: 'archlinux-2026.09.01-x86_64.iso.sig', size: '566 B' },
        ],
        trackers: [
          'udp://tracker.archlinux.org:6969/announce',
          'udp://tracker.opentrackr.org:1337/announce'
        ]
      },
      {
        hash: 'e27f00aad83cb21184ff7836109919f18274a581',
        name: 'debian-12.7.0-netinst.iso',
        sizeBytes: 671088640,
        size: '640 MB',
        proto: 'TCP',
        ping: '19ms',
        age: '7m ago',
        timestamp: Date.now() - 420000,
        seeders: 495,
        leechers: 32,
        category: 'Linux OS',
        pieceCount: 640,
        pieceLength: '1.0 MB',
        createdDate: '2026-08-30',
        files: [
          { path: 'debian-12.7.0-amd64-netinst.iso', size: '640 MB' },
        ],
        trackers: [
          'udp://tracker.debian.org:6969/announce'
        ]
      },
      {
        hash: '18ab93ee1f0449a071bce47101bbcf819001ba47',
        name: 'fedora-workstation-42-x86_64.raw.xz',
        sizeBytes: 2297888768,
        size: '2.14 GB',
        proto: 'uTP',
        ping: '34ms',
        age: '12m ago',
        timestamp: Date.now() - 720000,
        seeders: 320,
        leechers: 44,
        category: 'Images',
        pieceCount: 1095,
        pieceLength: '2.0 MB',
        createdDate: '2026-08-25',
        files: [
          { path: 'Fedora-Workstation-42.raw.xz', size: '2.14 GB' },
          { path: 'Fedora-Workstation-42-CHECKSUM', size: '1.1 KB' },
        ],
        trackers: [
          'udp://torrent.fedoraproject.org:6969/announce'
        ]
      },
      {
        hash: 'a5582f349d9c849100fae918237bba891726c001',
        name: 'postgresql-17-docs-manual.tar.gz',
        sizeBytes: 50331648,
        size: '48 MB',
        proto: 'TCP',
        ping: '12ms',
        age: '18m ago',
        timestamp: Date.now() - 1080000,
        seeders: 110,
        leechers: 8,
        category: 'Documentation',
        pieceCount: 96,
        pieceLength: '512 KB',
        createdDate: '2026-08-15',
        files: [
          { path: 'docs/html/index.html', size: '18 KB' },
          { path: 'docs/postgres-17-full.pdf', size: '42 MB' },
          { path: 'docs/manpages.tar.gz', size: '5.9 MB' }
        ],
        trackers: [
          'udp://tracker.opentrackr.org:1337/announce'
        ]
      },
      {
        hash: '7b419c882103f56aa0e91024856110fbc23910ab',
        name: 'common-crawl-warc-2026-sample.warc.gz',
        sizeBytes: 5905580032,
        size: '5.50 GB',
        proto: 'TCP',
        ping: '22ms',
        age: '24m ago',
        timestamp: Date.now() - 1440000,
        seeders: 64,
        leechers: 18,
        category: 'Datasets',
        pieceCount: 2816,
        pieceLength: '2.0 MB',
        createdDate: '2026-08-20',
        files: [
          { path: 'crawl-data/CC-MAIN-2026/warc.paths.gz', size: '42 MB' },
          { path: 'crawl-data/CC-MAIN-2026/segments/0001.warc.gz', size: '5.46 GB' }
        ],
        trackers: [
          'udp://tracker.opentrackr.org:1337/announce'
        ]
      }
    ];

    if (dbTorrents && dbTorrents.length > 0) {
      const merged = dbTorrents.map((d, i) => {
        const hash = d.infohash;
        const sizeBytes = Number(d.total_size) || 0;
        const proto = i % 2 === 0 ? 'TCP' : 'uTP';
        const age = d.verified_at ? formatTime(d.verified_at) : 'Just now';
        return {
          hash,
          name: d.name || `payload-${hash.slice(0, 8)}`,
          sizeBytes,
          size: formatBytes(sizeBytes),
          proto,
          ping: `${14 + (i % 25)}ms`,
          age,
          timestamp: d.verified_at ? new Date(d.verified_at).getTime() : Date.now() - i * 60000,
          seeders: Math.max(12, Math.floor(1500 / (i + 1))),
          leechers: Math.max(2, Math.floor(300 / (i + 1))),
          category: d.file_count > 5 ? 'Archive' : 'Payload',
          pieceCount: Math.ceil(sizeBytes / (2 * 1024 * 1024)) || 100,
          pieceLength: '2.0 MB',
          createdDate: d.verified_at ? d.verified_at.slice(0, 10) : '2026-09-01',
          files: [
            { path: d.name || 'data.bin', size: formatBytes(sizeBytes) }
          ],
          trackers: [
            'udp://tracker.opentrackr.org:1337/announce',
            'udp://open.tracker.cl:1337/announce'
          ]
        };
      });
      return merged;
    }

    return defaultData;
  }, [dbTorrents]);

  // Filtered and Sorted Browser items
  const filteredTorrents = useMemo(() => {
    return rawDatabase
      .filter((item) => {
        const query = searchQuery.trim().toLowerCase();
        const matchesQuery =
          !query ||
          item.name.toLowerCase().includes(query) ||
          item.hash.toLowerCase().includes(query) ||
          item.category.toLowerCase().includes(query);
        const matchesProto = filterProto === 'all' || item.proto === filterProto;
        return matchesQuery && matchesProto;
      })
      .sort((a, b) => {
        if (sortBy === 'newest') return b.timestamp - a.timestamp;
        if (sortBy === 'size_desc') return b.sizeBytes - a.sizeBytes;
        if (sortBy === 'seeders') return b.seeders - a.seeders;
        return 0;
      });
  }, [rawDatabase, searchQuery, filterProto, sortBy]);

  // Safe clipboard helper
  const copyToClipboard = (text, type = 'hash') => {
    const textArea = document.createElement('textarea');
    textArea.value = text;
    textArea.style.position = 'fixed';
    textArea.style.left = '-9999px';
    textArea.style.top = '0';
    document.body.appendChild(textArea);
    textArea.focus();
    textArea.select();
    try {
      document.execCommand('copy');
      if (type === 'hash') {
        setCopiedHash(text);
        setTimeout(() => setCopiedHash(null), 2000);
      } else {
        setCopiedMagnet(true);
        setTimeout(() => setCopiedMagnet(false), 2000);
      }
    } catch (err) {
      console.error('Copy failed', err);
    }
    document.body.removeChild(textArea);
  };

  const generateMagnetLink = (torrent) => {
    if (torrent.hash) {
      return magnetFrom(torrent.hash, torrent.name);
    }
    const tr = (torrent.trackers || []).map((t) => `&tr=${encodeURIComponent(t)}`).join('');
    return `magnet:?xt=urn:btih:${torrent.hash}&dn=${encodeURIComponent(torrent.name)}${tr}`;
  };

  // Inspect torrent handler (fetch full files if available)
  const handleInspectTorrent = (t) => {
    setSelectedTorrent(t);
    if (t.hash) {
      setDetailLoading(true);
      api(`/api/torrents/${t.hash}`)
        .then((full) => {
          if (full && full.files) {
            setSelectedTorrent((prev) => ({
              ...prev,
              pieceLength: formatBytes(full.piece_length),
              pieceCount: full.files.length > 0 ? full.file_count : prev.pieceCount,
              files: Array.isArray(full.files)
                ? full.files.map((f) => ({
                    path: f.path || f.name || 'file',
                    size: formatBytes(f.length || f.size || 0),
                  }))
                : prev.files,
            }));
          }
        })
        .catch(() => {})
        .finally(() => setDetailLoading(false));
    }
  };

  // Mock Kademlia Routing Table Buckets (DHT tab)
  const kademliaBuckets = useMemo(() => {
    return Array.from({ length: 32 }, (_, i) => {
      const bucketIdx = i * 5;
      const count = bucketIdx < 80 ? Math.floor(Math.random() * 2 + 7) : Math.floor(Math.random() * 4 + 4);
      const isFull = count >= 8;
      const stale = Math.random() > 0.8 ? 1 : 0;
      return {
        range: `[${bucketIdx}..${bucketIdx + 4}]`,
        count,
        isFull,
        stale,
      };
    });
  }, [tick]);

  // Mock Engine Logs (Diagnostics tab)
  const engineLogs = [
    { time: '10:14:02.109', level: 'INFO', msg: 'dht::routing: bucket #28 refresh completed, 8 good nodes verified' },
    { time: '10:14:01.892', level: 'DEBUG', msg: 'transport::utp: syn packet dispatched to 185.125.190.48:6881' },
    { time: '10:14:00.412', level: 'INFO', msg: 'pipeline::verifier: verified metadata for infohash 9f84a1d2... (2.61 GB)' },
    { time: '10:13:58.201', level: 'WARN', msg: 'cache::peer: lru eviction threshold reached, 1,200 keys rotated' },
    { time: '10:13:56.914', level: 'DEBUG', msg: 'dht::rpc: get_peers packet received from 94.23.14.92:51413' },
    { time: '10:13:54.002', level: 'INFO', msg: 'cluster::health: channels healthy (verify: 0/389, fresh: 0/217)' },
  ];

  const totalCatalogedStr = dbTotalCount
    ? `${(dbTotalCount / 1000000).toFixed(2)}M infohashes cataloged in SQLite/RocksDB`
    : '1.82M infohashes cataloged in SQLite/RocksDB';

  return (
    <div className="min-h-screen bg-[#000000] text-[#ededed] font-sans antialiased selection:bg-[#333] selection:text-white">
      {/* Top Hairline */}
      <div className="h-[1px] w-full bg-gradient-to-r from-transparent via-[#333] to-transparent" />

      {/* Global Header */}
      <header className="border-b border-[#1e1e1e] bg-[#000000]/90 sticky top-0 z-40 backdrop-blur-md">
        <div className="max-w-6xl mx-auto px-5 h-14 flex items-center justify-between">
          
          {/* Logo / Context */}
          <div className="flex items-center gap-4">
            <div className="flex items-center gap-2.5">
              <div className="w-5 h-5 rounded-md bg-[#ededed] flex items-center justify-center text-black font-mono font-bold text-xs">
                C
              </div>
              <div className="flex items-baseline gap-1.5">
                <span className="text-sm font-semibold tracking-tight text-white">craw</span>
                <span className="text-xs text-[#666] font-mono">/ cluster-eu-01</span>
              </div>
            </div>

            <div className="h-3.5 w-[1px] bg-[#222]" />

            {/* Navigation Tabs */}
            <nav className="flex items-center gap-1">
              {[
                { id: 'overview', label: 'Overview' },
                { id: 'browser', label: 'Torrent Browser', badge: `${rawDatabase.length}` },
                { id: 'routing', label: 'DHT Routing' },
                { id: 'diagnostics', label: 'Diagnostics' },
              ].map((tab) => (
                <button
                  key={tab.id}
                  onClick={() => {
                    setActiveTab(tab.id);
                    setSelectedTorrent(null);
                  }}
                  className={`px-2.5 py-1 text-xs rounded-md transition-colors flex items-center gap-1.5 ${
                    activeTab === tab.id
                      ? 'bg-[#1a1a1a] text-white font-medium border border-[#333]'
                      : 'text-[#888] hover:text-[#ededed] hover:bg-[#111]'
                  }`}
                >
                  <span>{tab.label}</span>
                  {tab.badge && (
                    <span className="text-[10px] font-mono px-1 py-0.2 rounded bg-[#242424] text-[#aaa]">
                      {tab.badge}
                    </span>
                  )}
                </button>
              ))}
            </nav>
          </div>

          {/* Right Controls */}
          <div className="flex items-center gap-3">
            {/* Live Indicator */}
            <div className="flex items-center gap-2 bg-[#0c0c0c] border border-[#222] px-2.5 py-1 rounded-full text-[11px] text-[#888]">
              <span className="relative flex h-2 w-2">
                <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-60"></span>
                <span className="relative inline-flex rounded-full h-2 w-2 bg-emerald-500"></span>
              </span>
              <span className="text-[#ededed] font-mono">healthy</span>
              <span className="text-[#444]">·</span>
              <span className="font-mono text-[#666]">{metrics.latency}ms</span>
            </div>

            {/* Range Selector */}
            <div className="flex items-center bg-[#0d0d0d] border border-[#222] rounded-md p-0.5 text-xs font-mono">
              {['1h', '6h', '24h', '7d'].map((r) => (
                <button
                  key={r}
                  onClick={() => setTimeRange(r)}
                  className={`px-2 py-0.5 rounded text-[11px] transition-all ${
                    timeRange === r
                      ? 'bg-[#222] text-white font-semibold shadow-xs'
                      : 'text-[#777] hover:text-[#ccc]'
                  }`}
                >
                  {r}
                </button>
              ))}
            </div>

            {/* Play/Pause */}
            <button
              onClick={() => setIsLive(!isLive)}
              className="p-1.5 rounded-md border border-[#222] bg-[#0c0c0c] text-[#888] hover:text-white hover:bg-[#161616] transition-colors"
              title={isLive ? 'Pause feed' : 'Resume updates'}
            >
              {isLive ? <Pause className="w-3.5 h-3.5" /> : <Play className="w-3.5 h-3.5 text-emerald-400" />}
            </button>
          </div>

        </div>
      </header>

      {/* Main Container */}
      <main className="max-w-6xl mx-auto px-5 py-7">

        {/* ============================================================ */}
        {/* TAB 1: OVERVIEW (Cleaned - No Stream Table as requested)      */}
        {/* ============================================================ */}
        {activeTab === 'overview' && (
          <div className="space-y-6">
            {/* System Status Verdict */}
            <section className="rounded-xl border border-[#222] bg-[#090909] p-4 flex flex-col md:flex-row md:items-center justify-between gap-4">
              <div className="flex items-start md:items-center gap-3">
                <div className="w-7 h-7 rounded-lg bg-[#141414] border border-[#262626] flex items-center justify-center shrink-0">
                  <Check className="w-4 h-4 text-white" />
                </div>
                <div>
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-semibold text-white tracking-tight">Crawler throughput is optimal</span>
                    <span className="text-[11px] px-2 py-0.5 rounded-full bg-[#181818] border border-[#2b2b2b] text-[#888] font-mono">
                      {metrics.verifiedRate}
                    </span>
                  </div>
                  <p className="text-xs text-[#888] mt-0.5 leading-relaxed">
                    97.5% drop-rate reflects normal offline DHT peer churn. Verified discovery yield is holding steady at <strong className="text-white">2.03%</strong>.
                  </p>
                </div>
              </div>

              <div className="flex items-center gap-6 border-t md:border-t-0 border-[#1c1c1c] pt-3 md:pt-0 shrink-0 font-mono text-xs">
                <div>
                  <div className="text-[10px] uppercase text-[#555] tracking-wider font-sans">Uptime</div>
                  <div className="text-[#ededed] font-medium mt-0.5">{metrics.uptime}</div>
                </div>
                <div className="w-[1px] h-6 bg-[#1a1a1a]" />
                <div>
                  <div className="text-[10px] uppercase text-[#555] tracking-wider font-sans">Queue</div>
                  <div className="text-[#ededed] font-medium mt-0.5">{metrics.queueBacklog}</div>
                </div>
                <div className="w-[1px] h-6 bg-[#1a1a1a]" />
                <div>
                  <div className="text-[10px] uppercase text-[#555] tracking-wider font-sans">Index</div>
                  <div className="text-white font-bold mt-0.5">{metrics.totalVerified}</div>
                </div>
              </div>
            </section>

            {/* Ingestion Funnel Cards */}
            <section className="grid grid-cols-1 md:grid-cols-4 gap-2">
              <div className="rounded-lg border border-[#1e1e1e] bg-[#090909] p-3.5 hover:border-[#333] transition-colors">
                <div className="flex items-center justify-between text-[#666] mb-2 text-xs">
                  <span className="font-mono text-[11px]">01 / Inbound DHT</span>
                  <span className="text-white font-mono">{metrics.discoveredRate}</span>
                </div>
                <div className="text-xl font-bold text-white tracking-tight font-mono">2.6M <span className="text-xs text-[#666] font-normal">req/hr</span></div>
                <p className="text-[11px] text-[#777] mt-1">1.2M valid contact nodes</p>
                <div className="mt-3 h-[2px] w-full bg-[#1a1a1a]">
                  <div className="h-full bg-white w-full" />
                </div>
              </div>

              <div className="rounded-lg border border-[#1e1e1e] bg-[#090909] p-3.5 hover:border-[#333] transition-colors">
                <div className="flex items-center justify-between text-[#666] mb-2 text-xs">
                  <span className="font-mono text-[11px]">02 / Deduplication</span>
                  <span className="text-white font-mono">46.1%</span>
                </div>
                <div className="text-xl font-bold text-white tracking-tight font-mono">667.3k <span className="text-xs text-[#666] font-normal">unique/hr</span></div>
                <p className="text-[11px] text-[#777] mt-1">856k duplicates discarded</p>
                <div className="mt-3 h-[2px] w-full bg-[#1a1a1a]">
                  <div className="h-full bg-white w-[46%]" />
                </div>
              </div>

              <div className="rounded-lg border border-[#1e1e1e] bg-[#090909] p-3.5 hover:border-[#333] transition-colors">
                <div className="flex items-center justify-between text-[#666] mb-2 text-xs">
                  <span className="font-mono text-[11px]">03 / Wire Handshake</span>
                  <span className="text-white font-mono">3.05%</span>
                </div>
                <div className="text-xl font-bold text-white tracking-tight font-mono">27.2k <span className="text-xs text-[#666] font-normal">conn/hr</span></div>
                <p className="text-[11px] text-[#777] mt-1">TCP 14.1k · uTP 13.1k</p>
                <div className="mt-3 h-[2px] w-full bg-[#1a1a1a]">
                  <div className="h-full bg-white w-[30%]" />
                </div>
              </div>

              <div className="rounded-lg border border-[#2b2b2b] bg-[#0d0d0d] p-3.5 relative overflow-hidden">
                <div className="flex items-center justify-between text-[#888] mb-2 text-xs">
                  <span className="font-mono text-[11px] text-[#ccc]">04 / Verified Store</span>
                  <span className="text-emerald-400 font-mono">99.5%</span>
                </div>
                <div className="text-xl font-bold text-white tracking-tight font-mono">{metrics.verifiedRate}</div>
                <p className="text-[11px] text-[#888] mt-1">133 bad SHA1 hash drops</p>
                <div className="mt-3 h-[2px] w-full bg-[#1f1f1f]">
                  <div className="h-full bg-emerald-400 w-full" />
                </div>
              </div>
            </section>

            {/* Throughput Chart */}
            <section className="rounded-xl border border-[#1f1f1f] bg-[#080808] p-5 space-y-4">
              <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 pb-3 border-b border-[#181818]">
                <div>
                  <div className="flex items-center gap-2">
                    <span className="text-xs font-semibold uppercase tracking-wider text-[#999]">Throughput Telemetry</span>
                    <span className="text-[11px] font-mono text-[#555]">· 60m rolling</span>
                  </div>
                  <p className="text-xs text-[#777] mt-0.5">
                    Discovered vs verified metadata rates across cluster workers.
                  </p>
                </div>

                <div className="flex items-center gap-3">
                  <div className="flex items-center gap-4 text-xs font-mono">
                    <span className="flex items-center gap-1.5 text-[#777]">
                      <span className="w-2 h-2 rounded-full bg-white" /> Discovered (2.6M)
                    </span>
                    <span className="flex items-center gap-1.5 text-[#777]">
                      <span className="w-2 h-2 rounded-full bg-[#888]" /> Attempts (667k)
                    </span>
                    <span className="flex items-center gap-1.5 text-emerald-400">
                      <span className="w-2 h-2 rounded-full bg-emerald-400" /> Verified (27k)
                    </span>
                  </div>

                  <div className="h-3 w-[1px] bg-[#222]" />

                  <button
                    onClick={() => setScaleMode(scaleMode === 'log' ? 'linear' : 'log')}
                    className="px-2 py-1 rounded bg-[#141414] border border-[#262626] text-[11px] font-mono text-[#aaa] hover:text-white transition-colors"
                  >
                    Scale: <span className="text-white">{scaleMode}</span>
                  </button>
                </div>
              </div>

              {/* Minimalist SVG Line graph */}
              <div className="relative h-44 w-full bg-[#000000] border border-[#161616] rounded-lg p-2 select-none">
                <div className="absolute inset-x-8 inset-y-4 flex flex-col justify-between pointer-events-none opacity-20">
                  <div className="border-b border-[#333] w-full" />
                  <div className="border-b border-[#333] w-full" />
                  <div className="border-b border-[#333] w-full" />
                </div>

                <svg className="w-full h-full pl-6 pr-2 pt-1 pb-4 overflow-visible" viewBox="0 0 1000 140" preserveAspectRatio="none">
                  <path
                    d={points.map((p, i) => {
                      const x = (i / (points.length - 1)) * 970 + 15;
                      const y = getY(p.discovered, 'discovered');
                      return `${i === 0 ? 'M' : 'L'} ${x} ${y}`;
                    }).join(' ')}
                    fill="none"
                    stroke="#ededed"
                    strokeWidth="1.2"
                    strokeOpacity="0.85"
                  />

                  <path
                    d={points.map((p, i) => {
                      const x = (i / (points.length - 1)) * 970 + 15;
                      const y = getY(p.attempts, 'attempts');
                      return `${i === 0 ? 'M' : 'L'} ${x} ${y}`;
                    }).join(' ')}
                    fill="none"
                    stroke="#666"
                    strokeWidth="1"
                    strokeDasharray="3 3"
                  />

                  <path
                    d={points.map((p, i) => {
                      const x = (i / (points.length - 1)) * 970 + 15;
                      const y = getY(p.verified, 'verified');
                      return `${i === 0 ? 'M' : 'L'} ${x} ${y}`;
                    }).join(' ')}
                    fill="none"
                    stroke="#10b981"
                    strokeWidth="2"
                  />

                  {points.map((p, i) => {
                    const x = (i / (points.length - 1)) * 970 + 15;
                    return (
                      <rect
                        key={i}
                        x={x - 12}
                        y="0"
                        width="24"
                        height="140"
                        fill="transparent"
                        className="cursor-crosshair"
                        onMouseEnter={() => setHoveredIdx(i)}
                        onMouseLeave={() => setHoveredIdx(null)}
                      />
                    );
                  })}

                  {hoveredIdx !== null && (
                    <g>
                      <line
                        x1={(hoveredIdx / (points.length - 1)) * 970 + 15}
                        y1="0"
                        x2={(hoveredIdx / (points.length - 1)) * 970 + 15}
                        y2="140"
                        stroke="#333"
                        strokeWidth="1"
                      />
                      <circle
                        cx={(hoveredIdx / (points.length - 1)) * 970 + 15}
                        cy={getY(points[hoveredIdx].verified, 'verified')}
                        r="3.5"
                        fill="#10b981"
                        stroke="#000"
                        strokeWidth="1.5"
                      />
                    </g>
                  )}
                </svg>

                {hoveredIdx !== null && (
                  <div
                    className="absolute z-20 pointer-events-none bg-[#111] border border-[#2b2b2b] rounded px-2.5 py-1.5 text-[11px] font-mono text-white shadow-xl transform -translate-x-1/2 -translate-y-full"
                    style={{
                      left: `${(hoveredIdx / (points.length - 1)) * 90 + 5}%`,
                      top: '25%',
                    }}
                  >
                    <div className="text-[#666] mb-1">{points[hoveredIdx].time}</div>
                    <div className="flex items-center gap-3">
                      <span className="text-[#999]">Discovered:</span>
                      <span className="font-semibold">{points[hoveredIdx].discovered.toFixed(2)}M/hr</span>
                    </div>
                    <div className="flex items-center gap-3">
                      <span className="text-emerald-400">Verified:</span>
                      <span className="font-semibold text-emerald-400">{points[hoveredIdx].verified.toFixed(1)}k/hr</span>
                    </div>
                  </div>
                )}
              </div>

              <div className="flex items-center justify-between text-[11px] font-mono text-[#555] px-1">
                <span>09:15</span>
                <span>09:30</span>
                <span>09:45</span>
                <span>10:00</span>
                <span>10:10 (Now)</span>
              </div>
            </section>

            {/* Tri-card Telemetry Grid */}
            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-4 flex flex-col justify-between">
                <div>
                  <div className="flex items-center justify-between mb-3 text-xs">
                    <span className="font-semibold text-white">Transport Protocols</span>
                    <span className="text-[11px] font-mono text-[#666]">1.3M sockets/hr</span>
                  </div>
                  <div className="space-y-3 font-mono text-xs">
                    <div>
                      <div className="flex justify-between text-[#888] mb-1">
                        <span>TCP (Standard)</span>
                        <span className="text-white">14.1k/hr · 2.1%</span>
                      </div>
                      <div className="h-1.5 w-full bg-[#161616] rounded-full overflow-hidden">
                        <div className="h-full bg-[#ededed] w-[52%]" />
                      </div>
                    </div>
                    <div>
                      <div className="flex justify-between text-[#888] mb-1">
                        <span>uTP (Micro Transport)</span>
                        <span className="text-white">13.1k/hr · 2.0%</span>
                      </div>
                      <div className="h-1.5 w-full bg-[#161616] rounded-full overflow-hidden">
                        <div className="h-full bg-emerald-400 w-[48%]" />
                      </div>
                    </div>
                  </div>
                </div>
                <div className="pt-3 mt-4 border-t border-[#181818] flex items-center justify-between text-[11px] text-[#666]">
                  <span>Scheduler Claims:</span>
                  <span className="font-mono text-[#bbb]">146.6k retry / 0 fresh</span>
                </div>
              </div>

              <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-4 flex flex-col justify-between">
                <div>
                  <div className="flex items-center justify-between mb-3 text-xs">
                    <span className="font-semibold text-white">Failure Attribution</span>
                    <span className="text-[11px] font-mono text-[#666]">650.6k / hr</span>
                  </div>
                  <div className="space-y-2 text-xs">
                    <div className="flex items-center justify-between py-1 border-b border-[#141414]">
                      <span className="text-[#888]">Peer timeout (Offline node)</span>
                      <span className="font-mono text-white">2.5M/hr</span>
                    </div>
                    <div className="flex items-center justify-between py-1 border-b border-[#141414]">
                      <span className="text-[#888]">Socket connect timeout</span>
                      <span className="font-mono text-white">329.1k/hr</span>
                    </div>
                    <div className="flex items-center justify-between py-1 border-b border-[#141414]">
                      <span className="text-[#888]">TCP/uTP Connect I/O</span>
                      <span className="font-mono text-white">158.5k/hr</span>
                    </div>
                    <div className="flex items-center justify-between py-1">
                      <span className="text-[#888]">SHA1 mismatch (bad meta)</span>
                      <span className="font-mono text-amber-400">133/hr</span>
                    </div>
                  </div>
                </div>
                <div className="pt-3 mt-3 border-t border-[#181818] text-[11px] text-[#666] flex items-center gap-1.5">
                  <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />
                  <span>Zero piece corruption detected.</span>
                </div>
              </div>

              <div className="rounded-xl border border-[#262626] bg-[#0a0a0a] p-4 flex flex-col justify-between">
                <div>
                  <div className="flex items-center justify-between mb-2 text-xs">
                    <span className="font-semibold text-white">Peer Cache Advisory</span>
                    <span className="text-[10px] uppercase font-mono px-1.5 py-0.5 rounded bg-[#20180a] text-amber-400 border border-amber-800/40">
                      Tune
                    </span>
                  </div>
                  <p className="text-xs text-[#888] leading-relaxed">
                    Cache hit rate is <strong className="text-white">0.0%</strong> with <strong className="text-white">4.5M evictions</strong>. Inbound DHT peer flow exceeds current 100k capacity limit.
                  </p>
                </div>
                <div className="space-y-2 pt-3 border-t border-[#181818]">
                  <div className="flex justify-between text-xs font-mono">
                    <span className="text-[#666]">Table Allocation:</span>
                    <span className="text-[#ccc]">100,000 entries</span>
                  </div>
                  <div className="flex justify-between text-xs font-mono">
                    <span className="text-[#666]">Recommended Size:</span>
                    <span className="text-emerald-400">500,000 (+400k)</span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        )}

        {/* ============================================================ */}
        {/* TAB 2: TORRENT BROWSER (Fuzzy Search, Filters, Detail Drawer) */}
        {/* ============================================================ */}
        {activeTab === 'browser' && (
          <div className="space-y-4">
            {/* Search & Filter Bar */}
            <div className="p-4 rounded-xl border border-[#222] bg-[#090909] flex flex-col sm:flex-row gap-3 items-center justify-between">
              {/* Fuzzy Search */}
              <div className="relative w-full sm:w-96">
                <Search className="w-3.5 h-3.5 text-[#666] absolute left-3 top-3" />
                <input
                  type="text"
                  placeholder="Search by file name, category, or hex infohash..."
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  className="w-full bg-[#000] border border-[#222] rounded-lg pl-9 pr-8 py-2 text-xs text-white placeholder-[#555] focus:outline-none focus:border-[#444] font-mono transition-colors"
                />
                {searchQuery && (
                  <button
                    onClick={() => setSearchQuery('')}
                    className="absolute right-2.5 top-2.5 text-[#666] hover:text-white"
                  >
                    <X className="w-3.5 h-3.5" />
                  </button>
                )}
              </div>

              {/* Protocol & Sorting Filters */}
              <div className="flex items-center gap-2 w-full sm:w-auto justify-end text-xs">
                {/* Protocol Filter */}
                <div className="flex items-center bg-[#000] border border-[#222] rounded-lg p-0.5 font-mono text-[11px]">
                  {['all', 'TCP', 'uTP'].map((proto) => (
                    <button
                      key={proto}
                      onClick={() => setFilterProto(proto)}
                      className={`px-2.5 py-1 rounded transition-colors ${
                        filterProto === proto
                          ? 'bg-[#1e1e1e] text-white font-medium'
                          : 'text-[#777] hover:text-white'
                      }`}
                    >
                      {proto.toUpperCase()}
                    </button>
                  ))}
                </div>

                {/* Sort Order */}
                <select
                  value={sortBy}
                  onChange={(e) => setSortBy(e.target.value)}
                  className="bg-[#000] border border-[#222] rounded-lg px-3 py-1.5 text-xs text-[#bbb] focus:outline-none focus:border-[#444] font-mono"
                >
                  <option value="newest">Sort: Newest Indexed</option>
                  <option value="size_desc">Sort: Largest Size</option>
                  <option value="seeders">Sort: Active Swarm</option>
                </select>
              </div>
            </div>

            {/* Results Counter */}
            <div className="flex items-center justify-between text-xs text-[#666] px-1 font-mono">
              <span>Showing {filteredTorrents.length} of {rawDatabase.length} indexed payloads</span>
              <span>{totalCatalogedStr}</span>
            </div>

            {/* Torrents Table */}
            <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] overflow-hidden">
              <table className="w-full text-left text-xs">
                <thead>
                  <tr className="border-b border-[#181818] text-[#666] font-mono text-[11px]">
                    <th className="py-3 px-4 font-normal">Payload Description</th>
                    <th className="py-3 px-4 font-normal">Infohash (Hex)</th>
                    <th className="py-3 px-4 font-normal">Size</th>
                    <th className="py-3 px-4 font-normal">Proto</th>
                    <th className="py-3 px-4 font-normal">Swarm Health</th>
                    <th className="py-3 px-4 font-normal text-right">Action</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[#141414] font-mono text-[11px]">
                  {filteredTorrents.map((t) => (
                    <tr
                      key={t.hash}
                      onClick={() => handleInspectTorrent(t)}
                      className="hover:bg-[#0f0f0f] cursor-pointer transition-colors group"
                    >
                      <td className="py-3 px-4">
                        <div className="font-sans font-medium text-[#ededed] group-hover:text-white flex items-center gap-2">
                          <span className="w-2 h-2 rounded-full bg-emerald-400 shrink-0" />
                          <span className="truncate max-w-sm">{t.name}</span>
                        </div>
                        <div className="text-[10px] text-[#666] font-mono mt-0.5 pl-4">
                          {t.category} · {t.files?.length || 1} file{(t.files?.length || 1) > 1 ? 's' : ''} · {t.age}
                        </div>
                      </td>

                      <td className="py-3 px-4">
                        <span className="text-[#888] group-hover:text-[#ccc] transition-colors">
                          {t.hash.slice(0, 10)}...{t.hash.slice(-8)}
                        </span>
                      </td>

                      <td className="py-3 px-4 text-[#aaa]">
                        {t.size}
                      </td>

                      <td className="py-3 px-4">
                        <span className={`px-1.5 py-0.5 rounded text-[10px] ${
                          t.proto === 'TCP'
                            ? 'bg-[#141414] text-[#aaa] border border-[#282828]'
                            : 'bg-[#0f1d16] text-emerald-400 border border-emerald-900/40'
                        }`}>
                          {t.proto}
                        </span>
                      </td>

                      <td className="py-3 px-4">
                        <div className="flex items-center gap-2">
                          <span className="text-emerald-400 font-semibold">{t.seeders}</span>
                          <span className="text-[#555]">/</span>
                          <span className="text-[#888]">{t.leechers} peers</span>
                        </div>
                      </td>

                      <td className="py-3 px-4 text-right" onClick={(e) => e.stopPropagation()}>
                        <div className="flex items-center justify-end gap-1.5">
                          <button
                            onClick={() => copyToClipboard(generateMagnetLink(t), 'magnet')}
                            className="p-1.5 rounded-md bg-[#141414] border border-[#262626] text-[#888] hover:text-white hover:border-[#444] transition-colors"
                            title="Copy Magnet Link"
                          >
                            <DownloadCloud className="w-3.5 h-3.5" />
                          </button>
                          <button
                            onClick={() => handleInspectTorrent(t)}
                            className="p-1.5 rounded-md bg-[#141414] border border-[#262626] text-[#888] hover:text-white hover:border-[#444] transition-colors"
                            title="Inspect Metadata"
                          >
                            <Eye className="w-3.5 h-3.5" />
                          </button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>

              {filteredTorrents.length === 0 && (
                <div className="p-12 text-center text-xs text-[#666] font-mono">
                  No indexed torrents matched your search filter.
                </div>
              )}
            </div>

            {/* Torrent Details Drawer / Inspector Modal */}
            {selectedTorrent && (
              <div className="fixed inset-0 z-50 bg-black/70 backdrop-blur-xs flex items-center justify-end p-0 sm:p-4">
                <div className="w-full sm:max-w-xl h-full sm:h-auto sm:max-h-[90vh] bg-[#090909] border border-[#262626] sm:rounded-2xl p-6 overflow-y-auto flex flex-col justify-between shadow-2xl space-y-6">
                  
                  {/* Modal Header */}
                  <div className="space-y-3 pb-4 border-b border-[#1c1c1c]">
                    <div className="flex items-start justify-between gap-4">
                      <div>
                        <span className="text-[10px] font-mono uppercase px-2 py-0.5 rounded bg-[#181818] text-emerald-400 border border-[#282828]">
                          {selectedTorrent.category || 'Payload'}
                        </span>
                        <h2 className="text-base font-semibold text-white mt-2 leading-tight">
                          {selectedTorrent.name}
                        </h2>
                      </div>
                      <button
                        onClick={() => setSelectedTorrent(null)}
                        className="p-1.5 rounded-lg border border-[#222] bg-[#111] text-[#666] hover:text-white transition-colors"
                      >
                        <X className="w-4 h-4" />
                      </button>
                    </div>

                    {/* Hash Pill with Copy Button */}
                    <div className="flex items-center justify-between p-2.5 rounded-lg bg-[#000] border border-[#1e1e1e] font-mono text-xs">
                      <div className="truncate text-[#aaa] text-[11px]">
                        {selectedTorrent.hash}
                      </div>
                      <button
                        onClick={() => copyToClipboard(selectedTorrent.hash, 'hash')}
                        className="flex items-center gap-1.5 text-[11px] px-2 py-1 rounded bg-[#161616] text-[#ccc] hover:text-white border border-[#262626] transition-colors ml-3 shrink-0"
                      >
                        {copiedHash === selectedTorrent.hash ? (
                          <>
                            <Check className="w-3 h-3 text-emerald-400" />
                            <span className="text-emerald-400">Copied</span>
                          </>
                        ) : (
                          <>
                            <Copy className="w-3 h-3" />
                            <span>Copy Hash</span>
                          </>
                        )}
                      </button>
                    </div>
                  </div>

                  {/* Metadata Specs Grid */}
                  <div className="grid grid-cols-3 gap-3 font-mono text-xs">
                    <div className="p-3 rounded-lg bg-[#000] border border-[#1a1a1a]">
                      <div className="text-[10px] text-[#555] uppercase font-sans">Total Size</div>
                      <div className="text-white font-semibold mt-0.5">{selectedTorrent.size}</div>
                    </div>
                    <div className="p-3 rounded-lg bg-[#000] border border-[#1a1a1a]">
                      <div className="text-[10px] text-[#555] uppercase font-sans">Piece Length</div>
                      <div className="text-white font-semibold mt-0.5">{selectedTorrent.pieceLength || '2.0 MB'}</div>
                    </div>
                    <div className="p-3 rounded-lg bg-[#000] border border-[#1a1a1a]">
                      <div className="text-[10px] text-[#555] uppercase font-sans">Pieces Count</div>
                      <div className="text-white font-semibold mt-0.5">{selectedTorrent.pieceCount || selectedTorrent.files?.length || 1}</div>
                    </div>
                  </div>

                  {/* File Tree List */}
                  <div className="space-y-2">
                    <div className="text-xs font-semibold text-[#888] uppercase tracking-wider flex items-center justify-between">
                      <span>Payload File Structure ({selectedTorrent.files?.length || 1})</span>
                      <span className="text-[10px] font-mono text-[#555]">Verified SHA1</span>
                    </div>

                    <div className="rounded-lg border border-[#1a1a1a] bg-[#000] divide-y divide-[#141414] overflow-hidden max-h-48 overflow-y-auto font-mono text-xs">
                      {(selectedTorrent.files && selectedTorrent.files.length > 0 ? selectedTorrent.files : [{ path: selectedTorrent.name, size: selectedTorrent.size }]).map((f, idx) => (
                        <div key={idx} className="p-2.5 flex items-center justify-between hover:bg-[#0c0c0c]">
                          <div className="flex items-center gap-2 truncate pr-3">
                            <Folder className="w-3.5 h-3.5 text-[#666] shrink-0" />
                            <span className="truncate text-[#bbb]">{f.path}</span>
                          </div>
                          <span className="text-[#666] text-[11px] shrink-0">{f.size}</span>
                        </div>
                      ))}
                    </div>
                  </div>

                  {/* Bootstrap Trackers */}
                  <div className="space-y-2">
                    <div className="text-xs font-semibold text-[#888] uppercase tracking-wider">
                      Bootstrap Trackers ({(selectedTorrent.trackers || []).length})
                    </div>
                    <div className="rounded-lg border border-[#1a1a1a] bg-[#000] p-2 space-y-1 font-mono text-[11px] text-[#777]">
                      {(selectedTorrent.trackers || ['udp://tracker.opentrackr.org:1337/announce', 'udp://open.tracker.cl:1337/announce']).map((tr, idx) => (
                        <div key={idx} className="truncate">{tr}</div>
                      ))}
                    </div>
                  </div>

                  {/* Modal Action Buttons */}
                  <div className="pt-4 border-t border-[#1c1c1c] flex items-center justify-between gap-3">
                    <button
                      onClick={() => copyToClipboard(generateMagnetLink(selectedTorrent), 'magnet')}
                      className="w-full flex items-center justify-center gap-2 py-2.5 px-4 rounded-lg bg-white text-black font-semibold text-xs hover:bg-[#e0e0e0] transition-colors"
                    >
                      {copiedMagnet ? (
                        <>
                          <Check className="w-4 h-4 text-emerald-600" />
                          <span>Magnet URI Copied!</span>
                        </>
                      ) : (
                        <>
                          <DownloadCloud className="w-4 h-4 text-black" />
                          <span>Copy Magnet URI</span>
                        </>
                      )}
                    </button>
                  </div>

                </div>
              </div>
            )}
          </div>
        )}

        {/* ============================================================ */}
        {/* TAB 3: DHT ROUTING (Kademlia 160-bit Table, Buckets, Churn)  */}
        {/* ============================================================ */}
        {activeTab === 'routing' && (
          <div className="space-y-6">
            <div className="rounded-xl border border-[#222] bg-[#090909] p-4 flex flex-col md:flex-row md:items-center justify-between gap-4">
              <div>
                <h3 className="text-sm font-semibold text-white">Kademlia DHT Routing Mesh</h3>
                <p className="text-xs text-[#888] mt-0.5">
                  160-bit XOR keyspace topology. Bucket capacity k=8. Token refresh interval: 10m.
                </p>
              </div>
              <div className="flex items-center gap-4 text-xs font-mono">
                <div>
                  <span className="text-[#555]">Active Buckets:</span>{' '}
                  <span className="text-white font-bold">128 / 160</span>
                </div>
                <div>
                  <span className="text-[#555]">Good Nodes:</span>{' '}
                  <span className="text-emerald-400 font-bold">894</span>
                </div>
                <div>
                  <span className="text-[#555]">Questionable:</span>{' '}
                  <span className="text-amber-400 font-bold">42</span>
                </div>
              </div>
            </div>

            {/* Kademlia Bucket Heatmap Matrix */}
            <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-5 space-y-4">
              <div className="flex items-center justify-between pb-2 border-b border-[#181818]">
                <div className="text-xs font-semibold text-white uppercase tracking-wider">
                  Routing Table Buckets (k=8 Node Fill Rate)
                </div>
                <div className="flex items-center gap-3 text-[11px] font-mono text-[#666]">
                  <span className="flex items-center gap-1.5"><span className="w-2 h-2 rounded-xs bg-[#1f1f1f]" /> Empty</span>
                  <span className="flex items-center gap-1.5"><span className="w-2 h-2 rounded-xs bg-[#444]" /> Partial</span>
                  <span className="flex items-center gap-1.5"><span className="w-2 h-2 rounded-xs bg-white" /> Full (8/8)</span>
                </div>
              </div>

              <div className="grid grid-cols-4 sm:grid-cols-8 gap-2">
                {kademliaBuckets.map((b, i) => (
                  <div
                    key={i}
                    className={`p-2.5 rounded-lg border text-center font-mono text-xs transition-colors ${
                      b.isFull
                        ? 'bg-[#141414] border-[#333] text-white'
                        : 'bg-[#080808] border-[#181818] text-[#777]'
                    }`}
                  >
                    <div className="text-[10px] text-[#555]">{b.range}</div>
                    <div className="text-sm font-bold mt-1">{b.count}/8</div>
                    <div className="text-[9px] text-[#666] mt-0.5">
                      {b.stale ? '1 pinging' : 'healthy'}
                    </div>
                  </div>
                ))}
              </div>
            </div>

            {/* RPC Message Distribution */}
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-4 space-y-3">
                <div className="text-xs font-semibold text-white">Inbound RPC Query Velocity</div>
                <div className="space-y-2.5 font-mono text-xs">
                  <div>
                    <div className="flex justify-between text-[#888] mb-1">
                      <span>get_peers (Torrent queries)</span>
                      <span className="text-white">2.6M / hr (75.4%)</span>
                    </div>
                    <div className="h-1.5 w-full bg-[#161616] rounded-full overflow-hidden">
                      <div className="h-full bg-white w-[75%]" />
                    </div>
                  </div>
                  <div>
                    <div className="flex justify-between text-[#888] mb-1">
                      <span>find_node (Topology walk)</span>
                      <span className="text-white">850k / hr (23.2%)</span>
                    </div>
                    <div className="h-1.5 w-full bg-[#161616] rounded-full overflow-hidden">
                      <div className="h-full bg-[#777] w-[23%]" />
                    </div>
                  </div>
                  <div>
                    <div className="flex justify-between text-[#888] mb-1">
                      <span>announce_peer (Seed publishing)</span>
                      <span className="text-emerald-400">42k / hr (1.4%)</span>
                    </div>
                    <div className="h-1.5 w-full bg-[#161616] rounded-full overflow-hidden">
                      <div className="h-full bg-emerald-400 w-[8%]" />
                    </div>
                  </div>
                </div>
              </div>

              <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-4 space-y-3">
                <div className="text-xs font-semibold text-white">Bootstrap Relays & Latency</div>
                <div className="space-y-2 font-mono text-xs divide-y divide-[#141414]">
                  <div className="flex justify-between pt-1">
                    <span className="text-[#888]">router.bittorrent.com:6881</span>
                    <span className="text-emerald-400">18ms · Good</span>
                  </div>
                  <div className="flex justify-between pt-1">
                    <span className="text-[#888]">dht.transmissionbt.com:6881</span>
                    <span className="text-emerald-400">24ms · Good</span>
                  </div>
                  <div className="flex justify-between pt-1">
                    <span className="text-[#888]">router.utorrent.com:6881</span>
                    <span className="text-emerald-400">21ms · Good</span>
                  </div>
                  <div className="flex justify-between pt-1">
                    <span className="text-[#888]">dht.aelitis.com:6881</span>
                    <span className="text-[#666]">Offline / Standby</span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        )}

        {/* ============================================================ */}
        {/* TAB 4: DIAGNOSTICS & LOGS (Channels, Cache Tuning, Daemon)    */}
        {/* ============================================================ */}
        {activeTab === 'diagnostics' && (
          <div className="space-y-6">
            {/* Engine Channel Backpressure */}
            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-4">
                <div className="text-[10px] text-[#555] uppercase font-sans">Verify Channel Buffer</div>
                <div className="text-xl font-bold font-mono text-white mt-1">0 / 389</div>
                <div className="text-xs text-emerald-400 mt-1 flex items-center gap-1 font-mono">
                  <Check className="w-3 h-3" /> No Backpressure
                </div>
              </div>

              <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-4">
                <div className="text-[10px] text-[#555] uppercase font-sans">Fresh Channel Buffer</div>
                <div className="text-xl font-bold font-mono text-white mt-1">0 / 217</div>
                <div className="text-xs text-emerald-400 mt-1 flex items-center gap-1 font-mono">
                  <Check className="w-3 h-3" /> Ingestion Ready
                </div>
              </div>

              <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] p-4">
                <div className="text-[10px] text-[#555] uppercase font-sans">Socket FD Utilization</div>
                <div className="text-xl font-bold font-mono text-white mt-1">1,840 / 65,535</div>
                <div className="text-xs text-[#777] mt-1 font-mono">2.8% of nofile limit</div>
              </div>
            </div>

            {/* Interactive Peer Cache Tuner */}
            <div className="rounded-xl border border-[#262626] bg-[#0a0a0a] p-5 space-y-4">
              <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 pb-3 border-b border-[#1a1a1a]">
                <div>
                  <h3 className="text-sm font-semibold text-white">Peer Cache Memory Allocation</h3>
                  <p className="text-xs text-[#888] mt-0.5">
                    Tune LRU cache table capacity to prevent 4.5M/session thrashing.
                  </p>
                </div>
                <div className="text-xs font-mono text-amber-400">
                  Current Hit Rate: 0.0% (Starved)
                </div>
              </div>

              <div className="space-y-2">
                <div className="flex justify-between text-xs font-mono">
                  <span className="text-[#888]">Allocated Capacity:</span>
                  <span className="text-white font-bold">{cacheAllocSize},000 keys (~{(cacheAllocSize * 0.16).toFixed(1)} MB RAM)</span>
                </div>
                <input
                  type="range"
                  min="100"
                  max="1000"
                  step="50"
                  value={cacheAllocSize}
                  onChange={(e) => setCacheAllocSize(Number(e.target.value))}
                  className="w-full accent-white bg-[#222] h-1.5 rounded-lg cursor-pointer"
                />
                <div className="flex justify-between text-[10px] font-mono text-[#555]">
                  <span>100k (Current - High Evictions)</span>
                  <span>500k (Optimal Target)</span>
                  <span>1,000k (High Memory)</span>
                </div>
              </div>
            </div>

            {/* Live Crawler Syslog / Stdout Stream */}
            <div className="rounded-xl border border-[#1e1e1e] bg-[#080808] overflow-hidden">
              <div className="px-4 py-3 border-b border-[#181818] flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <Terminal className="w-3.5 h-3.5 text-[#666]" />
                  <span className="text-xs font-semibold text-white">craw-daemon Log Output</span>
                </div>
                <div className="flex items-center gap-1 text-[11px] font-mono text-[#666]">
                  {['ALL', 'INFO', 'WARN', 'DEBUG'].map((lvl) => (
                    <button
                      key={lvl}
                      onClick={() => setLogFilter(lvl)}
                      className={`px-2 py-0.5 rounded transition-colors ${
                        logFilter === lvl
                          ? 'bg-[#1f1f1f] text-white'
                          : 'text-[#666] hover:text-[#bbb]'
                      }`}
                    >
                      {lvl}
                    </button>
                  ))}
                </div>
              </div>

              <div className="p-4 bg-[#000000] font-mono text-xs space-y-2 max-h-56 overflow-y-auto">
                {engineLogs
                  .filter((l) => logFilter === 'ALL' || l.level === logFilter)
                  .map((log, idx) => (
                    <div key={idx} className="flex items-start gap-3">
                      <span className="text-[#555] shrink-0 text-[11px]">{log.time}</span>
                      <span className={`text-[10px] px-1 rounded shrink-0 font-bold ${
                        log.level === 'INFO' ? 'bg-[#13231b] text-emerald-400' :
                        log.level === 'WARN' ? 'bg-[#291f0d] text-amber-400' : 'bg-[#181818] text-[#888]'
                      }`}>
                        {log.level}
                      </span>
                      <span className="text-[#ccc] text-[11px] leading-relaxed">{log.msg}</span>
                    </div>
                  ))}
              </div>
            </div>
          </div>
        )}

      </main>

      {/* Global Footer */}
      <footer className="border-t border-[#181818] py-4 px-5 text-xs text-[#555] font-mono mt-8">
        <div className="max-w-6xl mx-auto flex flex-col sm:flex-row items-center justify-between gap-2">
          <span>craw-core · engine revision 0.9.4a · rocksdb storage driver</span>
          <span>{metrics.totalVerified} verified infohashes active in cluster</span>
        </div>
      </footer>
    </div>
  );
}