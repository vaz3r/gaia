import { useEffect, useState } from 'react'
import { api, downloadTorrent, magnetFrom } from '../api.js'
import { formatBytes, formatTime } from '../utils.js'
import FileTree from './FileTree.jsx'
import { Copy, Download, ExternalLink, Check, ShieldCheck, Activity, Database } from 'lucide-react'

function Attr({ label, value, highlight, mono }) {
  return (
    <div className="bg-ink-950/60 p-2.5 rounded-lg border border-ink-800">
      <div className="text-[11px] font-mono uppercase tracking-wider text-slate-400">{label}</div>
      <div className={`text-xs mt-1 truncate ${highlight ? 'text-cyan-400 font-semibold' : 'text-slate-200'} ${mono ? 'font-mono' : ''}`}>
        {value ?? '—'}
      </div>
    </div>
  )
}

export default function TorrentDetail({ infohash, onClose }) {
  const [tor, setTor] = useState(null)
  const [error, setError] = useState(null)
  const [copied, setCopied] = useState(false)
  const [downloading, setDownloading] = useState(false)

  useEffect(() => {
    setTor(null)
    setError(null)
    api(`/api/torrents/${infohash}`)
      .then(setTor)
      .catch((e) => setError(e.message))
  }, [infohash])

  function copyMagnet() {
    const uri = tor?.magnet || magnetFrom(tor?.infohash || infohash, tor?.name)
    navigator.clipboard?.writeText(uri).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    }).catch(() => {})
  }

  async function handleDownload() {
    try {
      setDownloading(true)
      await downloadTorrent(tor.infohash, tor.name)
    } catch (e) {
      alert(`Download failed: ${e.message}`)
    } finally {
      setDownloading(false)
    }
  }

  const magnetUri = tor?.magnet || magnetFrom(tor?.infohash || infohash, tor?.name)

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 backdrop-blur-xs p-4 animate-fade-in" onClick={onClose}>
      <div
        className="w-full max-w-3xl max-h-[90vh] overflow-hidden flex flex-col rounded-xl border border-ink-700 bg-ink-900 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Modal Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-ink-700/80 bg-ink-950/70">
          <div className="flex items-center gap-3 min-w-0 pr-4">
            <div className="w-2.5 h-2.5 rounded-full bg-cyan-400 animate-pulse shrink-0" />
            <div className="min-w-0">
              <h2 className="text-sm font-semibold text-white truncate tracking-wide">
                {tor?.name || infohash}
              </h2>
              <div className="text-[11px] font-mono text-slate-400 truncate mt-0.5">
                {infohash}
              </div>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-1.5 rounded-lg text-slate-400 hover:text-white hover:bg-ink-800 transition-colors shrink-0"
            title="Close"
          >
            ✕
          </button>
        </div>

        {/* Modal Body */}
        <div className="px-6 py-5 overflow-y-auto space-y-5">
          {error && (
            <div className="p-4 rounded-lg bg-rose-500/10 border border-rose-500/30 text-rose-300 text-xs font-mono">
              Error: {error}
            </div>
          )}

          {!tor && !error && (
            <div className="flex items-center justify-center py-16 gap-3 text-slate-400 text-sm">
              <div className="w-5 h-5 border-2 border-cyan-400 border-t-transparent rounded-full animate-spin" />
              <span>Fetching complete swarm telemetry...</span>
            </div>
          )}

          {tor && (
            <>
              {/* Action Bar */}
              <div className="flex flex-wrap items-center justify-between gap-3 p-3.5 rounded-xl border border-ink-700/60 bg-ink-950">
                <div className="flex items-center gap-2">
                  <button
                    onClick={copyMagnet}
                    className="flex items-center gap-1.5 px-3.5 py-2 rounded-lg text-xs font-medium bg-ink-800 border border-ink-700 text-slate-200 hover:text-white hover:bg-ink-700 hover:border-ink-600 transition-all shadow-xs"
                  >
                    {copied ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5 text-slate-400" />}
                    <span>{copied ? 'Magnet Copied!' : 'Copy Magnet'}</span>
                  </button>

                  <button
                    onClick={handleDownload}
                    disabled={downloading}
                    className="flex items-center gap-1.5 px-3.5 py-2 rounded-lg text-xs font-medium bg-cyan-500/10 border border-cyan-500/30 text-cyan-300 hover:bg-cyan-500/20 transition-all shadow-xs"
                  >
                    <Download className="w-3.5 h-3.5 text-cyan-400" />
                    <span>{downloading ? 'Generating...' : 'Download .torrent'}</span>
                  </button>
                </div>

                <a
                  href={magnetUri}
                  className="flex items-center gap-1.5 px-3.5 py-2 rounded-lg text-xs font-semibold bg-accent-dark text-ink-950 hover:bg-accent transition-all shadow-xs"
                >
                  <ExternalLink className="w-3.5 h-3.5" />
                  <span>Launch in Torrent Client</span>
                </a>
              </div>

              {/* Core Telemetry Grid (28 attributes organized) */}
              <div>
                <div className="flex items-center gap-2 mb-2 text-xs font-semibold uppercase tracking-wider text-slate-400">
                  <Database className="w-3.5 h-3.5 text-cyan-400" />
                  <span>Swarm & Storage Specifications</span>
                </div>
                <div className="grid grid-cols-2 sm:grid-cols-4 gap-2.5">
                  <Attr label="Total Size" value={formatBytes(tor.total_size)} highlight />
                  <Attr label="File Count" value={tor.file_count} />
                  <Attr label="Piece Length" value={formatBytes(tor.piece_length)} />
                  <Attr label="Category" value={tor.category || 'Uncategorized'} highlight />
                  <Attr label="Health Score" value={(tor.canonical_health_score ?? tor.health_score) !== null && (tor.canonical_health_score ?? tor.health_score) !== undefined ? `${tor.canonical_health_score ?? tor.health_score} / 100` : '—'} highlight={(tor.canonical_health_score ?? tor.health_score) >= 70} />
                  <Attr label="Health State" value={tor.canonical_health_state ?? tor.health_state ?? 'ACTIVE'} />
                  <Attr label="Active Swarm" value={`${tor.swarm_peers ?? 0} peers`} />
                  <Attr label="Popularity (Demand)" value={`${tor.popularity_score ?? 0} pts`} />
                </div>
              </div>

              {/* Surveillance & Chronology */}
              <div>
                <div className="flex items-center gap-2 mb-2 text-xs font-semibold uppercase tracking-wider text-slate-400">
                  <Activity className="w-3.5 h-3.5 text-amber-400" />
                  <span>Chronology & Verification</span>
                </div>
                <div className="grid grid-cols-2 sm:grid-cols-4 gap-2.5">
                  <Attr label="First Seen" value={formatTime(tor.first_seen)} mono />
                  <Attr label="Verified At" value={formatTime(tor.verified_at)} mono />
                  <Attr label="Last Seen" value={formatTime(tor.last_seen)} mono />
                  <Attr label="Total Sightings" value={tor.total_seen} />
                  <Attr label="Fetch Attempts" value={tor.fetch_attempts} />
                  <Attr label="Risk Tier" value={tor.risk_tier} />
                  <Attr label="Policy Action" value={tor.policy_action} />
                  <Attr label="Seed Confirmed" value={tor.seed_confirmed ? 'YES' : 'NO'} />
                </div>
              </div>

              {/* Magnet URI View */}
              <div>
                <div className="text-[11px] font-mono uppercase tracking-wider text-slate-400 mb-1.5">
                  Direct Magnet Stream URI
                </div>
                <div className="p-2.5 rounded-lg border border-ink-800 bg-ink-950 font-mono text-[11px] text-slate-400 break-all select-all">
                  {magnetUri}
                </div>
              </div>

              {/* Interactive File Tree */}
              <div>
                <div className="flex items-center justify-between mb-2">
                  <span className="text-xs font-semibold uppercase tracking-wider text-slate-400">
                    File Manifest ({Array.isArray(tor.files) ? tor.files.length : (tor.file_count || 1)})
                  </span>
                  <span className="text-[11px] font-mono text-slate-500">
                    {formatBytes(tor.total_size)}
                  </span>
                </div>

                {Array.isArray(tor.files) && tor.files.length > 0 ? (
                  <FileTree files={tor.files} />
                ) : tor.file_count === 1 ? (
                  <div className="rounded-lg border border-ink-800 bg-ink-950 px-3 py-2 font-mono text-xs text-slate-300 flex items-center justify-between">
                    <span className="truncate">{tor.name || '(single file)'}</span>
                    <span className="text-slate-500 ml-2">{formatBytes(tor.total_size)}</span>
                  </div>
                ) : (
                  <div className="p-4 rounded-lg border border-ink-800 bg-ink-950 text-xs text-slate-500 font-mono text-center">
                    No discrete file manifest in metadata.
                  </div>
                )}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  )
}
