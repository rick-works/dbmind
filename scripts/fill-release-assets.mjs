#!/usr/bin/env node
/**
 * 给各版本的「发版说明」补上安装包表格 —— 也就是 Release 正文里那份"文件的说明"。
 *
 * 为什么要有它：这张表（哪个文件是干什么的、多大）以前是手写的，每次发版照产物敲一遍文件名和
 * 体积，容易漏平台、也容易抄错。改成从 Release API 读**真实资产**之后：哪个版本实际发了什么就
 * 列什么（早期版本只有两三个包，表格自然就短），体积也不必再手抄。
 *
 * 用法（需要 GH_TOKEN；在哪个目录下跑都行）：
 *   node scripts/fill-release-assets.mjs                # 处理全部 Release
 *   node scripts/fill-release-assets.mjs v1.2.12        # 只处理指定版本
 *   node scripts/fill-release-assets.mjs --dry-run      # 只看会写成什么，不落盘、不推送
 *
 * 它会就地改写 release-notes/<版本>.md，并用该文件更新对应 Release 的正文。
 * **幂等**：要点之后的内容整体重写，所以重复跑不会叠出两张表。
 *
 * 正文结构（与 release-notes/TEMPLATE.md 配套）：
 *   - 要点若干条（人工写，只讲更新了什么）
 *
 *   ## 安装包
 *   | 平台 | 文件 | 形态与适用场景 | 大小 |     ← 本脚本生成
 */
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const REPO = 'rick-works/dbmind'
const NOTES_DIR = path.join(ROOT, 'release-notes')

// 顺序固定：Windows → macOS → Linux。形态说明沿用既有的口径。
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

const argv = process.argv.slice(2)
const dryRun = argv.includes('--dry-run')
const only = argv.filter((a) => !a.startsWith('--'))

const tags = only.length
  ? only
  : gh(['release', 'list', '--repo', REPO, '--limit', '100', '--json', 'tagName']).map((r) => r.tagName)

let done = 0
for (const tag of tags) {
  const file = path.join(NOTES_DIR, tag + '.md')
  if (!fs.existsSync(file)) {
    console.log(`${tag}  跳过（release-notes/${tag}.md 不存在）`)
    continue
  }
  // 幂等：切掉上一次可能生成的表格，只留人工写的要点
  const bullets = fs.readFileSync(file, 'utf8').split('\n## 安装包')[0].trimEnd()

  const { assets } = gh(['release', 'view', tag, '--repo', REPO, '--json', 'assets'])
  const rows = []
  for (const [re, platform, desc] of ORDER) {
    for (const a of assets) {
      if (re.test(a.name)) rows.push(`| ${platform} | ${a.name} | ${desc} | ${human(a.size)} |`)
    }
  }
  if (!rows.length) {
    console.log(`${tag}  跳过（这个版本还没有安装包资产）`)
    continue
  }

  const body = [
    bullets,
    '',
    '## 安装包',
    '',
    '| 平台 | 文件 | 形态与适用场景 | 大小 |',
    '|---|---|---|---|',
    ...rows,
    '',
    '- 每个包都自带精简 Java 运行时（目标机器不必另装 Java）。',
    ''
  ].join('\n')

  if (dryRun) {
    console.log(`--- ${tag}（${rows.length} 个包，${body.length} 字符，未写入）`)
    continue
  }
  fs.writeFileSync(file, body, 'utf8')
  execFileSync('gh', ['release', 'edit', tag, '--repo', REPO, '--notes-file', file], {
    stdio: ['ignore', 'ignore', 'inherit']
  })
  console.log(`${tag}  ${rows.length} 个包，正文 ${body.length} 字符`)
  done++
}
console.log(dryRun ? '（dry-run，未改动任何文件）' : `完成：${done} 个版本`)
