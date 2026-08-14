import { create } from "zustand";

import type { SearchParams, SearchResult } from "@/lib/api";

export interface SearchState {
  params: SearchParams;
  result: SearchResult | null;
  loading: boolean;
  error: string | null;
  setParams: (patch: Partial<SearchParams>) => void;
  setResult: (result: SearchResult | null) => void;
  setLoading: (loading: boolean) => void;
  setError: (error: string | null) => void;
}

export const useSearchStore = create<SearchState>((set) => ({
  params: { q: "", limit: 50, sort: "relevance", order: "desc", from: 0 },
  result: null,
  loading: false,
  error: null,
  setParams: (patch) =>
    set((s) => ({
      params: { ...s.params, ...patch, from: patch.from ?? 0 },
    })),
  setResult: (result) => set({ result, loading: false, error: null }),
  setLoading: (loading) => set({ loading }),
  setError: (error) => set({ error, loading: false }),
}));
