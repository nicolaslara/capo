import { Panel, PanelHeader } from '@/components/ui'

export function Placeholder({ title, prompt }: { title: string; prompt: string }) {
  return (
    <div className="p-4">
      <Panel className="min-h-[60vh]">
        <PanelHeader prompt={prompt} />
        <div className="grid flex-1 place-items-center p-10 text-center">
          <div>
            <div className="mono text-[13px] text-fg-2">{title}</div>
            <div className="mt-1 text-[12px] text-fg-3">This screen is being built next.</div>
          </div>
        </div>
      </Panel>
    </div>
  )
}
