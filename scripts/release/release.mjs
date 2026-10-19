#!/usr/bin/env node
/**
 * 发布新版本：一键改版本号 + 追加版本说明骨架。
 *
 * 用法：
 *   node scripts/release/release.mjs 1.1.0 minor 2026-11-01
 *   node scripts/release/release.mjs 1.0.2 patch            # 日期默认今天
 *   node scripts/release/release.mjs 1.1.0 minor --dry-run  # 只看会改什么，不落盘
 *
 * 参数：
 *   version  目标版本号（不带 v），如 1.1.0
 *   level    版本类型：patch（修 bug/优化，默认）| minor（新功能）| major（大版本）
 *   date     发布日期 YYYY-MM-DD，默认今天
 *
 * 它会做四件事：
 *   1. 同步 Cargo.toml / frontend/package.json / tauri.conf.json 的版本号
 *   2. 在 locales 的 rel1 位置插入新版本组（version/level/date + 3 条待填说明），
 *      原有版本自动顺延为 rel2、rel3…
 *   3. 关于页折叠头显示的版本号同步为最新版本
 *   4. 打印需要你补写的文案键
 *
 * 说明文案写在 frontend/src/locales/zh-CN.js 与 en-US.js 的
 * `settings.about.relN.note1..3`，把占位文字换成真实更新要点即可。
 */
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const argv = process.argv.slice(2).filter((a) => a !== '--dry-run')
const dryRun = process.argv.includes('--dry-run')

const [rawVersion, rawLevel = 'patch', rawDate] = argv
if (!rawVersion || !/^\d+\.\d+\.\d+$/.test(rawVersion)) {
  console.error('用法: node scripts/release/release.mjs <version> [patch|minor|major] [YYYY-MM-DD] [--dry-run]')
  console.error('示例: node scripts/release/release.mjs 1.1.0 minor 2026-11-01')
  process.exit(1)
}
if (!['patch', 'minor', 'major'].includes(rawLevel)) {
  console.error(`level 只能是 patch / minor / major，收到: ${rawLevel}`)
  process.exit(1)
}
const version = rawVersion
const level = rawLevel
const date = /^\d{4}-\d{2}-\d{2}$/.test(rawDate || '')
  ? rawDate
  : new Date().toISOString().slice(0, 10)

const changed = []
const write = (relPath, transform) => {
  const abs = path.join(ROOT, relPath)
  const before = fs.readFileSync(abs, 'utf8')
  const after = transform(before)
  if (after === before) return
  changed.push(relPath)
  if (!dryRun) fs.writeFileSync(abs, after, 'utf8')
}

// 1) 三个版本号文件
write('Cargo.toml', (s) => s.replace(/(\[workspace\.package\][\s\S]*?version = ")[^"]+(")/, `$1${version}$2`))
write('frontend/package.json', (s) => s.replace(/("version": ")[^"]+(")/, `$1${version}$2`))
write('crates/dbmind-desktop/tauri.conf.json', (s) => s.replace(/("version": ")[^"]+(")/, `$1${version}$2`))

// 1b) 还有两处"跟着版本走"的产物，原先靠人记得，漏过：
//   · Cargo.lock —— 各 workspace crate（dbmind-*）的版本号。漏了很隐蔽：Cargo.toml 已是新版本、
//     锁文件里还写着旧版本，要等谁跑一次 cargo 才被悄悄改回来（1.2.7 那次就漏了，事后手工补的）。
//   · macOS 的 DMG 说明书 —— 里面那条 curl 一行命令写死了版本号（要能直接复制粘贴才顺手），
//     说明书是随包发出去的，不跟着改就会指向上一版的文件名。
write('Cargo.lock', (s) => s.replace(/(\[\[package\]\]\nname = "dbmind-[^"]*"\nversion = ")[^"]+(")/g, `$1${version}$2`))
write('resources/mac/首次打开请看这里.txt', (s) => s.replace(/\d+\.\d+\.\d+/g, version))

// 2) locales：新版本插到 rel1，旧版本顺延
const NOTE_SLOTS = 3
const notes = {
  'zh-CN.js': Array.from({ length: NOTE_SLOTS }, (_, i) => `（待填写：更新要点 ${i + 1}）`),
  'en-US.js': Array.from({ length: NOTE_SLOTS }, (_, i) => `(TODO: highlight ${i + 1})`)
}

for (const [file, noteLines] of Object.entries(notes)) {
  write(`frontend/src/locales/${file}`, (s) => {
    // 保留原文件的换行风格：下面按行切分再拼回去，若一律用 '\n' 会把行尾的 \r 吃掉，
    // 让工作区的这两个文件从 CRLF 变成 LF（本仓库 core.autocrlf=true，提交时会被规范化回去，
    // 所以 git 层看不出问题，但本地编辑器里整个文件会显示成「改过了」）。按原风格拼回最省事。
    // 顺带说明一个**常见误解**：发版时这两个文件动辄整片 diff（v1.2.10 是 182/184 行、v1.2.11 是
    // 190/192 行）**不是换行问题**，而是本脚本的设计 —— 新增版本插到 rel1，旧版本整体顺延为
    // rel2、rel3…，于是 rel 段里每一行的键名都变了。要核验内容有没有丢，看两边的键数量是否相等即可。
    const crlf = s.includes('\r\n')
    const eol = crlf ? '\r\n' : '\n'
    const lines = s.split(/\r?\n/)
    const firstIdx = lines.findIndex((l) => /'settings\.about\.rel\d+\.version':/.test(l))
    if (firstIdx < 0) {
      console.error(`  ! ${file} 里找不到 settings.about.relN.version，请手工处理`)
      return s
    }
    // 已有版本数量（含本次新增）
    let maxN = 0
    for (const l of lines) {
      const m = l.match(/'settings\.about\.rel(\d+)\.version':/)
      if (m) maxN = Math.max(maxN, Number(m[1]))
    }
    // 旧的 relN → relN+1（倒序，避免连锁覆盖）
    for (let n = maxN; n >= 1; n--) {
      for (let i = 0; i < lines.length; i++) {
        if (lines[i].includes(`settings.about.rel${n}.`)) {
          lines[i] = lines[i].replace(`settings.about.rel${n}.`, `settings.about.rel${n + 1}.`)
        }
      }
    }
    // 新版本块插到原 rel1（现已变成 rel2）位置之前
    const block = [
      `  'settings.about.rel1.version': 'v${version}',`,
      `  'settings.about.rel1.level': '${level}',`,
      `  'settings.about.rel1.date': '${date}',`,
      ...noteLines.map((t, i) => `  'settings.about.rel1.note${i + 1}': '${t}',`)
    ]
    const insertAt = lines.findIndex((l) => /'settings\.about\.rel2\.version':/.test(l))
    lines.splice(insertAt < 0 ? firstIdx : insertAt, 0, ...block)
    // 折叠头显示的版本号 = 最新版本
    return lines
      .map((l) =>
        /'settings\.about\.releaseVersion':/.test(l)
          ? `  'settings.about.releaseVersion': 'v${version}',`
          : l
      )
      .join('\n')
  })
}

// 3) 官网：版本区要跟着加条目（源在主仓 website/index.html，发布用 scripts/release/publish-site.ps1）
//    这一步曾经整段被漏掉 —— 官网一连落后 8 个版本（用户反馈"官网版本内容都没更新"），
//    所以这里做一次检查并明确提示，而不是只在文档里写一句。
const site = path.join(ROOT, 'website/index.html')
if (fs.existsSync(site) && !fs.readFileSync(site, 'utf8').includes(`>v${version}<`)) {
  console.log(`\n注意：官网 website/index.html 里还没出现 v${version} —— 记得`)
  console.log('      ① 在版本区顶部加一条（并把「最新」标从上一版挪过来）；')
  console.log('      ② 跑 scripts\\publish-site.ps1 推到 gh-pages，线上才会更新。')
}

console.log(`${dryRun ? '[dry-run] 将修改' : '已修改'}：${version}（${level}，${date}）`)
if (changed.length) changed.forEach((f) => console.log('  · ' + f))
console.log('\n接下来把两处占位说明换成真实更新要点：')
console.log(`  frontend/src/locales/zh-CN.js  → settings.about.rel1.note1..${NOTE_SLOTS}`)
console.log(`  frontend/src/locales/en-US.js  → settings.about.rel1.note1..${NOTE_SLOTS}`)
console.log('\n然后构建：cd frontend && npm run build')

// 4) GitHub Release 的**标题只写版本号**（用户要求）。
//    v1.2.11 那次我手动 gh release edit 时传了 --title 'v1.2.11：修复…'，于是在 Release
//    列表里只有它是一行长句、其余 18 个都是纯版本号，看着很乱。描述写进正文第一行的 H1
//    （release-notes/v<版本>.md 的第一行）就够了，标题不必重复。
//    注意 CI 那条路本来就没问题：action-gh-release 不传 name 时默认用 tag 当标题；
//    只有手动改的时候才要当心 —— 别传 --title，或显式传 --title v<版本>。
console.log(`\n注意：Release 标题只写版本号（v${version}）`)
console.log('      手动 gh release edit 时不要传 --title 描述；' +
  `要改就 --title v${version}。`)
console.log('      正文照 release-notes/TEMPLATE.md 写：只写「更新了什么」的要点（- 一句 一行），')
console.log('      然后跑 node scripts/release/sync-release-notes.mjs <版本> —— 它一次同步三处：')
console.log('      Release 正文、官网版本区、应用「关于」页要点（标题与安装包表格都由它生成）。')
if (dryRun) console.log('\n（dry-run 未写入任何文件）')
