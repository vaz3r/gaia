import http from 'k6/http';
import { check } from 'k6';
import { Trend, Rate } from 'k6/metrics';

const searchLatency = new Trend('search_latency', true);
const errorRate = new Rate('error_rate');

// Random query terms to test search backend
const TERMS = [
  'movie', 'action', 'series', 'season', 'soundtrack', '1080p', '4k', '2024', '2025', '2026',
  'rock', 'pop', 'lossless', 'flac', 'remastered', 'edition', 'complete', 'collection', 'pack',
  'windows', 'linux', 'macos', 'game', 'update', 'repack', 'crack', 'patch', 'v1', 'v2',
  'novel', 'audiobook', 'epub', 'pdf', 'guide', 'tutorial', 'course', 'documentary', 'nature',
  'drama', 'comedy', 'thriller', 'horror', 'scifi', 'animation', 'anime', 'manga', 'subtitle'
];

export const options = {
  insecureSkipTLSVerify: true,
  vus: 80,
  duration: '20s',
  thresholds: {
    'http_req_failed': ['rate<0.02'],
    'search_latency': ['p(95)<500'],
  },
};

const BASE_URL = __ENV.TARGET_URL || 'https://gaia-gateway';

export default function () {
  const term = TERMS[Math.floor(Math.random() * TERMS.length)];
  const page = Math.floor(Math.random() * 5) + 1;
  // Use a cache-busting timestamp or random salt so we test raw Meilisearch & API backend capacity
  const salt = Math.floor(Math.random() * 1000);
  const url = `${BASE_URL}/api/torrents?search=${encodeURIComponent(term)}&page=${page}&limit=25&_cb=${salt}`;

  const res = http.get(url);
  const ok = check(res, {
    'status is 200': (r) => r.status === 200,
  });

  searchLatency.add(res.timings.duration);
  errorRate.add(!ok);
}
