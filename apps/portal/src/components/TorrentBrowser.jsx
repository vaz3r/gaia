import { useEffect, useRef, useState } from 'react'
import { api, downloadTorrent, magnetFrom } from '../api.js'
import { formatBytes, formatTime } from '../utils.js'
import { Flame, Zap, TrendingUp, Layers, Search, Download, Copy, Check, Filter } from 'lucide-react'
import TorrentDetail from './TorrentDetail.jsx'

const COLS = [
  { key: 'name', label: 'Torrent Name', sortKey: 'name' },
  { key: 'category', label: 'Category', sortKey: 'category' },
  { key: 'total_size', label: 'Size', sortKey: 'size' },
  { key: 'file_count', label: 'Files', sortKey: 'files' },
  { key: 'health_score', label: 'Health', sortKey: 'health' },
  { key: 'verified_at', label: 'Indexed', sortKey: 'verified_at' },
]

export default function TorrentBrowser() {
  const [mode, setMode] = useState('all') // 'all' | 'trending' | 'new' | 'top'
  const [input, setInput] = useState('')
  const [search, setSearch] = useState('')
  const [category, setCategory] = useState('All')
  const [sort, setSort] = useState('verified_at')
  const [order, setOrder] = useState('desc')
  const [page, setPage] = useState(1)
  const [limit] = useState(25)
  const [data, setData] = useState(null)
  const [error, setError] = useState(null)
  const [loading, setLoading] = useState(false)
  const [detailInfohash, setDetailInfohash] = useState(null)
  const [copiedHash, setCopiedHash] = useState(null)
  const debounce = useRef()

  function handleModeChange(newMode) {
    setMode(newMode)
    setPage(1)
    if (newMode === 'trending') {
      setSort('popularity')
      setOrder('desc')
    } else if (newMode === 'new') {
      setSort('first_seen')
      setOrder('desc')
    } else if (newMode === 'top') {
      setSort('sightings')
      setOrder('desc')
    } else {
      setSort('verified_at')
      setOrder('desc')
    }
  }

  useEffect(() => {
    clearTimeout(debounce.current)
    debounce.current = setTimeout(() => {
      setSearch(input.trim())
      setPage(1)
    }, 300)
    return () => clearTimeout(debounce.current)
  }, [input])

  useEffect(() => {
    const controller = new AbortController()
    setLoading(true)
    setError(null)
    const params = new URLSearchParams({ page, limit })
    if (search) params.set('search', search)
    if (category && category !== 'All') params.set('category', category)
    if (sort) params.set('sort', sort)
    params.set('order', order)

    api(`/api/torrents?${params}`, { signal: controller.signal })
      .then((res) => {
        setData(res)
      })
      .catch((e) => {
        if (e.name !== 'AbortError' && !controller.signal.aborted) {
          setError(e.message)
        }
      })
      .finally(() => {
        if (!controller.signal.aborted) {
          setLoading(false)
        }
      })

    return () => {
      controller.abort()
    }
  }, [search, category, sort, order, page, limit])

  function toggleSort(key) {
    if (sort === key) {
      setOrder(order === 'asc' ? 'desc' : 'asc')
    } else {
      setSort(key)
      setOrder(key === 'name' ? 'asc' : 'desc')
    }
    setPage(1)
  }

  function handleCopyMagnet(e, row) {
    e.stopPropagation()
    const uri = magnetFrom(row.infohash, row.name)
    navigator.clipboard?.writeText(uri).then(() => {
      setCopiedHash(row.infohash)
      setTimeout(() => setCopiedHash(null), 2000)
    }).catch(() => {})
  }

  async function handleDownloadFile(e, row) {
    e.stopPropagation()
    try {
      await downloadTorrent(row.infohash, row.name)
    } catch (err) {
      alert(`Download failed: ${err.message}`)
    }
  }

  const torrents = data?.data || []
  const total = data?.total || 0
  const totalPages = data?.pages || Math.max(1, Math.ceil(total / limit))

  return (
    <div className="space-y-4">
      {/* Controls Header */}
      <div className="flex flex-col lg:flex-row lg:items-center justify-between gap-3 bg-ink-900/60 p-3.5 rounded-xl border border-ink-800">
        {/* Swarm Mode Pills */}
        <div className="flex items-center gap-1.5 bg-ink-950 p-1 rounded-lg border border-ink-800/80 overflow-x-auto">
          <button
            type="button"
            onClick={() => handleModeChange('all')}
            className={`px-3 py-1.5 text-xs font-medium rounded-md transition-all flex items-center gap-1.5 whitespace-nowrap ${
              mode === 'all'
                ? 'bg-ink-800 text-white shadow-xs border border-ink-700'
                : 'text-slate-400 hover:text-slate-200 hover:bg-ink-900'
            }`}
          >
            <Layers className="w-3.5 h-3.5 text-cyan-400" />
            <span>All Torrents</span>
          </button>

          <button
            type="button"
            onClick={() => handleModeChange('trending')}
            className={`px-3 py-1.5 text-xs font-medium rounded-md transition-all flex items-center gap-1.5 whitespace-nowrap ${
              mode === 'trending'
                ? 'bg-ink-800 text-cyan-300 font-semibold shadow-xs border border-cyan-500/40'
                : 'text-slate-400 hover:text-cyan-300 hover:bg-ink-900'
            }`}
          >
            <TrendingUp className="w-3.5 h-3.5 text-cyan-400" />
            <span>Trending Swarms</span>
          </button>

          <button
            type="button"
            onClick={() => handleModeChange('new')}
            className={`px-3 py-1.5 text-xs font-medium rounded-md transition-all flex items-center gap-1.5 whitespace-nowrap ${
              mode === 'new'
                ? 'bg-ink-800 text-amber-300 font-semibold shadow-xs border border-amber-500/40'
                : 'text-slate-400 hover:text-amber-300 hover:bg-ink-900'
            }`}
          >
            <Zap className="w-3.5 h-3.5 text-amber-400" />
            <span>New Releases (&lt;48h)</span>
          </button>

          <button
            type="button"
            onClick={() => handleModeChange('top')}
            className={`px-3 py-1.5 text-xs font-medium rounded-md transition-all flex items-center gap-1.5 whitespace-nowrap ${
              mode === 'top'
                ? 'bg-ink-800 text-rose-300 font-semibold shadow-xs border border-rose-500/40'
                : 'text-slate-400 hover:text-rose-300 hover:bg-ink-900'
            }`}
          >
            <Flame className="w-3.5 h-3.5 text-rose-400" />
            <span>Top Sightings</span>
          </button>
        </div>

        {/* Search & Category Filter */}
        <div className="flex items-center gap-2.5">
          <div className="relative flex-1 sm:w-72">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-slate-500" />
            <input
              type="text"
              value={input}
              onChange={(e) => setInput(e.target.value)}
              placeholder="Search 3.4M+ torrents (sub-50ms)..."
              className="w-full bg-ink-950 border border-ink-700/80 rounded-lg pl-9 pr-8 py-1.5 text-xs text-slate-100 placeholder-slate-500 focus:outline-hidden focus:border-cyan-500 transition-colors"
            />
            {input && (
              <button
                onClick={() => setInput('')}
                className="absolute right-2.5 top-1/2 -translate-y-1/2 text-slate-500 hover:text-slate-300 text-xs"
              >
                ✕
              </button>
            )}
          </div>

          <select
            value={category}
            onChange={(e) => {
              setCategory(e.target.value)
              setPage(1)
            }}
            className="bg-ink-950 border border-ink-700/80 rounded-lg px-3 py-1.5 text-xs text-slate-300 focus:outline-hidden focus:border-cyan-500"
          >
            <option value="All">All Categories</option>
            <option value="Video">Video</option>
            <option value="Audio">Audio</option>
            <option value="Applications">Apps</option>
            <option value="Games">Games</option>
            <option value="Television">TV</option>
            <option value="Other">Other</option>
          </select>
        </div>
      </div>

      {/* Main Torrent Table */}
      <div className="rounded-xl border border-ink-800 bg-ink-900/40 overflow-hidden">
        <div className="overflow-x-auto">
          <table className="w-full text-left border-collapse">
            <thead>
              <tr className="border-b border-ink-800 bg-ink-950/80 text-[11px] font-mono uppercase tracking-wider text-slate-400">
                {COLS.map((col) => (
                  <th
                    key={col.key}
                    onClick={() => toggleSort(col.sortKey)}
                    className="px-4 py-3 cursor-pointer hover:text-slate-200 transition-colors select-none"
                  >
                    <div className="flex items-center gap-1">
                      <span>{col.label}</span>
                      {sort === col.sortKey && (
                        <span className="text-cyan-400 text-xs">{order === 'asc' ? '▲' : '▼'}</span>
                      )}
                    </div>
                  </th>
                ))}
                <th className="px-4 py-3 text-right">Actions</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-ink-800/60 text-xs font-mono">
              {loading ? (
                <tr>
                  <td colSpan={COLS.length + 1} className="py-16 text-center text-slate-500">
                    <div className="flex items-center justify-center gap-2">
                      <div className="w-4 h-4 border-2 border-cyan-400 border-t-transparent rounded-full animate-spin" />
                      <span>Querying Quickwit distributed index...</span>
                    </div>
                  </td>
                </tr>
              ) : error ? (
                <tr>
                  <td colSpan={COLS.length + 1} className="py-12 text-center text-rose-400">
                    Error querying index: {error}
                  </td>
                </tr>
              ) : torrents.length === 0 ? (
                <tr>
                  <td colSpan={COLS.length + 1} className="py-16 text-center text-slate-500">
                    No torrents match your query.
                  </td>
                </tr>
              ) : (
                torrents.map((row) => (
                  <tr
                    key={row.infohash}
                    onClick={() => setDetailInfohash(row.infohash)}
                    className="hover:bg-ink-800/50 cursor-pointer transition-colors group"
                  >
                    {/* Name */}
                    <td className="px-4 py-3 max-w-md min-w-[240px]">
                      <div className="font-sans font-medium text-slate-200 group-hover:text-cyan-300 transition-colors line-clamp-1">
                        {row.name || row.infohash}
                      </div>
                      <div className="text-[10px] text-slate-500 truncate mt-0.5">
                        {row.infohash}
                      </div>
                    </td>

                    {/* Category */}
                    <td className="px-4 py-3 whitespace-nowrap">
                      <span className="inline-block px-2 py-0.5 rounded text-[10px] bg-ink-800 border border-ink-700 text-slate-300">
                        {row.category || 'General'}
                      </span>
                    </td>

                    {/* Size */}
                    <td className="px-4 py-3 text-slate-300 whitespace-nowrap">
                      {formatBytes(row.total_size)}
                    </td>

                    {/* Files */}
                    <td className="px-4 py-3 text-slate-400 whitespace-nowrap">
                      {row.file_count ?? 1}
                    </td>

                    {/* Health */}
                    <td className="px-4 py-3 whitespace-nowrap">
                      <div className="flex items-center gap-2">
                        <div className="w-12 bg-ink-950 rounded-full h-1.5 overflow-hidden border border-ink-800">
                          <div
                            className={`h-full rounded-full ${
                              (row.health_score ?? 0) >= 70
                                ? 'bg-emerald-400'
                                : (row.health_score ?? 0) >= 40
                                ? 'bg-amber-400'
                                : 'bg-slate-600'
                            }`}
                            style={{ width: `${Math.min(100, Math.max(5, row.health_score ?? 0))}%` }}
                          />
                        </div>
                        <span className="text-[11px] text-slate-400">
                          {row.health_score ?? 0}
                        </span>
                      </div>
                    </td>

                    {/* Verified */}
                    <td className="px-4 py-3 text-slate-400 whitespace-nowrap text-[11px]">
                      {formatTime(row.verified_at)}
                    </td>

                    {/* Actions */}
                    <td className="px-4 py-3 text-right whitespace-nowrap" onClick={(e) => e.stopPropagation()}>
                      <div className="flex items-center justify-end gap-1.5">
                        <button
                          type="button"
                          onClick={(e) => handleCopyMagnet(e, row)}
                          className="p-1.5 rounded-md bg-ink-800 border border-ink-700/80 text-slate-400 hover:text-white hover:bg-ink-700 transition-colors"
                          title="Copy Magnet Link"
                        >
                          {copiedHash === row.infohash ? (
                            <Check className="w-3.5 h-3.5 text-emerald-400" />
                          ) : (
                            <Copy className="w-3.5 h-3.5" />
                          )}
                        </button>
                        <button
                          type="button"
                          onClick={(e) => handleDownloadFile(e, row)}
                          className="p-1.5 rounded-md bg-cyan-500/10 border border-cyan-500/30 text-cyan-400 hover:bg-cyan-500/20 transition-colors"
                          title="Download .torrent file"
                        >
                          <Download className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>

        {/* Pagination Bar */}
        <div className="flex items-center justify-between px-4 py-3 border-t border-ink-800 bg-ink-950/60 text-xs font-mono text-slate-400">
          <div>
            Showing <span className="text-slate-200">{torrents.length}</span> of <span className="text-slate-200">{total.toLocaleString()}</span> torrents
          </div>

          <div className="flex items-center gap-2">
            <button
              onClick={() => setPage((p) => Math.max(1, p - 1))}
              disabled={page <= 1 || loading}
              className="px-2.5 py-1 rounded-md bg-ink-800 border border-ink-700 text-slate-300 disabled:opacity-40 disabled:cursor-not-allowed hover:text-white hover:bg-ink-700 transition-colors"
            >
              Previous
            </button>
            <span>
              Page <span className="text-slate-200">{page}</span> of <span className="text-slate-200">{totalPages.toLocaleString()}</span>
            </span>
            <button
              onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
              disabled={page >= totalPages || loading}
              className="px-2.5 py-1 rounded-md bg-ink-800 border border-ink-700 text-slate-300 disabled:opacity-40 disabled:cursor-not-allowed hover:text-white hover:bg-ink-700 transition-colors"
            >
              Next
            </button>
          </div>
        </div>
      </div>

      {/* Details Modal */}
      {detailInfohash && (
        <TorrentDetail
          infohash={detailInfohash}
          onClose={() => setDetailInfohash(null)}
        />
      )}
    </div>
  )
}
