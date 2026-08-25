import { useEffect, useRef, useState } from 'react'
import { api, magnetFrom } from '../api.js'
import { formatBytes, formatTime } from '../utils.js'
import TorrentDetail from './TorrentDetail.jsx'

const COLS = [
  { key: 'name', label: 'Name', sortKey: 'name' },
  { key: 'total_size', label: 'Size', sortKey: 'size' },
  { key: 'file_count', label: 'Files', sortKey: 'files' },
  { key: 'verified_at', label: 'Verified', sortKey: 'verified_at' },
]

export default function TorrentBrowser() {
  const [input, setInput] = useState('')
  const [search, setSearch] = useState('')
  const [sort, setSort] = useState('verified_at')
  const [order, setOrder] = useState('desc')
  const [page, setPage] = useState(1)
  const [limit] = useState(25)
  const [data, setData] = useState(null)
  const [error, setError] = useState(null)
  const [loading, setLoading] = useState(false)
  const [detail, setDetail] = useState(null)
  const debounce = useRef()

  useEffect(() => {
    clearTimeout(debounce.current)
    debounce.current = setTimeout(() => {
      setSearch(input.trim())
      setPage(1)
    }, 300)
    return () => clearTimeout(debounce.current)
  }, [input])

  useEffect(() => {
    setLoading(true)
    setError(null)
    const params = new URLSearchParams({ page, limit })
    if (search) params.set('search', search)
    if (sort) params.set('sort', sort)
    params.set('order', order)
    api(`/api/torrents?${params}`)
      .then(setData)
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false))
  }, [search, sort, order, page, limit])

  function toggleSort(key) {
    if (sort === key) {
      setOrder(order === 'asc' ? 'desc' : 'asc')
    } else {
      setSort(key)
      setOrder(key === 'name' ? 'asc' : 'desc')
    }
  }

  function magnet(row) {
    return magnetFrom(row.infohash, row.name)
  }

  return (
    <div>
      <div className="flex items-center gap-3 mb-4">
        <div className="relative flex-1 max-w-md">
          <svg
            className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-slate-500"
            fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}
          >
            <path strokeLinecap="round" strokeLinejoin="round" d="M21 21l-4.35-4.35M17 10a7 7 0 11-14 0 7 7 0 0114 0z" />
          </svg>
          <input
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="Fuzzy search by name…"
            className="w-full rounded-lg bg-ink-800 border border-ink-700 pl-9 pr-3 py-2 text-sm focus:outline-none focus:border-accent focus:ring-1 focus:ring-accent"
          />
        </div>
        <span className="text-xs text-slate-400 whitespace-nowrap">
          {loading ? 'loading…' : data ? `${data.total.toLocaleString()} results` : ''}
        </span>
      </div>

      {error && (
        <div className="rounded-lg border border-red-900 bg-red-950/40 text-red-300 text-sm px-4 py-2 mb-4">
          Error: {error}
        </div>
      )}

      <div className="rounded-xl border border-ink-700 overflow-hidden">
        <div className="overflow-x-auto">
          <table className="w-full table-fixed text-sm">
            <colgroup>
              <col />
              <col className="w-[130px]" />
              <col className="w-[80px]" />
              <col className="w-[170px]" />
              <col className="w-[130px]" />
            </colgroup>
            <thead>
              <tr className="bg-ink-800 text-left text-xs uppercase tracking-wider text-slate-400">
                {COLS.map((c) => (
                  <th key={c.key} className="px-4 py-3">
                    <button
                      onClick={() => toggleSort(c.sortKey)}
                      className={`inline-flex items-center gap-1.5 hover:text-white transition-colors ${
                        sort === c.sortKey ? 'text-accent' : ''
                      }`}
                    >
                      {c.label}
                      {sort === c.sortKey ? (
                        <span className="text-[10px]">{order === 'asc' ? '▲' : '▼'}</span>
                      ) : (
                        <span className="text-[10px] text-slate-600">↕</span>
                      )}
                    </button>
                  </th>
                ))}
                <th className="px-4 py-3 text-right font-medium">Actions</th>
              </tr>
            </thead>
            <tbody>
              {(data?.data ?? []).map((row, i) => (
                <tr
                  key={row.infohash}
                  onClick={() => setDetail(row.infohash)}
                  className={`border-t border-ink-800 cursor-pointer transition-colors hover:bg-ink-800/70 ${
                    i % 2 === 1 ? 'bg-ink-900/40' : ''
                  }`}
                >
                  <td className="px-4 py-3 truncate font-medium text-slate-100 align-middle">
                    <span className="block truncate" title={row.name || undefined}>
                      {row.name || <span className="italic text-slate-500">unnamed torrent</span>}
                    </span>
                    <span className="block font-mono text-[10px] text-slate-500 mt-0.5 truncate">
                      {row.infohash}
                    </span>
                  </td>
                  <td className="px-4 py-3 text-right tabular-nums text-slate-300 align-middle">
                    {formatBytes(row.total_size)}
                  </td>
                  <td className="px-4 py-3 text-right tabular-nums text-slate-300 align-middle">
                    {row.file_count ?? '—'}
                  </td>
                  <td className="px-4 py-3 text-right tabular-nums text-slate-400 align-middle whitespace-nowrap">
                    {formatTime(row.verified_at)}
                  </td>
                  <td className="px-4 py-3 text-right align-middle">
                    <div className="inline-flex gap-1.5">
                      <a
                        href={magnet(row)}
                        title="Open in your torrent client"
                        onClick={(e) => e.stopPropagation()}
                        className="px-2.5 py-1.5 rounded-md text-xs bg-ink-700 hover:bg-ink-600 hover:text-white text-slate-300 transition-colors inline-flex items-center gap-1"
                      >
                        <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                          <path strokeLinecap="round" strokeLinejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z" />
                        </svg>
                        magnet
                      </a>
                      <button
                        title="Copy magnet link"
                        onClick={(e) => {
                          e.stopPropagation()
                          navigator.clipboard?.writeText(magnet(row)).catch(() => {})
                        }}
                        className="px-2 py-1.5 rounded-md text-xs bg-ink-800 border border-ink-700 hover:bg-ink-600 hover:text-white text-slate-400 hover:border-ink-500 transition-colors"
                      >
                        copy
                      </button>
                    </div>
                  </td>
                </tr>
              ))}
              {data && data.data.length === 0 && (
                <tr>
                  <td colSpan={5} className="px-4 py-14 text-center text-slate-500">
                    No torrents match{search ? ` “${search}”` : ''}.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>

      {data && data.pages > 1 && (
        <div className="flex items-center justify-between mt-4 text-sm">
          <span className="text-slate-400">
            Page <span className="text-slate-200 font-medium">{data.page}</span> of{' '}
            {data.pages}
          </span>
          <div className="flex gap-2">
            <button
              disabled={page <= 1}
              onClick={() => setPage((p) => p - 1)}
              className="px-3 py-1.5 rounded-md bg-ink-800 border border-ink-700 disabled:opacity-40 hover:bg-ink-700 disabled:hover:bg-ink-800 transition-colors"
            >
              ← Prev
            </button>
            <button
              disabled={page >= data.pages}
              onClick={() => setPage((p) => p + 1)}
              className="px-3 py-1.5 rounded-md bg-ink-800 border border-ink-700 disabled:opacity-40 hover:bg-ink-700 disabled:hover:bg-ink-800 transition-colors"
            >
              Next →
            </button>
          </div>
        </div>
      )}

      {detail && <TorrentDetail infohash={detail} onClose={() => setDetail(null)} />}
    </div>
  )
}