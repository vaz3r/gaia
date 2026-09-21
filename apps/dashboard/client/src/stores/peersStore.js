import { create } from 'zustand';
import { api } from '../api.js';

let abortController = null;
let searchDebounceTimeout = null;

export const usePeersStore = create((set, get) => ({
  page: 1,
  limit: 25,
  sortField: 'metadata_provided_count',
  sortOrder: 'desc',
  searchInput: '',
  searchQuery: '',

  peersData: { data: [], total: 0, pages: 1, page: 1, summary: {} },
  loading: false,

  selectedPeer: null,
  peerTorrentsList: [],
  peerTorrentsLoading: false,

  fetchPeers: async () => {
    if (abortController) {
      abortController.abort();
    }
    abortController = new AbortController();

    const { page, limit, sortField, sortOrder, searchQuery } = get();
    set({ loading: true });

    const params = new URLSearchParams({
      page: String(page),
      limit: String(limit),
      sort: sortField,
      order: sortOrder,
    });
    if (searchQuery) params.set('search', searchQuery);

    try {
      const res = await api(`/api/peers?${params.toString()}`, {
        signal: abortController.signal,
      });
      set({ peersData: res, loading: false });
    } catch (err) {
      if (err.name !== 'AbortError') {
        console.error('Failed to load stable peers:', err);
        set({ loading: false });
      }
    }
  },

  handleSearchChange: (val) => {
    set({ searchInput: val });
    clearTimeout(searchDebounceTimeout);
    searchDebounceTimeout = setTimeout(() => {
      set({ searchQuery: val.trim(), page: 1 });
      get().fetchPeers();
    }, 350);
  },

  clearSearch: () => {
    set({ searchInput: '', searchQuery: '', page: 1 });
    get().fetchPeers();
  },

  setSorting: (field, order) => {
    set({ sortField: field, sortOrder: order, page: 1 });
    get().fetchPeers();
  },

  setPage: (page) => {
    set({ page });
    get().fetchPeers();
  },

  setLimit: (limit) => {
    set({ limit, page: 1 });
    get().fetchPeers();
  },

  selectPeer: async (peer) => {
    if (!peer) {
      set({ selectedPeer: null, peerTorrentsList: [] });
      return;
    }
    set({ selectedPeer: peer, peerTorrentsLoading: true, peerTorrentsList: [] });
    try {
      const ip = peer.ip;
      const res = await api(`/api/peers/${encodeURIComponent(ip)}/torrents`);
      set({ peerTorrentsList: res?.torrents || res || [], peerTorrentsLoading: false });
    } catch (err) {
      console.warn('Failed to load peer torrents:', err);
      set({ peerTorrentsLoading: false });
    }
  },

  closePeerModal: () => {
    set({ selectedPeer: null, peerTorrentsList: [] });
  },
}));
