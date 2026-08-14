import { useQuery } from "@tanstack/react-query";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

interface SeriesPoint {
  ts: string;
  value: number | null;
}

interface RateChartProps {
  title: string;
  data: SeriesPoint[];
  color?: string;
}

function formatAxis(value: string): string {
  const d = new Date(value);
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function RateChart({ title, data, color = "#2563eb" }: RateChartProps): JSX.Element {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm font-medium">{title}</CardTitle>
      </CardHeader>
      <CardContent>
        <ResponsiveContainer width="100%" height={180}>
          <AreaChart data={data} margin={{ top: 4, right: 4, bottom: 0, left: 0 }}>
            <CartesianGrid strokeDasharray="3 3" opacity={0.2} />
            <XAxis
              dataKey="ts"
              tickFormatter={formatAxis}
              tick={{ fontSize: 11 }}
              minTickGap={40}
            />
            <YAxis tick={{ fontSize: 11 }} width={48} />
            <Tooltip
              labelFormatter={formatAxis}
              formatter={(v) => [String(v ?? "-"), "value"]}
            />
            <Area
              type="monotone"
              dataKey="value"
              stroke={color}
              fill={color}
              fillOpacity={0.15}
              isAnimationActive={false}
              dot={false}
            />
          </AreaChart>
        </ResponsiveContainer>
      </CardContent>
    </Card>
  );
}

export function useRateQuery(metric: string, range: string) {
  return useQuery({
    queryKey: ["rates", metric, range],
    queryFn: async () => {
      const res = await fetch(`/api/admin/monitor/rates?metric=${metric}&range=${range}`);
      if (!res.ok) throw new Error(`rates ${metric} failed`);
      const body = (await res.json()) as { data: SeriesPoint[] };
      return body.data;
    },
  });
}
