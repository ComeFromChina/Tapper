import { MainApp } from './components/MainApp'
import { isOverlayWindow } from './lib/api'
import { OverlayApp } from './overlay/OverlayApp'

export default function App() {
  return isOverlayWindow() ? <OverlayApp /> : <MainApp />
}
