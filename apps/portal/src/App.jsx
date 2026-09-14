import React, { useState, useEffect, useCallback } from 'react';
import { 
  Globe, 
  Sparkles, 
  ArrowUpDown, 
  ChevronLeft, 
  ChevronRight, 
  Database, 
  ShieldCheck, 
  Zap 
} from 'lucide-react';
import SearchBar from './components/SearchBar.jsx';
import CategoryFilter from './components/CategoryFilter.jsx';
import TorrentCard from './components/TorrentCard.jsx';
import TorrentModal from './components/TorrentModal.jsx';

export default function App() {
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState('All');
  const [sortBy, setSortBy] = useState('date');
  const [order, setOrder] = useState('desc');
  const [page, setPage] = useState(1);
  const [limit] = useState(25);

  const [results, setResults] = useState({ total: 0, hits: [], elapsedMicros: 0 });
  const [loading, setLoading] = useState(false);
  const [selectedTorrent, setSelectedTorrent] = useState(null);
  const [stats, setStats] = useState(null);

  // Fetch dashboard stats once for hero banner
  useEffect(() => {
    fetch('/api/stats')
      .then((r) => r.json())
      .then((data) => setStats(data))
      .catch((e) => console.error('Failed to load stats:', e));
  }, []);

  const fetchTorrents = useCallback(async () => {
    setLoading(true);
    try {
      const params = new URLSearchParams({
        q: query,
        category: category === 'All' ? '' : category,
        page: page.toString(),
        limit: limit.toString(),
        sort_by: sortBy,
        order: order,
      });

      const res = await fetch(`/api/torrents?${params.toString()}`);
      if (!res.ok) throw new Error('Search request failed');
      const data = await res.json();
      setResults(data);
    } catch (err) {
      console.error('Failed to fetch torrents:', err);
    } finally {
      setLoading(false);
    }
  }, [query, category, page, limit, sortBy, order]);

  useEffect(() => {
    fetchTorrents();
  }, [fetchTorrents]);

  const handleSearchSubmit = (newQuery) => {
    setQuery(newQuery);
    setPage(1);
  };

  const handleCategorySelect = (newCategory) => {
    setCategory(newCategory);
    setPage(1);
  };

  const totalPages = Math.ceil((results.total || 0) / limit);

  return (
    <div className="min-h-screen flex flex-col bg-gaia-950 text-slate-100">
      {/* Top Navigation */}
      <header className="sticky top-0 z-40 bg-slate-950/80 backdrop-blur-xl border-b border-slate-800/80">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 h-16 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="w-10 h-10 rounded-2xl bg-gradient-to-tr from-blue-600 via-indigo-600 to-purple-600 flex items-center justify-center shadow-lg shadow-blue-500/20">
              <Globe className="w-5 h-5 text-white" />
            </div>
            <div>
              <span className="text-lg font-black tracking-tight text-white flex items-center gap-1.5">
                GAIA <span className="text-xs font-mono px-1.5 py-0.5 rounded bg-blue-500/20 text-blue-400 font-normal">INDEXER</span>
              </span>
              <p className="text-[10px] text-slate-400 font-mono tracking-wider uppercase">Decentralized DHT Search</p>
            </div>
          </div>

          <div className="flex items-center gap-3">
            {stats?.totalTorrents > 0 && (
              <div className="hidden sm:flex items-center gap-2 px-3 py-1.5 rounded-full bg-slate-900 border border-slate-800 text-xs font-mono text-slate-300">
                <Database className="w-3.5 h-3.5 text-blue-400" />
                <span>{(stats.totalTorrents).toLocaleString()} Releases</span>
              </div>
            )}
            <div className="flex items-center gap-1.5 px-3 py-1.5 rounded-full bg-emerald-500/10 border border-emerald-500/20 text-xs font-medium text-emerald-400">
              <span className="w-2 h-2 rounded-full bg-emerald-400 animate-pulse" />
              <span>Direct Download</span>
            </div>
          </div>
        </div>
      </header>

      {/* Main Content */}
      <main className="flex-1 max-w-7xl w-full mx-auto px-4 sm:px-6 lg:px-8 py-8 sm:py-12 space-y-8">
        {/* Hero Section */}
        <div className="text-center max-w-3xl mx-auto space-y-4">
          <div className="inline-flex items-center gap-2 px-3 py-1 rounded-full bg-blue-500/10 border border-blue-500/20 text-blue-400 text-xs font-medium">
            <Zap className="w-3.5 h-3.5" />
            <span>Sub-Millisecond Tantivy Inverted Index Search</span>
          </div>
          <h1 className="text-3xl sm:text-5xl font-black text-slate-100 tracking-tight leading-tight">
            Discover & Download Without Limits
          </h1>
          <p className="text-sm sm:text-base text-slate-400">
            Clean, AI-classified BitTorrent DHT releases with live swarm health telemetry and instant 1-click magnet links.
          </p>
        </div>

        {/* Search & Filter Controls */}
        <div className="max-w-4xl mx-auto space-y-4">
          <SearchBar 
            value={query} 
            onChange={setQuery} 
            onSearch={handleSearchSubmit} 
            isLoading={loading} 
          />

          <CategoryFilter 
            selected={category} 
            onSelect={handleCategorySelect} 
          />
        </div>

        {/* Meta Bar: Results count, sorting, latency */}
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 pt-4 border-t border-slate-800/80 text-xs sm:text-sm text-slate-400">
          <div className="flex items-center gap-2">
            <span className="font-semibold text-slate-200">
              {(results.total || 0).toLocaleString()} releases found
            </span>
            {results.elapsedMicros > 0 && (
              <span className="text-slate-500 font-mono">
                ({(results.elapsedMicros / 1000).toFixed(1)} ms)
              </span>
            )}
          </div>

          {/* Sorting */}
          <div className="flex items-center gap-2">
            <ArrowUpDown className="w-4 h-4 text-slate-500" />
            <span className="text-slate-400">Sort:</span>
            <select
              value={sortBy}
              onChange={(e) => {
                setSortBy(e.target.value);
                setPage(1);
              }}
              className="bg-slate-900 border border-slate-800 rounded-lg px-2.5 py-1 text-slate-200 text-xs focus:outline-none focus:ring-1 focus:ring-blue-500"
            >
              <option value="date">Verified Date</option>
              <option value="health">Swarm Health</option>
              <option value="size">Total Size</option>
              <option value="popularity">Popularity</option>
            </select>
            <button
              onClick={() => setOrder(order === 'asc' ? 'desc' : 'asc')}
              className="px-2 py-1 bg-slate-900 border border-slate-800 rounded-lg text-slate-300 hover:text-white uppercase font-mono text-xs"
              title="Toggle ascending / descending"
            >
              {order}
            </button>
          </div>
        </div>

        {/* Results List */}
        <div className="space-y-3">
          {loading && results.hits.length === 0 ? (
            <div className="py-20 text-center text-slate-500 space-y-3">
              <span className="inline-block w-8 h-8 border-3 border-blue-500 border-t-transparent rounded-full animate-spin" />
              <p className="text-sm">Searching 3.4M+ releases in Quickwit...</p>
            </div>
          ) : results.hits.length > 0 ? (
            results.hits.map((torrent) => (
              <TorrentCard
                key={torrent.infohash}
                torrent={torrent}
                onSelect={(t) => setSelectedTorrent(t)}
              />
            ))
          ) : (
            <div className="py-20 text-center text-slate-500 space-y-2 border border-dashed border-slate-800 rounded-3xl">
              <Sparkles className="w-8 h-8 mx-auto text-slate-600" />
              <h3 className="text-base font-semibold text-slate-300">No releases found</h3>
              <p className="text-xs text-slate-500">Try adjusting your search terms or selecting 'All Releases'.</p>
            </div>
          )}
        </div>

        {/* Pagination */}
        {totalPages > 1 && (
          <div className="flex items-center justify-center gap-2 pt-6">
            <button
              disabled={page <= 1}
              onClick={() => setPage(p => Math.max(1, p - 1))}
              className="p-2 bg-slate-900 border border-slate-800 rounded-xl text-slate-400 hover:text-white disabled:opacity-30 disabled:pointer-events-none transition"
              title="Previous Page"
            >
              <ChevronLeft className="w-5 h-5" />
            </button>

            <span className="px-4 py-2 bg-slate-900 border border-slate-800 rounded-xl text-xs sm:text-sm font-mono text-slate-300">
              Page {page} of {totalPages.toLocaleString()}
            </span>

            <button
              disabled={page >= totalPages}
              onClick={() => setPage(p => Math.min(totalPages, p + 1))}
              className="p-2 bg-slate-900 border border-slate-800 rounded-xl text-slate-400 hover:text-white disabled:opacity-30 disabled:pointer-events-none transition"
              title="Next Page"
            >
              <ChevronRight className="w-5 h-5" />
            </button>
          </div>
        )}
      </main>

      {/* Footer */}
      <footer className="border-t border-slate-900 bg-slate-950/60 py-6 text-center text-xs text-slate-500 space-y-1">
        <p>GAIA V2 Decentralized BitTorrent Indexer — Sub-Millisecond Search Engine</p>
        <p className="font-mono text-[11px] text-slate-600">Pure client-side magnet generation • Zero tracking • Powered by Quickwit & .NET 10</p>
      </footer>

      {/* Detail Modal */}
      {selectedTorrent && (
        <TorrentModal
          torrent={selectedTorrent}
          onClose={() => setSelectedTorrent(null)}
        />
      )}
    </div>
  );
}
