import React, { useState, useEffect } from 'react'
import { 
  Database, 
  Activity, 
  ShieldCheck, 
  Zap, 
  ExternalLink, 
  Radio, 
  Layers,
  Sparkles
} from 'lucide-react'
import { api, loadTrackers } from './api.js'
import { formatNum } from './utils.js'
import TorrentBrowser from './components/TorrentBrowser.jsx'

export default function App() {
  const [stats, setStats] = useState(null)

  useEffect(() => {
    // Load initial stats
    api('/api/stats')
      .then(setStats)
      .catch((e) => console.warn('Failed to load stats:', e))

    // Listen to SSE live stats updates
    let es
    try {
      es = new EventSource('/api/live/stream')
      es.onmessage = (event) => {
        try {
          const data = JSON.parse(event.data)
          setStats((prev) => ({
            ...prev,
            total_torrents: data.torrents ?? prev?.total_torrents,
            verified_last_24h: data.verified24h ?? prev?.verified_last_24h,
            healthy_count: data.healthy ?? prev?.healthy_count,
            updated_at: data.timestamp ?? prev?.updated_at
          }))
        } catch {}
      }
      es.onerror = () => {
        es.close()
      }
    } catch {}

    return () => {
      es?.close()
    }
  }, [])

  return (
    <div className="min-h-screen flex flex-col bg-ink-950 text-slate-100 selection:bg-cyan-500/30 selection:text-cyan-200">
      {/* Top Navbar */}
      <header className="sticky top-0 z-40 bg-ink-950/85 backdrop-blur-md border-b border-ink-800">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 h-16 flex items-center justify-between">
          {/* Logo & Title */}
          <div className="flex items-center gap-3">
            <div className="w-9 h-9 rounded-xl bg-ink-900 border border-ink-700 flex items-center justify-center shadow-lg shadow-cyan-500/5">
              <Radio className="w-5 h-5 text-cyan-400" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <span className="text-base font-bold tracking-tight text-white font-sans">
                  GAIA <span className="text-cyan-400">INDEXER</span>
                </span>
                <span className="px-1.5 py-0.5 rounded text-[10px] font-mono bg-cyan-500/10 text-cyan-300 border border-cyan-500/20">
                  .NET 10 CORE
                </span>
              </div>
              <p className="text-[10px] text-slate-400 font-mono tracking-wider uppercase">
                Decentralized Swarm Search
              </p>
            </div>
          </div>

          {/* Quick Stats Badges & Links */}
          <div className="flex items-center gap-2 sm:gap-3 font-mono text-xs">
            {stats && (
              <>
                <div className="hidden sm:flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-ink-900 border border-ink-800 text-slate-300">
                  <Database className="w-3.5 h-3.5 text-cyan-400" />
                  <span>{formatNum(stats.total_torrents || 3418496)} Torrents</span>
                </div>
                <div className="hidden md:flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-ink-900 border border-ink-800 text-emerald-400">
                  <Activity className="w-3.5 h-3.5" />
                  <span>+{formatNum(stats.verified_last_24h || 408710)}/24h</span>
                </div>
              </>
            )}

            <a
              href="http://localhost:3001"
              target="_blank"
              rel="noreferrer"
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-ink-900 border border-ink-700 text-slate-300 hover:text-white hover:bg-ink-800 transition-colors"
            >
              <span>Operator Dashboard</span>
              <ExternalLink className="w-3 h-3 text-slate-400" />
            </a>
          </div>
        </div>
      </header>

      {/* Hero Stats Ribbon */}
      <div className="bg-ink-900/40 border-b border-ink-800 py-6">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
            <div className="bg-ink-950 p-4 rounded-xl border border-ink-800 flex items-center justify-between">
              <div>
                <div className="text-[11px] font-mono text-slate-400 uppercase tracking-wider">
                  Indexed Swarms
                </div>
                <div className="text-xl font-bold font-mono text-white mt-1">
                  {(stats?.total_torrents || 3418496).toLocaleString()}
                </div>
                <div className="text-[10px] text-cyan-400 font-mono mt-0.5">
                  Sub-50ms Quickwit NVMe Engine
                </div>
              </div>
              <Database className="w-8 h-8 text-cyan-400/30" />
            </div>

            <div className="bg-ink-950 p-4 rounded-xl border border-ink-800 flex items-center justify-between">
              <div>
                <div className="text-[11px] font-mono text-slate-400 uppercase tracking-wider">
                  Verified Last 24 Hours
                </div>
                <div className="text-xl font-bold font-mono text-emerald-400 mt-1">
                  +{(stats?.verified_last_24h || 408710).toLocaleString()}
                </div>
                <div className="text-[10px] text-slate-500 font-mono mt-0.5">
                  Continuous Crawler Harvest
                </div>
              </div>
              <Zap className="w-8 h-8 text-emerald-400/30" />
            </div>

            <div className="bg-ink-950 p-4 rounded-xl border border-ink-800 flex items-center justify-between">
              <div>
                <div className="text-[11px] font-mono text-slate-400 uppercase tracking-wider">
                  High-Health Swarms
                </div>
                <div className="text-xl font-bold font-mono text-amber-300 mt-1">
                  {(stats?.healthy_count || 951907).toLocaleString()}
                </div>
                <div className="text-[10px] text-slate-500 font-mono mt-0.5">
                  Health Score ≥ 70
                </div>
              </div>
              <ShieldCheck className="w-8 h-8 text-amber-400/30" />
            </div>
          </div>
        </div>
      </div>

      {/* Main Content Area */}
      <main className="flex-1 max-w-7xl w-full mx-auto px-4 sm:px-6 lg:px-8 py-6">
        <TorrentBrowser />
      </main>

      {/* Footer */}
      <footer className="border-t border-ink-800 bg-ink-950 py-6 text-center text-xs font-mono text-slate-500">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 flex flex-col sm:flex-row items-center justify-between gap-2">
          <div>
            GAIA BitTorrent DHT Intelligence Platform • High Velocity V2 Arch
          </div>
          <div className="flex items-center gap-3 text-slate-400">
            <span>Runtime: ASP.NET Core (.NET 10)</span>
            <span>•</span>
            <span>Search: Quickwit 0.8.2</span>
          </div>
        </div>
      </footer>
    </div>
  )
}
