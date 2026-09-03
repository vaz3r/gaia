import type { Prediction } from "../api";

const CATEGORY_COLORS: Record<string, string> = {
  Adult: "bg-red-500",
  Anime: "bg-pink-500",
  Applications: "bg-blue-500",
  Documentaries: "bg-teal-500",
  Games: "bg-purple-500",
  Movies: "bg-amber-500",
  Music: "bg-green-500",
  Other: "bg-gray-500",
  Television: "bg-indigo-500",
};

interface PredictionResultProps {
  prediction: Prediction;
  torrentName: string;
}

export function PredictionResult({ prediction, torrentName }: PredictionResultProps) {
  const maxProb = Math.max(...Object.values(prediction.probabilities));

  return (
    <div className="rounded-lg border border-blue-200 bg-blue-50 p-4">
      <div className="mb-3 flex items-center gap-2">
        <span
          className={`inline-block h-3 w-3 rounded-full ${CATEGORY_COLORS[prediction.label] || "bg-gray-400"}`}
        />
        <span className="text-sm font-semibold text-gray-900">
          {prediction.label}
        </span>
        <span className="rounded bg-blue-100 px-2 py-0.5 text-xs font-medium text-blue-700">
          {(prediction.confidence * 100).toFixed(1)}%
        </span>
      </div>

      <p className="mb-3 truncate text-xs text-gray-500" title={torrentName}>
        {torrentName}
      </p>

      <div className="space-y-1.5">
        {Object.entries(prediction.probabilities).map(([cat, prob]) => (
          <div key={cat} className="flex items-center gap-2 text-xs">
            <span className="w-24 shrink-0 text-right text-gray-600">{cat}</span>
            <div className="relative h-4 flex-1 overflow-hidden rounded bg-gray-200">
              <div
                className={`absolute left-0 top-0 h-full rounded ${CATEGORY_COLORS[cat] || "bg-gray-400"}`}
                style={{
                  width: `${maxProb > 0 ? (prob / maxProb) * 100 : 0}%`,
                  opacity: prob === maxProb ? 1 : 0.6,
                }}
              />
            </div>
            <span className="w-12 text-right font-mono text-[10px] text-gray-500">
              {(prob * 100).toFixed(1)}%
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
