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

let cachedTrackers = ''

export async function loadTrackers() {
  if (cachedTrackers) return cachedTrackers;
  try {
    const res = await fetch('https://raw.githubusercontent.com/ngosang/trackerslist/refs/heads/master/trackers_all.txt');
    if (res.ok) {
      const text = await res.text();
      const trackers = text.split('\n').map(t => t.trim()).filter(t => t.length > 0);
      cachedTrackers = trackers.map(t => `&tr=${encodeURIComponent(t)}`).join('');
    }
  } catch (e) {
    console.warn("Failed to load trackers", e);
  }
  return cachedTrackers;
}

export function magnetFrom(infohash, name) {
  const dn = name ? `&dn=${encodeURIComponent(name)}` : ''
  return `magnet:?xt=urn:btih:${infohash}${dn}${cachedTrackers}`
}
