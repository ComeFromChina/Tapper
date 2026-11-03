#!/usr/bin/env node
/**
 * Tauri 应用图标生成脚本（通用版）
 *
 * 纯 Node 实现，零第三方依赖、零原生二进制：
 *   1) 数学光栅化绘制图形 → RGBA 像素缓冲（2×2 超采样抗锯齿）
 *   2) 手写 PNG 编码器（zlib deflate + CRC32）
 *   3) 手拼 ICO 容器（Vista+ 支持 ICO 内嵌 PNG）
 *
 * 用法：
 *   node scripts/generate-icons.mjs [选项]
 *
 * 选项：
 *   --out <dir>          输出目录（默认 src-tauri/icons）
 *   --shape <power|circle|square|bolt>  图形（默认 power）
 *   --surface <#rrggbb>  底色（默认 0b1220）
 *   --primary <#rrggbb>  主色（默认 3b82f6）
 *   --accent  <#rrggbb>  强调色（默认 06b6d4）
 *   --no-tray            不生成托盘三态图标
 *
 * 生成的图标会被 tauri.conf.json 的 bundle.icon 引用，缺文件会导致打包直接失败。
 */

import { deflateSync, crc32 } from 'node:zlib'
import { writeFileSync, mkdirSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))

// ---------- 参数解析 ----------
function parseArgs(argv) {
  const opts = {
    // 以**当前工作目录**为项目根（而非脚本所在位置），这样从任意项目根目录执行都能正确工作。
    out: join(process.cwd(), 'src-tauri', 'icons'),
    shape: 'power',
    surface: '0b1220',
    primary: '3b82f6',
    accent: '06b6d4',
    tray: true,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i]
    if (a === '--out') opts.out = resolve(argv[++i])
    else if (a === '--shape') opts.shape = argv[++i]
    else if (a === '--surface') opts.surface = argv[++i].replace('#', '')
    else if (a === '--primary') opts.primary = argv[++i].replace('#', '')
    else if (a === '--accent') opts.accent = argv[++i].replace('#', '')
    else if (a === '--no-tray') opts.tray = false
  }
  return opts
}

function hexToRgb(hex) {
  const h = hex.replace('#', '')
  const n = Number.parseInt(h.length === 3 ? h.split('').map((c) => c + c).join('') : h, 16)
  if (Number.isNaN(n)) throw new Error(`颜色格式错误：${hex}（应形如 3b82f6）`)
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff]
}

const opts = parseArgs(process.argv.slice(2))

const PALETTE = {
  surface: [...hexToRgb(opts.surface), 0xff],
  primary: [...hexToRgb(opts.primary), 0xff],
  accent: [...hexToRgb(opts.accent), 0xff],
  warning: [0xf5, 0x9e, 0x0b, 0xff],
}

// ---------- PNG 编码器 ----------
function chunk(type, data) {
  const len = Buffer.alloc(4)
  len.writeUInt32BE(data.length, 0)
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
  const crc = Buffer.alloc(4)
  crc.writeUInt32BE(crc32(body) >>> 0, 0)
  return Buffer.concat([len, body, crc])
}

/** rgba: Buffer(width*height*4) → PNG Buffer */
function encodePng(width, height, rgba) {
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])

  const ihdr = Buffer.alloc(13)
  ihdr.writeUInt32BE(width, 0)
  ihdr.writeUInt32BE(height, 4)
  ihdr[8] = 8 // bit depth
  ihdr[9] = 6 // color type = RGBA
  ihdr[10] = 0 // compression
  ihdr[11] = 0 // filter
  ihdr[12] = 0 // interlace

  const stride = width * 4
  const raw = Buffer.alloc((stride + 1) * height)
  for (let y = 0; y < height; y += 1) {
    raw[y * (stride + 1)] = 0 // filter type: None
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride)
  }
  const idat = deflateSync(raw, { level: 9 })

  return Buffer.concat([
    sig,
    chunk('IHDR', ihdr),
    chunk('IDAT', idat),
    chunk('IEND', Buffer.alloc(0)),
  ])
}

// ---------- 几何绘制（符号距离场） ----------
/** 圆角矩形有符号距离（负 = 内部） */
function sdRoundRect(px, py, halfW, halfH, r) {
  const qx = Math.abs(px) - halfW + r
  const qy = Math.abs(py) - halfH + r
  const ax = Math.max(qx, 0)
  const ay = Math.max(qy, 0)
  return Math.hypot(ax, ay) + Math.min(Math.max(qx, qy), 0) - r
}

/** 圆环（圆心到点距离与半径之差） */
function sdRing(px, py, radius, thickness) {
  return Math.abs(Math.hypot(px, py) - radius) - thickness / 2
}

/** 判断点是否在图形内部 */
function insideGlyph(cx, cy, s) {
  const shape = opts.shape
  if (shape === 'circle') {
    return sdRing(cx, cy, 16 * s, 3.4 * s) <= 0
  }
  if (shape === 'square') {
    const hw = 15 * s
    return Math.abs(cx) <= hw && Math.abs(cy) <= hw
  }
  if (shape === 'bolt') {
    // 闪电：折线近似
    const pts = [
      [3, -20], [-6, 2], [0, 2], [-3, 20], [8, -4], [2, -4],
    ]
    for (let i = 0; i < pts.length - 1; i += 1) {
      const [x1, y1] = [pts[i][0] * s, pts[i][1] * s]
      const [x2, y2] = [pts[i + 1][0] * s, pts[i + 1][1] * s]
      const vx = x2 - x1
      const vy = y2 - y1
      const len2 = vx * vx + vy * vy
      const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((cx - x1) * vx + (cy - y1) * vy) / len2))
      const dx = cx - (x1 + t * vx)
      const dy = cy - (y1 + t * vy)
      if (Math.hypot(dx, dy) <= 2.2 * s) return true
    }
    return false
  }
  // power：圆环缺口在正上方 + 中间竖线
  const ringR = 15 * s
  const ringT = 3.4 * s
  const barW = 3.4 * s
  const barTop = 9 * s
  const barBottom = 16 * s
  const inBar = Math.abs(cx) <= barW / 2 && cy <= -barTop && cy >= -barBottom
  const ang = (Math.atan2(cy, cx) * 180) / Math.PI
  const gap = ang > -130 && ang < -50
  const inRing = !gap && sdRing(cx, cy, ringR, ringT) <= 0
  return inBar || inRing
}

/**
 * 绘制图标。
 * @param {number} size 画布边长
 * @param {number[]} glyph 主色
 */
function renderIcon(size, glyph) {
  const rgba = Buffer.alloc(size * size * 4)
  const s = size / 64 // 以 64 单位为设计基准
  const corner = 14 * s
  const ss = 2 // 2×2 超采样抗锯齿
  const half = size / 2

  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      let r = 0
      let g = 0
      let b = 0
      let a = 0

      for (let sy = 0; sy < ss; sy += 1) {
        for (let sx = 0; sx < ss; sx += 1) {
          const px = x + (sx + 0.5) / ss
          const py = y + (sy + 0.5) / ss
          const cx = px - half
          const cy = py - half

          const dBg = sdRoundRect(cx, cy, half, half, corner)
          if (dBg <= 0) {
            let col
            if (insideGlyph(cx, cy, s)) {
              // 竖直渐变：主色 → 强调色
              const t = Math.min(1, Math.max(0, (cy + half) / size))
              col = [
                Math.round(glyph[0] + (PALETTE.accent[0] - glyph[0]) * t),
                Math.round(glyph[1] + (PALETTE.accent[1] - glyph[1]) * t),
                Math.round(glyph[2] + (PALETTE.accent[2] - glyph[2]) * t),
                255,
              ]
            } else {
              col = PALETTE.surface
            }
            r += col[0]
            g += col[1]
            b += col[2]
            a += col[3]
          }
        }
      }

      const n = ss * ss
      const i = (y * size + x) * 4
      rgba[i] = Math.round(r / n)
      rgba[i + 1] = Math.round(g / n)
      rgba[i + 2] = Math.round(b / n)
      rgba[i + 3] = Math.round(a / n)
    }
  }
  return rgba
}

// ---------- ICO 容器 ----------
/** 用 PNG 字节拼 ICO（Vista+ 支持内嵌 PNG） */
function encodeIco(entries) {
  const count = entries.length
  const header = Buffer.alloc(6)
  header.writeUInt16LE(0, 0) // reserved
  header.writeUInt16LE(1, 2) // type = icon
  header.writeUInt16LE(count, 4)

  const dir = Buffer.alloc(16 * count)
  let offset = 6 + 16 * count

  entries.forEach((e, i) => {
    const o = i * 16
    dir[o] = e.size >= 256 ? 0 : e.size // 256 写 0
    dir[o + 1] = e.size >= 256 ? 0 : e.size
    dir[o + 2] = 0 // colorCount
    dir[o + 3] = 0 // reserved
    dir.writeUInt16LE(1, o + 4) // planes
    dir.writeUInt16LE(32, o + 6) // bitCount
    dir.writeUInt32LE(e.png.length, o + 8) // bytesInRes
    dir.writeUInt32LE(offset, o + 12) // imageOffset
    offset += e.png.length
  })

  return Buffer.concat([header, dir, ...entries.map((e) => e.png)])
}

// ---------- 主流程 ----------
function main() {
  mkdirSync(opts.out, { recursive: true })

  const written = []
  const pngSizes = [
    [32, '32x32.png'],
    [128, '128x128.png'],
    [256, '128x128@2x.png'],
    [512, 'icon.png'],
  ]

  const icoEntries = []
  for (const size of [16, 32, 48, 64, 128, 256]) {
    const rgba = renderIcon(size, PALETTE.primary)
    icoEntries.push({ size, png: encodePng(size, size, rgba) })
  }

  for (const [size, name] of pngSizes) {
    const rgba = renderIcon(size, PALETTE.primary)
    const buf = encodePng(size, size, rgba)
    writeFileSync(join(opts.out, name), buf)
    written.push([name, buf.length])
  }

  if (opts.tray) {
    const trays = [
      ['tray-idle.png', PALETTE.primary],
      ['tray-running.png', PALETTE.accent],
      ['tray-warning.png', PALETTE.warning],
    ]
    for (const [name, color] of trays) {
      const rgba = renderIcon(32, color)
      const buf = encodePng(32, 32, rgba)
      writeFileSync(join(opts.out, name), buf)
      written.push([name, buf.length])
    }
  }

  const ico = encodeIco(icoEntries)
  writeFileSync(join(opts.out, 'icon.ico'), ico)
  written.push(['icon.ico', ico.length])

  if (ico.length < 1024) {
    throw new Error(`icon.ico 体积异常（${ico.length} 字节），PNG 编码可能有误`)
  }

  console.log('图标生成完成 →', opts.out)
  for (const [name, bytes] of written) {
    console.log(`  ${name.padEnd(18)} ${String(bytes).padStart(8)} bytes`)
  }
}

main()
