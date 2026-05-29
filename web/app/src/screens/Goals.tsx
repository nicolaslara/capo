import { Check, Circle } from 'lucide-react'
import { useStore } from '@/data/store'
import { Panel, PanelHeader, PanelBody, Chip, ConfidenceMeter, Eyebrow } from '@/components/ui'

export function Goals() {
  const { data } = useStore()
  return (
    <div className="flex flex-col gap-3 p-4">
      {data.goals.map((g) => {
        const done = g.requirements.filter((r) => r.done).length
        return (
          <Panel key={g.id}>
            <PanelHeader
              prompt="$ goal"
              title={g.id}
              right={<Chip tone={g.status === 'completed' ? 'green' : g.status === 'blocked' ? 'red' : 'cyan'}>{g.status}</Chip>}
            />
            <PanelBody className="flex flex-col gap-4">
              <div className="flex flex-wrap items-center gap-3">
                <h2 className="text-[15px] font-semibold text-fg">{g.title}</h2>
                {g.confidence && <ConfidenceMeter value={g.confidence} />}
                <span className="mono ml-auto text-[11px] text-fg-2">{done}/{g.requirements.length} requirements · validation: {g.validation}</span>
              </div>

              <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
                <div>
                  <Eyebrow>Requirements</Eyebrow>
                  <ul className="mt-1.5 flex flex-col gap-1">
                    {g.requirements.map((r) => (
                      <li key={r.text} className="flex items-center gap-2 text-[13px]">
                        {r.done
                          ? <Check size={14} className="text-green" />
                          : <Circle size={14} className="text-fg-3" />}
                        <span className={r.done ? 'text-fg-1' : 'text-fg-2'}>{r.text}</span>
                      </li>
                    ))}
                  </ul>
                  {g.blockers.length > 0 && (
                    <div className="mt-3">
                      <Eyebrow>Blockers</Eyebrow>
                      <ul className="mt-1.5 flex flex-col gap-1">
                        {g.blockers.map((b) => (
                          <li key={b} className="mono flex items-center gap-2 text-[12px] text-amber">
                            <span>[!]</span> {b}
                          </li>
                        ))}
                      </ul>
                    </div>
                  )}
                </div>

                <div>
                  <Eyebrow>Story</Eyebrow>
                  <ol className="mt-1.5 border-l border-line pl-4">
                    {g.story.map((s, i) => (
                      <li key={i} className="relative pb-3 last:pb-0">
                        <span className="absolute -left-[21px] top-1 size-2 rounded-full bg-cyan" />
                        <div className="mono text-[11px] text-fg-3">{s.time}</div>
                        <div className="text-[12px] text-fg-1">{s.text}</div>
                      </li>
                    ))}
                  </ol>
                </div>
              </div>
            </PanelBody>
          </Panel>
        )
      })}
    </div>
  )
}
