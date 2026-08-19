export async function api(path) {
  const r = await fetch(path)
  if (!r.ok) {
    let msg = `HTTP ${r.status}`
    try {
      const j = await r.json()
      msg = j.error || msg
    } catch {}
    throw new Error(msg)
  }
  return r.json()
}

export function magnetFrom(infohash, name) {
  const dn = name ? `&dn=${encodeURIComponent(name)}` : ''
  return `magnet:?xt=urn:btih:${infohash}${dn}`
}