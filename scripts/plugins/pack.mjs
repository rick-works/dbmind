#!/usr/bin/env node
// 把 plugins/skills/<id>/ 打成官网分发的插件包，并重建官网索引。
//
//   node scripts/plugins/pack.mjs            # 打包 + 写 index.json
//   node scripts/plugins/pack.mjs --check    # 只校验（CI/发版前用，发现索引过期就退出 1）
//
// 为什么不引 zip 依赖：插件包就是几个文本文件，用 store（不压缩）的 zip 就够，
// 自己写 60 行既快又**结果确定**（同样输入永远同样的字节）——这样 index.json 里的
// sha256 才是稳定的，不会因为打一次包就变一次。
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import { fileURLToPath } from 'node:url'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const SRC = path.join(ROOT, 'plugins', 'skills')
const OUT = path.join(ROOT, 'website', 'plugins', 'packages')
const INDEX = path.join(ROOT, 'website', 'plugins', 'index.json')
const BASE = 'https://www.dbmind.top/plugins/packages'
// 固定时间戳：zip 里每个条目的 mtime 都写这个（1980-01-01 是 zip 能表示的最早时间）
const DOS_TIME = 0
const DOS_DATE = (0 << 9) | (1 << 5) | 1

const crcTable = (() => {
  const table = new Int32Array(256)
  for (let i = 0; i < 256; i++) {
    let c = i
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    table[i] = c
  }
  return table
})()

const crc32 = (buf) => {
  let c = -1
  for (const byte of buf) c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8)
  return (c ^ -1) >>> 0
}

/** store-only zip（不压缩：插件体积本来就小，换来的是字节级确定性） */
function zip(entries) {
  const locals = []
  const centrals = []
  let offset = 0
  for (const { name, data } of entries) {
    const nameBuf = Buffer.from(name, 'utf8')
    const crc = crc32(data)
    const local = Buffer.alloc(30)
    local.writeUInt32LE(0x04034b50, 0)
    local.writeUInt16LE(20, 4) // version needed
    local.writeUInt16LE(0, 6) // flags
    local.writeUInt16LE(0, 8) // method: store
    local.writeUInt16LE(DOS_TIME, 10)
    local.writeUInt16LE(DOS_DATE, 12)
    local.writeUInt32LE(crc, 14)
    local.writeUInt32LE(data.length, 18)
    local.writeUInt32LE(data.length, 22)
    local.writeUInt16LE(nameBuf.length, 26)
    local.writeUInt16LE(0, 28)
    locals.push(local, nameBuf, data)

    const central = Buffer.alloc(46)
    central.writeUInt32LE(0x02014b50, 0)
    central.writeUInt16LE(20, 4) // version made by
    central.writeUInt16LE(20, 6) // version needed
    central.writeUInt16LE(0, 8)
    central.writeUInt16LE(0, 10)
    central.writeUInt16LE(DOS_TIME, 12)
    central.writeUInt16LE(DOS_DATE, 14)
    central.writeUInt32LE(crc, 16)
    central.writeUInt32LE(data.length, 20)
    central.writeUInt32LE(data.length, 24)
    central.writeUInt16LE(nameBuf.length, 28)
    central.writeUInt16LE(0, 30)
    central.writeUInt16LE(0, 32)
    central.writeUInt16LE(0, 34)
    central.writeUInt16LE(0, 36)
    central.writeUInt32LE(0, 38)
    central.writeUInt32LE(offset, 42)
    centrals.push(central, nameBuf)

    offset += local.length + nameBuf.length + data.length
  }
  const centralBuf = Buffer.concat(centrals)
  const eocd = Buffer.alloc(22)
  eocd.writeUInt32LE(0x06054b50, 0)
  eocd.writeUInt16LE(entries.length, 8)
  eocd.writeUInt16LE(entries.length, 10)
  eocd.writeUInt32LE(centralBuf.length, 12)
  eocd.writeUInt32LE(offset, 16)
  return Buffer.concat([...locals, centralBuf, eocd])
}

/** 目录下的文件按相对路径排序收集（保证包内顺序确定） */
function walk(dir, base = dir) {
  const out = []
  for (const name of fs.readdirSync(dir).sort()) {
    const full = path.join(dir, name)
    const stat = fs.statSync(full)
    if (stat.isDirectory()) out.push(...walk(full, base))
    else out.push({ name: path.relative(base, full).split(path.sep).join('/'), data: fs.readFileSync(full) })
  }
  return out
}

const pick = (v, lang) => (v && typeof v === 'object' ? v[lang] || v.zh || v.en || '' : v || '')

const checkOnly = process.argv.includes('--check')
const plugins = []
const packages = []

// ---- 清单规范（标准化）----
// 官网分发的每个包都必须满足这些约束：少一项就直接拒绝打包，而不是等用户装上去才发现
// 界面缺图标 / 名称只有一种语言 / 版本号没法比较大小。
const REQUIRED_FIELDS = ['id', 'type', 'name', 'version', 'description', 'icon', 'prompt', 'chatPrompt']
const fail = (id, msg) => {
  console.error(`✗ ${id}: ${msg}`)
  process.exit(1)
}

for (const id of fs.readdirSync(SRC).sort()) {
  const dir = path.join(SRC, id)
  if (!fs.statSync(dir).isDirectory()) continue
  const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8'))
  if (manifest.id !== id) {
    console.error(`✗ ${id}: 目录名与 manifest.id「${manifest.id}」不一致`)
    process.exit(1)
  }
  for (const key of REQUIRED_FIELDS) {
    if (!manifest[key]) fail(id, `manifest 缺少必填字段 ${key}`)
  }
  if (manifest.schemaVersion !== 1) fail(id, `schemaVersion 必须是 1（当前 ${manifest.schemaVersion}）`)
  if (!/^\d+\.\d+\.\d+$/.test(manifest.version)) {
    fail(id, `version 必须是 x.y.z（当前「${manifest.version}」），否则前端没法比较新旧`)
  }
  for (const key of ['name', 'description']) {
    const v = manifest[key]
    if (!v || typeof v !== 'object' || !v.zh || !v.en) fail(id, `${key} 必须是 { zh, en } 双语对象`)
  }
  // emoji 在 JS 里按码点算：单个图标 1~2 个码点（带变体选择符/零宽连接符的会更多一点）
  if ([...String(manifest.icon)].length > 4) fail(id, `icon 应是 1~2 个 emoji（当前「${manifest.icon}」）`)
  if (manifest.inputs) fail(id, 'inputs 已废弃：技能只在 AI 对话里运行，请从 manifest 里删掉')
  if (manifest.type !== 'skill' || !manifest.prompt) {
    console.error(`✗ ${id}: 只支持 skill 类型且必须声明 prompt`)
    process.exit(1)
  }
  if (!fs.existsSync(path.join(dir, manifest.prompt))) {
    console.error(`✗ ${id}: 缺少提示词文件 ${manifest.prompt}`)
    process.exit(1)
  }
  // 对话形态的提示词（可选）：后端会优先用它，缺文件会让安装直接被拒
  if (manifest.chatPrompt && !fs.existsSync(path.join(dir, manifest.chatPrompt))) {
    console.error(`✗ ${id}: 缺少对话提示词文件 ${manifest.chatPrompt}`)
    process.exit(1)
  }
  const buf = zip(walk(dir))
  const file = `${id}-${manifest.version}.zip`
  const target = path.join(OUT, file)
  const changed = !fs.existsSync(target) || !fs.readFileSync(target).equals(buf)
  if (!checkOnly && changed) {
    fs.mkdirSync(OUT, { recursive: true })
    fs.writeFileSync(target, buf)
  }
  if (checkOnly && changed) {
    console.error(`✗ ${file} 与源码不一致（忘了跑 node scripts/plugins/pack.mjs）`)
    process.exit(1)
  }
  // 版本升级后旧包不再被索引引用：留在官网只会让用户从缓存里装回旧版
  if (!checkOnly && fs.existsSync(OUT)) {
    for (const old of fs.readdirSync(OUT)) {
      if (old.startsWith(`${id}-`) && old.endsWith('.zip') && old !== file) {
        fs.unlinkSync(path.join(OUT, old))
        console.log(`  − 删除旧包 ${old}`)
      }
    }
  }
  console.log(`${changed ? '✎' : '='} ${file}  ${(buf.length / 1024).toFixed(1)} KB`)
  packages.push({ file, removed: false })
  plugins.push({
    id: manifest.id,
    type: manifest.type,
    name: manifest.name,
    version: manifest.version,
    author: manifest.author || '',
    description: manifest.description || '',
    icon: manifest.icon || '',
    homepage: manifest.homepage || '',
    needsContext: !!manifest.needsContext,
    sha256: crypto.createHash('sha256').update(buf).digest('hex'),
    size: buf.length,
    download: `${BASE}/${file}`
  })
}

const index = {
  schemaVersion: 1,
  updatedAt: new Date().toISOString().slice(0, 10),
  plugins
}
const text = `${JSON.stringify(index, null, 2)}\n`
if (checkOnly) {
  const existing = fs.existsSync(INDEX) ? fs.readFileSync(INDEX, 'utf8') : ''
  if (existing !== text) {
    console.error('✗ website/plugins/index.json 与插件源码不一致')
    process.exit(1)
  }
  console.log(`✓ 索引与 ${plugins.length} 个插件包一致`)
} else {
  fs.mkdirSync(path.dirname(INDEX), { recursive: true })
  fs.writeFileSync(INDEX, text)
  console.log(`✓ 索引已更新：${plugins.length} 个插件 → ${path.relative(ROOT, INDEX)}`)
}
