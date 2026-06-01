import { useLocation } from 'react-router-dom'
import { Search, Sun, Moon, RefreshCw } from 'lucide-react'
import { NAV } from './nav'
import { useTheme } from '@/lib/theme'
import { useStore } from '@/data/store'
import { Kbd } from './ui'
import { cn } from '@/lib/cn'

export function Topbar({ onOpenPalette }: { onOpenPalette: () => void }) {
  const { pathname } = useLocation()
  const { theme, toggle } = useTheme()
  const { data } = useStore()
  const current = [...NAV].sort((a, b) => b.to.length - a.to.length).find((n) => pathname === n.to || (n.to !== '/' && pathname.startsWith(n.to)))

  return (
    <header className="flex h-13 min-h-[52px] shrink-0 items-center gap-3 border-b border-line bg-surface-2 px-4">
      <div className="flex items-baseline gap-2">
        <span className="text-[13px] font-semibold tracking-tight">Capo</span>
        <span className="text-fg-3">/</span>
        <span className="text-[13px] text-fg-1">{current?.label ?? 'Operator Console'}</span>
      </div>

      <button
        onClick={onOpenPalette}
        className="ml-4 hidden items-center gap-2 rounded-md border border-line bg-surface-3 px-2.5 py-1.5 text-[12px] text-fg-2 transition-colors hover:bg-surface-hover sm:flex"
      >
        <Search size={13} />
        <span>Search & commands</span>
        <Kbd>⌘K</Kbd>
      </button>

      <div className="ml-auto flex items-center gap-2">
        <span className="mono hidden items-center gap-2 rounded-md border border-line bg-surface-3 px-2.5 py-1.5 text-[11px] text-fg-2 md:flex">
          <span className="size-1.5 rounded-full bg-cyan" />
          {data.project.addr}
        </span>
        <span
          className={cn(
            'rounded-full border px-2 py-1 text-[11px] font-medium',
            data.project.mode === 'live'
              ? 'border-green-ln bg-green-bg text-green'
              : 'border-amber-ln bg-amber-bg text-amber',
          )}
        >
          {data.project.mode}
        </span>
        <button
          title="Refresh"
          className="grid size-8 place-items-center rounded-md border border-line text-fg-2 transition-colors hover:bg-surface-hover hover:text-fg"
        >
          <RefreshCw size={15} />
        </button>
        <button
          onClick={toggle}
          title={theme === 'dark' ? 'Switch to light' : 'Switch to dark'}
          className="grid size-8 place-items-center rounded-md border border-line text-fg-2 transition-colors hover:bg-surface-hover hover:text-fg"
        >
          {theme === 'dark' ? <Sun size={15} /> : <Moon size={15} />}
        </button>
      </div>
    </header>
  )
}
