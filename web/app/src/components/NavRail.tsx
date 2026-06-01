import { NavLink } from 'react-router-dom'
import * as Tooltip from '@radix-ui/react-tooltip'
import { cn } from '@/lib/cn'
import { NAV } from './nav'

export function NavRail() {
  return (
    <nav className="flex w-[56px] shrink-0 flex-col items-center gap-1 border-r border-line bg-surface-1 py-3">
      <div className="mb-2 grid size-8 place-items-center rounded-lg border border-violet-ln bg-violet-bg text-violet">
        <span className="mono text-[15px] font-bold">C</span>
      </div>
      <Tooltip.Provider delayDuration={300}>
        {NAV.map((item) => (
          <Tooltip.Root key={item.to}>
            <Tooltip.Trigger asChild>
              <NavLink
                to={item.to}
                end={item.to === '/'}
                className={({ isActive }) =>
                  cn(
                    'relative grid size-10 place-items-center rounded-lg text-fg-2 transition-colors',
                    'hover:bg-surface-hover hover:text-fg outline-none focus-visible:ring-2 focus-visible:ring-cyan/50',
                    isActive && 'bg-surface-active text-violet',
                  )
                }
              >
                {({ isActive }) => (
                  <>
                    {isActive && (
                      <span className="absolute left-0 top-1/2 h-5 w-0.5 -translate-y-1/2 rounded-full bg-violet" />
                    )}
                    <item.icon size={18} strokeWidth={2} />
                  </>
                )}
              </NavLink>
            </Tooltip.Trigger>
            <Tooltip.Portal>
              <Tooltip.Content
                side="right"
                sideOffset={8}
                className="z-50 rounded-md border border-line bg-surface-2 px-2 py-1 text-[12px] text-fg shadow-[var(--shadow)]"
              >
                {item.label}
              </Tooltip.Content>
            </Tooltip.Portal>
          </Tooltip.Root>
        ))}
      </Tooltip.Provider>
    </nav>
  )
}
