export function formatBytes(n) {
  if (n === null || n === undefined) return '—'
  if (n === 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)))
  return `${(n / 1024 ** i).toFixed(i === 0 ? 0 : 1)} ${units[i]}`
}

export function formatNum(n) {
  if (n === null || n === undefined) return '—'
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}k`
  return String(Math.round(n))
}

export const TIMEZONE = 'Asia/Dubai';

export function formatTime(ts) {
  if (!ts) return '—'
  const d = new Date(ts)
  return d.toLocaleString('en-US', {
    timeZone: TIMEZONE,
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  })
}

export function formatDubaiTimeHM(ts) {
  if (!ts) return '—'
  const d = new Date(ts)
  return d.toLocaleTimeString('en-US', {
    timeZone: TIMEZONE,
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })
}

export function formatDubaiDate(ts) {
  if (!ts) return '—'
  const d = new Date(ts)
  return d.toLocaleString('en-US', {
    timeZone: TIMEZONE,
    month: 'short',
    day: 'numeric',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  })
}

export function formatRate(n) {
  if (n === null || n === undefined) return '—'
  const abs = Math.abs(n)
  let formatted
  if (abs >= 1e6) formatted = `${(n / 1e6).toFixed(1)}M`
  else if (abs >= 1e3) formatted = `${(n / 1e3).toFixed(1)}k`
  else formatted = String(Math.round(n))
  return `${formatted}/hr`
}

export function formatPct(n, total) {
  if (!total || total === 0) return '—'
  return `${((n / total) * 100).toFixed(1)}%`
}

export function formatUptime(s) {
  if (s === null || s === undefined) return '—'
  if (s < 60) return `${Math.round(s)}s`
  if (s < 3600) return `${Math.round(s / 60)}m`
  const h = Math.floor(s / 3600)
  const m = Math.round((s % 3600) / 60)
  return m > 0 ? `${h}h ${m}m` : `${h}h`
}

/* Cumulative counters → per-hour rate series from evenly-bucketed points. */
export function toRates(data) {
  const out = []
  for (let i = 1; i < data.length; i++) {
    const prev = data[i - 1]
    const cur = data[i]
    const hours = ((cur.t - prev.t) / 3600000)
    if (hours > 0) {
      const delta = cur.value - prev.value
      out.push({ t: cur.t, value: delta >= 0 ? delta / hours : null })
    }
  }
  return out
}