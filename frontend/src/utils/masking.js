// 字段脱敏的**前端**实现：与后端 `dbmind_core::mask` 同算法，只服务一个出口 ——
// 结果表格（grid）。发给大模型的内容与 MCP 返回的数据都在后端打码；表格的数据本来就
// 在本地，没必要为了显示再往返一次后端。
//
// 为什么必须同算法：用户在设置里给 `phone` 选了 MD5，AI 上下文里看到的是
// `900150983cd24fb0d6963f7d28e17f72`，表格里却是另一串 —— 那就不是「同一份规则」，
// 而是两套实现，迟早对不上（比如想在表格里找出 AI 刚提到的那串摘要）。
//
// 三条边界与后端一字不差：
// 1. 空值保持空（NULL / 空串没有信息可泄露，写成 `***` 反而误导读表的人）；
// 2. **只打「显示」这一层**：复制、导出、就地编辑拿到的仍是原文；
// 3. 表名 / 列名 / 注释不脱敏，SQL 里手写的字面量也不脱敏。

import { getMaskRules } from '../api'

// ---------------------------------------------------------------- 配置缓存

/** 最近一次拿到的配置（`null` = 还没取过） */
let config = null
/** 正在取的那一次（并发调用共用，避免一屏好几个表格各发一次请求） */
let loading = null

/** 设置页保存 / 读取后调用：界面上的开关与表格里的打码立刻一致 */
export function setMaskConfig(data) {
  config = {
    llm: !!data?.llm,
    mcp: !!data?.mcp,
    grid: !!data?.grid,
    export: !!data?.export,
    aesKey: data?.aesKey || '',
    rules: Array.isArray(data?.rules) ? data.rules : []
  }
  return config
}

/** 同步读缓存（渲染时用；没取过就是 null，表格按「不打码」处理） */
export function peekMaskConfig() {
  return config
}

/** 取配置：有缓存直接用，没有就请求一次（失败按「没配规则」处理，不让表格打不开） */
export async function ensureMaskConfig() {
  if (config) return config
  if (!loading) {
    loading = getMaskRules()
      .then((data) => setMaskConfig(data))
      .catch(() => null)
      .finally(() => {
        loading = null
      })
  }
  return loading
}

// ---------------------------------------------------------------- 规则匹配

const trim = (value) => String(value == null ? '' : value).trim()
const sameName = (a, b) => trim(a).toLowerCase() === trim(b).toLowerCase()

/** 列 / 表的通配匹配：`*`、`*后缀`、`前缀*`、等值（与后端 `pattern_match` 同口径） */
function patternMatch(pattern, name) {
  const want = trim(pattern).toLowerCase()
  if (!want || want === '*') return true
  const got = trim(name).toLowerCase()
  if (want.startsWith('*')) {
    const suffix = want.slice(1)
    return !!suffix && got.endsWith(suffix)
  }
  if (want.endsWith('*')) {
    const prefix = want.slice(0, -1)
    return !!prefix && got.startsWith(prefix)
  }
  return want === got
}

/** `库.模式` 拆开：两边都非空才拆（与后端 `split_db` 一致） */
function splitDb(database) {
  const text = trim(database)
  const cut = text.indexOf('.')
  if (cut > 0 && cut < text.length - 1) {
    const db = text.slice(0, cut).trim()
    const schema = text.slice(cut + 1).trim()
    if (db && schema) return { db, schema }
  }
  return { db: text, schema: '' }
}

/** 一条规则是否命中「某连接 / 某库（可带模式）/ 某表（可未知）的某一列」 */
function ruleHits(rule, connectionId, database, table, column) {
  const wantConn = trim(rule.connectionId)
  if (wantConn && !sameName(wantConn, connectionId)) return false

  const got = splitDb(database)
  const wantDbText = trim(rule.database)
  if (wantDbText) {
    const want = splitDb(wantDbText)
    if (!sameName(want.db, got.db)) return false
    // 规则把模式写在库名里（`demo.dbo`）：调用方也必须带着模式
    if (want.schema && !(got.schema && sameName(want.schema, got.schema))) return false
  }
  const wantSchema = trim(rule.schema)
  if (wantSchema && !(got.schema && sameName(wantSchema, got.schema))) return false

  const wantTable = trim(rule.table)
  if (wantTable) {
    // 规则钉了表：这次也知道表名 → 比表名；不知道表名 → 不命中
    if (!table || !patternMatch(wantTable, table)) return false
  }
  return patternMatch(rule.column, column)
}

/** 某个出口的开关是否打开 */
export function channelOn(channel, data = config) {
  if (!data) return false
  return channel === 'mcp' ? data.mcp : channel === 'grid' ? data.grid : data.llm
}

/**
 * 这一列在这个出口要不要脱敏、用哪种方式；不要就是 `null`。
 * 多条命中时取**第一条**（顺序即优先级，与后端一致）。
 */
export function methodFor(channel, { connectionId, database, table, column }, data = config) {
  if (!data || !channelOn(channel, data)) return null
  const hit = data.rules.find((rule) => ruleHits(rule, connectionId || '', database || '', table || '', column))
  return hit ? hit.method || 'md5' : null
}

/** 一次算出整批列的处理方式：`{ 列名: 方式 }`，没命中的列不出现在结果里 */
export function gridMethods({ connectionId, database, table, columns }, data = config) {
  const out = {}
  for (const column of columns || []) {
    const method = methodFor('grid', { connectionId, database, table, column }, data)
    if (method) out[column] = method
  }
  return out
}

/** 规则里有没有 AES（决定要不要去取密钥文本） */
export function usesAes(data = config) {
  return !!data && data.rules.some((rule) => (rule.method || 'md5') === 'aes')
}

// ---------------------------------------------------------------- 算法实现
// MD5 / SHA-256 手写同步版：表格渲染是同步的，`crypto.subtle` 只有异步接口，
// 而 MD5 在 WebCrypto 里根本没有。实现只到这三种摘要 + 两种掩码，其它一律不碰。

const utf8 = (text) => new TextEncoder().encode(String(text))

const HEX = '0123456789abcdef'
function toHex(bytes) {
  let out = ''
  for (const byte of bytes) out += HEX[(byte >> 4) & 15] + HEX[byte & 15]
  return out
}
/** 32 位整数按**小端**写成 8 位十六进制（MD5 的输出是 little-endian） */
function wordToHexLE(word) {
  let out = ''
  for (let i = 0; i < 4; i += 1) {
    const byte = (word >>> (i * 8)) & 0xff
    out += HEX[(byte >> 4) & 15] + HEX[byte & 15]
  }
  return out
}

const MD5_S = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21]
const MD5_K = (() => {
  const table = new Uint32Array(64)
  for (let i = 0; i < 64; i += 1) table[i] = Math.floor(Math.abs(Math.sin(i + 1)) * 4294967296) >>> 0
  return table
})()

/** MD5（32 位小写十六进制），与 `DigestUtils.md5Hex()` / MySQL `md5()` 一致 */
export function md5Hex(text) {
  const bytes = utf8(text)
  const length = bytes.length
  const blocks = ((length + 8) >>> 6) + 1
  const buffer = new Uint8Array(blocks * 64)
  buffer.set(bytes)
  buffer[length] = 0x80
  const view = new DataView(buffer.buffer)
  const bitLo = (length * 8) >>> 0
  const bitHi = Math.floor(length / 536870912) >>> 0
  view.setUint32(blocks * 64 - 8, bitLo, true)
  view.setUint32(blocks * 64 - 4, bitHi, true)

  let a0 = 0x67452301
  let b0 = 0xefcdab89
  let c0 = 0x98badcfe
  let d0 = 0x10325476
  const m = new Uint32Array(16)
  for (let block = 0; block < blocks; block += 1) {
    for (let i = 0; i < 16; i += 1) m[i] = view.getUint32(block * 64 + i * 4, true)
    let a = a0
    let b = b0
    let c = c0
    let d = d0
    for (let i = 0; i < 64; i += 1) {
      let f
      let g
      if (i < 16) {
        f = (b & c) | (~b & d)
        g = i
      } else if (i < 32) {
        f = (d & b) | (~d & c)
        g = (5 * i + 1) % 16
      } else if (i < 48) {
        f = b ^ c ^ d
        g = (3 * i + 5) % 16
      } else {
        f = c ^ (b | ~d)
        g = (7 * i) % 16
      }
      const rotated = MD5_S[(i >> 4) * 4 + (i % 4)]
      const sum = (a + f + MD5_K[i] + m[g]) >>> 0
      a = d
      d = c
      c = b
      b = (b + ((sum << rotated) | (sum >>> (32 - rotated)))) >>> 0
    }
    a0 = (a0 + a) >>> 0
    b0 = (b0 + b) >>> 0
    c0 = (c0 + c) >>> 0
    d0 = (d0 + d) >>> 0
  }
  return wordToHexLE(a0) + wordToHexLE(b0) + wordToHexLE(c0) + wordToHexLE(d0)
}

const SHA256_K = [
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
]
const rotr = (word, bits) => (word >>> bits) | (word << (32 - bits))

/** SHA-256 原始字节（AES 的 nonce 派生也用它） */
export function sha256Bytes(bytes) {
  const length = bytes.length
  const blocks = ((length + 8) >>> 6) + 1
  const buffer = new Uint8Array(blocks * 64)
  buffer.set(bytes)
  buffer[length] = 0x80
  const view = new DataView(buffer.buffer)
  view.setUint32(blocks * 64 - 8, Math.floor(length / 536870912) >>> 0)
  view.setUint32(blocks * 64 - 4, (length * 8) >>> 0)

  const h = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]
  const w = new Uint32Array(64)
  for (let block = 0; block < blocks; block += 1) {
    for (let i = 0; i < 16; i += 1) w[i] = view.getUint32(block * 64 + i * 4)
    for (let i = 16; i < 64; i += 1) {
      const s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >>> 3)
      const s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >>> 10)
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) >>> 0
    }
    let [a, b, c, d, e, f, g, hh] = h
    for (let i = 0; i < 64; i += 1) {
      const s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)
      const ch = (e & f) ^ (~e & g)
      const t1 = (hh + s1 + ch + SHA256_K[i] + w[i]) >>> 0
      const s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)
      const maj = (a & b) ^ (a & c) ^ (b & c)
      const t2 = (s0 + maj) >>> 0
      hh = g
      g = f
      f = e
      e = (d + t1) >>> 0
      d = c
      c = b
      b = a
      a = (t1 + t2) >>> 0
    }
    h[0] = (h[0] + a) >>> 0
    h[1] = (h[1] + b) >>> 0
    h[2] = (h[2] + c) >>> 0
    h[3] = (h[3] + d) >>> 0
    h[4] = (h[4] + e) >>> 0
    h[5] = (h[5] + f) >>> 0
    h[6] = (h[6] + g) >>> 0
    h[7] = (h[7] + hh) >>> 0
  }
  const out = new Uint8Array(32)
  const outView = new DataView(out.buffer)
  h.forEach((word, i) => outView.setUint32(i * 4, word))
  return out
}

/** SHA-256（64 位小写十六进制） */
export function sha256Hex(text) {
  return toHex(sha256Bytes(utf8(text)))
}

// ---- base64（密钥与 AES 结果都是 base64） ----

function base64ToBytes(text) {
  try {
    const binary = atob(trim(text))
    const bytes = new Uint8Array(binary.length)
    for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i)
    return bytes
  } catch {
    return null
  }
}

function bytesToBase64(bytes) {
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary)
}

/**
 * 密钥文本 → 32 字节。与后端 `key_from_text` 一致：
 * 能解出 base64 的 32 字节就直接用，否则按 SHA-256 派生（用户自己写的口令也能用）。
 */
export function keyFromText(text) {
  const decoded = base64ToBytes(text)
  if (decoded && decoded.length === 32) return decoded
  return sha256Bytes(utf8(trim(text)))
}

async function aesEncrypt(keyBytes, text) {
  const subtle = globalThis.crypto?.subtle
  if (!subtle) return sha256Hex(text) // 极老环境没有 WebCrypto：退回摘要，绝不把明文显示出来
  // nonce = SHA-256(key ‖ 0x00 ‖ 明文) 前 12 字节 —— 与后端 same-value-same-ciphertext 的约定
  const seed = new Uint8Array(keyBytes.length + 1 + text.length)
  seed.set(keyBytes, 0)
  seed[keyBytes.length] = 0
  seed.set(utf8(text), keyBytes.length + 1)
  const nonce = sha256Bytes(seed).slice(0, 12)
  const key = await subtle.importKey('raw', keyBytes, 'AES-GCM', false, ['encrypt'])
  const sealed = new Uint8Array(await subtle.encrypt({ name: 'AES-GCM', iv: nonce }, key, utf8(text)))
  const out = new Uint8Array(12 + sealed.length)
  out.set(nonce, 0)
  out.set(sealed, 12)
  return bytesToBase64(out)
}

// ---------------------------------------------------------------- 取值

/**
 * 同步可算的方式（md5 / sha256 / star / last4）。
 * 返回 `null` 表示这个方式要异步（只有 AES）—— 调用方按「待算」处理。
 */
export function maskTextSync(text, method) {
  const raw = String(text == null ? '' : text).trim()
  if (!raw) return ''
  switch (method) {
    case 'sha256':
      return sha256Hex(raw)
    case 'star':
      return '***'
    case 'last4': {
      const chars = Array.from(raw)
      if (chars.length <= 4) return '***'
      return '****' + chars.slice(chars.length - 4).join('')
    }
    case 'aes':
      return null
    default:
      return md5Hex(raw)
  }
}

/** 异步版：只有 AES 需要，`keyText` 是设置里那串密钥文本 */
export async function maskTextAsync(text, method, keyText) {
  if (method !== 'aes') return maskTextSync(text, method)
  const raw = String(text == null ? '' : text).trim()
  if (!raw) return ''
  return aesEncrypt(keyFromText(keyText), raw)
}
