#!/usr/bin/env node
/**
 * 发版说明**三处同步**：一处改，三处一致。
 *
 *   ① Release 正文（GitHub）        —— 要点 + 安装包表格
 *   ② 官网 website/index.html       —— 版本区条目（要点 + 「最新」标）
 *   ③ 应用「关于」页（locales）      —— settings.about.relN.noteX 要点
 *
 * 唯一来源是仓库里的 `release-notes/<版本>.md`：你只写要点（`- 一句` 一行），其余由本脚本生成。
 *
 * 用法（需要 GH_TOKEN；在哪个目录下跑都行）：
 *   node scripts/sync-release-notes.mjs                 # 全部版本：写文件 + 推 Release
 *   node scripts/sync-release-notes.mjs v1.2.12         # 只处理指定版本
 *   node scripts/sync-release-notes.mjs --no-push       # 只改本地文件，不动 GitHub
 *
 * 关于「关于」页的**英文**要点：本脚本只同步中文（`zh-CN.js`）。英文那份（`en-US.js`）请手工
 * 同步 —— 脚本会在结尾提醒哪些版本与中文条数不一致。
 *
 * 幂等：安装包表格与要点顺序都按 md 重排，重复跑不会叠加。
 */
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const REPO = 'rick-works/dbmind'
const NOTES_DIR = path.join(ROOT, 'release-notes')
const SITE = path.join(ROOT, 'website', 'index.html')
const ZH = path.join(ROOT, 'frontend', 'src', 'locales', 'zh-CN.js')

// 安装包表格：顺序固定 Windows → macOS → Linux，形态说明沿用既有口径
const ORDER = [
  [/setup\.exe$/i, 'Windows', '安装向导（推荐）'],
  [/\.msi$/i, 'Windows', 'MSI 包（企业批量部署 / 静默安装）'],
  [/portable\.zip$/i, 'Windows', '**免安装**：解压即用，不写注册表'],
  [/aarch64\.dmg$/i, 'macOS（Apple 芯片）', '安装镜像：把图标拖进「应用程序」'],
  [/_x64\.dmg$/i, 'macOS（Intel）', '安装镜像：把图标拖进「应用程序」'],
  [/\.deb$/i, 'Linux（Debian / Ubuntu）', 'apt 安装，带开始菜单项'],
  [/\.AppImage$/i, 'Linux（通用发行版）', '**免安装**：`chmod +x` 后直接运行']
]

const human = (b) => (b < 1048576 ? (b / 1024).toFixed(1) + ' KB' : (b / 1048576).toFixed(1) + ' MB')
const gh = (args) => JSON.parse(execFileSync('gh', args, { encoding: 'utf8' }))
const num = (tag) => Number(tag.replace(/^v/, '').split('.').map((n) => n.padStart(3, '0')).join(''))

const argv = process.argv.slice(2)
const noPush = argv.includes('--no-push')
const only = argv.filter((a) => !a.startsWith('--'))

const tags = (
  only.length
    ? only
    : (gh(['release', 'list', '--repo', REPO, '--limit', '100', '--json', 'tagName']).map((r) => r.tagName))
).sort((a, b) => num(b) - num(a)) // 新版本在前，与官网/关于页的顺序一致

/** 读 md，取出要点（`- ` 开头的行），表格部分丢弃后重排 */
function readBullets(file) {
  const raw = fs.readFileSync(file, 'utf8')
  const text = raw.split('\n## 安装包')[0].replace(/^##\s*更新说明\s*\n+/, '')
  return text
    .split('\n')
    .filter((l) => l.trim())
    .map((l) => l.trim().replace(/^(?:\d+\.|[-*+])\s+/, ''))
}

const bulletsByTag = new Map()
const tableByTag = new Map()

for (const tag of tags) {
  const file = path.join(NOTES_DIR, tag + '.md')
  if (!fs.existsSync(file)) {
    console.log(`${tag}  跳过（没有 release-notes/${tag}.md）`)
    continue
  }
  const bullets = readBullets(file)
  if (!bullets.length) {
    console.log(`${tag}  跳过（要点是空的）`)
    continue
  }
  bulletsByTag.set(tag, bullets)

  // ---- ① Release 正文 = 要点 + 安装包表格
  const rows = []
  try {
    const { assets } = gh(['release', 'view', tag, '--repo', REPO, '--json', 'assets'])
    for (const [re, platform, desc] of ORDER) {
      for (const a of assets) {
        if (re.test(a.name)) rows.push(`| ${platform} | ${a.name} | ${desc} | ${human(a.size)} |`)
      }
    }
  } catch {
    console.log(`${tag}  ! 读不到 Release 资产（离线？），保留原有表格`)
  }
  if (rows.length) {
    tableByTag.set(tag, [
      '| 平台 | 文件 | 形态与适用场景 | 大小 |',
      '|---|---|---|---|',
      ...rows
    ])
  }
}

// ---------- ① 写 md（要点 + 表格），再推给 Release ----------
let pushed = 0
for (const [tag, bullets] of bulletsByTag) {
  const file = path.join(NOTES_DIR, tag + '.md')
  const table = tableByTag.get(tag)
  const body = [
    '## 更新说明',
    '',
    ...bullets.map((b) => '- ' + b),
    ...(table ? ['', '## 安装包', '', ...table] : []),
    ''
  ].join('\n')
  fs.writeFileSync(file, body, 'utf8')
  if (noPush) continue
  execFileSync('gh', ['release', 'edit', tag, '--repo', REPO, '--notes-file', file], {
    stdio: ['ignore', 'ignore', 'inherit']
  })
  pushed++
}

// ---------- ② 官网版本区：替换每个条目的要点列表，并把「最新」标挪到最新版 ----------
{
  let html = fs.readFileSync(SITE, 'utf8')
  const newest = [...bulletsByTag.keys()][0]
  let touched = 0
  html = html.replace(/<article class="rel-item reveal">([\s\S]*?)<\/article>/g, (whole, inner) => {
    const tag = (inner.match(/rel-ver">(v[\d.]+)</) || [])[1]
    const bullets = tag && bulletsByTag.get(tag)
    if (!bullets) return whole
    // 要点：`标题：说明` 时把标题加粗（与官网原有观感一致）
    const lis = bullets
      .map((b) => {
        const i = b.indexOf('：')
        const text =
          i > 0 && i <= 14
            ? `<b>${b.slice(0, i)}</b>${b.slice(i)}`
            : `<b>${b}</b>`
        return `            <li>${text}</li>`
      })
      .join('\n')
    let out = inner.replace(/<ul class="rel-notes">[\s\S]*?<\/ul>/, `<ul class="rel-notes">\n${lis}\n          </ul>`)
    // 「最新」标：先全部摘掉，只留最新那一条
    out = out.replace(/\s*<span class="rel-tag tag-latest">[^<]*<\/span>/, '')
    if (tag === newest) {
      out = out.replace(/(<span class="rel-tag tag-(?:patch|minor|major)">[^<]*<\/span>)/, '$1\n            <span class="rel-tag tag-latest">最新</span>')
    }
    touched++
    return `<article class="rel-item reveal">${out}</article>`
  })
  html = html.replace(/<!--\s*(v[\d.]+)（最新）\s*-->/, '<!-- $1 -->')
  html = html.replace('<!-- ' + newest + ' -->', `<!-- ${newest}（最新） -->`)
  fs.writeFileSync(SITE, html, 'utf8')
  console.log(`官网：更新 ${touched} 个版本条目，「最新」在 ${newest}`)
}

// ---------- ③ 应用「关于」页（中文）：relN.noteX ----------
{
  const lines = fs.readFileSync(ZH, 'utf8').split('\n')
  const order = [...bulletsByTag.keys()] // rel1 = 最新
  let changed = 0
  for (let n = 1; n <= order.length; n++) {
    const bullets = bulletsByTag.get(order[n - 1])
    const start = lines.findIndex((l) => l.includes(`'settings.about.rel${n}.version'`))
    if (start < 0) continue
    // 该版本的块 = 紧跟其后的 version/level/date/note 行（这几个键是连着的）
    const blockRe = new RegExp(`'settings\\.about\\.rel${n}\\.(version|level|date|note\\d+)'`)
    let end = start
    while (end + 1 < lines.length && blockRe.test(lines[end + 1])) end++
    const kept = lines.slice(start, end + 1).filter((l) => !/\.note\d+'/.test(l))
    const newNotes = bullets.map(
      (b, k) => `  'settings.about.rel${n}.note${k + 1}': '${b.replace(/'/g, "\\'")}',`
    )
    lines.splice(start, end - start + 1, ...kept, ...newNotes)
    changed++
  }
  fs.writeFileSync(ZH, lines.join('\n'), 'utf8')
  console.log(`应用「关于」页（中文）：更新 ${changed} 个版本的要点`)
}

// 英文那份提醒
const en = fs.readFileSync(path.join(ROOT, 'frontend', 'src', 'locales', 'en-US.js'), 'utf8')
const enCounts = {}
for (const m of en.matchAll(/'settings\.about\.rel(\d+)\.note(\d+)'/g)) {
  enCounts[m[1]] = Math.max(enCounts[m[1]] || 0, Number(m[2]))
}
const mismatch = [...bulletsByTag.entries()].filter(([, b], i) => (enCounts[i + 1] || 0) !== b.length)
console.log(
  mismatch.length
    ? `提醒：英文要点（en-US.js）需要手工同步，条数不一致的版本：${mismatch.map(([t]) => t).join(', ')}`
    : '英文要点条数已与中文一致'
)
console.log(noPush ? '（--no-push：未改动 GitHub）' : `已推送 ${pushed} 个 Release 正文`)
