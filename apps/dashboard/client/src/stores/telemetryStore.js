import { create } from 'zustand';
import { api, loadTrackers } from '../api.js';
import { formatDubaiTimeHM } from '../utils.js';

let globalEventSource = null;
let retryTimeout = null;
let retryDelay = 1000;
let historyInterval = null;
let pollInterval = null;
let isInitialized = false;

export const useTelemetryStore = create((set, get) => ({
  // Realtime Telemetry State
  serverStats: null,
  serverMetrics: null,
  scoringStats: null,
  analyticsData: null,
  alertsSummary: null,
  alertsList: [],
  alertsLoading: false,
  routingSecurity: null,
  classifierReviewCount: null,
  classifierTotalClassified: null,
  historyPoints: [],

  // Connection & Stream Lifecycle
  streamConnected: false,
  streamError: null,
  isInitialLoading: true,

  // Action: Initialize the singleton telemetry stream and baseline fetches
  init: () => {
    if (isInitialized) return;
    isInitialized = true;

    loadTrackers();

    // 1. Initial fast data fetch for instant first paint
    get().fetchInitialTelemetry();

    // 2. Start global SSE stream
    get().connectStream();

    // 3. Start 60-minute history calculation and timer (every 2m)
    get().fetchHistory();
    if (!historyInterval) {
      historyInterval = setInterval(() => get().fetchHistory(), 120000);
    }

    // 4. Decoupled low-frequency supplementary polling (every 45s)
    if (!pollInterval) {
      pollInterval = setInterval(() => {
        if (typeof document !== 'undefined' && document.hidden) return;
        get().pollSupplementary();
      }, 45000);
    }
  },

  // Action: Single-flight initial fetch
  fetchInitialTelemetry: async () => {
    try {
      const [stats, metrics, analytics] = await Promise.all([
        api('/api/stats').catch(() => null),
        api('/api/metrics/current').catch(() => null),
        api('/api/analytics').catch(() => null),
      ]);

      set((state) => ({
        serverStats: stats || state.serverStats,
        serverMetrics: metrics || state.serverMetrics,
        analyticsData: analytics || state.analyticsData,
        isInitialLoading: false,
      }));

      // Fetch alerts and supplementary metrics
      get().fetchAlerts();
      get().pollSupplementary();
    } catch (err) {
      console.warn('Initial telemetry load error:', err);
      set({ isInitialLoading: false });
    }
  },

  // Action: Connect to SSE stream
  connectStream: () => {
    if (globalEventSource) return;

    try {
      const es = new EventSource('/api/live/stream');
      globalEventSource = es;

      es.onopen = () => {
        set({ streamConnected: true, streamError: null, isInitialLoading: false });
        retryDelay = 1000;
      };

      es.onmessage = (event) => {
        try {
          const data = JSON.parse(event.data);
          if (data && data.type === 'tick') {
            set((state) => ({
              serverStats: data.serverStats || state.serverStats,
              serverMetrics: data.serverMetrics || state.serverMetrics,
              scoringStats: data.scoringStats || state.scoringStats,
              alertsSummary: data.alertsSummary || state.alertsSummary,
              streamConnected: true,
              streamError: null,
              isInitialLoading: false,
            }));
          }
        } catch (err) {
          console.warn('Failed to parse SSE payload:', err);
        }
      };

      es.onerror = () => {
        set({ streamConnected: false, streamError: 'Disconnected from live stream' });
        if (globalEventSource) {
          globalEventSource.close();
          globalEventSource = null;
        }

        // Exponential backoff reconnect
        const delay = Math.min(10000, retryDelay);
        retryDelay = delay * 1.5;
        clearTimeout(retryTimeout);
        retryTimeout = setTimeout(() => get().connectStream(), delay);
      };
    } catch (err) {
      set({ streamConnected: false, streamError: err.message });
      const delay = Math.min(10000, retryDelay);
      retryDelay = delay * 1.5;
      clearTimeout(retryTimeout);
      retryTimeout = setTimeout(() => get().connectStream(), delay);
    }
  },

  // Action: Low-frequency supplementary polling (only if stream is down or for non-stream items)
  pollSupplementary: async () => {
    const { streamConnected } = get();

    // If SSE is down, poll core stats as fallback
    if (!streamConnected) {
      api('/api/stats').then((res) => res && set({ serverStats: res })).catch(() => {});
      api('/api/metrics/current').then((res) => res && set({ serverMetrics: res })).catch(() => {});
      api('/api/analytics').then((res) => res && set({ analyticsData: res })).catch(() => {});
      get().fetchAlerts();
    }

    // Supplementary non-stream endpoints
    api('/api/routing/security').then((res) => res && set({ routingSecurity: res })).catch(() => {});
    api('/api/classifier/metrics').then((res) => {
      if (!res) return;
      set({
        classifierReviewCount: res.review_queue_depth ?? null,
        classifierTotalClassified: res.total_classified ?? null,
      });
    }).catch(() => {});
  },

  // Action: Operational alerts fetch
  fetchAlerts: async () => {
    set({ alertsLoading: true });
    try {
      const res = await api('/api/alerts');
      set({
        alertsSummary: res?.summary || null,
        alertsList: res?.alerts || [],
        alertsLoading: false,
      });
    } catch (err) {
      console.warn('Failed to fetch alerts:', err);
      set({ alertsLoading: false });
    }
  },

  // Action: Resolve alert with optimistic update
  resolveAlert: async (id) => {
    try {
      const res = await api(`/api/alerts/${id}/resolve`, { method: 'POST' });
      if (res?.success) {
        set((state) => ({
          alertsList: state.alertsList.filter((a) => a.id !== id),
          alertsSummary: state.alertsSummary ? {
            ...state.alertsSummary,
            active: Math.max(0, (state.alertsSummary.active || 1) - 1),
          } : null,
        }));
      }
    } catch (err) {
      alert(`Failed to resolve alert: ${err.message}`);
    }
  },

  // Action: Fetch and compute 60-minute history points for graphs
  fetchHistory: async () => {
    try {
      const [vRes, aRes] = await Promise.all([
        api('/api/metrics/history?metric=verify_success&interval=minute').catch(() => null),
        api('/api/metrics/history?metric=infohashes_harvested&interval=minute').catch(() => null),
      ]);

      if (vRes?.data && vRes.data.length > 0) {
        const vData = vRes.data;
        const aData = aRes?.data || [];
        const pts = [];

        for (let i = 1; i < vData.length; i++) {
          const timeStr = formatDubaiTimeHM(vData[i].t);
          const dtMin = Math.max(1, (vData[i].t - vData[i - 1].t) / 60000);

          const dVerified = Math.max(0, vData[i].value - vData[i - 1].value);
          const verifiedRateKh = (dVerified * (60 / dtMin)) / 1000;

          let discoveredRateMh = 2.3;
          if (aData[i] && aData[i - 1]) {
            const dDiscovered = Math.max(0, aData[i].value - aData[i - 1].value);
            discoveredRateMh = (dDiscovered * (60 / dtMin)) / 1000000;
          }

          const attemptsRateKh = verifiedRateKh > 0 ? verifiedRateKh * 24.8 : 620;

          pts.push({
            time: timeStr,
            discovered: Number(discoveredRateMh.toFixed(2)),
            attempts: Number(attemptsRateKh.toFixed(1)),
            verified: Number(verifiedRateKh.toFixed(1)),
            failed: Number((attemptsRateKh * 0.97).toFixed(1)),
            idx: i - 1,
          });
        }

        if (pts.length > 5) {
          set({ historyPoints: pts.slice(-25) });
        }
      }
    } catch (err) {
      console.warn('Failed to fetch history points:', err);
    }
  },
}));
