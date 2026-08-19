import { useMemo, useState } from 'react'
import { formatBytes } from '../utils.js'

/* Builds a nested tree from [{length, path:[segments...]}]. */
function buildTree(files) {
  const root = { name: '', children: new Map(), size: 0 }
  for (const f of files) {
    const parts = Array.isArray(f.path) && f.path.length ? f.path : ['<unnamed>']
    let node = root
    for (let i = 0; i < parts.length - 1; i++) {
      if (!node.children.has(parts[i])) {
        node.children.set(parts[i], { name: parts[i], children: new Map(), size: 0 })
      }
      node = node.children.get(parts[i])
    }
    const leaf = { name: parts[parts.length - 1], children: new Map(), size: f.length ?? 0 }
    node.children.set(leaf.name, leaf)
  }
  const sum = (n) => {
    let s = 0
    for (const c of n.children.values()) {
      s += sum(c)
    }
    n.size = s > 0 ? s : n.size
    return n.size
  }
  sum(root)
  return root
}

function Dir({ node, depth }) {
  const [open, setOpen] = useState(depth < 2)
  return (
    <div>
      <button
        onClick={() => setOpen(!open)}
        className="flex w-full items-center gap-1.5 py-0.5 text-left hover:text-white text-slate-300"
        style={{ paddingLeft: `${depth * 14}px` }}
      >
        <span className="text-slate-500 w-4">{open ? '▾' : '▸'}</span>
        <span className="font-medium">{node.name}/</span>
        <span className="text-[11px] text-slate-500 ml-1">{formatBytes(node.size)}</span>
      </button>
      {open && [...node.children.values()].map((c, i) => (
        c.children.size > 0 ? (
          <Dir key={c.name + i} node={c} depth={depth + 1} />
        ) : (
          <div
            key={c.name + i}
            className="flex items-center gap-1.5 py-0.5 text-slate-400"
            style={{ paddingLeft: `${(depth + 1) * 14}px` }}
          >
            <span className="text-slate-600 w-4">·</span>
            <span className="truncate">{c.name}</span>
            <span className="text-[11px] text-slate-500 ml-auto pl-3">{formatBytes(c.size)}</span>
          </div>
        )
      ))}
    </div>
  )
}

export default function FileTree({ files }) {
  const root = useMemo(() => buildTree(files), [files])
  return (
    <div className="rounded-lg border border-ink-800 bg-ink-950 px-2 py-2 max-h-72 overflow-y-auto font-mono text-xs">
      {[...root.children.values()].map((c, i) =>
        c.children.size > 0 ? (
          <Dir key={c.name + i} node={c} depth={0} />
        ) : (
          <div key={c.name + i} className="flex items-center gap-1.5 py-0.5 text-slate-400" style={{ paddingLeft: '14px' }}>
            <span className="text-slate-600 w-4">·</span>
            <span className="truncate">{c.name}</span>
            <span className="text-[11px] text-slate-500 ml-auto pl-3">{formatBytes(c.size)}</span>
          </div>
        )
      )}
    </div>
  )
}