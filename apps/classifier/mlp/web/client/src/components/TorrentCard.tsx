import type { Torrent } from "../api";
import { formatBytes } from "../api";

interface TorrentCardProps {
  torrent: Torrent;
  onClassify: (torrent: Torrent) => void;
  classifying: boolean;
}

export function TorrentCard({ torrent, onClassify, classifying }: TorrentCardProps) {
  return (
    <div className="rounded-lg border border-gray-200 bg-white p-4 shadow-sm hover:shadow-md transition-shadow">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <h3
            className="truncate text-sm font-medium text-gray-900"
            title={torrent.name}
          >
            {torrent.name}
          </h3>
          <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-xs text-gray-500">
            <span>{torrent.file_count} files</span>
            <span>{formatBytes(torrent.total_size)}</span>
            {torrent.extensions && torrent.extensions.length > 0 && (
              <span className="text-gray-400">
                {torrent.extensions.slice(0, 3).join(", ")}
              </span>
            )}
          </div>
          {torrent.top_folders && torrent.top_folders.length > 0 && (
            <div className="mt-1.5 flex flex-wrap gap-1">
              {torrent.top_folders.slice(0, 3).map((folder) => (
                <span
                  key={folder}
                  className="rounded bg-gray-100 px-1.5 py-0.5 text-[10px] text-gray-600"
                >
                  {folder}
                </span>
              ))}
            </div>
          )}
        </div>
        <button
          onClick={() => onClassify(torrent)}
          disabled={classifying}
          className="shrink-0 rounded-md bg-emerald-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-600 disabled:opacity-50"
        >
          {classifying ? "..." : "Classify"}
        </button>
      </div>
      <div className="mt-2 font-mono text-[10px] text-gray-400">
        {torrent.infohash}
      </div>
    </div>
  );
}
