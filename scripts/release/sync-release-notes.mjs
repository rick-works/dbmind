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
 *   node scripts/release/sync-release-notes.mjs                 # 全部版本：写文件 + 推 Release
 *   node scripts/release/sync-release-notes.mjs v1.2.12         # 只处理指定版本
 *   node scripts/release/sync-release-notes.mjs --no-push       # 只改本地文件，不动 GitHub
 *
 * 关于「关于」页的**英文**要点：中文从 `release-notes/<版本>.md` 自动取；英文另有一份
 * 翻译 `release-notes/en-notes.json`（键是版本号、值是该版本的英文要点数组），它会被同步进
 * `en-US.js`。新增版本时在那边补一段即可；缺翻译或条数与中文不一致时脚本会点名提醒。
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
const EN = path.join(ROOT, 'frontend', 'src', 'locales', 'en-US.js')

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

/**
 * md 的行内装饰 → 纯文本。
 *
 * md 是给 GitHub 看的（那边 `**粗**` 有意义），但它同时是官网 HTML 与应用「关于」页的源。
 * 那两处一个是 HTML、一个是 Vue 纯文本插值：星号/反引号落进去就是**字面符号**
 * （踩过：官网条目里出现 `**大树不再卡**`，关于页显示 `**一批稳定性修复**`）。
 */
const mdPlain = (s) => String(s).replace(/\*\*/g, '').replace(/`/g, '')

/**
 * 要点 → 官网 `<li>` 列表。
 *
 * `标题：说明` 时把标题加粗（与官网原有观感一致）。先剥 md 装饰：md 是给 GitHub 看的
 * （`**粗**` 有意义），而这一层是 **HTML** —— 星号会原样显示（曾把 `**大树不再卡**` 包进
 * `<b>`，线上就是字面星号）。反引号同理（HTML 里要有样式才有意义，这里按纯文本处理）。
 */
const noteLis = (bullets) =>
  bullets
    .map((b) => {
      const t = mdPlain(b)
      const i = t.indexOf('：')
      const text = i > 0 && i <= 14 ? `<b>${t.slice(0, i)}</b>${t.slice(i)}` : `<b>${t}</b>`
      return `            <li>${text}</li>`
    })
    .join('\n')

/** 版本类型 → 官网标签文字（与既有条目口径一致） */
const LEVEL_TEXT = { patch: '修订版', minor: '小版本', major: '大版本' }

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
    const lis = noteLis(bullets)
    const out = inner.replace(/<ul class="rel-notes">[\s\S]*?<\/ul>/, `<ul class="rel-notes">\n${lis}\n          </ul>`)
    touched++
    return `<article class="rel-item reveal">${out}</article>`
  })
  // 新版本在官网里**还没有条目** → 直接建一条插到时间线最上面。
  //
  // 原先这里只更新"已存在"的条目，新增版本得靠 release.mjs 的**提醒**去手工加
  // （提示原文就是"记得① 在版本区顶部加一条"）—— 而"记得"这种事已经漏过两次：官网一连
  // 落后 8 个版本、v1.2.20 又是只推了源没推分支。要点、类型、日期都在仓库里，没有理由靠记性。
  if (!new RegExp(`rel-ver">${newest}<`).test(html)) {
    const zh = fs.readFileSync(ZH, 'utf8')
    const pick = (key) =>
      (zh.match(new RegExp(`'settings\\.about\\.rel1\\.${key}': '([^']*)'`)) || [])[1] || ''
    const level = pick('level') || 'patch'
    const item = [
      `      <!-- ${newest}（最新） -->`,
      '      <article class="rel-item reveal">',
      '        <div class="rel-rail"><span class="rel-dot"></span></div>',
      '        <div class="rel-body">',
      '          <div class="rel-head">',
      `            <span class="rel-ver">${newest}</span>`,
      `            <span class="rel-tag tag-${level}">${LEVEL_TEXT[level] || LEVEL_TEXT.patch}</span>`,
      '            <span class="rel-tag tag-latest">最新</span>',
      `            <span class="rel-date">${pick('date')}</span>`,
      '          </div>',
      '          <ul class="rel-notes">',
      noteLis(bulletsByTag.get(newest) || []),
      '          </ul>',
      '          <div class="rel-links">',
      `            <a class="btn btn-primary" href="https://github.com/${REPO}/releases/tag/${newest}" target="_blank" rel="noopener">发布说明</a>`,
      `            <a class="btn btn-ghost" href="https://github.com/${REPO}/releases/tag/${newest}" target="_blank" rel="noopener">下载 ${newest}</a>`,
      '          </div>',
      '        </div>',
      '      </article>',
      '',
    ].join('\n')
    // 锚点要容忍 CRLF：文件在 Windows 上是 \r\n，写死 \n 会**匹配不上** —— 而日志照旧会喊
    // "已自动新建"（因为它只看 touched），线上却什么都没变（踩过）。
    const eol = html.includes('\r\n') ? '\r\n' : '\n'
    html = html.replace(/(<div class="rel-timeline">\r?\n)/, `$1${item.replace(/\n/g, eol)}`)
    touched++
    console.log(`官网：${newest} 原先没有条目，已自动新建（要点 ${(bulletsByTag.get(newest) || []).length} 条）`)
  }
  // 「最新」标**全局只留一个**：先全摘掉，再给最新那条加上。
  // 不能只在上面那个循环里做 —— 单版本同步（`sync-release-notes.mjs v1.2.21`）时循环只认这一个
  // 版本，其它条目原样返回，上一版的「最新」就留下来了（踩过：官网同时出现两个「最新」）。
  html = html.replace(/\s*<span class="rel-tag tag-latest">[^<]*<\/span>/g, '')
  html = html.replace(
    new RegExp(
      `(<span class="rel-ver">${newest}</span>[\\s\\S]*?<span class="rel-tag tag-(?:patch|minor|major)">[^<]*</span>)`,
    ),
    '$1\n            <span class="rel-tag tag-latest">最新</span>',
  )
  html = html.replace(/<!--\s*(v[\d.]+)（最新）\s*-->/, '<!-- $1 -->')
  html = html.replace('<!-- ' + newest + ' -->', `<!-- ${newest}（最新） -->`)
  fs.writeFileSync(SITE, html, 'utf8')
  console.log(`官网：更新 ${touched} 个版本条目，「最新」在 ${newest}`)
}

// ---------- ③ 应用「关于」页（中文）：relN.noteX ----------
{
  let lines = fs.readFileSync(ZH, 'utf8').split('\n')
  const order = [...bulletsByTag.keys()] // rel1 = 最新
  // 槽位**按版本号反查**，不要按顺序去数：`order` 来自"有 md 的版本"，中间某版本缺 md、
  // 或只同步某一个版本时，顺序就会与词典里的 relN 错位 —— 结果是 A 版本的要点写进 B 版本的槽位
  // （清旧行时也会清错版本）。用 `relN.version` 建"版本 → 槽位"映射，就与顺序无关了。
  const slotOf = (() => {
    const map = {}
    const re = /'settings\.about\.rel(\d+)\.version': '([^']+)'/g
    let m
    while ((m = re.exec(lines.join('\n')))) map[m[2]] = Number(m[1])
    return map
  })()
  let changed = 0
  for (const tag of order) {
    const n = slotOf[tag]
    if (!n) continue
    const bullets = bulletsByTag.get(tag)
    // 先清掉**这个版本**散落各处的旧 note 行，再插到 version/level/date 之后。
    //
    // 为什么不能只替换"紧跟 version 的那个连续块"：历史上要点块被手工搬过家（发版时把 relN 的
    // version/level/date 整体下移，而 note 行留在原处），于是同一个版本的 note 行散落在别处 ——
    // 只替换连续块会漏掉它们，结果是**重复键**（guard 报"重复键 99 个"，页面上一套新一套旧）。
    //
    // 也不能"整体清空再重插"：本脚本支持只同步某一个版本（`sync-release-notes.mjs v1.2.21`，
    // 文档里的常规用法），整体清空会把**其它版本**的要点一起删掉（踩过：zh 的 rel2/rel3 要点被清成 0）。
    const noteRe = new RegExp(`^\\s*'settings\\.about\\.rel${n}\\.note\\d+'`)
    for (let k = lines.length - 1; k >= 0; k--) if (noteRe.test(lines[k])) lines.splice(k, 1)
    const start = lines.findIndex((l) => l.includes(`'settings.about.rel${n}.version'`))
    if (start < 0) continue
    // 该版本的块 = 紧跟其后的 version/level/date 行
    const blockRe = new RegExp(`'settings\\.about\\.rel${n}\\.(version|level|date)'`)
    let end = start
    while (end + 1 < lines.length && blockRe.test(lines[end + 1])) end++
    // 关于页是**纯文本**（Vue 直接插值，不渲染 HTML），所以剥掉 md 装饰：
    // `**加粗**` 会显示字面星号，反引号也没有对应样式。
    const newNotes = bullets.map(
      (b, k) => `  'settings.about.rel${n}.note${k + 1}': '${mdPlain(b).replace(/'/g, "\\'")}',`
    )
    lines.splice(end + 1, 0, ...newNotes)
    changed++
  }
  fs.writeFileSync(ZH, lines.join('\n'), 'utf8')
  console.log(`应用「关于」页（中文）：更新 ${changed} 个版本的要点`)
}

// ---------- ④ 应用「关于」页（英文）：从 release-notes/en-notes.json 同步 ----------
{
  const src = path.join(NOTES_DIR, 'en-notes.json')
  if (!fs.existsSync(src)) {
    console.log('应用「关于」页（英文）：release-notes/en-notes.json 不存在，跳过')
  } else {
    const map = JSON.parse(fs.readFileSync(src, 'utf8'))
    const lines = fs.readFileSync(EN, 'utf8').split('\n')
    const order = [...bulletsByTag.keys()]
    const missing = []
    // 槽位按版本号反查（同 ③）：与 order 的顺序无关
    const slotOf = (() => {
      const m2 = {}
      const re = /'settings\.about\.rel(\d+)\.version': '([^']+)'/g
      let m
      while ((m = re.exec(lines.join('\n')))) m2[m[2]] = Number(m[1])
      return m2
    })()
    let changed = 0
    for (const tag of order) {
      const n = slotOf[tag]
      const notes = map[tag]
      if (!n) {
        missing.push(`${tag}(词典里没有它的槽位)`)
        continue
      }
      if (!Array.isArray(notes) || !notes.length) {
        missing.push(tag)
        continue
      }
      // 先清掉**这个版本**散落各处的旧 note 行（原因见 ③ 的注释：块不保证连续，
      // 只替换连续块会留下重复键）。这里按版本清、不整体清：某个版本缺英文翻译时
      // 上面的 `continue` 会跳过它，整体清会把它的英文要点抹掉。
      const noteRe = new RegExp(`^\\s*'settings\\.about\\.rel${n}\\.note\\d+'`)
      for (let k = lines.length - 1; k >= 0; k--) if (noteRe.test(lines[k])) lines.splice(k, 1)
      const start = lines.findIndex((l) => l.includes(`'settings.about.rel${n}.version'`))
      if (start < 0) continue
      const blockRe = new RegExp(`'settings\\.about\\.rel${n}\\.(version|level|date)'`)
      let end = start
      while (end + 1 < lines.length && blockRe.test(lines[end + 1])) end++
      const kept = lines.slice(start, end + 1)
      const newNotes = notes.map(
        (b, k) => `  'settings.about.rel${n}.note${k + 1}': '${mdPlain(b).replace(/'/g, "\\'")}',`
      )
      lines.splice(start, end - start + 1, ...kept, ...newNotes)
      changed++
      if (notes.length !== bulletsByTag.get(tag).length) {
        missing.push(`${tag}(条数 ${notes.length} vs 中文 ${bulletsByTag.get(tag).length})`)
      }
    }
    fs.writeFileSync(EN, lines.join('\n'), 'utf8')
    console.log(
      `应用「关于」页（英文）：更新 ${changed} 个版本的要点` +
        (missing.length ? `\n注意：以下版本英文待补或条数与中文不一致 —— ${missing.join('、')}` : '')
    )
  }
}

// ---------- ⑤ 「当前版本」+ 一致性提醒 ----------
//
// 这里处理的是发版最容易漏的两处（v1.2.20 就漏了第一处，用户在「关于」页看出来了）：
//  ① `settings.about.releaseVersion` —— 折叠头那个"当前版本"，**必须跟着最新 tag 走**；
//  ② `relN.version / level / date` 三行 —— ③ 只重写要点（noteX），**不搬这三行**，
//     所以新增版本时要手工把这三行**整体下移一格**、并在最前面插入新版本的三行。
//     没搬的后果：页面顶部写着新版号、列表第一格还挂着旧版号。
//
// 第 ② 条**只提醒、不自动改**：自动挪位一旦判断错（比如重复跑、或中间漏过一版），
// 会把 27 个版本的槽位整体错位 —— 那比漏改一处严重得多。宁可让它吵一句。
{
  const newest = tags[0]
  for (const [file, name] of [
    [ZH, '中文'],
    [EN, '英文'],
  ]) {
    const lines = fs.readFileSync(file, 'utf8').split('\n')
    const i = lines.findIndex((l) => l.includes("'settings.about.releaseVersion'"))
    if (i >= 0 && !lines[i].includes(`'${newest}'`)) {
      lines[i] = `  'settings.about.releaseVersion': '${newest}',`
      fs.writeFileSync(file, lines.join('\n'), 'utf8')
      console.log(`应用「关于」页（${name}）：当前版本 → ${newest}`)
    }
    const first = lines.find((l) => l.includes("'settings.about.rel1.version'"))
    if (first && !first.includes(`'${newest}'`)) {
      console.log(
        `⚠ 应用「关于」页（${name}）：rel1.version 仍是 ${first.trim()} —— ` +
          `请把 relN 的 version/level/date 三行整体下移一格，并在最前面插入 ${newest} 的三行`
      )
    }
  }
}

// ---------- ⑥ 把官网推到 gh-pages ----------
//
// **改好源文件 ≠ 线上更新**：官网线上是 `gh-pages` 分支，必须跑一次
// `scripts/release/publish-site.ps1`（在 worktree 里换掉 index.html → commit → push）。
// 这一步被整段漏掉过两次：官网一连落后 8 个版本（publish-site.ps1 的注释里就记着用户那句
// "官网版本内容都没更新"），v1.2.20 又是只改了源、没推分支，用户在官网版本页又看出来了。
// 所以不再"只提醒"——直接在这里执行：本脚本跑完，Release / 官网 / 关于页三处就该都到位。
if (noPush) {
  console.log('（--no-push：跳过官网发布）')
} else {
  const ps1 = path.join(ROOT, 'scripts', 'publish-site.ps1')
  if (!fs.existsSync(ps1)) {
    console.log('⚠ 没找到 scripts/release/publish-site.ps1 —— 官网需要手动发布到 gh-pages')
  } else {
    // PowerShell 的可执行名按平台不同：Windows 是 `powershell`，mac/Linux 是 `pwsh`。
    // 写死 `powershell` 的话在 mac/Linux 上发版会 ENOENT → 被下面 catch 吞掉 → **官网永远不更新
    // 且没有任何醒目的提示**（正是"改好源≠线上更新"那类漏项）。
    const shell = process.platform === 'win32' ? 'powershell' : 'pwsh'
    try {
      execFileSync(
        shell,
        ['-ExecutionPolicy', 'Bypass', '-File', ps1, '-Message', `官网：加 v${tags[0].replace(/^v/, '')} 版本条目`],
        { cwd: ROOT, stdio: 'inherit' },
      )
    } catch (err) {
      // 发布失败不该把同步脚本判为失败（本地文件与 Release 正文都已经好了），
      // 但必须**大声**说清楚"线上还没更新"，并给出可重复执行的补救命令与真实原因。
      console.error(`\n⚠⚠ 官网没有发布成功：${err && err.message ? err.message : err}`)
      console.error('   线上仍是旧内容。请手动执行（可重复跑，不会叠加）：')
      console.error('   powershell -ExecutionPolicy Bypass -File scripts\\publish-site.ps1\n')
    }
  }
}

console.log(noPush ? '（--no-push：未改动 GitHub）' : `已推送 ${pushed} 个 Release 正文`)
