export async function api(path, options = {}) {
  const fetchOptions = { ...options };
  if (fetchOptions.body && typeof fetchOptions.body === 'string' && !fetchOptions.headers) {
    fetchOptions.headers = { 'Content-Type': 'application/json' };
  } else if (fetchOptions.body && typeof fetchOptions.body === 'string' && fetchOptions.headers && !fetchOptions.headers['Content-Type']) {
    fetchOptions.headers['Content-Type'] = 'application/json';
  }
  const r = await fetch(path, fetchOptions);
  if (!r.ok) {
    let msg = `HTTP ${r.status}`;
    try {
      const j = await r.json();
      msg = j.error || j.detail || msg;
    } catch {}
    throw new Error(msg);
  }
  return r.json();
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

export async function downloadTorrent(infohash, name) {
  const res = await fetch(`/api/torrents/${infohash}/torrent`);
  if (!res.ok) {
    let errMsg = `HTTP ${res.status}`;
    let fallbackMagnet = null;
    try {
      const j = await res.json();
      errMsg = j.error || errMsg;
      fallbackMagnet = j.magnet;
    } catch {}
    const err = new Error(errMsg);
    err.magnet = fallbackMagnet;
    throw err;
  }
  const blob = await res.blob();
  const url = window.URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  const cleanName = (name || `torrent-${infohash.slice(0, 8)}`)
    .replace(/[/\\?%*:|"<>]/g, '_')
    .replace(/\s+/g, ' ')
    .trim();
  a.download = `${cleanName}.torrent`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  window.URL.revokeObjectURL(url);
}
