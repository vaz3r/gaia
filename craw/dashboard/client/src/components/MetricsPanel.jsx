import { useEffect, useMemo, useState } from 'react'
import {
  ResponsiveContainer,
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
} from 'recharts'
import { api } from '../api.js'
import { formatNum, toRates } from '../utils.js'

const RANGES = [
  { key: '1h', hours: 1, interval: 'minute' },
  { key: '6h', hours: 6, interval: 'minute' },
  { key: '24h', hours: 24, interval: 'hour' },
  { key: '7d', hours: 168, interval: 'hour' },
]

const SERIES = [
  { key: 'verify_success', label: 'Verified', color: '#38bdf8' },
  { key: 'infohashes_harvested', label: 'Discovered', color: '#34d399' },
  { key: 'fetch_attempts', label: 'Fetch attempts', color: '#fbbf24' },
  { key: 'verify_fail', label: 'Failed', color: '#f87171' },
  { key: 'verify_timeouts', label: 'Timeouts', color: '#c084fc' },
]

function Card({ label, value, sub, accent }) {
  return (
    <div className="rounded-lg bg-ink-800 px-4 py-3 border border-ink-700">
      <div className="text-xs uppercase tracking-wide text-slate-400">{label}</div>
      <div className={`text-xl font-semibold mt-1 tabular-nums ${accent ? 'text-accent' : 'text-slate-100'}`}>
        {value}
      </div>
      {sub && <div className="text-xs text-slate-400 mt-0.5">{sub}</div>}
    </div>
  )
}

const fmt = (v) => {
  const n = Number(v)
  return Number.isFinite(n) ? formatNum(Math.round(n)) : '—'
}

function Stat({ label, value, sub, accent }) {
  return <Card label={label} value={fmt(value)} sub={sub} accent={accent} />
}

export default function MetricsPanel() {
  const [range, setRange] = useState(RANGES[0])
  const [cur, setCur] = useState(null)
  const [series, setSeries] = useState({})

  useEffect(() => {
    api('/api/metrics/current').then(setCur).catch(() => {})
    const id = setInterval(() => api('/api/metrics/current').then(setCur).catch(() => {}), 20000)
    return () => clearInterval(id)
  }, [])

  useEffect(() => {
    const from = new Date(Date.now() - range.hours * 3600 * 1000).toISOString()
    let cancelled = false
    Promise.all(SERIES.map((s) => api(`/api/metrics/history?metric=${s.key}&from=${from}&interval=${range.interval}`)))
      .then((res) => {
        if (cancelled) return
        const byKey = {}
        res.forEach((r, i) => (byKey[SERIES[i].key] = r.data))
        setSeries(byKey)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [range])

  const chartData = useMemo(() => {
    const seriesKeys = SERIES.map((s) => s.key)
    const any = seriesKeys.find((k) => series[k]?.length)
    if (!any) return []
    const times = series[any].map((p) => p.t)
    return times.map((t) => {
      const point = { t }
      for (const key of seriesKeys) {
        if (!series[key]) continue
        const rates = toRates(series[key])
        const found = rates.find((r) => Math.abs(r.t - t) < 1)
        point[key] = found ? Math.round(found.value) : null
        point[`${key}Raw`] = series[key].find((r) => Math.abs(r.t - t) < 1)?.value ?? null
      }
      return point
    })
  }, [series])

  const rates = cur?.rates ?? {}
  const snap = cur?.snapshot ?? {}
  const failRate = (rates.verify_fail ?? 0) + (rates.verify_timeouts ?? 0)

  const stats = [
    { label: 'Verified /hr', value: rates.verify_success, sub: 'success rate', accent: true },
    { label: 'Discovered /hr', value: rates.infohashes_harvested, sub: 'infohashes harvested', accent: true },
    { label: 'Unique infohashes', value: snap.unique_infohashes, sub: 'total tracked', accent: false },
    { label: 'Routing table', value: snap.routing_table_len, sub: 'peers in routing table', accent: false },
    { label: 'Inbound get_peers /hr', value: rates.inbound_get_peers, sub: 'peers querying us', accent: true },
    { label: 'Fetch attempts /hr', value: rates.fetch_attempts, sub: 'outbound fetches', accent: true },
    { label: 'Fail + timeout /hr', value: failRate, sub: 'failed verification', accent: false },
    { label: 'Verified stored', value: snap.verify_success, sub: 'metadata stored', accent: false },
  ]

  const chartT = (t) => {
    const d = new Date(t)
    const hh = String(d.getHours()).padStart(2, '0')
    const mm = String(d.getMinutes()).padStart(2, '0')
    return `${hh}:${mm}`
  }

  return (
    <div>
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3 mb-6">
        {stats.map((s) => (
          <Stat key={s.label} {...s} />
        ))}
      </div>

      <div className="flex items-center justify-between mb-3">
        <div className="text-sm font-medium text-slate-300">Throughput (per hour)</div>
        <div className="flex gap-1 text-xs">
          {RANGES.map((r) => (
            <button
              key={r.key}
              onClick={() => setRange(r)}
              className={`px-3 py-1 rounded-md ${
                range.key === r.key ? 'bg-ink-700 text-white' : 'bg-ink-800 text-slate-400 hover:text-white'
              } border border-ink-700`}
            >
              {r.key}
            </button>
          ))}
        </div>
      </div>

      <div className="rounded-lg border border-ink-700 bg-ink-900 p-4 h-80">
        <ResponsiveContainer width="100%" height="100%">
          <LineChart data={chartData} margin={{ top: 5, right: 10, bottom: 5, left: 0 }}>
            <CartesianGrid stroke="#1e293b" strokeDasharray="3 3" />
            <XAxis dataKey="t" tickFormatter={chartT} stroke="#475569" fontSize={11} minTickGap={40} />
            <YAxis stroke="#475569" fontSize={11} tickFormatter={fmt} width={60} />
            <Tooltip
              contentStyle={{ background: '#101626', border: '1px solid #26304d', borderRadius: 8 }}
              labelFormatter={chartT}
              formatter={(v, name) => [formatNum(v), name]}
            />
            <Legend wrapperStyle={{ fontSize: 12 }} />
            {SERIES.map((s) => (
              <Line
                key={s.key}
                type="monotone"
                dataKey={s.key}
                name={s.label}
                stroke={s.color}
                strokeWidth={1.8}
                dot={false}
                isAnimationActive={false}
                connectNulls
              />
            ))}
          </LineChart>
        </ResponsiveContainer>
      </div>
    </div>
  )
}