const API_BASE = "/api";

export interface Torrent {
  infohash: string;
  name: string;
  file_count: number;
  total_size: number;
  extensions: string[] | null;
  top_folders: string[] | null;
  largest_files: { name: string; size: number }[] | null;
}

export interface Prediction {
  label: string;
  confidence: number;
  probabilities: Record<string, number>;
}

export interface Stats {
  totalTorrents: number;
  totalLabeled: number;
  categoryDistribution: { category: string; count: number }[];
}

export async function searchTorrents(
  search: string,
  page = 1,
  limit = 20
): Promise<{ data: Torrent[]; total: number; page: number; limit: number }> {
  const params = new URLSearchParams({ page: String(page), limit: String(limit) });
  if (search) params.set("search", search);
  const res = await fetch(`${API_BASE}/torrents?${params}`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

export async function randomTorrents(count = 10): Promise<{ data: Torrent[] }> {
  const res = await fetch(`${API_BASE}/torrents/random?count=${count}`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

export async function classifyTorrent(
  torrent: Record<string, unknown>
): Promise<Prediction> {
  const res = await fetch(`${API_BASE}/classify`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(torrent),
  });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

export async function getStats(): Promise<Stats> {
  const res = await fetch(`${API_BASE}/stats`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

export function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + " " + sizes[i];
}
