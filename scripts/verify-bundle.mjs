#!/usr/bin/env node
/**
 * Tauri 打包产物自检：防止「白屏」与「旧资源」两类回归。
 *
 * 背景：Tauri v2 由 `tauri` crate 编译期的 `dev` cfg 决定注入 devUrl 还是 frontendDist：
 *   - `tauri` crate 的 build.rs：`let dev = !custom_protocol;`
 *     （`custom-protocol` feature 由 tauri CLI 通过 `--features tauri/custom-protocol` 打开）
 *   - `tauri-codegen` 资源嵌入分支：
 *       dev 模式  → `EmbeddedAssets::default()`（**不嵌入任何前端资源**）
 *       release   → `EmbeddedAssets::new(assets_path, ...)`（嵌入整个 dist/）
 *   - 运行时：`#[cfg(dev)]` 加载 devUrl（http://localhost:1420）
 *              `#[cfg(not(dev))]` 加载 frontendDist（tauri://localhost）
 *
 * 因此：**exe 里能不能搜到 dist/assets 的文件名，就是 release 版是否正确的唯一判据。**
 * 白屏版（裸 `cargo build --release`）的 exe 里一定搜不到。
 *
 * 用法：
 *   node scripts/verify-bundle.mjs [选项]
 *
 * 选项：
 *   --dist <dir>    前端产物目录（默认 dist）
 *   --exe <path>   待检 exe（默认自动在 src-tauri/target/release 下查找）
 *   --bundle <dir> NSIS 产物目录（默认 src-tauri/target/release/bundle/nsis）
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
// 以**当前工作目录**为项目根，而不是脚本所在位置 ——
// 这样从任意项目的根目录执行 `node <skill>/scripts/verify-bundle.mjs` 都能正确工作。
const ROOT = process.cwd()

function parseArgs(argv) {
  const opts = {
    dist: path.join(ROOT, 'dist'),
    exe: null,
    bundle: path.join(ROOT, 'src-tauri', 'target', 'release', 'bundle', 'nsis'),
  }
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i]
    if (a === '--dist') opts.dist = path.resolve(argv[++i])
    else if (a === '--exe') opts.exe = path.resolve(argv[++i])
    else if (a === '--bundle') opts.bundle = path.resolve(argv[++i])
  }
  return opts
}

const opts = parseArgs(process.argv.slice(2))

/** 自动发现 release 目录下的 exe（排除 installer/setup 文件） */
function discoverExe() {
  const releaseDir = path.join(ROOT, 'src-tauri', 'target', 'release')
  if (!fs.existsSync(releaseDir)) return null
  const found = fs
    .readdirSync(releaseDir, { withFileTypes: true })
    .filter((d) => d.isFile() && d.name.toLowerCase().endsWith('.exe'))
    .map((d) => path.join(releaseDir, d.name))
  if (found.length === 0) return null
  // 优先选体积最大者（主程序通常比辅助 exe 大）
  found.sort((a, b) => fs.statSync(b).size - fs.statSync(a).size)
  return found[0]
}

let failures = 0
function check(ok, label, detail = '') {
  console.log(`  ${ok ? '✅' : '❌'} ${label}${detail ? '  ' + detail : ''}`)
  if (!ok) failures += 1
}
function note(label, detail = '') {
  console.log(`  ℹ ${label}${detail ? '  ' + detail : ''}`)
}

console.log('════ 打包产物自检（白屏 / 旧资源回归防护）════\n')

// ---- 1. dist 存在且有产物 ----
if (!fs.existsSync(opts.dist)) {
  console.error(`  ❌ 前端产物目录不存在：${opts.dist}\n     请先执行 npm run build`)
  process.exit(1)
}
const assetsDir = path.join(opts.dist, 'assets')
const assetFiles = fs.existsSync(assetsDir)
  ? fs.readdirSync(assetsDir).filter((f) => /\.(js|css)$/.test(f))
  : []
check(assetFiles.length > 0, 'dist/assets 存在前端产物', `${assetFiles.length} 个文件`)

// ---- 2. 定位待检 exe ----
const exePath = opts.exe ?? discoverExe()
if (!exePath || !fs.existsSync(exePath)) {
  console.error(
    `  ❌ 未找到待检 exe${exePath ? `：${exePath}` : ''}\n` +
      '     请执行 npx tauri build（不要用裸 cargo build --release）',
  )
  process.exit(1)
}
console.log(`  ℹ 待检产物：${path.relative(ROOT, exePath)}`)

// ---- 3. 核心判据：前端资源必须内嵌进 exe ----
const exe = fs.readFileSync(exePath)
const exeText = exe.toString('latin1')
const embedded = assetFiles.filter((f) => exeText.includes(f))
check(
  embedded.length === assetFiles.length,
  '前端资源已内嵌进 exe（证明是 release 模式，非 devUrl 白屏版）',
  `${embedded.length}/${assetFiles.length}`,
)
if (embedded.length !== assetFiles.length) {
  const missing = assetFiles.filter((f) => !embedded.includes(f))
  console.log(`     缺失：${missing.join(', ')}`)
  console.log('     → exe 内嵌的是旧版本前端资源，或是用裸 `cargo build --release` 构建的。')
  console.log('     → 请执行：npm run build && npx tauri build')
}

// ---- 4. exe 必须不早于 dist（仅提示）----
// 说明：前端构建是确定性的（同样源码 → 同样文件名），dist 可能被重建而内容不变，
// 此时时间戳会误报。真正能判定"资源过期"的是上面第 3 条（内嵌文件名是否与 dist 一致）。
const exeTime = fs.statSync(exePath).mtimeMs
if (assetFiles.length > 0) {
  const newestAsset = Math.max(
    ...assetFiles.map((f) => fs.statSync(path.join(assetsDir, f)).mtimeMs),
  )
  note(
    `exe 时间戳${exeTime >= newestAsset ? '不早于' : '早于'}前端产物` +
      '（仅提示；资源是否过期以上一条"内嵌文件名"为准）',
  )
}

// ---- 5. 发布产物绝不能带 DevTools 远程调试端口（安全底线）----
const debugPortHits = exeText.split('--remote-debugging-port').length - 1
check(
  debugPortHits === 0,
  '发布产物未开启 DevTools 远程调试端口',
  debugPortHits === 0 ? '' : `发现 ${debugPortHits} 处（仅测试构建才允许）`,
)

// ---- 6. devUrl 特征：仅作提示（配置里本就有 devUrl 字段，不作为失败判据）----
const devUrlHits = exeText.split('localhost:1420').length - 1
note(`exe 内 "localhost:1420" 出现 ${devUrlHits} 次（配置字段本身，不作为判据）`)

// ---- 7. NSIS 安装包（若有）----
if (fs.existsSync(opts.bundle)) {
  const installers = fs
    .readdirSync(opts.bundle)
    .filter((f) => f.toLowerCase().endsWith('.exe'))
  check(
    installers.length > 0,
    'NSIS 安装包已生成',
    installers.length > 0
      ? installers.map((f) => `${f} (${(fs.statSync(path.join(opts.bundle, f)).size / 1024 / 1024).toFixed(2)} MiB)`).join(', ')
      : '未找到 .exe',
  )
} else {
  note('NSIS bundle 目录不存在（若只需 exe 可忽略）')
}

console.log('')
if (failures > 0) {
  console.error(`════ 自检未通过：${failures} 项失败 ════`)
  process.exit(1)
}
console.log('════ 自检全部通过 ════')
