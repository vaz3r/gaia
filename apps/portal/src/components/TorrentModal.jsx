import React, { useState, useEffect } from 'react';
import { 
  X, 
  Magnet, 
  Download, 
  Copy, 
  Check, 
  FolderTree, 
  FileText, 
  ShieldCheck, 
  Layers, 
  Radio,
  ExternalLink 
} from 'lucide-react';

function formatBytes(bytes, decimals = 2) {
  if (!bytes || bytes === 0) return '0 B';
  const k = 1024;
  const dm = decimals < 0 ? 0 : decimals;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + ' ' + sizes[i];
}

export default function TorrentModal({ torrent, onClose }) {
  const [copiedIh, setCopiedIh] = useState(false);
  const [copiedMag, setCopiedMag] = useState(false);
  const [details, setDetails] = useState(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!torrent?.infohash) return;

    setLoading(true);
    fetch(`/api/torrents/${torrent.infohash}`)
      .then((r) => r.json())
      .then((data) => {
        setDetails(data);
        setLoading(false);
      })
      .catch((err) => {
        console.error('Failed to load torrent details:', err);
        setLoading(false);
      });
  }, [torrent?.infohash]);

  if (!torrent) return null;

  const handleCopyInfohash = () => {
    navigator.clipboard.writeText(torrent.infohash);
    setCopiedIh(true);
    setTimeout(() => setCopiedIh(false), 2000);
  };

  const handleCopyMagnet = async () => {
    try {
      const res = await fetch(`/api/torrents/${torrent.infohash}/magnet`);
      const data = await res.json();
      const magnetUrl = data.magnetUri || `magnet:?xt=urn:btih:${torrent.infohash}&dn=${encodeURIComponent(torrent.name)}`;
      await navigator.clipboard.writeText(magnetUrl);
      setCopiedMag(true);
      setTimeout(() => setCopiedMag(false), 2000);
    } catch {
      const fallback = `magnet:?xt=urn:btih:${torrent.infohash}&dn=${encodeURIComponent(torrent.name)}`;
      await navigator.clipboard.writeText(fallback);
      setCopiedMag(true);
      setTimeout(() => setCopiedMag(false), 2000);
    }
  };

  const handleDownload = () => {
    window.location.href = `/api/torrents/${torrent.infohash}/torrent`;
  };

  // Parse files json if present
  let filesList = [];
  if (details?.filesJson) {
    try {
      const parsed = typeof details.filesJson === 'string' ? JSON.parse(details.filesJson) : details.filesJson;
      if (Array.isArray(parsed)) {
        filesList = parsed;
      }
    } catch (e) {
      console.error('Could not parse filesJson:', e);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 sm:p-6 bg-black/80 backdrop-blur-md animate-in fade-in duration-150">
      <div 
        className="relative w-full max-w-3xl max-h-[90vh] bg-slate-900 border border-slate-800 rounded-3xl shadow-2xl overflow-hidden flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-start justify-between p-6 border-b border-slate-800/80 bg-slate-900/50">
          <div className="pr-4 min-w-0">
            <span className="inline-block px-2.5 py-0.5 rounded-md text-xs font-semibold bg-blue-500/10 text-blue-400 border border-blue-500/20 mb-2">
              {torrent.category || 'Other'}
            </span>
            <h2 className="text-xl sm:text-2xl font-bold text-slate-100 break-words leading-tight">
              {torrent.name}
            </h2>
          </div>
          <button
            onClick={onClose}
            className="p-2 text-slate-400 hover:text-white rounded-xl hover:bg-slate-800 transition flex-shrink-0"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Content */}
        <div className="p-6 overflow-y-auto space-y-6 flex-1">
          {/* Metadata Grid */}
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
            <div className="p-3.5 bg-slate-800/40 border border-slate-800 rounded-2xl">
              <span className="text-xs text-slate-400">Total Size</span>
              <p className="text-base sm:text-lg font-bold text-slate-200 mt-0.5">
                {formatBytes(torrent.total_size)}
              </p>
            </div>
            <div className="p-3.5 bg-slate-800/40 border border-slate-800 rounded-2xl">
              <span className="text-xs text-slate-400">Files Count</span>
              <p className="text-base sm:text-lg font-bold text-slate-200 mt-0.5">
                {torrent.file_count || 1}
              </p>
            </div>
            <div className="p-3.5 bg-slate-800/40 border border-slate-800 rounded-2xl">
              <span className="text-xs text-slate-400">Swarm Health</span>
              <p className="text-base sm:text-lg font-bold text-emerald-400 mt-0.5">
                {torrent.health_score || 0}%
              </p>
            </div>
            <div className="p-3.5 bg-slate-800/40 border border-slate-800 rounded-2xl">
              <span className="text-xs text-slate-400">Popularity</span>
              <p className="text-base sm:text-lg font-bold text-blue-400 mt-0.5">
                {torrent.popularity_score || 0}
              </p>
            </div>
          </div>

          {/* Infohash */}
          <div className="p-4 bg-slate-950/60 border border-slate-800/80 rounded-2xl flex items-center justify-between gap-3">
            <div className="min-w-0">
              <span className="text-xs font-medium text-slate-500 uppercase tracking-wider block">SHA-1 Infohash</span>
              <code className="text-xs sm:text-sm font-mono text-slate-300 break-all select-all">
                {torrent.infohash}
              </code>
            </div>
            <button
              onClick={handleCopyInfohash}
              className="p-2 bg-slate-800 hover:bg-slate-700 text-slate-300 rounded-xl transition flex-shrink-0 flex items-center gap-1 text-xs font-medium"
              title="Copy Infohash"
            >
              {copiedIh ? <Check className="w-4 h-4 text-emerald-400" /> : <Copy className="w-4 h-4" />}
            </button>
          </div>

          {/* File Tree / Contents */}
          <div>
            <h4 className="text-sm font-semibold text-slate-300 mb-3 flex items-center gap-2">
              <FolderTree className="w-4 h-4 text-blue-400" />
              <span>File Contents ({filesList.length > 0 ? filesList.length : torrent.file_count || 1} items)</span>
            </h4>

            {loading ? (
              <div className="py-8 text-center text-slate-500 text-sm">
                <span className="inline-block w-5 h-5 border-2 border-blue-500 border-t-transparent rounded-full animate-spin mb-2" />
                <p>Loading file tree metadata...</p>
              </div>
            ) : filesList.length > 0 ? (
              <div className="max-h-64 overflow-y-auto border border-slate-800 rounded-2xl bg-slate-950/40 divide-y divide-slate-800/60 font-mono text-xs">
                {filesList.map((file, idx) => (
                  <div key={idx} className="p-3 flex items-center justify-between hover:bg-slate-800/30">
                    <span className="text-slate-300 truncate pr-3">{file.path || file.name || `File ${idx+1}`}</span>
                    <span className="text-slate-500 whitespace-nowrap">{formatBytes(file.length || file.size || 0)}</span>
                  </div>
                ))}
              </div>
            ) : (
              <div className="p-4 rounded-2xl bg-slate-950/40 border border-slate-800 text-xs text-slate-400">
                Single-file torrent: <span className="font-mono text-slate-200">{torrent.name}</span> ({formatBytes(torrent.total_size)})
              </div>
            )}
          </div>
        </div>

        {/* Footer Actions */}
        <div className="p-6 border-t border-slate-800/80 bg-slate-900/80 flex flex-col sm:flex-row items-center justify-end gap-3">
          <button
            onClick={handleCopyMagnet}
            className={`w-full sm:w-auto flex items-center justify-center gap-2 px-5 py-3 rounded-xl font-medium text-sm transition shadow-lg ${
              copiedMag
                ? 'bg-emerald-600 text-white shadow-emerald-500/25'
                : 'bg-gradient-to-r from-rose-600 to-pink-600 hover:from-rose-500 hover:to-pink-500 text-white shadow-rose-500/20'
            }`}
          >
            {copiedMag ? (
              <>
                <Check className="w-4 h-4" />
                <span>Magnet Link Copied!</span>
              </>
            ) : (
              <>
                <Magnet className="w-4 h-4" />
                <span>Copy Magnet URI</span>
              </>
            )}
          </button>

          <button
            onClick={handleDownload}
            className="w-full sm:w-auto flex items-center justify-center gap-2 px-5 py-3 rounded-xl font-medium text-sm bg-blue-600 hover:bg-blue-500 text-white shadow-lg shadow-blue-500/20 transition"
          >
            <Download className="w-4 h-4" />
            <span>Download .torrent</span>
          </button>
        </div>
      </div>
    </div>
  );
}
