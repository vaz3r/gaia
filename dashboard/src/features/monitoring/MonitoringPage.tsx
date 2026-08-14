import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";

import { api, type CrawlSnapshot, type RangeKey } from "@/lib/api";
import { useMonitorStore } from "@/stores/monitor.store";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { MetricCard } from "./MetricCard";
import { RateChart, useRateQuery } from "./RateChart";
import { FailureBreakdown } from "./FailureBreakdown";

const RANGES: RangeKey[] = ["5m", "30m", "1h", "6h", "24h", "7d"];
const SYSTEM_KINDS = ["network", "memory", "cpu", "disk", "loadavg"] as const;

function formatBytes(b: number | null): string {
  if (b === null) return "-";
  if (b >= 1024 ** 3) return `${(b / 1024 ** 3).toFixed(1)} GiB`;
  if (b >= 1024 ** 2) return `${(b / 1024 ** 2).toFixed(1)} MiB`;
  return `${b} B`;
}

function formatRate(bps: number | null): string {
  if (bps === null) return "-";
  if (bps >= 1_000_000) return `${(bps / 1_000_000).toFixed(2)} Mbit/s`;
  if (bps >= 1_000) return `${(bps / 1_000).toFixed(1)} Kbit/s`;
  return `${bps.toFixed(0)} bit/s`;
}

export function MonitoringPage(): JSX.Element {
  const { range, systemKind, setRange, setSystemKind } = useMonitorStore();
  const [live, setLive] = useState<CrawlSnapshot | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    const load = (): void => {
      api.latest().then((r) => {
        if (r) setLive(r);
      });
    };
    load();
    timerRef.current = setInterval(load, 5_000);
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, []);

  const verified = useRateQuery("metadata_verified", range);
  const unique = useRateQuery("hashes_unique", range);
  const routing = useRateQuery("routing_nodes", range);
  const jemalloc = useRateQuery("jemalloc_allocated", range);
  const system = useQuery({
    queryKey: ["system", systemKind, range],
    queryFn: () => api.system(systemKind, range),
    refetchInterval: 30_000,
  });

  const systemData: Record<string, unknown>[] =
    system.data?.data.map((r) => r as Record<string, unknown>) ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-semibold">Monitoring</h1>
        <Select value={range} onValueChange={(v) => setRange(v as RangeKey)}>
          <SelectTrigger className="w-28">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {RANGES.map((r) => (
              <SelectItem key={r} value={r}>
                {r}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="grid grid-cols-2 gap-4 md:grid-cols-4">
        <MetricCard
          label="Verified (per hr)"
          value={live ? Math.round((live.metadata_verified ?? 0) / Math.max(1, (Date.now() - Date.parse(live.ts)) / 3.6e6)).toString() : "-"}
          sub={live ? `total ${live.metadata_verified}` : undefined}
          accent
        />
        <MetricCard
          label="Unique hashes"
          value={live ? live.hashes_unique.toLocaleString() : "-"}
          sub={live ? `sampled ${live.hashes_sampled.toLocaleString()}` : undefined}
        />
        <MetricCard
          label="Routing nodes"
          value={live ? String(live.routing_nodes) : "-"}
          sub={live ? `lookups recv ${live.lookups_received.toLocaleString()}` : undefined}
        />
        <MetricCard
          label="Container RAM"
          value={live ? formatBytes(live.container_mem_current) : "-"}
          sub={live ? `host free ${formatBytes(live.host_mem_available)}` : undefined}
        />
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <RateChart title="Verified / hr" data={verified.data ?? []} />
        <RateChart title="Unique hashes / hr" data={unique.data ?? []} color="#059669" />
        <RateChart title="Routing nodes" data={routing.data ?? []} color="#7c3aed" />
        <RateChart title="jemalloc allocated (MB)" data={jemalloc.data ?? []} color="#ea580c" />
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-medium">System: {systemKind}</h3>
              <Select value={systemKind} onValueChange={(v) => setSystemKind(v as (typeof SYSTEM_KINDS)[number])}>
                <SelectTrigger className="w-32">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {SYSTEM_KINDS.map((k) => (
                    <SelectItem key={k} value={k}>
                      {k}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </CardHeader>
          <CardContent>
            <p className="text-2xl font-semibold">
              {systemData.length > 0
                ? summarizeSystem(systemKind, systemData[systemData.length - 1]!)
                : "-"}
            </p>
          </CardContent>
        </Card>
        <FailureBreakdown range={range} />
      </div>

      {live && (
        <Card>
          <CardHeader>
            <h3 className="text-sm font-medium">Snapshot @ {new Date(live.ts).toLocaleTimeString()}</h3>
          </CardHeader>
          <CardContent>
            <p className="text-sm text-muted-foreground">
              Network {formatBytes(live.net_rx_bytes)} rx / {formatBytes(live.net_tx_bytes)} tx ·
              rate {formatRate(live.net_rx_rate_bps)} rx · CPU {live.cpu_percent?.toFixed(1)}% ·
              load {live.loadavg_1} · disk free {formatBytes(live.disk_free_bytes)} ·
              fetch pool {live.fetch_in_flight} in-flight
            </p>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

function summarizeSystem(kind: string, last: Record<string, unknown>): string {
  switch (kind) {
    case "network":
      return `rx ${formatRate(last.rx_rate as number | null)} · tx ${formatRate(last.tx_rate as number | null)}`;
    case "memory":
      return formatBytes(last.container_current as number | null);
    case "cpu":
      return `${(last.cpu_percent as number | null)?.toFixed(1) ?? "-"}%`;
    case "disk":
      return `free ${formatBytes(last.free as number | null)}`;
    case "loadavg":
      return `1m ${last.load1} · 5m ${last.load5} · 15m ${last.load15}`;
    default:
      return "-";
  }
}
