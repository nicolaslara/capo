import { Routes, Route } from 'react-router-dom'
import { AppShell } from './components/AppShell'
import { Overview } from './screens/Overview'
import { Agents } from './screens/Agents'
import { Chat } from './screens/Chat'
import { Goals } from './screens/Goals'
import { Activity } from './screens/Activity'
import { Tools } from './screens/Tools'
import { Settings } from './screens/Settings'
import { Placeholder } from './screens/Placeholder'

export default function App() {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route path="/" element={<Overview />} />
        <Route path="/agents" element={<Agents />} />
        <Route path="/chat" element={<Chat />} />
        <Route path="/goals" element={<Goals />} />
        <Route path="/activity" element={<Activity />} />
        <Route path="/tools" element={<Tools />} />
        <Route path="/settings" element={<Settings />} />
        <Route path="*" element={<Placeholder title="Not found" prompt="$ 404" />} />
      </Route>
    </Routes>
  )
}
