import { create } from 'zustand';
import { api, downloadTorrent, magnetFrom } from '../api.js';
import { formatBytes } from '../utils.js';

let abortController = null;
let searchDebounceTimeout = null;

export const useBrowserStore = create((set, get) => ({
  // Pagination & Filtering
  page: 1,
  limit: 25,
  sortField: 'verified_at',
  sortOrder: 'desc',
  searchInput: '',
  searchQuery: '',
  categoryFilter: '',
  riskFilter: '',

  // Results & Loading
  torrentsData: { data: [], total: 0, pages: 1, page: 1 },
  loading: false,

  // Selected Torrent Inspector
  selectedTorrent: null,
  detailLoading: false,
  refreshingHealth: false,
  downloadingIh: null,
  modalOverriding: false,
  modalRelabeling: false,
  modalRelabelMsg: null,

  // Action: Fetch paginated torrents with cancellation
  fetchTorrents: async () => {
    if (abortController) {
      abortController.abort();
    }
    abortController = new AbortController();

    const { page, limit, sortField, sortOrder, searchQuery, categoryFilter, riskFilter } = get();
    set({ loading: true });

    const params = new URLSearchParams({
      page: String(page),
      limit: String(limit),
    });
    if (sortField && sortField !== 'relevance') {
      params.set('sort', sortField);
      params.set('order', sortOrder);
    }
    if (searchQuery) params.set('search', searchQuery);
    if (categoryFilter) params.set('category', categoryFilter);
    if (riskFilter) params.set('risk', riskFilter);

    try {
      const res = await api(`/api/dashboard/torrents?${params.toString()}`, {
        signal: abortController.signal,
      });
      set({ torrentsData: res, loading: false });
    } catch (err) {
      if (err.name !== 'AbortError') {
        console.error('Failed to load torrents:', err);
        set({ loading: false });
      }
    }
  },

  // Action: Handle search input with 350ms debounce
  handleSearchChange: (val) => {
    set({ searchInput: val });
    clearTimeout(searchDebounceTimeout);
    searchDebounceTimeout = setTimeout(() => {
      const trimmed = val.trim();
      const currentSort = get().sortField;
      let nextSort = currentSort;
      if (trimmed && currentSort === 'verified_at') {
        nextSort = 'relevance';
      } else if (!trimmed && currentSort === 'relevance') {
        nextSort = 'verified_at';
      }
      set({ searchQuery: trimmed, sortField: nextSort, page: 1 });
      get().fetchTorrents();
    }, 350);
  },

  clearSearch: () => {
    const currentSort = get().sortField;
    const nextSort = currentSort === 'relevance' ? 'verified_at' : currentSort;
    set({ searchInput: '', searchQuery: '', sortField: nextSort, page: 1 });
    get().fetchTorrents();
  },

  setCategoryFilter: (category) => {
    set({ categoryFilter: category, page: 1 });
    get().fetchTorrents();
  },

  setRiskFilter: (risk) => {
    set({ riskFilter: risk, page: 1 });
    get().fetchTorrents();
  },

  setSorting: (field, order) => {
    set({ sortField: field, sortOrder: order, page: 1 });
    get().fetchTorrents();
  },

  setPage: (page) => {
    set({ page });
    get().fetchTorrents();
  },

  setLimit: (limit) => {
    set({ limit, page: 1 });
    get().fetchTorrents();
  },

  setSelectedTorrent: (torrent) => {
    if (!torrent) {
      set({ selectedTorrent: null, modalRelabelMsg: null });
      return;
    }
    const safeFiles = Array.isArray(torrent.files) ? torrent.files : [];
    set({ selectedTorrent: { ...torrent, files: safeFiles }, modalRelabelMsg: null });
  },

  setDetailLoading: (detailLoading) => set({ detailLoading }),

  inspectTorrent: async (torrent) => {
    if (!torrent) {
      set({ selectedTorrent: null, modalRelabelMsg: null });
      return;
    }
    const hash = torrent.infohash || torrent.hash;
    set({
      selectedTorrent: { ...torrent, hash, files: [] },
      detailLoading: Boolean(hash),
      modalRelabelMsg: null,
    });
    if (hash) {
      try {
        const full = await api(`/api/torrents/${hash}`);
        if (full) {
          let normalizedFiles = [];
          if (Array.isArray(full.files)) {
            normalizedFiles = full.files;
          } else if (typeof full.files === 'string') {
            try {
              const parsed = JSON.parse(full.files);
              normalizedFiles = Array.isArray(parsed) ? parsed : Object.values(parsed);
            } catch {}
          } else if (full.files && typeof full.files === 'object') {
            normalizedFiles = Object.values(full.files);
          }

          set((state) => ({
            selectedTorrent: {
              ...state.selectedTorrent,
              ...full,
              hash,
              pieceLength: formatBytes(full.piece_length),
              pieceCount: full.file_count || normalizedFiles.length || 1,
              files: normalizedFiles,
            },
          }));
        }
      } catch (err) {
        console.warn('Failed to load torrent details:', err);
      } finally {
        set({ detailLoading: false });
      }
    }
  },

  // Action: Refresh health metadata
  refreshTorrentHealth: async (infohash) => {
    if (!infohash || get().refreshingHealth) return;
    set({ refreshingHealth: true });
    try {
      const res = await api(`/api/torrents/${infohash}/refresh-health`, { method: 'POST' });
      if (res && res.infohash) {
        set((state) => ({
          selectedTorrent: state.selectedTorrent?.infohash === res.infohash
            ? { ...state.selectedTorrent, ...res }
            : state.selectedTorrent,
          torrentsData: {
            ...state.torrentsData,
            data: state.torrentsData.data.map((t) => (t.infohash === res.infohash ? { ...t, ...res } : t)),
          },
        }));
      }
    } catch (err) {
      console.error('Failed to refresh torrent health:', err);
    } finally {
      set({ refreshingHealth: false });
    }
  },

  // Action: Score override
  scoreOverride: async (infohash, action) => {
    if (!infohash || get().modalOverriding) return;
    set({ modalOverriding: true });
    try {
      const res = await api('/api/scoring/override', {
        method: 'POST',
        body: JSON.stringify({ infohash, action, notes: 'Inspector modal manual triage' }),
      });
      if (res?.success) {
        const patch = {
          risk_tier: action === 'ALLOW' ? 'SAFE' : action === 'SUPPRESS' ? 'BLOCKED' : 'REVIEW',
          policy_action: action,
          decision_source: 'MANUAL',
        };
        set((state) => ({
          selectedTorrent: state.selectedTorrent?.infohash === infohash
            ? { ...state.selectedTorrent, ...patch }
            : state.selectedTorrent,
          torrentsData: {
            ...state.torrentsData,
            data: state.torrentsData.data.map((t) => (t.infohash === infohash ? { ...t, ...patch } : t)),
          },
        }));
      }
    } catch (err) {
      alert(`Override failed: ${err.message}`);
    } finally {
      set({ modalOverriding: false });
    }
  },

  // Action: Save category ground truth
  saveCategoryLabel: async (infohash, category) => {
    if (!infohash || !category || get().modalRelabeling) return;
    set({ modalRelabeling: true, modalRelabelMsg: null });
    try {
      const res = await api('/api/classifier/labels', {
        method: 'POST',
        body: JSON.stringify({
          infohash,
          category,
          reason: 'Manual category relabel via Inspector modal',
        }),
      });

      const patch = {
        category,
        category_confidence: 1.0,
        needs_review: false,
      };

      set((state) => ({
        selectedTorrent: state.selectedTorrent?.infohash === infohash
          ? { ...state.selectedTorrent, ...patch }
          : state.selectedTorrent,
        torrentsData: {
          ...state.torrentsData,
          data: state.torrentsData.data.map((t) => (t.infohash === infohash ? { ...t, ...patch } : t)),
        },
        modalRelabelMsg: { type: 'success', text: `Saved "${category}" as ground truth!` },
      }));

      setTimeout(() => set({ modalRelabelMsg: null }), 3000);
    } catch (err) {
      set({ modalRelabelMsg: { type: 'error', text: err.message } });
    } finally {
      set({ modalRelabeling: false });
    }
  },

  // Action: Download torrent file
  downloadTorrent: async (torrent, fallbackMagnet) => {
    const hash = torrent?.infohash || torrent?.hash;
    if (!hash || get().downloadingIh) return;
    set({ downloadingIh: hash });
    try {
      await downloadTorrent(hash, torrent.name);
    } catch (err) {
      console.warn('Direct .torrent assembly fallback:', err.message);
      const mag = err.magnet || fallbackMagnet || magnetFrom(hash, torrent.name);
      if (typeof navigator !== 'undefined' && navigator.clipboard && mag) {
        navigator.clipboard.writeText(mag);
        alert('Active seeders busy for direct assembly. Copied Magnet link to clipboard!');
      }
    } finally {
      set({ downloadingIh: null });
    }
  },
}));
