import { useCallback, useEffect, useState } from 'react'
import { api } from './api.js'
import TorrentBrowser from './components/TorrentBrowser.jsx'
import MetricsPanel from './components/MetricsPanel.jsx'
import { formatNum } from './utils.js'

function Stat({ label, value, sub }) {
  return (
    <div className="rounded-lg bg-ink-800 px-4 py-3 border border-ink-700">
      <div className="text-xs uppercase tracking-wide text-slate-400">{label}</div>
      <div className="text-2xl font-semibold text-slate-100 mt-1">{value}</div>
      {sub ? <div className="text-xs text-slate-400 mt-0.5">{sub}</div> : null}
    </div>
  )
}

export default function App() {
  const [tab, setTab] = useState('torrents')
  const [stats, setStats] = useState(null)

  useEffect(() => {
    api('/api/stats').then(setStats).catch(() => {})
    const id = setInterval(() => api('/api/stats').then(setStats).catch(() => {}), 15000)
    return () => clearInterval(id)
  }, [])

  const heartbeat = stats?.crawler_heartbeat_ts
  const stale = stats?.crawler_stale_s ?? null

  return (
    <div className="min-h-screen">
      <header className="border-b border-ink-700 bg-ink-900/60 sticky top-0 z-10 backdrop-blur">
        <div className="max-w-7xl mx-auto px-6 py-3 flex items-center justify-between">
          <div className="flex items-center gap-8">
            <span className="text-lg font-bold tracking-tight">
              craw<span className="text-accent">/dashboard</span>
            </span>
            <nav className="flex gap-1 text-sm">
              {[
                ['torrents', 'Torrents'],
                ['metrics', 'Metrics'],
              ].map(([key, label]) => (
                <button
                  key={key}
                  onClick={() => setTab(key)}
                  className={`px-3 py-1.5 rounded-md transition-colors ${
                    tab === key ? 'bg-ink-700 text-white' : 'text-slate-400 hover:text-white'
                  }`}
                >
                  {label}
                </button>
              ))}
            </nav>
          </div>
          <div className="flex items-center gap-2 text-xs text-slate-400">
            <span
              className={`inline-block w-2 h-2 rounded-full ${
                stale === null ? 'bg-slate-500' : stale > 180 ? 'bg-red-400' : 'bg-emerald-400'
              }`}
            />
            crawler {heartbeat ? `seen ${stale}s ago` : 'no data'}
          </div>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-6 py-6">
        <section className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-6 gap-3 mb-8">
          <Stat label="Torrents indexed" value={formatNum(stats?.total_torrents)} />
          <Stat label="Verified total" value={formatNum(stats?.verified_total)} sub={`${formatNum(stats?.verified_last_24h)} / 24h`} />
          <Stat label="Verified 1h" value={formatNum(stats?.verified_last_1h)} />
          <Stat label="Seen 1h" value={formatNum(stats?.seen_last_1h)} sub={`${formatNum(stats?.new_last_1h)} new`} />
          <Stat label="Queue backlog" value={formatNum(stats?.queue_backlog)} sub={`${formatNum(stats?.verifying)} verifying`} />
          <Stat label="Crawler" value={stale === null ? '—' : stale > 180 ? 'stale' : 'online'} />
        </section>

        {tab === 'torrents' ? <TorrentBrowser /> : <MetricsPanel />}
      </main>
    </div>
  )
}