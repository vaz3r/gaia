import { useQuery } from "@tanstack/react-query";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

interface FailureRow {
  reason: string;
  count: string;
}

export function FailureBreakdown({ range }: { range: string }): JSX.Element {
  const { data, isLoading, error } = useQuery({
    queryKey: ["failures", range],
    queryFn: async () => {
      const res = await fetch(`/api/admin/monitor/failures?range=${range}`);
      if (!res.ok) throw new Error("failures failed");
      const body = (await res.json()) as { data: FailureRow[] };
      return body.data;
    },
  });

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm font-medium">Fetch failures ({range})</CardTitle>
      </CardHeader>
      <CardContent>
        {isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
        {error && <p className="text-sm text-destructive">{(error as Error).message}</p>}
        {data && data.length === 0 && (
          <p className="text-sm text-muted-foreground">No failures in range.</p>
        )}
        {data && data.length > 0 && (
          <div className="space-y-1">
            {data.slice(0, 12).map((f) => (
              <div key={f.reason} className="flex items-center justify-between text-sm">
                <span className="truncate pr-2">{f.reason}</span>
                <span className="tabular-nums text-muted-foreground">{f.count}</span>
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
