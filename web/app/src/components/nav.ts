import {
  LayoutDashboard,
  Boxes,
  MessageSquare,
  Target,
  Activity,
  Wrench,
  Settings,
  type LucideIcon,
} from 'lucide-react'

export interface NavItem {
  to: string
  label: string
  icon: LucideIcon
  prompt: string
}

export const NAV: NavItem[] = [
  { to: '/', label: 'Overview', icon: LayoutDashboard, prompt: '$ overview' },
  { to: '/agents', label: 'Agents', icon: Boxes, prompt: '$ agents --watch' },
  { to: '/chat', label: 'Chat', icon: MessageSquare, prompt: '> chat' },
  { to: '/goals', label: 'Goals', icon: Target, prompt: '$ goals' },
  { to: '/activity', label: 'Activity', icon: Activity, prompt: '$ activity --tail' },
  { to: '/tools', label: 'Tools', icon: Wrench, prompt: '$ tools' },
  { to: '/settings', label: 'Settings', icon: Settings, prompt: '$ settings' },
]
