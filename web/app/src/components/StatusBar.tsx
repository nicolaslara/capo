import { useStore } from '@/data/store'

export function StatusBar() {
  const { data } = useStore()
  const blocked = data.agents.filter((a) => a.status === 'blocked' || a.status === 'timed out').length
  const active = data.agents.filter((a) => a.status === 'running').length
  const finished = data.agents.filter((a) => a.status === 'finished').length

  return (
    <footer className="mono flex h-7 shrink-0 items-center gap-4 overflow-x-auto whitespace-nowrap border-t border-line bg-surface-2 px-4 text-[11px] text-fg-2">
      <span className="flex items-center gap-1.5">
        <span className="size-1.5 rounded-full bg-cyan" /> connected
      </span>
      <span className="text-fg-3">·</span>
      <span>mode {data.project.mode}</span>
      <span className="text-fg-3">·</span>
      <span className={blocked > 0 ? 'text-amber' : ''}>blocked {blocked}</span>
      <span className="text-fg-3">·</span>
      <span>active {active} / finished {finished}</span>
      <span className="ml-auto text-fg-3">{data.project.id}</span>
      <span className="text-fg-3">·</span>
      <span>updated {data.project.updatedAt.slice(11, 16)}</span>
    </footer>
  )
}
