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
import { formatNum, formatRate, toRates } from '../utils.js'

const RANGES = [
  { key: '1h', hours: 1, interval: 'minute' },
  { key: '6h', hours: 6, interval: 'minute' },
  { key: '24h', hours: 24, interval: 'hour' },
  { key: '7d', hours: 168, interval: 'hour' },
]

const THROUGHPUT_SERIES = [
  { key: 'verify_success', label: 'Verified', color: '#38bdf8' },
  { key: 'infohashes_harvested', label: 'Discovered', color: '#34d399' },
  { key: 'fetch_attempts', label: 'Fetch attempts', color: '#fbbf24' },
  { key: 'verify_fail', label: 'Failed', color: '#f87171' },
  { key: 'verify_timeouts', label: 'Timeouts', color: '#c084fc' },
]

const TRANSPORT_SERIES = [
  { key: 'tcp_metadata_ok', label: 'TCP metadata', color: '#38bdf8' },
  { key: 'utp_metadata_ok', label: 'uTP metadata', color: '#34d399' },
  { key: 'tcp_connect_ok', label: 'TCP connect', color: '#60a5fa' },
  { key: 'utp_connect_ok', label: 'uTP connect', color: '#6ee7b7' },
]

const DHT_SERIES = [
  { key: 'inbound_get_peers', label: 'get_peers', color: '#38bdf8' },
  { key: 'inbound_find_node', label: 'find_node', color: '#fbbf24' },
  { key: 'inbound_announce_peer', label: 'announce', color: '#34d399' },
]

function Panel({ title, children, className = '' }) {
  return (
    <div className={`rounded-lg bg-ink-800 px-4 py-3 border border-ink-700 ${className}`}>
      <div className="text-xs uppercase tracking-wide text-slate-400 mb-2">{title}</div>
      {children}
    </div>
  )
}

function Metric({ label, value, sub, accent, rate }) {
  return (
    <div className="flex flex-col">
      <div className="text-xs text-slate-400">{label}</div>
      <div className={`text-lg font-semibold tabular-nums ${accent ? 'text-accent' : 'text-slate-100'}`}>
        {value}
      </div>
      {(sub || rate) && (
        <div className="text-xs text-slate-500">
          {rate && <span className="text-slate-400">{rate}</span>}
          {rate && sub && ' · '}
          {sub}
        </div>
      )}
    </div>
  )
}

function PctBar({ value, total, color = 'bg-accent' }) {
  if (!total || total === 0) return null
  const pct = Math.min(100, (value / total) * 100)
  return (
    <div className="w-full bg-ink-700 rounded-full h-1.5 mt-1">
      <div className={`${color} h-1.5 rounded-full`} style={{ width: `${pct}%` }} />
    </div>
  )
}

function Row({ label, value, rate, pctOf }) {
  return (
    <div className="flex justify-between items-center">
      <span className="text-slate-400 text-sm">{label}</span>
      <div className="flex items-center gap-3">
        {rate !== undefined && (
          <span className="text-slate-500 text-xs tabular-nums">{rate}</span>
        )}
        <span className="text-slate-200 tabular-nums text-sm">{value}</span>
      </div>
    </div>
  )
}

const fmt = (v) => {
  if (v === null || v === undefined) return '—'
  const n = Number(v)
  return Number.isFinite(n) ? formatNum(Math.round(n)) : '—'
}

const fmtRate = (v) => {
  if (v === null || v === undefined) return '—'
  const n = Number(v)
  return Number.isFinite(n) ? formatRate(Math.round(n)) : '—'
}

const fmtPct = (n, t) => {
  if (!t || t === 0) return '—'
  return `${((n / t) * 100).toFixed(1)}%`
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
    const allSeries = [...THROUGHPUT_SERIES, ...TRANSPORT_SERIES, ...DHT_SERIES]
    Promise.all(allSeries.map((s) => api(`/api/metrics/history?metric=${s.key}&from=${from}&interval=${range.interval}`)))
      .then((res) => {
        if (cancelled) return
        const byKey = {}
        res.forEach((r, i) => (byKey[allSeries[i].key] = r.data))
        setSeries(byKey)
      })
      .catch(() => {})
    return () => { cancelled = true }
  }, [range])

  const rates = cur?.rates ?? {}
  const snap = cur?.snapshot ?? {}

  // Derived metrics
  const tcpAttempts = rates.tcp_attempts || 0
  const utpAttempts = rates.utp_attempts || 0
  const tcpConnectOk = rates.tcp_connect_ok || 0
  const utpConnectOk = rates.utp_connect_ok || 0
  const tcpMetadataOk = rates.tcp_metadata_ok || 0
  const utpMetadataOk = rates.utp_metadata_ok || 0
  const totalAttempts = tcpAttempts + utpAttempts
  const totalConnectOk = tcpConnectOk + utpConnectOk
  const totalMetadataOk = tcpMetadataOk + utpMetadataOk
  const tcpConnectRate = tcpAttempts > 0 ? (tcpConnectOk / tcpAttempts) * 100 : 0
  const utpConnectRate = utpAttempts > 0 ? (utpConnectOk / utpAttempts) * 100 : 0
  const tcpMetadataRate = tcpConnectOk > 0 ? (tcpMetadataOk / tcpConnectOk) * 100 : 0
  const utpMetadataRate = utpConnectOk > 0 ? (utpMetadataOk / utpConnectOk) * 100 : 0

  const findNodeTotal = (rates.inbound_find_node || 0) + (rates.inbound_find_node_dropped || 0)
  const findNodeDropPct = findNodeTotal > 0 ? ((rates.inbound_find_node_dropped || 0) / findNodeTotal * 100) : 0

  const failTotal = (rates.verify_fail || 0) + (rates.verify_timeouts || 0)
  const cacheHits = snap.peer_cache_hits || 0
  const cacheTotal = cacheHits + (snap.peer_cache_evictions || 0)
  const cacheHitRate = cacheTotal > 0 ? (cacheHits / cacheTotal) * 100 : 0

  const verifyDepth = snap.verify_channel_depth || 0
  const verifyMax = snap.verify_channel_depth_max || 1
  const freshDepth = snap.fresh_channel_depth || 0
  const freshMax = snap.fresh_channel_depth_max || 1

  // Throughput chart
  const throughputData = useMemo(() => {
    const keys = THROUGHPUT_SERIES.map((s) => s.key)
    const any = keys.find((k) => series[k]?.length)
    if (!any) return []
    return series[any].map((p) => {
      const point = { t: p.t }
      for (const key of keys) {
        if (!series[key]) continue
        const rates = toRates(series[key])
        const found = rates.find((r) => Math.abs(r.t - p.t) < 1)
        point[key] = found ? Math.round(found.value) : null
      }
      return point
    })
  }, [series])

  // Transport chart
  const transportData = useMemo(() => {
    const keys = TRANSPORT_SERIES.map((s) => s.key)
    const any = keys.find((k) => series[k]?.length)
    if (!any) return []
    return series[any].map((p) => {
      const point = { t: p.t }
      for (const key of keys) {
        if (!series[key]) continue
        const rates = toRates(series[key])
        const found = rates.find((r) => Math.abs(r.t - p.t) < 1)
        point[key] = found ? Math.round(found.value) : null
      }
      return point
    })
  }, [series])

  // DHT chart
  const dhtData = useMemo(() => {
    const keys = DHT_SERIES.map((s) => s.key)
    const any = keys.find((k) => series[k]?.length)
    if (!any) return []
    return series[any].map((p) => {
      const point = { t: p.t }
      for (const key of keys) {
        if (!series[key]) continue
        const rates = toRates(series[key])
        const found = rates.find((r) => Math.abs(r.t - p.t) < 1)
        point[key] = found ? Math.round(found.value) : null
      }
      return point
    })
  }, [series])

  const chartT = (t) => {
    const d = new Date(t)
    const hh = String(d.getHours()).padStart(2, '0')
    const mm = String(d.getMinutes()).padStart(2, '0')
    return `${hh}:${mm}`
  }

  const chartFmt = (v) => {
    if (v === null || v === undefined) return ''
    return formatNum(v)
  }

  return (
    <div>
      {/* Throughput */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3 mb-4">
        <Metric label="Verified /hr" value={fmtRate(rates.verify_success)} accent />
        <Metric label="Discovered /hr" value={fmtRate(rates.infohashes_harvested)} accent />
        <Metric label="Fetch attempts /hr" value={fmtRate(rates.fetch_attempts)} accent />
        <Metric label="Failures /hr" value={fmtRate(failTotal)} sub={`${fmtPct(failTotal, rates.fetch_attempts || 1)} of fetches`} />
      </div>

      {/* Transport Pipeline */}
      <div className="mb-4">
        <Panel title="Transport Pipeline">
          <div className="grid grid-cols-2 md:grid-cols-4 gap-6">
            <Metric
              label="TCP"
              value={fmtRate(tcpMetadataOk)}
              rate={`${fmtPct(tcpMetadataOk, tcpAttempts)} success`}
              sub={`${fmtRate(tcpAttempts)} attempts`}
            />
            <Metric
              label="uTP"
              value={fmtRate(utpMetadataOk)}
              rate={`${fmtPct(utpMetadataOk, utpAttempts)} success`}
              sub={`${fmtRate(utpAttempts)} attempts`}
            />
            <Metric
              label="Combined"
              value={fmtRate(totalMetadataOk)}
              rate={`${fmtPct(totalMetadataOk, totalAttempts)} success`}
              sub={`${fmtRate(totalAttempts)} total`}
            />
            <div className="flex flex-col gap-2">
              <div>
                <div className="text-xs text-slate-400">TCP connect rate</div>
                <div className="text-sm text-slate-200 tabular-nums">{tcpConnectRate.toFixed(1)}%</div>
                <PctBar value={tcpConnectRate} total={100} />
              </div>
              <div>
                <div className="text-xs text-slate-400">uTP connect rate</div>
                <div className="text-sm text-slate-200 tabular-nums">{utpConnectRate.toFixed(1)}%</div>
                <PctBar value={utpConnectRate} total={100} color="bg-emerald-400" />
              </div>
            </div>
          </div>
        </Panel>
      </div>

      {/* DHT / Source + Pipeline Health */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mb-4">
        <Panel title="DHT / Source">
          <div className="grid grid-cols-1 gap-3">
            <Metric label="Inbound get_peers /hr" value={fmtRate(rates.inbound_get_peers)} accent />
            <Metric label="Source peers returned /hr" value={fmtRate(rates.source_peers_returned)} />
            <Metric label="Source filtered /hr" value={fmtRate(rates.source_filtered_by_cache)} sub="by peer cache" />
            <div>
              <div className="flex justify-between items-center">
                <span className="text-xs text-slate-400">find_node drop %</span>
                <span className="text-sm text-slate-200 tabular-nums">{findNodeDropPct.toFixed(1)}%</span>
              </div>
              <PctBar value={findNodeDropPct} total={100} color="bg-amber-400" />
            </div>
          </div>
        </Panel>

        <Panel title="Pipeline Health">
          <div className="grid grid-cols-1 gap-3">
            <div>
              <div className="flex justify-between items-center">
                <span className="text-xs text-slate-400">Verify channel</span>
                <span className="text-sm text-slate-200 tabular-nums">{fmtNum(verifyDepth)} / {fmtNum(verifyMax)}</span>
              </div>
              <PctBar value={verifyDepth} total={verifyMax} />
            </div>
            <div>
              <div className="flex justify-between items-center">
                <span className="text-xs text-slate-400">Fresh channel</span>
                <span className="text-sm text-slate-200 tabular-nums">{fmtNum(freshDepth)} / {fmtNum(freshMax)}</span>
              </div>
              <PctBar value={freshDepth} total={freshMax} color="bg-emerald-400" />
            </div>
            <Metric label="Scheduler claims /hr" value={fmtRate(rates.scheduler_claims)} />
            <div className="grid grid-cols-2 gap-3">
              <Metric label="Fresh claims" value={fmtRate(rates.scheduler_claimed_fresh)} />
              <Metric label="Retry claims" value={fmtRate(rates.scheduler_claimed_retry)} />
            </div>
          </div>
        </Panel>
      </div>

      {/* Failure Breakdown */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mb-4">
        <Panel title="Failure Breakdown (/hr)">
          <div className="grid grid-cols-1 gap-1 text-sm">
            {[
              { label: 'Source timeout', key: 'source_timeout' },
              { label: 'Source no peers', key: 'source_no_peers' },
              { label: 'Connect timeout', key: 'fetch_connect_timeout' },
              { label: 'Connect I/O', key: 'fetch_connect_io' },
              { label: 'Handshake', key: 'fetch_handshake' },
              { label: 'No extension', key: 'fetch_no_extension' },
              { label: 'Reject', key: 'fetch_reject' },
              { label: 'Bad piece', key: 'fetch_bad_piece' },
              { label: 'Transfer I/O', key: 'fetch_io' },
              { label: 'SHA1 mismatch', key: 'sha1_mismatch' },
            ].map(({ label, key }) => {
              const val = rates[key] || 0
              return (
                <Row
                  key={key}
                  label={label}
                  value={fmtRate(val)}
                  pctOf={failTotal}
                />
              )
            })}
          </div>
        </Panel>

        <Panel title="Peer Cache">
          <div className="grid grid-cols-1 gap-3">
            <div className="grid grid-cols-2 gap-3">
              <Metric label="Size" value={fmt(snap.peer_cache_size)} />
              <Metric
                label="Hit rate"
                value={`${cacheHitRate.toFixed(1)}%`}
                sub={`${fmtNum(cacheHits)} hits`}
              />
            </div>
            <div className="grid grid-cols-2 gap-3">
              <Metric label="Evictions" value={fmt(snap.peer_cache_evictions)} />
              <Metric label="Harvest drops" value={fmt(snap.harvest_try_send_dropped)} />
            </div>
          </div>
        </Panel>
      </div>

      {/* Charts */}
      <div className="space-y-4">
        <div>
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
              <LineChart data={throughputData} margin={{ top: 5, right: 10, bottom: 5, left: 0 }}>
                <CartesianGrid stroke="#1e293b" strokeDasharray="3 3" />
                <XAxis dataKey="t" tickFormatter={chartT} stroke="#475569" fontSize={11} minTickGap={40} />
                <YAxis stroke="#475569" fontSize={11} tickFormatter={chartFmt} width={60} />
                <Tooltip
                  contentStyle={{ background: '#101626', border: '1px solid #26304d', borderRadius: 8 }}
                  labelFormatter={chartT}
                  formatter={(v, name) => [formatNum(v), `${name}/hr`]}
                />
                <Legend wrapperStyle={{ fontSize: 12 }} />
                {THROUGHPUT_SERIES.map((s) => (
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

        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <div>
            <div className="text-sm font-medium text-slate-300 mb-3">Transport (per hour)</div>
            <div className="rounded-lg border border-ink-700 bg-ink-900 p-4 h-64">
              <ResponsiveContainer width="100%" height="100%">
                <LineChart data={transportData} margin={{ top: 5, right: 10, bottom: 5, left: 0 }}>
                  <CartesianGrid stroke="#1e293b" strokeDasharray="3 3" />
                  <XAxis dataKey="t" tickFormatter={chartT} stroke="#475569" fontSize={11} minTickGap={40} />
                  <YAxis stroke="#475569" fontSize={11} tickFormatter={chartFmt} width={50} />
                  <Tooltip
                    contentStyle={{ background: '#101626', border: '1px solid #26304d', borderRadius: 8 }}
                    labelFormatter={chartT}
                    formatter={(v, name) => [formatNum(v), `${name}/hr`]}
                  />
                  <Legend wrapperStyle={{ fontSize: 11 }} />
                  {TRANSPORT_SERIES.map((s) => (
                    <Line
                      key={s.key}
                      type="monotone"
                      dataKey={s.key}
                      name={s.label}
                      stroke={s.color}
                      strokeWidth={1.5}
                      dot={false}
                      isAnimationActive={false}
                      connectNulls
                    />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            </div>
          </div>

          <div>
            <div className="text-sm font-medium text-slate-300 mb-3">DHT Traffic (per hour)</div>
            <div className="rounded-lg border border-ink-700 bg-ink-900 p-4 h-64">
              <ResponsiveContainer width="100%" height="100%">
                <LineChart data={dhtData} margin={{ top: 5, right: 10, bottom: 5, left: 0 }}>
                  <CartesianGrid stroke="#1e293b" strokeDasharray="3 3" />
                  <XAxis dataKey="t" tickFormatter={chartT} stroke="#475569" fontSize={11} minTickGap={40} />
                  <YAxis stroke="#475569" fontSize={11} tickFormatter={chartFmt} width={50} />
                  <Tooltip
                    contentStyle={{ background: '#101626', border: '1px solid #26304d', borderRadius: 8 }}
                    labelFormatter={chartT}
                    formatter={(v, name) => [formatNum(v), `${name}/hr`]}
                  />
                  <Legend wrapperStyle={{ fontSize: 11 }} />
                  {DHT_SERIES.map((s) => (
                    <Line
                      key={s.key}
                      type="monotone"
                      dataKey={s.key}
                      name={s.label}
                      stroke={s.color}
                      strokeWidth={1.5}
                      dot={false}
                      isAnimationActive={false}
                      connectNulls
                    />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
