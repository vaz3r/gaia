import http from 'k6/http';
import { check, group, sleep } from 'k6';
import { Counter, Rate, Trend } from 'k6/metrics';

// Custom Metrics
const statsLatency = new Trend('portal_stats_latency', true);
const searchLatency = new Trend('portal_search_latency', true);
const categoryLatency = new Trend('portal_category_latency', true);
const detailLatency = new Trend('portal_detail_latency', true);
const homeLatency = new Trend('portal_home_latency', true);
const errorRate = new Rate('portal_error_rate');

// Sample search terms representative of real BitTorrent portal queries
const SEARCH_TERMS = [
  'ubuntu', 'linux', '1080p', '2024', 'remux',
  'flac', 'lossless', 's01', 'x265', 'hevc',
  'season', '720p', 'complete', 'soundtrack', 'iso'
];

// Canonical categories on GAIA portal
const CATEGORIES = [
  'Movies', 'Television', 'Anime', 'Games',
  'Music', 'Applications', 'Books & Learning', 'Documentaries'
];

// Pre-seeded list of sample infohashes from real catalog
const SAMPLE_INFOHASHES = [
  '5753ddc7834817e283903861dba64a83a8fb1a4b',
  '7c9ce3404b6098f1e516767b881d3adc69240597',
  '29267fc26410ce9cf5a3c2602fb4daeef04ad7d7',
  '8d3d92415d86cb5aa1a18274a1cb27f8a7e44a9e',
  '8a8ff7ef990b79ba4dbdf7b147326e5da5895781'
];

export const options = {
  insecureSkipTLSVerify: true,
  scenarios: {
    // Stepped Ramp-up Capacity Test
    portal_capacity_test: {
      executor: 'ramping-vus',
      startVUs: 2,
      stages: [
        { duration: '15s', target: 10 },  // Warmup stage
        { duration: '30s', target: 25 },  // Moderate user concurrency
        { duration: '30s', target: 50 },  // Peak normal concurrency
        { duration: '30s', target: 100 }, // High load concurrency
        { duration: '30s', target: 150 }, // Stress saturation test
        { duration: '15s', target: 0 },   // Graceful cooldown
      ],
      gracefulRampDown: '10s',
    },
  },
  thresholds: {
    'http_req_failed': ['rate<0.02'],              // Error rate under 2%
    'http_req_duration': ['p(95)<500', 'p(99)<1000'], // 95% under 500ms, 99% under 1s
    'portal_stats_latency': ['p(95)<50'],          // Microcached stats under 50ms
    'portal_search_latency': ['p(95)<600'],        // Meilisearch search under 600ms
  },
};

const BASE_URL = __ENV.TARGET_URL || 'https://gaia-gateway';

export function setup() {
  const res = http.get(`${BASE_URL}/api/torrents?page=1&limit=50`, {
    insecureSkipTLSVerify: true,
  });
  if (res.status === 200) {
    try {
      const body = JSON.parse(res.body);
      if (body && Array.isArray(body.data)) {
        const hashes = body.data.map((t) => t.infohash).filter(Boolean);
        if (hashes.length > 0) {
          return { hashes };
        }
      }
    } catch {}
  }
  return { hashes: SAMPLE_INFOHASHES };
}

export default function (setupData) {
  const hashes = (setupData && setupData.hashes && setupData.hashes.length > 0)
    ? setupData.hashes
    : SAMPLE_INFOHASHES;
  const rand = Math.random();

  // 1. Full-Text Search Journey (40% probability)
  if (rand < 0.40) {
    const term = SEARCH_TERMS[Math.floor(Math.random() * SEARCH_TERMS.length)];
    const page = Math.random() < 0.3 ? 2 : 1;
    const url = `${BASE_URL}/api/torrents?search=${encodeURIComponent(term)}&page=${page}&limit=25`;

    const res = http.get(url, { tags: { name: 'api_search' } });
    const success = check(res, {
      'search status is 200': (r) => r.status === 200,
      'search returns json': (r) => r.headers['Content-Type'] && r.headers['Content-Type'].includes('json'),
      'search has data array': (r) => {
        try {
          const body = JSON.parse(r.body);
          return Array.isArray(body.data);
        } catch {
          return false;
        }
      },
    });

    searchLatency.add(res.timings.duration);
    errorRate.add(!success);
    sleep(Math.random() * 1.0 + 0.5); // 0.5s - 1.5s think time
  }
  // 2. Homepage & Realtime Stats (30% probability)
  else if (rand < 0.70) {
    group('homepage_and_stats', function () {
      // Fetch HTML shell
      const homeRes = http.get(`${BASE_URL}/`, { tags: { name: 'portal_home' } });
      const homeOk = check(homeRes, {
        'home status is 200': (r) => r.status === 200,
      });
      homeLatency.add(homeRes.timings.duration);
      errorRate.add(!homeOk);

      // Fetch Stats
      const statsRes = http.get(`${BASE_URL}/api/stats`, { tags: { name: 'api_stats' } });
      const statsOk = check(statsRes, {
        'stats status is 200': (r) => r.status === 200,
        'stats has total_torrents': (r) => {
          try {
            return JSON.parse(r.body).total_torrents > 0;
          } catch {
            return false;
          }
        },
      });
      statsLatency.add(statsRes.timings.duration);
      errorRate.add(!statsOk);

      // Fetch Recent Listing
      const recentRes = http.get(`${BASE_URL}/api/torrents?page=1&limit=25&sort=verified_at&order=desc`, {
        tags: { name: 'api_recent_torrents' },
      });
      const recentOk = check(recentRes, {
        'recent status is 200': (r) => r.status === 200,
      });
      errorRate.add(!recentOk);
    });

    sleep(Math.random() * 1.5 + 1.0); // 1.0s - 2.5s think time
  }
  // 3. Category Exploration (20% probability)
  else if (rand < 0.90) {
    const cat = CATEGORIES[Math.floor(Math.random() * CATEGORIES.length)];
    const url = `${BASE_URL}/api/torrents?category=${encodeURIComponent(cat)}&page=1&limit=25`;

    const res = http.get(url, { tags: { name: 'api_category' } });
    const ok = check(res, {
      'category status is 200': (r) => r.status === 200,
    });

    categoryLatency.add(res.timings.duration);
    errorRate.add(!ok);
    sleep(Math.random() * 1.2 + 0.8); // 0.8s - 2.0s think time
  }
  // 4. Torrent Detail & File Tree Inspection (10% probability)
  else {
    const hash = hashes[Math.floor(Math.random() * hashes.length)];
    const url = `${BASE_URL}/api/torrents/${hash}`;

    const res = http.get(url, { tags: { name: 'api_detail' } });
    const ok = check(res, {
      'detail status is 200': (r) => r.status === 200,
    });

    detailLatency.add(res.timings.duration);
    errorRate.add(!ok);
    sleep(Math.random() * 2.0 + 1.0); // 1.0s - 3.0s think time
  }
}
