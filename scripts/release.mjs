#!/usr/bin/env node
/**
 * 发布新版本：一键改版本号 + 追加版本说明骨架。
 *
 * 用法：
 *   node scripts/release.mjs 1.1.0 minor 2026-11-01
 *   node scripts/release.mjs 1.0.2 patch            # 日期默认今天
 *   node scripts/release.mjs 1.1.0 minor --dry-run  # 只看会改什么，不落盘
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
  console.error('用法: node scripts/release.mjs <version> [patch|minor|major] [YYYY-MM-DD] [--dry-run]')
  console.error('示例: node scripts/release.mjs 1.1.0 minor 2026-11-01')
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

// 2) locales：新版本插到 rel1，旧版本顺延
const NOTE_SLOTS = 3
const notes = {
  'zh-CN.js': Array.from({ length: NOTE_SLOTS }, (_, i) => `（待填写：更新要点 ${i + 1}）`),
  'en-US.js': Array.from({ length: NOTE_SLOTS }, (_, i) => `(TODO: highlight ${i + 1})`)
}

for (const [file, noteLines] of Object.entries(notes)) {
  write(`frontend/src/locales/${file}`, (s) => {
    const lines = s.split('\n')
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

console.log(`${dryRun ? '[dry-run] 将修改' : '已修改'}：${version}（${level}，${date}）`)
if (changed.length) changed.forEach((f) => console.log('  · ' + f))
console.log('\n接下来把两处占位说明换成真实更新要点：')
console.log(`  frontend/src/locales/zh-CN.js  → settings.about.rel1.note1..${NOTE_SLOTS}`)
console.log(`  frontend/src/locales/en-US.js  → settings.about.rel1.note1..${NOTE_SLOTS}`)
console.log('\n然后构建：cd frontend && npm run build')
if (dryRun) console.log('\n（dry-run 未写入任何文件）')
