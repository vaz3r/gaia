import { useState, useEffect, useCallback } from "react";
import {
  searchTorrents,
  randomTorrents,
  classifyTorrent,
  getStats,
  type Torrent,
  type Prediction,
  type Stats,
} from "./api";
import { SearchBar } from "./components/SearchBar";
import { TorrentCard } from "./components/TorrentCard";
import { PredictionResult } from "./components/PredictionResult";

export default function App() {
  const [searchQuery, setSearchQuery] = useState("");
  const [torrents, setTorrents] = useState<Torrent[]>([]);
  const [loading, setLoading] = useState(false);
  const [stats, setStats] = useState<Stats | null>(null);

  // Classification state
  const [classifying, setClassifying] = useState<string | null>(null);
  const [prediction, setPrediction] = useState<{
    torrent: Torrent;
    result: Prediction;
  } | null>(null);

  // Load initial random torrents
  const loadRandom = useCallback(async () => {
    setLoading(true);
    try {
      const res = await randomTorrents(12);
      setTorrents(res.data);
    } catch (err) {
      console.error("Failed to load random torrents:", err);
    } finally {
      setLoading(false);
    }
  }, []);

  // Load stats
  const loadStats = useCallback(async () => {
    try {
      const res = await getStats();
      setStats(res);
    } catch (err) {
      console.error("Failed to load stats:", err);
    }
  }, []);

  useEffect(() => {
    loadRandom();
    loadStats();
  }, [loadRandom, loadStats]);

  // Search handler
  const handleSearch = useCallback(async () => {
    if (!searchQuery.trim()) {
      loadRandom();
      return;
    }
    setLoading(true);
    try {
      const res = await searchTorrents(searchQuery);
      setTorrents(res.data);
    } catch (err) {
      console.error("Search failed:", err);
    } finally {
      setLoading(false);
    }
  }, [searchQuery, loadRandom]);

  // Classify handler
  const handleClassify = useCallback(
    async (torrent: Torrent) => {
      setClassifying(torrent.infohash);
      setPrediction(null);
      try {
        const result = await classifyTorrent(torrent as unknown as Record<string, unknown>);
        setPrediction({ torrent, result });
      } catch (err) {
        console.error("Classification failed:", err);
        alert(`Classification failed: ${err}`);
      } finally {
        setClassifying(null);
      }
    },
    []
  );

  return (
    <div className="min-h-screen bg-gray-50">
      {/* Header */}
      <header className="border-b border-gray-200 bg-white">
        <div className="mx-auto max-w-6xl px-4 py-4 sm:px-6">
          <div className="flex items-center justify-between">
            <div>
              <h1 className="text-xl font-bold text-gray-900">
                Torrent Classifier
              </h1>
              <p className="text-sm text-gray-500">
                MLP + TF-IDF — 90.7% accuracy, 9 categories
              </p>
            </div>
            {stats && (
              <div className="text-right text-sm">
                <div className="font-medium text-gray-900">
                  {stats.totalTorrents.toLocaleString()} torrents
                </div>
                <div className="text-gray-500">
                  {stats.totalLabeled.toLocaleString()} labeled
                </div>
              </div>
            )}
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-6xl px-4 py-6 sm:px-6">
        <div className="grid grid-cols-1 gap-6 lg:grid-cols-5">
          {/* Left: Search + Torrent list */}
          <div className="lg:col-span-3 space-y-4">
            <SearchBar
              value={searchQuery}
              onChange={setSearchQuery}
              onSearch={handleSearch}
              loading={loading}
            />

            <div className="flex items-center justify-between">
              <h2 className="text-sm font-medium text-gray-700">
                {searchQuery
                  ? `Search results (${torrents.length})`
                  : "Random torrents"}
              </h2>
              <button
                onClick={loadRandom}
                disabled={loading}
                className="text-xs text-blue-600 hover:text-blue-800 disabled:opacity-50"
              >
                Refresh
              </button>
            </div>

            {loading ? (
              <div className="py-12 text-center text-sm text-gray-400">
                Loading...
              </div>
            ) : torrents.length === 0 ? (
              <div className="py-12 text-center text-sm text-gray-400">
                No torrents found
              </div>
            ) : (
              <div className="space-y-2">
                {torrents.map((t) => (
                  <TorrentCard
                    key={t.infohash}
                    torrent={t}
                    onClassify={handleClassify}
                    classifying={classifying === t.infohash}
                  />
                ))}
              </div>
            )}
          </div>

          {/* Right: Prediction result + Stats */}
          <div className="lg:col-span-2 space-y-4">
            {prediction ? (
              <PredictionResult
                prediction={prediction.result}
                torrentName={prediction.torrent.name}
              />
            ) : (
              <div className="rounded-lg border border-gray-200 bg-white p-6 text-center text-sm text-gray-400">
                Click "Classify" on a torrent to see the prediction
              </div>
            )}

            {/* Category distribution */}
            {stats && stats.categoryDistribution.length > 0 && (
              <div className="rounded-lg border border-gray-200 bg-white p-4">
                <h3 className="mb-3 text-sm font-medium text-gray-700">
                  Category Distribution
                </h3>
                <div className="space-y-2">
                  {stats.categoryDistribution.map((item) => {
                    const maxCount = Math.max(
                      ...stats.categoryDistribution.map((c) => c.count)
                    );
                    return (
                      <div
                        key={item.category}
                        className="flex items-center gap-2 text-xs"
                      >
                        <span className="w-28 shrink-0 text-right text-gray-600">
                          {item.category}
                        </span>
                        <div className="relative h-4 flex-1 overflow-hidden rounded bg-gray-100">
                          <div
                            className="absolute left-0 top-0 h-full rounded bg-blue-500"
                            style={{
                              width: `${
                                maxCount > 0
                                  ? (item.count / maxCount) * 100
                                  : 0
                              }%`,
                            }}
                          />
                        </div>
                        <span className="w-10 text-right font-mono text-[10px] text-gray-500">
                          {item.count}
                        </span>
                      </div>
                    );
                  })}
                </div>
              </div>
            )}

            {/* Model info */}
            <div className="rounded-lg border border-gray-200 bg-white p-4">
              <h3 className="mb-2 text-sm font-medium text-gray-700">
                Model Info
              </h3>
              <dl className="space-y-1 text-xs text-gray-600">
                <div className="flex justify-between">
                  <dt>Classifier</dt>
                  <dd className="font-medium">MLP (sklearn)</dd>
                </div>
                <div className="flex justify-between">
                  <dt>Features</dt>
                  <dd className="font-medium">
                    TF-IDF (word + char) + 15 numeric
                  </dd>
                </div>
                <div className="flex justify-between">
                  <dt>Training data</dt>
                  <dd className="font-medium">7,477 human-labeled</dd>
                </div>
                <div className="flex justify-between">
                  <dt>Accuracy</dt>
                  <dd className="font-medium">90.7% (test set)</dd>
                </div>
              </dl>
            </div>
          </div>
        </div>
      </main>
    </div>
  );
}
