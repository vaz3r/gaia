import http from 'k6/http';
import { check, sleep } from 'k6';
import { Counter, Rate, Trend } from 'k6/metrics';

// Custom Metrics
const reqLatency = new Trend('req_latency', true);
const cacheHitRate = new Rate('cache_hit_rate');
const errorRate = new Rate('error_rate');

// Sample search terms representative of real BitTorrent portal queries
const SEARCH_TERMS = [
  'ubuntu', 'linux', '1080p', '2024', 'remux',
  'flac', 'lossless', 's01', 'x265', 'hevc',
  'season', '720p', 'complete', 'soundtrack', 'iso'
];

const CATEGORIES = [
  'Movies', 'Television', 'Anime', 'Games',
  'Music', 'Applications', 'Books & Learning', 'Documentaries'
];

export const options = {
  insecureSkipTLSVerify: true,
  scenarios: {
    high_throughput_burst: {
      executor: 'ramping-vus',
      startVUs: 10,
      stages: [
        { duration: '10s', target: 50 },   // Fast ramp to 50 VUs
        { duration: '20s', target: 100 },  // Push to 100 VUs
        { duration: '20s', target: 200 },  // Peak load: 200 VUs
        { duration: '15s', target: 250 },  // Saturation: 250 VUs
        { duration: '10s', target: 0 },    // Cooldown
      ],
      gracefulRampDown: '5s',
    },
  },
  thresholds: {
    'http_req_failed': ['rate<0.01'],             // Less than 1% failures
    'http_req_duration': ['p(95)<300', 'p(99)<800'],
  },
};

const BASE_URL = __ENV.TARGET_URL || 'https://gaia-gateway';

export default function () {
  const rand = Math.random();
  let res;

  if (rand < 0.20) {
    // 20% Homepage / UI Shell
    res = http.get(`${BASE_URL}/`, { tags: { name: 'home_shell' } });
  } else if (rand < 0.45) {
    // 25% Stats polling
    res = http.get(`${BASE_URL}/api/stats`, { tags: { name: 'api_stats' } });
  } else if (rand < 0.75) {
    // 30% Torrents Search (realistic search distribution with cache hits)
    const term = SEARCH_TERMS[Math.floor(Math.random() * SEARCH_TERMS.length)];
    const url = `${BASE_URL}/api/torrents?search=${encodeURIComponent(term)}&limit=25`;
    res = http.get(url, { tags: { name: 'api_search' } });
  } else {
    // 25% Category / Browse Catalog
    const cat = CATEGORIES[Math.floor(Math.random() * CATEGORIES.length)];
    const url = `${BASE_URL}/api/torrents?category=${encodeURIComponent(cat)}&page=1&limit=25`;
    res = http.get(url, { tags: { name: 'api_browse' } });
  }

  const ok = check(res, {
    'status is 200': (r) => r.status === 200,
  });

  const cacheHeader = res.headers['X-Cache-Status'];
  if (cacheHeader) {
    cacheHitRate.add(cacheHeader === 'HIT');
  }

  reqLatency.add(res.timings.duration);
  errorRate.add(!ok);

  // High-throughput simulation: 5ms - 25ms jitter between requests
  sleep(Math.random() * 0.02 + 0.005);
}
