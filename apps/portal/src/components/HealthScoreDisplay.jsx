import React from 'react'
import { Shield, ShieldCheck, ShieldAlert, Flame, Zap, TrendingUp } from 'lucide-react'

export function getHealthColor(score) {
  if (score == null) return { bar: 'bg-zinc-700', text: 'text-zinc-500', label: 'Unknown' }
  if (score >= 70) return { bar: 'bg-emerald-400', text: 'text-emerald-400', label: 'Verified' }
  if (score >= 40) return { bar: 'bg-amber-400', text: 'text-amber-400', label: 'Good' }
  if (score >= 15) return { bar: 'bg-orange-400', text: 'text-orange-400', label: 'Degraded' }
  return { bar: 'bg-rose-400', text: 'text-rose-400', label: 'Dead' }
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
    case 'VERIFIED':
    case 'HEALTHY':
    case 'ACTIVE':     return { bg: 'bg-emerald-950/30', text: 'text-emerald-400', border: 'border-emerald-800/30' }
    case 'GOOD':       return { bg: 'bg-sky-950/30', text: 'text-sky-400', border: 'border-sky-800/30' }
    case 'UNVERIFIED':
    case 'SPARSE':
    case 'DEGRADED':   return { bg: 'bg-amber-950/30', text: 'text-amber-300', border: 'border-amber-800/30' }
    case 'DORMANT':
    case 'DEAD':
    case 'POOR':
    case 'STALE':      return { bg: 'bg-rose-950/30', text: 'text-rose-400', border: 'border-rose-800/30' }
    default:           return { bg: 'bg-zinc-900/30', text: 'text-zinc-500', border: 'border-zinc-800/30' }
  }
}

export function deriveScoreFromState(state) {
  switch ((state || '').toUpperCase()) {
    case 'VERIFIED':  return 72   // mid of 70-100 range
    case 'ACTIVE':    return 55   // mid of 40-70 range
    case 'DEGRADED':
    case 'STALE':     return 25   // mid of 15-40 range
    case 'DORMANT':   return 8    // below 15
    case 'DEAD':      return 2
    default:          return null  // truly unknown — show dash
  }
}

export function HealthBar({ score, state, className = '' }) {
  const effectiveScore = score ?? deriveScoreFromState(state)
  const c = getHealthColor(effectiveScore)
  const isEstimated = score == null && effectiveScore != null
  return (
    <div className={`flex items-center gap-2 ${className}`}>
      <div className="w-12 bg-[#181818] rounded-full h-1.5 overflow-hidden">
        <div
          className={`h-full rounded-full ${c.bar}`}
          style={{ width: `${Math.min(100, Math.max(0, effectiveScore ?? 0))}%` }}
        />
      </div>
      {effectiveScore != null ? (
        <span
          className={`text-[11px] font-mono font-semibold ${c.text}`}
          title={isEstimated ? 'Estimated from availability state' : undefined}
        >
          {isEstimated ? `~${effectiveScore}%` : `${score}%`}
        </span>
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

export function HealthStatePill({ state, score, className = '' }) {
  // Intelligent state derivation if state is missing or unknown
  let displayState = state
  if (!displayState || displayState === 'UNKNOWN') {
    if (score != null) {
      if (score >= 70) displayState = 'VERIFIED'
      else if (score >= 40) displayState = 'ACTIVE'
      else if (score >= 15) displayState = 'DEGRADED'
      else if (score > 0) displayState = 'DORMANT'
      else displayState = 'DEAD'
    } else {
      displayState = 'UNPROBED'
    }
  }

  const c = getAvailabilityColor(displayState)
  return (
    <span className={`text-[9px] font-mono px-1 py-0.5 rounded border uppercase ${c.bg} ${c.text} ${c.border} ${className}`}>
      {displayState}
    </span>
  )
}

/**
 * Antivirus Security Shield Component
 * Renders security cleanliness score (0-100) and scan verdict based on integrity_score and policy enforcement
 */
export function SecurityShield({ score, riskTier, policyAction, showDetails = false, className = '' }) {
  const effectiveScore = score ?? (riskTier === 'BLOCKED' ? 0 : riskTier === 'REVIEW' ? 45 : 100)
  
  let verdict = 'Clean'
  let color = 'text-emerald-400 border-emerald-800/50 bg-emerald-950/40'
  let Icon = ShieldCheck

  if (effectiveScore >= 90 && riskTier !== 'BLOCKED') {
    verdict = 'Clean'
    color = 'text-emerald-400 border-emerald-800/50 bg-emerald-950/40'
    Icon = ShieldCheck
  } else if (effectiveScore >= 60 && riskTier !== 'BLOCKED') {
    verdict = 'Low Risk'
    color = 'text-sky-300 border-sky-800/50 bg-sky-950/40'
    Icon = Shield
  } else if (effectiveScore >= 30 && riskTier !== 'BLOCKED') {
    verdict = 'Suspicious'
    color = 'text-amber-300 border-amber-800/50 bg-amber-950/40'
    Icon = ShieldAlert
  } else {
    verdict = 'Threat Blocked'
    color = 'text-rose-300 border-rose-800/50 bg-rose-950/60'
    Icon = ShieldAlert
  }

  return (
    <div className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded border font-mono text-[10px] ${color} ${className}`} title={`Antivirus Scan: ${verdict} (${effectiveScore}/100)`}>
      <Icon className="w-3 h-3 shrink-0" />
      <span className="font-semibold">{effectiveScore}/100</span>
      <span className="opacity-80 uppercase text-[9px]">{verdict}</span>
    </div>
  )
}

/**
 * Search Indexer Trending Badge
 * Displays trending ranking based on search queries, sighting frequency and swarm velocity
 */
export function TrendingBadge({ score, className = '' }) {
  const s = score ?? 0
  if (s >= 80) {
    return (
      <span className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-mono bg-violet-950/40 text-violet-300 border border-violet-800/40 ${className}`} title={`Trending Score: ${s}% (Viral / High Query Traffic)`}>
        <Flame className="w-3 h-3 text-violet-400" />
        <span>Top Trend {s}%</span>
      </span>
    )
  }
  if (s >= 50) {
    return (
      <span className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-mono bg-blue-950/40 text-blue-300 border border-blue-800/40 ${className}`} title={`Trending Score: ${s}% (Rising Search Demand)`}>
        <Zap className="w-3 h-3 text-blue-400" />
        <span>Trending {s}%</span>
      </span>
    )
  }
  if (s >= 20) {
    return (
      <span className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-mono bg-cyan-950/30 text-cyan-400 border border-cyan-800/30 ${className}`} title={`Trending Score: ${s}% (Active)`}>
        <TrendingUp className="w-3 h-3 text-cyan-400" />
        <span>Active {s}%</span>
      </span>
    )
  }
  return (
    <span className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-mono bg-zinc-900/40 text-zinc-400 border border-zinc-800/40 ${className}`} title={`Trending Score: ${s}% (Low Activity)`}>
      <span>Quiet {s}%</span>
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
