import { create } from "zustand";

import type { RangeKey, SystemKind } from "@/lib/api";

export interface MonitorState {
  range: RangeKey;
  systemKind: SystemKind;
  setRange: (range: RangeKey) => void;
  setSystemKind: (kind: SystemKind) => void;
}

export const useMonitorStore = create<MonitorState>((set) => ({
  range: "1h",
  systemKind: "network",
  setRange: (range) => set({ range }),
  setSystemKind: (systemKind) => set({ systemKind }),
}));
