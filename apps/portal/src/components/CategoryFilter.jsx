import React from 'react';
import { 
  Film, 
  Tv, 
  Sparkles, 
  Gamepad2, 
  Layers, 
  Music, 
  BookOpen, 
  Video,
  Compass
} from 'lucide-react';

const CATEGORIES = [
  { id: 'All', label: 'All Releases', icon: Compass },
  { id: 'Movies', label: 'Movies', icon: Film },
  { id: 'Television', label: 'TV Shows', icon: Tv },
  { id: 'Anime', label: 'Anime', icon: Sparkles },
  { id: 'Games', label: 'Games', icon: Gamepad2 },
  { id: 'Applications', label: 'Software', icon: Layers },
  { id: 'Music', label: 'Music', icon: Music },
  { id: 'Books & Learning', label: 'Books', icon: BookOpen },
  { id: 'Documentaries', label: 'Docs', icon: Video },
];

export default function CategoryFilter({ selected, onSelect }) {
  return (
    <div className="flex items-center gap-2 overflow-x-auto pb-2 scrollbar-none no-scrollbar">
      {CATEGORIES.map((cat) => {
        const Icon = cat.icon;
        const isSelected = selected === cat.id;

        return (
          <button
            key={cat.id}
            onClick={() => onSelect(cat.id)}
            className={`flex items-center gap-2 px-3.5 py-2 rounded-xl text-xs sm:text-sm font-medium transition-all whitespace-nowrap ${
              isSelected
                ? 'bg-blue-600 text-white shadow-lg shadow-blue-500/25 border border-blue-400/30 font-semibold'
                : 'bg-slate-900/60 hover:bg-slate-800 text-slate-400 hover:text-slate-200 border border-slate-800/80'
            }`}
          >
            <Icon className={`w-3.5 h-3.5 ${isSelected ? 'text-white' : 'text-slate-400'}`} />
            <span>{cat.label}</span>
          </button>
        );
      })}
    </div>
  );
}
