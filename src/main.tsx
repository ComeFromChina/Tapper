import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import App from './App'
import './index.css'
import { isOverlayWindow } from './lib/api'

if (isOverlayWindow()) {
  document.documentElement.classList.add('is-overlay')
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
