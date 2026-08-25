import { useEffect, useState } from 'react'
import { api, magnetFrom } from '../api.js'
import { formatBytes, formatTime } from '../utils.js'
import FileTree from './FileTree.jsx'

function Attr({ label, value }) {
  return (
    <div>
      <div className="text-xs uppercase tracking-wide text-slate-500">{label}</div>
      <div className="text-sm text-slate-200 mt-0.5 break-all">{value ?? '—'}</div>
    </div>
  )
}

export default function TorrentDetail({ infohash, onClose }) {
  const [tor, setTor] = useState(null)
  const [error, setError] = useState(null)

  useEffect(() => {
    api(`/api/torrents/${infohash}`).then(setTor).catch((e) => setError(e.message))
  }, [infohash])

  function copyMagnet() {
    navigator.clipboard
      ?.writeText(magnetFrom(tor.infohash, tor.name))
      .catch(() => {})
  }

  return (
    <div className="fixed inset-0 z-20 flex items-center justify-center bg-black/60 p-4" onClick={onClose}>
      <div
        className="w-full max-w-2xl max-h-[85vh] overflow-hidden flex flex-col rounded-xl border border-ink-700 bg-ink-900"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 py-3 border-b border-ink-700">
          <div className="font-semibold truncate">{tor?.name || infohash}</div>
          <button onClick={onClose} className="text-slate-400 hover:text-white">✕</button>
        </div>

        <div className="px-5 py-4 overflow-y-auto">
          {error && <div className="text-red-400 text-sm">Error: {error}</div>}
          {!tor && !error && <div className="text-slate-400 text-sm">Loading…</div>}

          {tor && (
            <div className="space-y-4">
              <div className="grid grid-cols-2 gap-4">
                <Attr label="Infohash" value={tor.infohash} />
                <Attr label="Verified" value={formatTime(tor.verified_at)} />
                <Attr label="Size" value={formatBytes(tor.total_size)} />
                <Attr label="Piece length" value={formatBytes(tor.piece_length)} />
                <Attr label="Files" value={tor.file_count} />
                <Attr label="Fetch attempts" value={tor.fetch_attempts} />
                <Attr label="First seen" value={formatTime(tor.first_seen)} />
                <Attr label="Last seen" value={formatTime(tor.last_seen)} />
              </div>

              <div>
                <div className="flex items-center justify-between mb-2">
                  <span className="text-xs uppercase tracking-wide text-slate-500">Magnet</span>
                  <div className="flex gap-2">
                    <button
                      onClick={copyMagnet}
                      className="px-3 py-1.5 rounded-md text-xs bg-ink-800 border border-ink-700 text-slate-300 hover:text-white hover:bg-ink-700 transition-colors"
                    >
                      Copy
                    </button>
                    <a
                      href={magnetFrom(tor.infohash, tor.name)}
                      className="px-3 py-1.5 rounded-md text-xs bg-accent-dark text-ink-950 font-semibold hover:bg-accent transition-colors"
                    >
                      Launch magnet
                    </a>
                  </div>
                </div>
                <div className="font-mono text-[11px] text-slate-400 break-all bg-ink-950 rounded-md px-3 py-2 border border-ink-800">
                  {magnetFrom(tor.infohash, tor.name)}
                </div>
              </div>

              {Array.isArray(tor.files) && tor.files.length > 0 ? (
                <div>
                  <div className="text-xs uppercase tracking-wide text-slate-500 mb-2">
                    Files ({tor.files.length})
                  </div>
                  <FileTree files={tor.files} />
                </div>
              ) : tor.file_count === 1 ? (
                <div>
                  <div className="text-xs uppercase tracking-wide text-slate-500 mb-2">
                    Files (1)
                  </div>
                  <div className="rounded-lg border border-ink-800 bg-ink-950 px-2 py-2 max-h-72 overflow-y-auto font-mono text-xs">
                    <div className="flex items-center gap-1.5 py-0.5 text-slate-400">
                      <span className="text-slate-600 w-4">·</span>
                      <span className="truncate">{tor.name || '(unnamed)'}</span>
                      <span className="text-[11px] text-slate-500 ml-auto pl-3">
                        {formatBytes(tor.total_size)}
                      </span>
                    </div>
                  </div>
                </div>
              ) : (
                <div className="text-xs text-slate-500">No file list in metadata.</div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}