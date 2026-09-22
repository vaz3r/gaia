import React from 'react'

export function getHealthColor(score) {
  if (score == null) return { bar: 'bg-zinc-700', text: 'text-zinc-500', label: 'Unknown' }
  if (score >= 70) return { bar: 'bg-emerald-400', text: 'text-emerald-400', label: 'Verified' }
  if (score >= 40) return { bar: 'bg-amber-400', text: 'text-amber-400', label: 'Good' }
  if (score >= 15) return { bar: 'bg-orange-400', text: 'text-orange-400', label: 'Degraded' }
  return { bar: 'bg-rose-400', text: 'text-rose-400', label: 'Poor' }
}

export function getPopularityColor(score) {
  if (score == null || score === 0) return { bar: 'bg-zinc-700', text: 'text-zinc-600', label: 'Quiet' }
  if (score >= 70) return { bar: 'bg-violet-400', text: 'text-violet-400', label: 'Trending' }
  if (score >= 40) return { bar: 'bg-blue-400', text: 'text-blue-400', label: 'Popular' }
  if (score >= 15) return { bar: 'bg-sky-400', text: 'text-sky-400', label: 'Moderate' }
  return { bar: 'bg-zinc-400', text: 'text-zinc-400', label: 'Low' }
}

export function getAvailabilityColor(state) {
  switch (state) {
    case 'VERIFIED':  return { bg: 'bg-emerald-950/30', text: 'text-emerald-400', border: 'border-emerald-800/30' }
    case 'UNVERIFIED': return { bg: 'bg-amber-950/30', text: 'text-amber-300', border: 'border-amber-800/30' }
    case 'STALE':     return { bg: 'bg-rose-950/30', text: 'text-rose-400', border: 'border-rose-800/30' }
    default:          return { bg: 'bg-zinc-900/30', text: 'text-zinc-500', border: 'border-zinc-800/30' }
  }
}

export function HealthBar({ score, className = '' }) {
  const c = getHealthColor(score)
  return (
    <div className={`flex items-center gap-2 ${className}`}>
      <div className="w-12 bg-[#181818] rounded-full h-1.5 overflow-hidden">
        <div
          className={`h-full rounded-full ${c.bar}`}
          style={{ width: `${Math.min(100, Math.max(0, score ?? 0))}%` }}
        />
      </div>
      {score != null ? (
        <span className={`text-[11px] font-mono font-semibold ${c.text}`}>{score}%</span>
      ) : (
        <span className="text-[11px] font-mono text-zinc-500" title="Unprobed">—</span>
      )}
    </div>
  )
}

export function PopularityBar({ score, className = '' }) {
  const c = getPopularityColor(score)
  return (
    <div className={`flex items-center gap-2 ${className}`}>
      <div className="w-12 bg-[#181818] rounded-full h-1.5 overflow-hidden">
        <div
          className={`h-full rounded-full ${c.bar}`}
          style={{ width: `${Math.min(100, Math.max(0, score ?? 0))}%` }}
        />
      </div>
      <span className={`text-[11px] font-mono font-semibold ${c.text}`}>{score ?? 0}%</span>
    </div>
  )
}

export function HealthStatePill({ state, className = '' }) {
  const c = getAvailabilityColor(state)
  return (
    <span className={`text-[9px] font-mono px-1 py-0.5 rounded border ${c.bg} ${c.text} ${c.border} ${className}`}>
      {state || 'UNKNOWN'}
    </span>
  )
}

export function EvidenceBreakdown({ evidence, healthScore }) {
  if (!evidence) {
    return (
      <div className="pt-2 border-t border-[#141414] space-y-1 font-mono text-[10px]">
        <div className="flex items-center justify-between text-[#777]">
          <span>Health Score:</span>
          <span className="text-white font-semibold">{healthScore ?? '—'}/100</span>
        </div>
        <div className="text-[10px] text-[#555] italic">Legacy (pre-v2.0.0)</div>
      </div>
    )
  }

  const e = typeof evidence === 'string' ? JSON.parse(evidence) : evidence
  const total = (e.direct_component || 0) + (e.seed_component || 0) + (e.peer_component || 0) + (e.dht_component || 0) - (e.failure_penalty || 0)

  return (
    <div className="pt-2 border-t border-[#141414] space-y-1 font-mono text-[10px]">
      <div className="flex items-center justify-between text-[#777]">
        <span>Direct Probe:</span>
        <span className={e.direct_component > 0 ? 'text-emerald-400' : 'text-[#666]'}>
          +{(e.direct_component || 0).toFixed(1)} pts (max 40)
        </span>
      </div>
      <div className="flex items-center justify-between text-[#777]">
        <span>Seed Evidence:</span>
        <span className={e.seed_component > 0 ? 'text-emerald-400' : 'text-[#666]'}>
          +{(e.seed_component || 0).toFixed(1)} pts (max 30)
        </span>
      </div>
      <div className="flex items-center justify-between text-[#777]">
        <span>Peer Activity:</span>
        <span className={e.peer_component > 0 ? 'text-cyan-400' : 'text-[#666]'}>
          +{(e.peer_component || 0).toFixed(1)} pts (max 20)
        </span>
      </div>
      <div className="flex items-center justify-between text-[#777]">
        <span>DHT Sightings:</span>
        <span className={e.dht_component > 0 ? 'text-cyan-400' : 'text-[#666]'}>
          +{(e.dht_component || 0).toFixed(1)} pts (max 10)
        </span>
      </div>
      {e.failure_penalty > 0 && (
        <div className="flex items-center justify-between text-[#777]">
          <span>Failure Penalty:</span>
          <span className="text-rose-400">
            -{(e.failure_penalty || 0).toFixed(1)} pts (max 30)
          </span>
        </div>
      )}
      <div className="flex items-center justify-between text-white font-semibold pt-1 border-t border-[#141414]">
        <span>Total:</span>
        <span>{Math.round(total)}/100</span>
      </div>
    </div>
  )
}
