import { useEffect, useState } from 'react'
import { Outlet } from 'react-router-dom'
import { NavRail } from './NavRail'
import { Topbar } from './Topbar'
import { StatusBar } from './StatusBar'
import { CommandPalette } from './CommandPalette'

export function AppShell() {
  const [paletteOpen, setPaletteOpen] = useState(false)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setPaletteOpen((o) => !o)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  return (
    <div className="grid h-screen overflow-hidden" style={{ gridTemplateColumns: '56px minmax(0, 1fr)' }}>
      <NavRail />
      <div className="grid min-w-0 grid-cols-1 grid-rows-[auto_1fr_auto]">
        <Topbar onOpenPalette={() => setPaletteOpen(true)} />
        <main className="min-h-0 min-w-0 overflow-y-auto">
          <Outlet />
        </main>
        <StatusBar />
      </div>
      <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} />
    </div>
  )
}
