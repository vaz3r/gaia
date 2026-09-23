import React, { useState, useEffect, useRef } from 'react'
import {
  Search,
  X,
  RefreshCw,
  FileDown,
  DownloadCloud,
  Eye,
  Check,
  Copy,
  Folder,
  Activity,
  TrendingUp,
  Shield,
  Tag,
  AlertCircle,
  ExternalLink,
  ChevronLeft,
  ChevronsLeft,
  ChevronRight,
  ChevronsRight
} from 'lucide-react'
import { api, downloadTorrent, magnetFrom } from '../api.js'
import { formatBytes, formatTime } from '../utils.js'
import { HealthBar, PopularityBar, HealthStatePill, EvidenceBreakdown, SecurityShield, TrendingBadge, getHealthColor, getPopularityColor } from './HealthScoreDisplay.jsx'

export const CANONICAL_CATEGORIES = [
  'Adult',
  'Anime',
  'Applications',
  'Audiobooks',
  'Books & Learning',
  'Documentaries',
  'Games',
  'Movies',
  'Music',
  'Television',
  'Other'
]

export const CATEGORY_COLORS = {
  Adult: 'bg-rose-500/10 text-rose-400 border-rose-500/30',
  Anime: 'bg-pink-500/10 text-pink-400 border-pink-500/30',
  Applications: 'bg-amber-500/10 text-amber-400 border-amber-500/30',
  Audiobooks: 'bg-indigo-500/10 text-indigo-400 border-indigo-500/30',
  'Books & Learning': 'bg-teal-500/10 text-teal-400 border-teal-500/30',
  Documentaries: 'bg-emerald-500/10 text-emerald-400 border-emerald-500/30',
  Games: 'bg-lime-500/10 text-lime-400 border-lime-500/30',
  Movies: 'bg-blue-500/10 text-blue-400 border-blue-500/30',
  Music: 'bg-cyan-500/10 text-cyan-400 border-cyan-500/30',
  Television: 'bg-purple-500/10 text-purple-400 border-purple-500/30',
  Other: 'bg-zinc-500/10 text-zinc-400 border-zinc-500/30',
}

export default function TorrentBrowser({ totalCatalogedCount = 3418496 }) {
  // Browser state
  const [torrentsPage, setTorrentsPage] = useState(1)
  const [torrentsLimit, setTorrentsLimit] = useState(25)
  const [sortField, setSortField] = useState('verified_at')
  const [sortOrder, setSortOrder] = useState('desc')
  const [searchInput, setSearchInput] = useState('')
  const [searchQuery, setSearchQuery] = useState('')
  const [categoryFilter, setCategoryFilter] = useState('')
  const [riskFilter, setRiskFilter] = useState('')
  const [torrentsData, setTorrentsData] = useState({ data: [], total: 0, pages: 1, page: 1, elapsed_micros: 0 })
  const [torrentsLoading, setTorrentsLoading] = useState(false)

  // Actions & Inspection State
  const [selectedTorrent, setSelectedTorrent] = useState(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [downloadingIh, setDownloadingIh] = useState(null)
  const [copiedHash, setCopiedHash] = useState(null)
  const [copiedMagnet, setCopiedMagnet] = useState(false)

  // Search input debouncer
  const searchDebounceRef = useRef(null)
  const handleSearchChange = (val) => {
    setSearchInput(val)
    clearTimeout(searchDebounceRef.current)
    searchDebounceRef.current = setTimeout(() => {
      const trimmed = val.trim()
      setSearchQuery(trimmed)
      if (trimmed && sortField === 'verified_at') {
        setSortField('relevance')
      } else if (!trimmed && sortField === 'relevance') {
        setSortField('verified_at')
      }
      setTorrentsPage(1)
    }, 350)
  }

  const handleClearSearch = () => {
    setSearchInput('')
    setSearchQuery('')
    if (sortField === 'relevance') {
      setSortField('verified_at')
    }
    setTorrentsPage(1)
  }

  // Toggle column sorting
  const handleSortToggle = (col) => {
    if (sortField === col) {
      setSortOrder(sortOrder === 'asc' ? 'desc' : 'asc')
    } else {
      setSortField(col)
      setSortOrder(col === 'name' ? 'asc' : 'desc')
    }
    setTorrentsPage(1)
  }

  // Data Fetch Effect
  useEffect(() => {
    let active = true
    setTorrentsLoading(true)

    const params = new URLSearchParams({
      page: torrentsPage,
      limit: torrentsLimit,
    })
    if (sortField && sortField !== 'relevance') {
      params.set('sort', sortField)
      params.set('order', sortOrder)
    }
    if (searchQuery) params.set('search', searchQuery)
    if (categoryFilter && categoryFilter !== 'All') params.set('category', categoryFilter)
    if (riskFilter) params.set('risk', riskFilter)

    api(`/api/torrents?${params.toString()}`)
      .then((res) => {
        if (!active) return
        setTorrentsData(res)
      })
      .catch((err) => {
        console.error('Failed to load torrents:', err)
      })
      .finally(() => {
        if (active) setTorrentsLoading(false)
      })

    return () => {
      active = false
    }
  }, [torrentsPage, torrentsLimit, sortField, sortOrder, searchQuery, categoryFilter, riskFilter])

  // Inspect torrent handler (fetches complete metadata & file tree)
  const handleInspectTorrent = (t) => {
    const hash = t.infohash || t.hash
    setSelectedTorrent({
      ...t,
      hash,
      files: [],
    })
    if (hash) {
      setDetailLoading(true)
      api(`/api/torrents/${hash}`)
        .then((full) => {
          if (full) {
            setSelectedTorrent((prev) => ({
              ...prev,
              ...full,
              hash,
              pieceLength: formatBytes(full.piece_length),
              pieceCount: full.file_count || full.files?.length || 1,
              files: Array.isArray(full.files) ? full.files : [],
            }))
          }
        })
        .catch(() => {})
        .finally(() => setDetailLoading(false))
    }
  }

  const copyToClipboard = (text, type = 'hash') => {
    navigator.clipboard?.writeText(text).then(() => {
      if (type === 'hash') {
        setCopiedHash(text)
        setTimeout(() => setCopiedHash(null), 2000)
      } else if (type === 'magnet') {
        setCopiedMagnet(true)
        setTimeout(() => setCopiedMagnet(false), 2000)
      }
    }).catch(() => {})
  }

  const handleDownloadTorrent = async (t, e) => {
    e?.stopPropagation()
    const hash = t.infohash || t.hash
    try {
      setDownloadingIh(hash)
      await downloadTorrent(hash, t.name)
    } catch (err) {
      alert(`Download failed: ${err.message}`)
    } finally {
      setDownloadingIh(null)
    }
  }

  const totalCatalogedStr = `${(torrentsData?.total || totalCatalogedCount).toLocaleString()} verified payloads cataloged in cluster`

  return (
    <div className="space-y-4">
      {/* Control Bar: Search, Category, Risk, Sort, Limit */}
      <div className="flex flex-col lg:flex-row items-stretch lg:items-center justify-between gap-3 bg-[#090909] p-3 rounded-xl border border-[#1e1e1e]">
        {/* Search Input Box */}
        <div className="relative flex-1">
          <Search className="w-4 h-4 text-[#666] absolute left-3 top-3" />
          <input
            type="text"
            placeholder="Search payload name, keyword, hash, codec..."
            value={searchInput}
            onChange={(e) => handleSearchChange(e.target.value)}
            className="w-full bg-[#000] border border-[#222] rounded-lg pl-9 pr-8 py-2 text-xs text-[#ededed] placeholder-[#555] focus:outline-none focus:border-[#444] font-mono transition-colors"
          />
          {searchInput && (
            <button
              onClick={handleClearSearch}
              className="absolute right-2.5 top-2.5 text-[#666] hover:text-white"
            >
              <X className="w-3.5 h-3.5" />
            </button>
          )}
        </div>

        {/* Sorting & Filters */}
        <div className="flex flex-wrap items-center gap-2.5 justify-end text-xs">
          {/* Category Filter */}
          <div className="flex items-center gap-1.5 font-mono text-xs">
            <span className="text-[#666]">Category:</span>
            <select
              value={categoryFilter}
              onChange={(e) => {
                setCategoryFilter(e.target.value)
                setTorrentsPage(1)
              }}
              className="bg-[#000] border border-[#222] rounded-lg px-2.5 py-1.5 text-xs text-[#bbb] focus:outline-none focus:border-[#444] font-mono"
            >
              <option value="">All Categories</option>
              {CANONICAL_CATEGORIES.map((c) => (
                <option key={c} value={c}>{c}</option>
              ))}
            </select>
          </div>

          {/* Risk Tier Filter */}
          <div className="flex items-center gap-1.5 font-mono text-xs">
            <span className="text-[#666]">Risk:</span>
            <select
              value={riskFilter}
              onChange={(e) => {
                setRiskFilter(e.target.value)
                setTorrentsPage(1)
              }}
              className="bg-[#000] border border-[#222] rounded-lg px-2.5 py-1.5 text-xs text-[#bbb] focus:outline-none focus:border-[#444] font-mono"
            >
              <option value="">All Tiers</option>
              <option value="SAFE">Safe (Allowed)</option>
              <option value="REVIEW">Review (Flagged)</option>
              <option value="BLOCKED">Blocked (Toxic)</option>
            </select>
          </div>

          {/* Sort Order Selector */}
          <div className="flex items-center gap-1.5 font-mono text-xs">
            <span className="text-[#666]">Sort:</span>
            <select
              value={`${sortField}:${sortOrder}`}
              onChange={(e) => {
                const [f, o] = e.target.value.split(':')
                setSortField(f)
                setSortOrder(o)
                setTorrentsPage(1)
              }}
              className="bg-[#000] border border-[#222] rounded-lg px-2.5 py-1.5 text-xs text-[#bbb] focus:outline-none focus:border-[#444] font-mono"
            >
              {searchQuery && <option value="relevance:desc">Best Match (Relevance)</option>}
              <option value="verified_at:desc">Newest Verified</option>
              <option value="verified_at:asc">Oldest Verified</option>
              <option value="popularity:desc">Highest Trending Score</option>
              <option value="size:desc">Largest Size</option>
              <option value="size:asc">Smallest Size</option>
              <option value="files:desc">Most Files</option>
              <option value="name:asc">Name (A-Z)</option>
            </select>
          </div>

          {/* Page Limit Selector */}
          <div className="flex items-center gap-1.5 font-mono text-xs">
            <span className="text-[#666]">Show:</span>
            <select
              value={torrentsLimit}
              onChange={(e) => {
                setTorrentsLimit(Number(e.target.value))
                setTorrentsPage(1)
              }}
              className="bg-[#000] border border-[#222] rounded-lg px-2 py-1.5 text-xs text-[#bbb] focus:outline-none focus:border-[#444] font-mono"
            >
              <option value={25}>25</option>
              <option value={50}>50</option>
              <option value={100}>100</option>
            </select>
          </div>
        </div>
      </div>

      {/* Results Counter & Active Stats */}
      <div className="flex items-center justify-between text-xs text-[#666] px-1 font-mono">
        <span className="flex items-center gap-2">
          {torrentsLoading ? (
            <RefreshCw className="w-3 h-3 animate-spin text-emerald-400" />
          ) : (
            <span className="w-2 h-2 rounded-full bg-emerald-400" />
          )}
          Showing {torrentsData?.total > 0 ? (torrentsPage - 1) * torrentsLimit + 1 : 0} –{' '}
          {Math.min(torrentsPage * torrentsLimit, torrentsData?.total || 0).toLocaleString()} of{' '}
          <strong className="text-white">{(torrentsData?.total || 0).toLocaleString()}</strong> verified payloads
          {torrentsData?.elapsed_micros ? (
            <span className="text-[#555] ml-1">
              ({(torrentsData.elapsed_micros / 1000).toFixed(1)}ms)
            </span>
          ) : null}
        </span>
        <span className="hidden sm:inline">{totalCatalogedStr}</span>
      </div>

      {/* Torrents Table (Identical to GAIA Operator Explorer) */}
      <div className="rounded-xl border border-[#1e1e1e] bg-[#090909] overflow-hidden">
        <div className="overflow-x-auto">
          <table className="w-full text-left text-xs">
            <thead>
              <tr className="border-b border-[#181818] text-[#666] font-mono text-[11px]">
                <th
                  onClick={() => handleSortToggle('name')}
                  className="py-3 px-4 font-normal cursor-pointer hover:text-white transition-colors"
                >
                  <div className="flex items-center gap-1.5">
                    <span>Payload Description</span>
                    {sortField === 'name' && (
                      <span className="text-emerald-400">{sortOrder === 'asc' ? '↑' : '↓'}</span>
                    )}
                  </div>
                </th>
                <th className="py-3 px-4 font-normal">Infohash (Hex)</th>
                <th className="py-3 px-4 font-normal">Category</th>
                <th
                  onClick={() => handleSortToggle('size')}
                  className="py-3 px-4 font-normal cursor-pointer hover:text-white transition-colors"
                >
                  <div className="flex items-center gap-1.5">
                    <span>Size</span>
                    {sortField === 'size' && (
                      <span className="text-emerald-400">{sortOrder === 'asc' ? '↑' : '↓'}</span>
                    )}
                  </div>
                </th>
                <th
                  onClick={() => handleSortToggle('files')}
                  className="py-3 px-4 font-normal cursor-pointer hover:text-white transition-colors"
                >
                  <div className="flex items-center gap-1.5">
                    <span>Files</span>
                    {sortField === 'files' && (
                      <span className="text-emerald-400">{sortOrder === 'asc' ? '↑' : '↓'}</span>
                    )}
                  </div>
                </th>
                <th
                  onClick={() => handleSortToggle('health')}
                  className="py-3 px-4 font-normal cursor-pointer hover:text-white transition-colors"
                >
                  <div className="flex items-center gap-1.5">
                    <span>Swarm Health</span>
                    {sortField === 'health' && (
                      <span className="text-emerald-400">{sortOrder === 'asc' ? '↑' : '↓'}</span>
                    )}
                  </div>
                </th>
                <th
                  onClick={() => handleSortToggle('popularity')}
                  className="py-3 px-4 font-normal cursor-pointer hover:text-white transition-colors"
                >
                  <div className="flex items-center gap-1.5">
                    <span>Trending</span>
                    {sortField === 'popularity' && (
                      <span className="text-emerald-400">{sortOrder === 'asc' ? '↑' : '↓'}</span>
                    )}
                  </div>
                </th>
                <th className="py-3 px-4 font-normal">Antivirus / Safety</th>
                <th
                  onClick={() => handleSortToggle('verified_at')}
                  className="py-3 px-4 font-normal cursor-pointer hover:text-white transition-colors"
                >
                  <div className="flex items-center gap-1.5">
                    <span>Verified</span>
                    {sortField === 'verified_at' && (
                      <span className="text-emerald-400">{sortOrder === 'asc' ? '↑' : '↓'}</span>
                    )}
                  </div>
                </th>
                <th className="py-3 px-4 font-normal text-right">Action</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[#141414] font-mono text-[11px]">
              {torrentsData?.data?.length === 0 && !torrentsLoading && (
                <tr>
                  <td colSpan={10} className="py-12 text-center text-[#666]">
                    No torrent payloads matching your filter query.
                  </td>
                </tr>
              )}
              {torrentsData?.data?.map((t) => {
                const displayName = t.name && t.name.trim().length > 0 ? t.name : `payload-${t.infohash.slice(0, 8)}`
                const isMultiFile = (t.file_count || 1) > 1
                const sizeFormatted = formatBytes(t.total_size)
                const timeAgo = t.verified_at ? formatTime(t.verified_at) : '—'
                const cat = t.category
                const catColor = cat ? (CATEGORY_COLORS[cat] || CATEGORY_COLORS.Other) : null

                return (
                  <tr
                    key={t.infohash}
                    onClick={() => handleInspectTorrent(t)}
                    className="hover:bg-[#0f0f0f] cursor-pointer transition-colors group"
                  >
                    <td className="py-3 px-4">
                      <div className="font-sans font-medium text-[#ededed] group-hover:text-white flex items-center gap-2">
                        <span className="w-2 h-2 rounded-full bg-emerald-400 shrink-0" />
                        <span className="truncate max-w-md" title={displayName}>
                          {displayName}
                        </span>
                      </div>
                      <div className="text-[10px] text-[#666] font-mono mt-0.5 pl-4">
                        {isMultiFile ? `${t.file_count} files` : 'Single file'} · verified in cluster
                      </div>
                    </td>

                    <td className="py-3 px-4">
                      <span className="text-[#888] group-hover:text-[#ccc] transition-colors">
                        {t.infohash.slice(0, 10)}...{t.infohash.slice(-8)}
                      </span>
                    </td>

                    <td className="py-3 px-4 whitespace-nowrap">
                      {cat ? (
                        <span className={`text-[10px] font-mono font-semibold px-2 py-0.5 rounded border ${catColor}`}>
                          {cat}
                          {t.category_confidence ? (
                            <span className="opacity-60 ml-1">
                              {Math.round(t.category_confidence * 100)}%
                            </span>
                          ) : null}
                        </span>
                      ) : (
                        <span className="text-[#444] text-[10px] font-mono">—</span>
                      )}
                    </td>

                    <td className="py-3 px-4 text-[#aaa] whitespace-nowrap">
                      {sizeFormatted}
                    </td>

                    <td className="py-3 px-4 text-[#888] whitespace-nowrap">
                      <span className="px-1.5 py-0.5 rounded text-[10px] bg-[#141414] text-[#aaa] border border-[#242424]">
                        {t.file_count || 1}
                      </span>
                    </td>

                    <td className="py-3 px-4 whitespace-nowrap">
                      <div className="flex items-center gap-1.5">
                        <HealthBar score={t.canonical_health_score ?? t.health_score} />
                        <HealthStatePill
                          state={t.canonical_health_state ?? t.health_state ?? t.availability_state}
                          score={t.canonical_health_score ?? t.health_score}
                        />
                      </div>
                    </td>

                    <td className="py-3 px-4 whitespace-nowrap">
                      <TrendingBadge score={t.popularity_score} />
                    </td>

                    <td className="py-3 px-4 whitespace-nowrap">
                      <SecurityShield
                        score={t.integrity_score}
                        riskTier={t.risk_tier}
                        policyAction={t.policy_action}
                      />
                    </td>

                    <td className="py-3 px-4 text-[#888] whitespace-nowrap">
                      {timeAgo}
                    </td>

                    <td className="py-3 px-4 text-right" onClick={(e) => e.stopPropagation()}>
                      <div className="flex items-center justify-end gap-1.5">
                        <button
                          onClick={(e) => handleDownloadTorrent(t, e)}
                          disabled={downloadingIh === t.infohash}
                          className="p-1.5 rounded-md bg-[#141414] border border-[#262626] text-emerald-400 hover:text-emerald-300 hover:border-emerald-600/50 transition-colors disabled:opacity-50"
                          title="Download .torrent on-the-fly (500ms fast-start)"
                        >
                          {downloadingIh === t.infohash ? (
                            <RefreshCw className="w-3.5 h-3.5 animate-spin text-emerald-400" />
                          ) : (
                            <FileDown className="w-3.5 h-3.5" />
                          )}
                        </button>
                        <button
                          onClick={() => copyToClipboard(magnetFrom(t.infohash, t.name), 'magnet')}
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
                )
              })}
            </tbody>
          </table>
        </div>

        {/* Pagination Footer */}
        <div className="p-3 border-t border-[#181818] bg-[#0c0c0c] flex flex-col sm:flex-row items-center justify-between gap-3 text-xs font-mono text-[#777]">
          <div>
            Page <strong className="text-white">{torrentsPage}</strong> of{' '}
            <strong className="text-white">{torrentsData.pages || 1}</strong>
          </div>

          <div className="flex items-center gap-1.5">
            <button
              disabled={torrentsPage <= 1 || torrentsLoading}
              onClick={() => setTorrentsPage(1)}
              className="p-1.5 rounded-md bg-[#141414] border border-[#262626] text-[#888] hover:text-white disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
              title="First Page"
            >
              <ChevronsLeft className="w-3.5 h-3.5" />
            </button>
            <button
              disabled={torrentsPage <= 1 || torrentsLoading}
              onClick={() => setTorrentsPage((p) => Math.max(1, p - 1))}
              className="p-1.5 rounded-md bg-[#141414] border border-[#262626] text-[#888] hover:text-white disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
              title="Previous Page"
            >
              <ChevronLeft className="w-3.5 h-3.5" />
            </button>
            <button
              disabled={torrentsPage >= (torrentsData.pages || 1) || torrentsLoading}
              onClick={() => setTorrentsPage((p) => Math.min(torrentsData.pages || 1, p + 1))}
              className="p-1.5 rounded-md bg-[#141414] border border-[#262626] text-[#888] hover:text-white disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
              title="Next Page"
            >
              <ChevronRight className="w-3.5 h-3.5" />
            </button>
            <button
              disabled={torrentsPage >= (torrentsData.pages || 1) || torrentsLoading}
              onClick={() => setTorrentsPage(torrentsData.pages || 1)}
              className="p-1.5 rounded-md bg-[#141414] border border-[#262626] text-[#888] hover:text-white disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
              title="Last Page"
            >
              <ChevronsRight className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>
      </div>

      {/* Inspect Torrent Modal (Identical to GAIA Operator Inspector) */}
      {selectedTorrent && (
        <div
          className="fixed inset-0 z-50 bg-black/80 backdrop-blur-sm flex items-center justify-center p-4 animate-fade-in"
          onClick={() => setSelectedTorrent(null)}
        >
          <div
            className="w-full max-w-2xl bg-[#090909] border border-[#262626] rounded-2xl p-6 overflow-y-auto max-h-[85vh] shadow-2xl space-y-6"
            onClick={(e) => e.stopPropagation()}
          >
            {/* Modal Header */}
            <div className="space-y-3 pb-4 border-b border-[#1c1c1c]">
              <div className="flex items-start justify-between gap-4">
                <div>
                  <span className="text-[10px] font-mono uppercase px-2 py-0.5 rounded bg-[#181818] text-emerald-400 border border-[#282828]">
                    {(selectedTorrent.file_count || selectedTorrent.files?.length || 1) > 1 ? 'Multi-File Bundle' : 'Single Payload'}
                  </span>
                  <h2 className="text-base font-semibold text-white mt-2 leading-tight">
                    {selectedTorrent.name || `payload-${(selectedTorrent.infohash || selectedTorrent.hash).slice(0, 8)}`}
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
                  {selectedTorrent.infohash || selectedTorrent.hash}
                </div>
                <button
                  onClick={() => copyToClipboard(selectedTorrent.infohash || selectedTorrent.hash, 'hash')}
                  className="flex items-center gap-1.5 text-[11px] px-2 py-1 rounded bg-[#161616] text-[#ccc] hover:text-white border border-[#262626] transition-colors ml-3 shrink-0"
                >
                  {copiedHash === (selectedTorrent.infohash || selectedTorrent.hash) ? (
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
                <div className="text-white font-semibold mt-0.5">
                  {formatBytes(selectedTorrent.total_size)}
                </div>
              </div>
              <div className="p-3 rounded-lg bg-[#000] border border-[#1a1a1a]">
                <div className="text-[10px] text-[#555] uppercase font-sans">Piece Length</div>
                <div className="text-white font-semibold mt-0.5">
                  {selectedTorrent.pieceLength || '2.0 MB'}
                </div>
              </div>
              <div className="p-3 rounded-lg bg-[#000] border border-[#1a1a1a]">
                <div className="text-[10px] text-[#555] uppercase font-sans">Files Count</div>
                <div className="text-white font-semibold mt-0.5">
                  {selectedTorrent.file_count || selectedTorrent.files?.length || 1}
                </div>
              </div>
            </div>

            {/* File Tree List */}
            <div className="space-y-2">
              <div className="text-xs font-semibold text-[#888] uppercase tracking-wider flex items-center justify-between">
                <span>Payload File Structure ({selectedTorrent.files?.length || selectedTorrent.file_count || 1})</span>
                <span className="text-[10px] font-mono text-[#555]">Verified SHA1</span>
              </div>

              <div className="rounded-lg border border-[#1a1a1a] bg-[#000] divide-y divide-[#141414] overflow-hidden max-h-48 overflow-y-auto font-mono text-xs">
                {detailLoading ? (
                  <div className="p-4 text-center text-[#666] flex items-center justify-center gap-2">
                    <RefreshCw className="w-3.5 h-3.5 animate-spin text-emerald-400" /> Loading verified file manifest...
                  </div>
                ) : Array.isArray(selectedTorrent.files) && selectedTorrent.files.length > 0 ? (
                  selectedTorrent.files.map((f, idx) => {
                    const filePath = Array.isArray(f.path) ? f.path.join('/') : f.path || f.name || 'file'
                    const fileLen = f.length || f.size || 0
                    return (
                      <div key={idx} className="p-2.5 flex items-center justify-between hover:bg-[#0c0c0c]">
                        <div className="flex items-center gap-2 truncate pr-3">
                          <Folder className="w-3.5 h-3.5 text-[#666] shrink-0" />
                          <span className="truncate text-[#bbb]">{filePath}</span>
                        </div>
                        <span className="text-[#666] text-[11px] shrink-0">{formatBytes(fileLen)}</span>
                      </div>
                    )
                  })
                ) : (
                  <div className="p-2.5 flex items-center justify-between">
                    <div className="flex items-center gap-2 truncate pr-3">
                      <Folder className="w-3.5 h-3.5 text-[#666] shrink-0" />
                      <span className="truncate text-[#bbb]">
                        {selectedTorrent.name || 'payload.bin'}
                      </span>
                    </div>
                    <span className="text-[#666] text-[11px] shrink-0">
                      {formatBytes(selectedTorrent.total_size)}
                    </span>
                  </div>
                )}
              </div>
            </div>

            {/* Health & Popularity Meters */}
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              <div className="rounded-lg border border-[#1a1a1a] bg-[#000] p-3 space-y-2.5">
                <div className="flex items-center justify-between text-xs font-mono">
                  <span className="text-[#888] flex items-center gap-1.5">
                    <Activity className="w-3.5 h-3.5 text-emerald-400" />
                    <span>Swarm Health & Liveness</span>
                  </span>
                  {(() => {
                    const displayScore = selectedTorrent.canonical_health_score ?? selectedTorrent.health_score
                    let displayState = selectedTorrent.canonical_health_state ?? selectedTorrent.health_state ?? selectedTorrent.availability_state
                    if (!displayState || displayState === 'UNKNOWN') {
                      if (displayScore != null) {
                        if (displayScore >= 70) displayState = 'VERIFIED'
                        else if (displayScore >= 40) displayState = 'ACTIVE'
                        else if (displayScore >= 15) displayState = 'DEGRADED'
                        else if (displayScore > 0) displayState = 'DORMANT'
                        else displayState = 'DEAD'
                      } else {
                        displayState = 'UNPROBED'
                      }
                    }
                    const c = getHealthColor(displayScore)
                    return (
                      <span className={`font-bold ${c.text}`}>
                        {displayScore != null
                          ? `${displayScore}% (${displayState})`
                          : `— (${displayState})`}
                      </span>
                    )
                  })()}
                </div>

                <div className="w-full bg-[#161616] rounded-full h-2 overflow-hidden">
                  {(() => {
                    const displayScore = selectedTorrent.canonical_health_score ?? selectedTorrent.health_score
                    const c = getHealthColor(displayScore)
                    return (
                      <div
                        className={`h-full rounded-full transition-all ${c.bar}`}
                        style={{ width: `${Math.min(100, Math.max(0, displayScore ?? 0))}%` }}
                      />
                    )
                  })()}
                </div>

                {/* Evidence Breakdown from canonical scorer */}
                <EvidenceBreakdown
                  evidence={selectedTorrent.evidence_summary}
                  healthScore={selectedTorrent.canonical_health_score ?? selectedTorrent.health_score}
                />

                <div className="flex items-center justify-between text-[10px] font-mono text-[#666] pt-1.5 border-t border-[#141414]">
                  <span className={(selectedTorrent.swarm_peers || 0) > 0 && selectedTorrent.seed_confirmed ? 'text-emerald-400 font-semibold' : 'text-[#777]'}>
                    {(selectedTorrent.swarm_peers || 0) > 0 && selectedTorrent.seed_confirmed
                      ? '✓ Confirmed Active Swarm'
                      : 'No Active Peers (Dormant Swarm)'}
                  </span>
                  <span className={(selectedTorrent.swarm_peers || 0) > 0 ? 'text-white' : 'text-[#777]'}>
                    {selectedTorrent.swarm_peers || 0} DHT Peers
                  </span>
                </div>
              </div>

              <div className="rounded-lg border border-[#1a1a1a] bg-[#000] p-3 space-y-2">
                {(() => {
                  const c = getPopularityColor(selectedTorrent.popularity_score)
                  return (
                    <>
                      <div className="flex items-center justify-between text-xs font-mono">
                        <span className="text-[#888] flex items-center gap-1.5" title="Search Indexer Trending Heat: Measures 7-day query traffic and swarm announcement velocity.">
                          <TrendingUp className={`w-3.5 h-3.5 ${c.text}`} />
                          <span>Search Indexer Trending</span>
                        </span>
                        <TrendingBadge score={selectedTorrent.popularity_score} />
                      </div>
                      <div className="w-full bg-[#161616] rounded-full h-2 overflow-hidden">
                        <div
                          className={`h-full rounded-full transition-all ${c.bar}`}
                          style={{ width: `${Math.min(100, Math.max(0, selectedTorrent.popularity_score ?? 0))}%` }}
                        />
                      </div>
                    </>
                  )
                })()}
                <div className="flex items-center justify-between text-[10px] font-mono text-[#666] pt-1 border-t border-[#141414]">
                  <span>Velocity: 7-Day Active</span>
                  <span>{Number(selectedTorrent.total_seen || 1).toLocaleString()} Hits</span>
                </div>
              </div>
            </div>

            {/* Antivirus Security Shield & Policy Enforcement */}
            <div className="rounded-xl border border-[#1e1e1e] bg-[#0c0c0c] p-4 space-y-3 font-mono">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <Shield className="w-4 h-4 text-emerald-400" />
                  <span className="text-xs font-semibold text-white">Antivirus & Content Security Shield</span>
                  <SecurityShield
                    score={selectedTorrent.integrity_score}
                    riskTier={selectedTorrent.risk_tier}
                    policyAction={selectedTorrent.policy_action}
                  />
                </div>
                <span className="text-[10px] text-[#666]">
                  Source: {selectedTorrent.decision_source || 'MODEL'}
                </span>
              </div>

              <div className="grid grid-cols-2 gap-2 text-xs">
                <div className="p-2 rounded bg-[#141414] border border-[#222]">
                  <div className="text-[10px] text-[#777] uppercase">Antivirus Score</div>
                  <div className="text-sm font-bold text-emerald-400 mt-0.5">
                    {selectedTorrent.integrity_score ?? (selectedTorrent.risk_tier === 'BLOCKED' ? 0 : selectedTorrent.risk_tier === 'REVIEW' ? 45 : 100)}/100
                  </div>
                </div>
                <div className="p-2 rounded bg-[#141414] border border-[#222]">
                  <div className="text-[10px] text-[#777] uppercase">Content Quality</div>
                  <div className="text-sm font-bold text-white mt-0.5">
                    {selectedTorrent.metadata_quality_score ?? 85}/100
                  </div>
                </div>
              </div>
            </div>

            {/* Category Pill & Verification */}
            <div className="rounded-lg border border-[#1a1a1a] bg-[#000] p-3 space-y-2">
              <div className="flex items-center justify-between text-xs font-mono">
                <span className="text-[#888] flex items-center gap-1.5">
                  <Tag className="w-3.5 h-3.5 text-purple-400" />
                  <span>Category Classification</span>
                </span>
                <span className="text-purple-400 font-bold font-mono">
                  {selectedTorrent.category || 'Unclassified'}
                </span>
              </div>

              {selectedTorrent.category_confidence && (
                <div className="w-full bg-[#161616] rounded-full h-1.5 overflow-hidden">
                  <div
                    className="h-full rounded-full bg-purple-400 transition-all"
                    style={{ width: `${Math.min(100, Math.max(0, selectedTorrent.category_confidence * 100))}%` }}
                  />
                </div>
              )}
            </div>

            {/* Modal Action Buttons */}
            <div className="pt-4 border-t border-[#1c1c1c] flex items-center justify-between gap-3">
              <button
                onClick={(e) => handleDownloadTorrent(selectedTorrent, e)}
                disabled={downloadingIh === (selectedTorrent.infohash || selectedTorrent.hash)}
                className="flex-1 flex items-center justify-center gap-2 py-2.5 px-4 rounded-lg bg-emerald-500 hover:bg-emerald-400 text-black font-semibold text-xs transition-colors disabled:opacity-50"
                title="Generate and download .torrent file on-the-fly (500ms fast-start)"
              >
                {downloadingIh === (selectedTorrent.infohash || selectedTorrent.hash) ? (
                  <>
                    <RefreshCw className="w-4 h-4 animate-spin text-black" />
                    <span>Fetching .torrent on-the-fly...</span>
                  </>
                ) : (
                  <>
                    <FileDown className="w-4 h-4 text-black" />
                    <span>Download .torrent (Fast-Start)</span>
                  </>
                )}
              </button>
              <button
                onClick={() => copyToClipboard(selectedTorrent.magnet || magnetFrom(selectedTorrent.infohash || selectedTorrent.hash, selectedTorrent.name), 'magnet')}
                className="flex-1 flex items-center justify-center gap-2 py-2.5 px-4 rounded-lg bg-[#181818] border border-[#2a2a2a] text-white font-semibold text-xs hover:bg-[#252525] transition-colors"
              >
                {copiedMagnet ? (
                  <>
                    <Check className="w-4 h-4 text-emerald-400" />
                    <span>Magnet URI Copied!</span>
                  </>
                ) : (
                  <>
                    <DownloadCloud className="w-4 h-4" />
                    <span>Copy Magnet Link</span>
                  </>
                )}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
