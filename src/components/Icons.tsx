interface IconProps {
  size?: number
  className?: string
}

export function HandIcon({ size = 20, className }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      <path d="M8.5 12.5V5.8a1.4 1.4 0 0 1 2.8 0v5.4" />
      <path d="M11.3 11.2V4.4a1.4 1.4 0 0 1 2.8 0v6.6" />
      <path d="M14.1 11V6.2a1.4 1.4 0 0 1 2.8 0v7.4c0 4.2-2.3 6.9-6 6.9-2.3 0-3.7-1-4.8-2.7l-2.3-3.6a1.4 1.4 0 0 1 2.3-1.6l1.8 2.4" />
    </svg>
  )
}

/**
 * 「点击的箭头」：鼠标箭头 + 周围五道放射线，表示按下去的那一下。
 * iconfont 实心路径（viewBox 1024），靠 fill 上色。
 * 在圆环里整体居中显示，不做位移对齐（用户反馈偏移看着不正）。
 */
export function ClickArrowIcon({ size = 20, className }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 1024 1024"
      fill="currentColor"
      className={className}
      aria-hidden="true"
    >
      <path d="M287.924 487.14c2.78-28.08-17.12-54.22-45-59.22L58.684 397.6c-29.34-3.88-54.92 14.84-58.12 42.52-4.22 29.2 15.68 55.34 45 59.22l184.24 30.3c27.9 4.86 53.56-13.86 58.12-42.5zM264.764 659.8l-147.22 107.62c-22.26 16.3-26.28 47.64-8.66 70.86 17.62 23.16 50.14 29.26 72.32 13.04l147.22-107.62c22.74-16.64 26.56-49.3 8.94-72.46-17.52-23.22-49.84-28.08-72.6-11.44zM565.384 284.24c28.36 4.58 54.44-14.5 58.12-42.52l22.96-181.82c3.68-27.94-16.24-54.16-45-59.22-28.36-4.58-54.44 14.5-58.12 42.44l-23.02 181.88c-3.56 28.02 16.36 54.16 45.06 59.24zM276.624 105.52c-17.26-22.68-49.02-27.88-72.32-10.88-21.78 15.96-26.28 47.64-9.02 70.32l114.34 150.42c17.26 22.74 49.5 27.52 71.28 11.58 23.24-17 27.32-48.34 10.06-71.08l-114.34-150.36zM959.684 216.2c-17.62-23.16-49.44-28.36-72.68-11.36l-146.18 106.92c-23.22 16.98-27.26 48.4-9.64 71.56 17.62 23.16 50.14 29.26 73.36 12.28l146.18-106.94c23.24-16.98 26.56-49.3 8.96-72.46zM424.404 395.04l98.88 573.34s73.08-75.24 133.42-127.74l132.66 177.58c5.34 7.22 17 7.7 25.86 1.18l56.66-41.4c8.94-6.52 11.86-17.68 6.44-24.9l-129.04-172.74c67.4-27.18 178.42-58.12 178.42-58.12L424.404 395.04z" />
    </svg>
  )
}

export function CrosshairIcon({ size = 18, className }: IconProps) {  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      className={className}
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="6.5" />
      <path d="M12 1.8v4M12 18.2v4M1.8 12h4M18.2 12h4" />
      <circle cx="12" cy="12" r="1.4" fill="currentColor" stroke="none" />
    </svg>
  )
}

export function PlayIcon({ size = 18, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" className={className} aria-hidden="true">
      <path d="M8 5.2c0-.9 1-1.4 1.7-.9l9 6.2c.7.5.7 1.5 0 2l-9 6.2c-.7.5-1.7 0-1.7-.9V5.2Z" />
    </svg>
  )
}

export function StopIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" className={className} aria-hidden="true">
      <rect x="6" y="6" width="12" height="12" rx="2.4" />
    </svg>
  )
}

export function RecordIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" className={className} aria-hidden="true">
      <circle cx="12" cy="12" r="5.4" />
    </svg>
  )
}

export function ScreenIcon({ size = 18, className }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      <rect x="2.6" y="4.4" width="18.8" height="12.4" rx="2.2" />
      <path d="M9 20.4h6M12 16.8v3.6" />
      <path d="M8.4 9.6h7.2M8.4 12.4h4.4" />
    </svg>
  )
}

export function ShieldIcon({ size = 18, className }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      <path d="M12 2.8 19.4 5.6v6.2c0 4.6-3 7.7-7.4 9.4-4.4-1.7-7.4-4.8-7.4-9.4V5.6L12 2.8Z" />
      <path d="m9 11.8 2.2 2.2 4-4.2" />
    </svg>
  )
}

export function GaugeIcon({ size = 18, className }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      <path d="M4 18a8.5 8.5 0 1 1 16 0" />
      <path d="M12 14.4 16 9.6" />
      <circle cx="12" cy="15.2" r="1.6" fill="currentColor" stroke="none" />
    </svg>
  )
}

export function MinusIcon({ size = 14, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2" strokeLinecap="round" className={className} aria-hidden="true">
      <path d="M5 12h14" />
    </svg>
  )
}

export function CloseIcon({ size = 14, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2" strokeLinecap="round" className={className} aria-hidden="true">
      <path d="M6 6l12 12M18 6 6 18" />
    </svg>
  )
}

export function PlusIcon({ size = 14, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2" strokeLinecap="round" className={className} aria-hidden="true">
      <path d="M12 5v14M5 12h14" />
    </svg>
  )
}

export function AlertIcon({ size = 16, className }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      <path d="M12 3.4 21 19.6H3L12 3.4Z" />
      <path d="M12 9.4v4.4" />
      <circle cx="12" cy="16.8" r="0.9" fill="currentColor" stroke="none" />
    </svg>
  )
}
