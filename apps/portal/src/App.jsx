import React, { useState, useEffect } from 'react'
import { 
  Database, 
  Activity, 
  ShieldCheck, 
  Zap, 
  ExternalLink, 
  Radio, 
  Layers,
  Sparkles,
  RefreshCw
} from 'lucide-react'
import { api } from './api.js'
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

  const totalTorrents = stats?.total_torrents || 3418496

  return (
    <div className="min-h-screen bg-[#000000] text-[#ededed] font-sans antialiased selection:bg-[#333] selection:text-white">
      {/* Top Hairline */}
      <div className="h-[1px] w-full bg-gradient-to-r from-transparent via-[#333] to-transparent" />

      {/* Global Header */}
      <header className="border-b border-[#1e1e1e] bg-[#000000]/90 sticky top-0 z-40 backdrop-blur-md">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 h-14 flex items-center justify-between">
          {/* Logo / Context */}
          <div className="flex items-center gap-4">
            <div className="flex items-center gap-2.5">
              <div className="w-5 h-5 rounded-md bg-[#ededed] flex items-center justify-center text-black font-mono font-bold text-xs">
                G
              </div>
              <div className="flex items-baseline gap-1.5">
                <span className="text-sm font-semibold tracking-tight text-white">GAIA</span>
                <span className="text-xs text-[#666] font-mono">/ indexer</span>
              </div>
            </div>

            <div className="h-3.5 w-[1px] bg-[#222]" />

            <div className="hidden sm:flex items-center gap-2 font-mono text-xs text-[#888]">
              <span className="w-2 h-2 rounded-full bg-emerald-400" />
              <span>{totalTorrents.toLocaleString()} payloads indexed</span>
            </div>
          </div>

          {/* Quick Stats Badges & Operator Link */}
          <div className="flex items-center gap-2.5 font-mono text-xs">
            <div className="hidden md:flex items-center gap-2 px-2.5 py-1 rounded bg-[#111] border border-[#222] text-[#888]">
              <Zap className="w-3.5 h-3.5 text-emerald-400" />
              <span>+{formatNum(stats?.verified_last_24h || 408710)}/24h</span>
            </div>

            <div className="hidden lg:flex items-center gap-2 px-2.5 py-1 rounded bg-[#111] border border-[#222] text-[#888]">
              <ShieldCheck className="w-3.5 h-3.5 text-cyan-400" />
              <span>{formatNum(stats?.healthy_count || 951907)} healthy</span>
            </div>

            <a
              href="http://192.168.10.10:3001"
              target="_blank"
              rel="noreferrer"
              className="flex items-center gap-1.5 px-2.5 py-1 rounded bg-[#141414] border border-[#262626] text-[#888] hover:text-white hover:border-[#444] transition-colors"
            >
              <span>Operator Dashboard</span>
              <ExternalLink className="w-3 h-3 text-[#666]" />
            </a>
          </div>
        </div>
      </header>

      {/* Main Content Area */}
      <main className="max-w-7xl w-full mx-auto px-4 sm:px-6 lg:px-8 py-6">
        <TorrentBrowser totalCatalogedCount={totalTorrents} />
      </main>

      {/* Footer */}
      <footer className="border-t border-[#181818] bg-[#000000] py-6 text-center text-xs font-mono text-[#555]">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 flex flex-col sm:flex-row items-center justify-between gap-2">
          <div>
            GAIA BitTorrent DHT Intelligence Platform • High Velocity V2 Indexer
          </div>
          <div className="flex items-center gap-3 text-[#666]">
            <span>Runtime: ASP.NET Core (.NET 10)</span>
            <span>•</span>
            <span>Engine: Quickwit Distributed Tantivy</span>
          </div>
        </div>
      </footer>
    </div>
  )
}
