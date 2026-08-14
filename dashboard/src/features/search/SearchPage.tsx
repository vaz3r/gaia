import { useEffect, useMemo, useRef } from "react";

import { api } from "@/lib/api";
import { useSearchStore } from "@/stores/search.store";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

const DEBOUNCE_MS = 300;

function formatSize(bytes: number | null): string {
  if (bytes === null) return "-";
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GiB`;
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(1)} MiB`;
  return `${bytes} B`;
}

export function SearchPage(): JSX.Element {
  const { params, result, loading, error, setParams, setResult, setLoading, setError } =
    useSearchStore();
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const executeSearch = useMemo(
    () => () => {
      if (!params.q.trim()) {
        setResult(null);
        return;
      }
      setLoading(true);
      api
        .search(params)
        .then(setResult)
        .catch((e: Error) => setError(e.message));
    },
    [params, setResult, setLoading, setError],
  );

  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(executeSearch, DEBOUNCE_MS);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [executeSearch]);

  return (
    <div className="space-y-4">
      <div className="flex items-end gap-3">
        <div className="flex-1">
          <Input
            value={params.q}
            placeholder="Search torrent names…"
            onChange={(e) => setParams({ q: e.target.value })}
            autoFocus
          />
        </div>
        <Input
          className="w-32"
          type="number"
          placeholder="min size (B)"
          value={params.sizeMin ?? ""}
          onChange={(e) =>
            setParams({
              ...(e.target.value ? { sizeMin: Number(e.target.value) } : {}),
            })
          }
        />
        <Input
          className="w-32"
          type="number"
          placeholder="max size (B)"
          value={params.sizeMax ?? ""}
          onChange={(e) =>
            setParams({
              ...(e.target.value ? { sizeMax: Number(e.target.value) } : {}),
            })
          }
        />
        <Select
          value={params.sort}
          onValueChange={(v) =>
            setParams({ sort: v as "relevance" | "newest" | "largest" | "name" })
          }
        >
          <SelectTrigger className="w-36">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="relevance">Relevance</SelectItem>
            <SelectItem value="newest">Newest</SelectItem>
            <SelectItem value="largest">Largest</SelectItem>
            <SelectItem value="name">Name</SelectItem>
          </SelectContent>
        </Select>
        <Select
          value={params.order}
          onValueChange={(v) => setParams({ order: v as "asc" | "desc" })}
        >
          <SelectTrigger className="w-28">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="desc">Descending</SelectItem>
            <SelectItem value="asc">Ascending</SelectItem>
          </SelectContent>
        </Select>
        <Button
          variant="outline"
          onClick={() => {
            setParams({ from: 0 });
            executeSearch();
          }}
        >
          Search
        </Button>
      </div>

      {error && (
        <p className="text-sm text-destructive" role="alert">
          {error}
        </p>
      )}

      <Card>
        <CardHeader>
          <CardTitle>
            {loading ? "Searching…" : result ? `${result.total} results for “${result.query}”` : "Search"}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          {result && result.data.length > 0 ? (
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b text-left text-muted-foreground">
                  <th className="py-2">Name</th>
                  <th className="py-2 text-right">Size</th>
                  <th className="py-2 text-right">Files</th>
                  <th className="py-2 text-right">Similarity</th>
                </tr>
              </thead>
              <tbody>
                {result.data.map((hit) => (
                  <tr key={hit.info_hash} className="border-b last:border-0">
                    <td className="py-2 pr-4">{hit.name}</td>
                    <td className="py-2 text-right">{formatSize(hit.size_bytes)}</td>
                    <td className="py-2 text-right">{hit.file_count ?? "-"}</td>
                    <td className="py-2 text-right">
                      {hit.similarity !== null ? hit.similarity.toFixed(3) : "-"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            !loading && result && <p className="text-muted-foreground">No results.</p>
          )}

          <div className="flex items-center justify-between pt-2">
            <Button
              variant="outline"
              size="sm"
              disabled={params.from <= 0 || loading}
              onClick={() => setParams({ from: Math.max(0, params.from - params.limit) })}
            >
              Previous
            </Button>
            <span className="text-xs text-muted-foreground">
              {result ? `showing ${result.from + 1}–${Math.min(result.from + params.limit, result.total)} of ${result.total}` : ""}
            </span>
            <Button
              variant="outline"
              size="sm"
              disabled={!result || result.from + params.limit >= result.total || loading}
              onClick={() => setParams({ from: params.from + params.limit })}
            >
              Next
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
