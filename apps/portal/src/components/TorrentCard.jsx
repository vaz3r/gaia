import React, { useState } from 'react';
import { 
  Magnet, 
  Download, 
  FileText, 
  Check, 
  Activity, 
  ShieldCheck, 
  FolderTree, 
  Clock 
} from 'lucide-react';

function formatBytes(bytes, decimals = 2) {
  if (!bytes || bytes === 0) return '0 B';
  const k = 1024;
  const dm = decimals < 0 ? 0 : decimals;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + ' ' + sizes[i];
}

function getCategoryColor(cat) {
  switch (cat?.toLowerCase()) {
    case 'movies': return 'bg-purple-500/10 text-purple-400 border-purple-500/20';
    case 'television': return 'bg-blue-500/10 text-blue-400 border-blue-500/20';
    case 'anime': return 'bg-pink-500/10 text-pink-400 border-pink-500/20';
    case 'games': return 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20';
    case 'applications': return 'bg-amber-500/10 text-amber-400 border-amber-500/20';
    case 'music': return 'bg-cyan-500/10 text-cyan-400 border-cyan-500/20';
    case 'books & learning': return 'bg-indigo-500/10 text-indigo-400 border-indigo-500/20';
    default: return 'bg-slate-500/10 text-slate-400 border-slate-500/20';
  }
}

export default function TorrentCard({ torrent, onSelect }) {
  const [copied, setCopied] = useState(false);
  const [isCopying, setIsCopying] = useState(false);

  const handleCopyMagnet = async (e) => {
    e.stopPropagation();
    if (isCopying) return;
    setIsCopying(true);

    try {
      const res = await fetch(`/api/torrents/${torrent.infohash}/magnet`);
      const data = await res.json();
      const magnetUrl = data.magnetUri || `magnet:?xt=urn:btih:${torrent.infohash}&dn=${encodeURIComponent(torrent.name)}`;
      
      await navigator.clipboard.writeText(magnetUrl);
      setCopied(true);
      setTimeout(() => setCopied(false), 2500);
    } catch (err) {
      console.error('Failed to copy magnet:', err);
      // Fallback direct format
      const fallback = `magnet:?xt=urn:btih:${torrent.infohash}&dn=${encodeURIComponent(torrent.name)}`;
      await navigator.clipboard.writeText(fallback);
      setCopied(true);
      setTimeout(() => setCopied(false), 2500);
    } finally {
      setIsCopying(false);
    }
  };

  const handleDownloadTorrent = (e) => {
    e.stopPropagation();
    window.location.href = `/api/torrents/${torrent.infohash}/torrent`;
  };

  const isHealthy = torrent.health_score >= 70;
  const isFair = torrent.health_score >= 40 && torrent.health_score < 70;

  return (
    <div 
      onClick={() => onSelect(torrent)}
      className="group relative bg-slate-900/50 hover:bg-slate-900/80 border border-slate-800/80 hover:border-blue-500/40 rounded-2xl p-4 sm:p-5 transition-all duration-200 cursor-pointer shadow-md hover:shadow-xl hover:shadow-blue-500/5 backdrop-blur-sm"
    >
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
        {/* Left info */}
        <div className="flex-1 min-w-0 pr-2">
          <div className="flex items-center gap-2 mb-2 flex-wrap">
            <span className={`px-2.5 py-0.5 rounded-md text-xs font-medium border ${getCategoryColor(torrent.category)}`}>
              {torrent.category || 'Other'}
            </span>
            
            {/* Swarm Health Badge */}
            <span className={`inline-flex items-center gap-1 px-2.5 py-0.5 rounded-md text-xs font-medium border ${
              isHealthy 
                ? 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20' 
                : isFair
                ? 'bg-amber-500/10 text-amber-400 border-amber-500/20'
                : 'bg-slate-500/10 text-slate-400 border-slate-500/20'
            }`}>
              <span className={`w-1.5 h-1.5 rounded-full ${isHealthy ? 'bg-emerald-400 animate-pulse' : isFair ? 'bg-amber-400' : 'bg-slate-400'}`} />
              <span>Health: {torrent.health_score || 0}%</span>
            </span>

            {torrent.seed_confirmed && (
              <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-xs font-medium bg-blue-500/10 text-blue-400 border border-blue-500/20">
                <ShieldCheck className="w-3 h-3" />
                <span>Verified Peer</span>
              </span>
            )}
          </div>

          <h3 className="text-base sm:text-lg font-semibold text-slate-100 group-hover:text-blue-400 transition-colors line-clamp-2 break-words">
            {torrent.name}
          </h3>

          <div className="flex items-center gap-4 mt-2 text-xs sm:text-sm text-slate-400 flex-wrap">
            <span className="font-semibold text-slate-200">
              {formatBytes(torrent.total_size)}
            </span>
            <span className="flex items-center gap-1">
              <FolderTree className="w-3.5 h-3.5 text-slate-500" />
              <span>{torrent.file_count || 1} {torrent.file_count === 1 ? 'file' : 'files'}</span>
            </span>
            {torrent.verified_at && (
              <span className="flex items-center gap-1 text-slate-500">
                <Clock className="w-3.5 h-3.5" />
                <span>{new Date(torrent.verified_at).toLocaleDateString()}</span>
              </span>
            )}
          </div>
        </div>

        {/* Right actions */}
        <div className="flex items-center gap-2 pt-2 sm:pt-0 border-t sm:border-t-0 border-slate-800/80 justify-end flex-shrink-0">
          <button
            onClick={handleCopyMagnet}
            className={`flex items-center gap-1.5 px-3.5 py-2 rounded-xl text-xs sm:text-sm font-medium transition shadow-sm ${
              copied
                ? 'bg-emerald-600 text-white shadow-emerald-500/20'
                : 'bg-slate-800 hover:bg-slate-700 text-slate-200 hover:text-white border border-slate-700/80'
            }`}
            title="Copy Magnet Link"
          >
            {copied ? (
              <>
                <Check className="w-4 h-4 text-white" />
                <span>Copied!</span>
              </>
            ) : (
              <>
                <Magnet className="w-4 h-4 text-rose-400" />
                <span>Magnet</span>
              </>
            )}
          </button>

          <button
            onClick={handleDownloadTorrent}
            className="flex items-center gap-1.5 px-3.5 py-2 rounded-xl text-xs sm:text-sm font-medium bg-blue-600/20 hover:bg-blue-600 text-blue-300 hover:text-white border border-blue-500/30 transition shadow-sm"
            title="Download .torrent file"
          >
            <Download className="w-4 h-4" />
            <span>.torrent</span>
          </button>
        </div>
      </div>
    </div>
  );
}
