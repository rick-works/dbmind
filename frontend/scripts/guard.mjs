#!/usr/bin/env node
/**
 * 前端守卫（npm run guard / npm run guard:dom）
 *
 * 为什么需要它
 * ------------
 * 这个项目只有 `vite build` 一道验证，而下面这些坑**构建全查不出来**，只在运行时炸：
 *
 *   1. 调了 t() 却没从 utils/i18n 导入 —— 运行时 `t is not a function`
 *   2. 作用域里有个同名变量（const t = 某个对象）把翻译函数遮蔽掉 —— 同上
 *   3. 模板里写成 t('x')（应写 $t('x')）—— 模板作用域里的 t 往往是 v-for 的遍历项
 *   4. 词典 zh/en 键不一致、重复键、代码引用了不存在的键
 *   5. 产物 chunk 之间引用断链（缺文件 → 浏览器拿到回落的 index.html → 模块报错）
 *   6. 产物能构建但不能跑（打包器运行时助手漏生成等）——**只靠构建完全看不出来**
 *
 * 1~4 纯静态，`npm run guard` 即可；5 在 dist 存在时顺带查；
 * 6 必须真的用浏览器打开一次，所以放 `npm run guard:dom`。
 *
 * 用法
 * ----
 *   npm run guard                 # 静态检查（不联网、不启服务）
 *   npm run guard:dom             # 额外用无头浏览器打开页面，确认首屏真的渲染出来
 *   GUARD_URL=http://127.0.0.1:20361/#/main npm run guard:dom
 *   CHROME_PATH=/path/to/chrome  npm run guard:dom
 */

import { readFileSync, readdirSync, statSync, existsSync, mkdtempSync } from 'node:fs'
import { join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { tmpdir } from 'node:os'
import { spawnSync } from 'node:child_process'

const ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..')
const SRC = join(ROOT, 'src')
const DIST = join(ROOT, 'dist')

const wantDom = process.argv.includes('--dom')
const problems = []
const suspects = []
/** soft = true 时只记入「待人工确认」，不算失败（深度判定不可靠的文件会走这里） */
const warn = (file, line, msg, soft = false) => (soft ? suspects : problems).push({ file, line, msg })

const rel = (p) => (p.startsWith(ROOT) ? relative(ROOT, p).replace(/\\/g, '/') : p)
const read = (p) => readFileSync(p, 'utf8')

// ---------------------------------------------------------------- 文本处理

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) walk(p, out)
    else if (/\.(vue|js|mjs)$/.test(name)) out.push(p)
  }
  return out
}

/**
 * 抹掉注释。**保留字符串内容** —— 键名（t('a.b')）和调用形式（t(）都在字符串里，
 * 抹了它们就什么都查不到了。只把注释字符换成空格、换行原样保留，行号列号仍对得上。
 *
 * 必须做字符串状态跟踪：代码里有 `'http://x'` 这种含 `//` 的字符串，
 * 粗暴去注释会把后半行一起吃掉，于是漏报。
 */
function stripComments(src) {
  const n = src.length
  const keep = new Uint8Array(n).fill(1)
  let i = 0
  const blank = (from, to) => {
    for (let k = from; k < to && k < n; k++) if (src[k] !== '\n' && src[k] !== '\r') keep[k] = 0
  }
  while (i < n) {
    const c = src[i]
    const c2 = src[i + 1]
    if (c === '/' && c2 === '*') {
      const e = src.indexOf('*/', i + 2)
      const end = e < 0 ? n : e + 2
      blank(i, end)
      i = end
      continue
    }
    if (c === '<' && src.startsWith('<!--', i)) {
      const e = src.indexOf('-->', i + 4)
      const end = e < 0 ? n : e + 3
      blank(i, end)
      i = end
      continue
    }
    if (c === '/' && c2 === '/') {
      let e = src.indexOf('\n', i)
      if (e < 0) e = n
      blank(i, e)
      i = e
      continue
    }
    if (c === '"' || c === "'" || c === '`') {
      let j = i + 1
      while (j < n) {
        if (src[j] === '\\') { j += 2; continue }
        if (src[j] === c) break
        j++
      }
      i = Math.min(n, j + 1)
      continue
    }
    i++
  }
  let out = ''
  for (let k = 0; k < n; k++) out += keep[k] ? src[k] : src[k] === '\n' || src[k] === '\r' ? src[k] : ' '
  return out
}

/** 在 stripComments 基础上，再把字符串**内容**也抹平（只用来数大括号深度） */
function blankStrings(src) {
  const n = src.length
  const keep = new Uint8Array(n).fill(1)
  let i = 0
  while (i < n) {
    const c = src[i]
    if (c === '"' || c === "'" || c === '`') {
      let j = i + 1
      while (j < n) {
        if (src[j] === '\\') { j += 2; continue }
        if (src[j] === c) break
        j++
      }
      for (let k = i + 1; k < j && k < n; k++) if (src[k] !== '\n' && src[k] !== '\r') keep[k] = 0
      i = Math.min(n, j + 1)
      continue
    }
    i++
  }
  let out = ''
  for (let k = 0; k < n; k++) out += keep[k] ? src[k] : src[k] === '\n' || src[k] === '\r' ? src[k] : ' '
  return out
}

const isVue = (p) => p.endsWith('.vue')

/** 把 <script> 块内的内容抹平（只对 .vue 有意义；.js 整个文件都是脚本） */
function withoutScript(text, p) {
  if (!isVue(p)) return text.replace(/[^\r\n]/g, ' ')
  const n = text.length
  const keep = new Uint8Array(n).fill(1)
  for (const m of text.matchAll(/<script\b[^>]*>[\s\S]*?<\/script>/g)) {
    for (let k = m.index; k < m.index + m[0].length && k < n; k++) keep[k] = 0
  }
  let out = ''
  for (let k = 0; k < n; k++) out += keep[k] ? text[k] : text[k] === '\n' || text[k] === '\r' ? text[k] : ' '
  return out
}

/** 只保留 <script> 块内（只对 .vue 有意义；.js 原样） */
function onlyScript(text, p) {
  if (!isVue(p)) return text
  const n = text.length
  const keep = new Uint8Array(n).fill(0)
  for (const m of text.matchAll(/<script\b[^>]*>[\s\S]*?<\/script>/g)) {
    for (let k = m.index; k < m.index + m[0].length && k < n; k++) keep[k] = 1
  }
  let out = ''
  for (let k = 0; k < n; k++) out += keep[k] ? text[k] : text[k] === '\n' || text[k] === '\r' ? text[k] : ' '
  return out
}

const splitLines = (s) => s.split(/\r?\n/)

/**
 * 花括号净变化。不为 0 说明这个文件里有没被抹掉的花括号（正则字面量 / JSX 之类），
 * 于是「深度」判断不可靠 —— 这时遮蔽结论只能当参考，降级成待确认而不是直接报错。
 */
function netBraces(s) {
  let d = 0
  for (const ch of s) {
    if (ch === '{') d++
    else if (ch === '}') d--
  }
  return d
}

/** 每行“进入时”的大括号深度 */
function depths(blankedText) {
  const out = []
  let d = 0
  for (const l of splitLines(blankedText)) {
    out.push(d)
    for (const ch of l) {
      if (ch === '{') d++
      else if (ch === '}') d = Math.max(0, d - 1)
    }
  }
  return out
}

// ---------------------------------------------------------------- 规则

const CALL_T = /(?<![\w.$])t\(/
const IMPORT_I18N = /import\s*\{([^}]*)\}\s*from\s*['"][^'"]*i18n['"]/g
const DECL_T = /(?:^|[\s;{(,])(?:const|let|var)\s+t\s*[=,]/
const FUNC_T = /\bfunction\s+t\s*\(/
const CATCH_T = /catch\s*\(\s*t\s*\)/
const FOR_OF_T = /for\s*\(\s*(?:const|let)\s+t\s+of\b/
const FOR_OF_BLOCK_T = /for\s*\(\s*(?:const|let)\s+t\s+of[^)]*\)\s*\{/
const ARROW_PARAM_T = /(?:\([^()]*\bt\b[^()]*\)|(?:^|[^\w.])t)\s*=>/
const KEY_USE = /(?<![\w.$])[$]?t\('([a-zA-Z][a-zA-Z0-9_]*(?:\.[a-zA-Z0-9_]+)*)'/g
const KEY_DEF = /^ {2}'([^']+)':/gm

const KEY_WHITELIST = /^(shortcut\.|shortcut\.group\.|x)$/

// ---------------------------------------------------------------- 1. 词典

function checkDict(files) {
  const keys = (f) => {
    const txt = read(join(SRC, 'locales', f))
    const out = []
    let m
    while ((m = KEY_DEF.exec(txt))) out.push(m[1])
    KEY_DEF.lastIndex = 0
    return out
  }
  const zh = keys('zh-CN.js')
  const en = keys('en-US.js')

  const dup = [...new Set(zh.filter((k, i) => zh.indexOf(k) !== i))]
  if (dup.length) warn('src/locales/zh-CN.js', 0, `重复键 ${dup.length} 个：${dup.slice(0, 8).join(', ')}`)

  const onlyZh = zh.filter((k) => !en.includes(k))
  const onlyEn = en.filter((k) => !zh.includes(k))
  if (onlyZh.length) warn('src/locales/zh-CN.js', 0, `英文词典缺 ${onlyZh.length} 个键：${onlyZh.slice(0, 8).join(', ')}`)
  if (onlyEn.length) warn('src/locales/en-US.js', 0, `中文词典缺 ${onlyEn.length} 个键：${onlyEn.slice(0, 8).join(', ')}`)

  const used = new Set()
  for (const f of files) {
    if (f.includes('locales') || f.endsWith('i18n.js')) continue
    const txt = stripComments(read(f))
    let m
    while ((m = KEY_USE.exec(txt))) used.add(m[1])
    KEY_USE.lastIndex = 0
  }
  const missing = [...used].filter((k) => !zh.includes(k) && !KEY_WHITELIST.test(k) && !k.endsWith('.'))
  if (missing.length) warn('src/**', 0, `引用了词典里没有的键 ${missing.length} 个：${missing.slice(0, 10).join(', ')}`)

  return { zh: zh.length, en: en.length, used: used.size }
}

// ---------------------------------------------------------------- 2 + 3. 翻译函数的可见性 / 遮蔽

function checkT(files) {
  let filesWithCalls = 0
  for (const f of files) {
    if (f.includes('locales') || f.endsWith('i18n.js')) continue
    const text = stripComments(read(f))

    // —— ① 模板里不该出现裸 t(（只对 .vue 有意义）
    if (isVue(f)) {
      splitLines(withoutScript(text, f)).forEach((l, i) => {
        if (CALL_T.test(l)) warn(f, i + 1, '模板里用了裸 t()（模板作用域里没有它，应写 $t()）')
      })
    }

    // —— ② 脚本里调用 t( 时，t 必须在作用域内
    const imports = [...text.matchAll(IMPORT_I18N)]
    IMPORT_I18N.lastIndex = 0
    const importHasT = imports.some((m) => m[1].split(',').some((s) => s.trim().replace(/^t\s+as\s+\w+$/, 't') === 't'))

    // 两份行视图，行号完全对齐：
    //   scr      —— 保留字符串内容（markdown.js 里 t('…') 就写在模板字符串里，抹了就漏报）
    //   scrDecl  —— 抹掉字符串内容（否则「字符串里恰好像一段代码」会被当成真声明，大量误报）
    const scr = splitLines(onlyScript(text, f))
    const declText = blankStrings(onlyScript(text, f))
    const scrDecl = splitLines(declText)
    const dep = depths(declText)
    const reliable = netBraces(declText) === 0

    const declLines = []
    scrDecl.forEach((l, i) => {
      // `for (const t of xs) stmt` 不带花括号的写法，作用域只到本行末尾 —— 不计入声明，
      // 否则它**后面**的所有 t() 都会被误判成「被遮蔽」（SyncDialog 就是这么误报 22 条的）。
      if (FOR_OF_T.test(l) && !FOR_OF_BLOCK_T.test(l)) return
      if (DECL_T.test(l) || FUNC_T.test(l) || CATCH_T.test(l) || FOR_OF_T.test(l)) declLines.push(i)
    })
    const callLines = []
    scr.forEach((l, i) => {
      if (CALL_T.test(l)) callLines.push(i)
    })
    if (!callLines.length) continue
    filesWithCalls++

    if (!importHasT && !declLines.length) {
      warn(
        f,
        0,
        imports.length
          ? 'import 了 i18n 但没导入 t，却调用了 t()（运行时 t is not a function）'
          : '调用了 t() 但没从 utils/i18n 导入 t（运行时 t is not a function）'
      )
      continue
    }

    // —— ③ 遮蔽：声明必须“仍在同一作用域链上”才算遮蔽 ——
    //    判据：声明的深度 <= 调用的深度，**且**从声明到调用之间深度从未跌回声明深度
    //    （跌回说明那个块已经闭合，声明不可能再影响调用点）。
    for (const ci of callLines) {
      const d = dep[ci]
      for (const x of declLines) {
        if (x >= ci) continue
        // for (const t of xs) { … } 的 t 属于「循环体那块」，深度是进入该行的深度 +1
        const dx = dep[x] + (FOR_OF_BLOCK_T.test(scrDecl[x]) ? 1 : 0)
        if (dx > d) continue
        let closed = false
        for (let y = x + 1; y <= ci; y++) {
          if (dep[y] < dx) { closed = true; break } // 块整体闭合
          // try/catch/finally、if/else 的分支切换：花括号深度不变，但作用域已经换了一个
          if (dx > dep[y] && /\}\s*(catch|else|finally)\b/.test(scrDecl[y])) { closed = true; break }
        }
        if (closed) continue
        warn(
          f,
          ci + 1,
          `t() 被同作用域变量遮蔽（第 ${x + 1} 行声明了 t），运行时 t is not a function`,
          !reliable
        )
      }
      // 同行箭头参数：dirtyTabs.map(t => t('x'))
      // 只有 t( 出现在 => **之后**才算被参数遮蔽；`filter(t => t.x)` 这种同行里有别的 t( 不算
      const arrow = ARROW_PARAM_T.exec(scrDecl[ci])
      if (arrow && CALL_T.test(scr[ci].slice(arrow.index + arrow[0].length))) {
        warn(f, ci + 1, '同一行里箭头参数叫 t，且在箭头体内调用了 t()，参数会遮蔽翻译函数')
      }
    }
  }
  return filesWithCalls
}

// ---------------------------------------------------------------- 4. 产物完整性

function checkBundle() {
  const indexHtml = join(DIST, 'index.html')
  if (!existsSync(indexHtml)) return { skipped: true }
  const refs = new Set([...read(indexHtml).matchAll(/assets\/[A-Za-z0-9_.-]+\.js/g)].map((m) => m[0]))

  const seen = new Set()
  const missing = []
  const stack = [...refs]
  while (stack.length) {
    const r = stack.pop()
    if (seen.has(r)) continue
    seen.add(r)
    const file = join(DIST, r)
    if (!existsSync(file)) { missing.push(r); continue }
    // 只认真正的 ESM 引用（from "…" / import "…"），不认代码里恰好出现的 .js 字符串
    for (const m of read(file).matchAll(/(?:from|import)\s*\(?\s*["']\.\/([A-Za-z0-9_.-]+\.js)["']/g)) {
      const depRef = `assets/${m[1]}`
      if (!seen.has(depRef)) stack.push(depRef)
    }
  }
  if (missing.length) warn('dist/index.html', 0, `产物内部引用断链 ${missing.length} 处：${missing.slice(0, 5).join(', ')}`)
  return { chunks: seen.size, missing: missing.length }
}

// ---------------------------------------------------------------- 5. 版本号一致性

/**
 * 版本号三处必须一致：Cargo.toml（打包用）、tauri.conf.json（安装包用）、package.json。
 * 前端不再自己维护版本常量 —— 「关于」页读的是构建时从 Cargo.toml 注入的 __APP_VERSION__
 * （见 vite.config.mjs 与 src/version.js）。这里只守"三份文件别漂开"。
 */
function checkVersions() {
  const pick = (p, re) => {
    try {
      return (re.exec(read(p)) || [])[1] || ''
    } catch {
      return ''
    }
  }
  const ws = pick(join(ROOT, '..', 'Cargo.toml'), /\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)
  const conf = pick(join(ROOT, '..', 'crates', 'dbmind-desktop', 'tauri.conf.json'), /"version"\s*:\s*"([^"]+)"/)
  const pkg = pick(join(ROOT, 'package.json'), /"version"\s*:\s*"([^"]+)"/)
  if (ws && conf && ws !== conf) warn('crates/dbmind-desktop/tauri.conf.json', 0, `版本号不一致：Cargo.toml=${ws}  tauri.conf.json=${conf}`)
  if (ws && pkg && ws !== pkg) warn('package.json', 0, `版本号不一致：Cargo.toml=${ws}  frontend/package.json=${pkg}`)
  return { ws, conf, pkg }
}
// ---------------------------------------------------------------- 5. 无头浏览器打点

function findBrowser() {
  return [
    process.env.CHROME_PATH,
    'C:/Program Files/Google/Chrome/Application/chrome.exe',
    'C:/Program Files (x86)/Google/Chrome/Application/chrome.exe',
    'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
    'C:/Program Files/Microsoft/Edge/Application/msedge.exe',
    '/usr/bin/google-chrome',
    '/usr/bin/chromium',
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  ].filter(Boolean).find((p) => existsSync(p))
}

function checkDom() {
  const url = process.env.GUARD_URL || 'http://127.0.0.1:20361/#/main'
  const exe = findBrowser()
  if (!exe) return { skipped: '没找到 Chrome/Edge（可用 CHROME_PATH 指定）' }

  const res = spawnSync(
    exe,
    [
      '--headless=new', '--disable-gpu', '--no-sandbox', '--disable-dev-shm-usage',
      '--virtual-time-budget=15000', `--user-data-dir=${mkdtempSync(join(tmpdir(), 'dbmind-guard-'))}`,
      '--dump-dom', url,
    ],
    { encoding: 'utf8', maxBuffer: 128 * 1024 * 1024 }
  )
  const dom = res.stdout || ''
  if (!dom) return { skipped: `浏览器无输出（exit=${res.status}）——地址是否已起服务？${url}` }

  const m = /<div id="app"[^>]*>([\s\S]*)<\/body>/.exec(dom)
  if (!m) { warn(url, 0, '页面里没有 #app，入口脚本可能没执行'); return { url, textLen: 0, sample: '' } }
  const text = m[1].replace(/<(script|style)[\s\S]*?<\/\1>/g, ' ').replace(/<[^>]+>/g, ' ').replace(/&nbsp;/g, ' ').replace(/\s+/g, ' ').trim()
  if (text.length < 200) warn(url, 0, `#app 里几乎没有内容（可见文字 ${text.length} 字）——首屏没渲染出来`)
  if (/启动失败|Uncaught|is not a function|__commonJSMin/.test(dom)) warn(url, 0, '页面里出现了启动错误标记')
  return { url, textLen: text.length, sample: text.slice(0, 120) }
}

// ---------------------------------------------------------------- 6. prop 裸用（运行时必崩）

/**
 * `<script setup>` 的规则：**模板**里可以用 prop 的裸名，**脚本**里必须写 `props.xxx`。
 * 写成裸的就是 `ReferenceError: xxx is not defined` —— 而它**构建全绿**，只有渲染到那一步才炸：
 * 导出进度框就是这么崩的（脚本里写了 `canceling.value`，实际得写 `props.canceling`），
 * 早前整页白屏也是同一族（调 t() 却没导入 / 被局部变量遮蔽）。
 *
 * 只查这一族、不查"完全版 no-undef"：后者要精确解析声明位置，误报多得没法看，反而没人看。
 */
const JS_KEYWORDS = new Set(['if', 'else', 'for', 'while', 'do', 'switch', 'case', 'default', 'break', 'continue', 'return', 'function', 'class', 'extends', 'super', 'new', 'delete', 'typeof', 'instanceof', 'in', 'of', 'void', 'this', 'yield', 'await', 'async', 'try', 'catch', 'finally', 'throw', 'const', 'let', 'var', 'import', 'export', 'from', 'as', 'static', 'get', 'set', 'true', 'false', 'null', 'undefined', 'debugger', 'with', 'enum'])

const JS_GLOBALS = new Set(['window', 'document', 'console', 'navigator', 'location', 'history', 'localStorage', 'sessionStorage', 'performance', 'fetch', 'URL', 'Blob', 'File', 'FileReader', 'FormData', 'Headers', 'Request', 'Response', 'AbortController', 'Promise', 'JSON', 'Math', 'Date', 'Object', 'Array', 'String', 'Number', 'Boolean', 'Symbol', 'BigInt', 'RegExp', 'Function', 'Error', 'TypeError', 'RangeError', 'Map', 'Set', 'WeakMap', 'WeakSet', 'Proxy', 'Reflect', 'Intl', 'isNaN', 'isFinite', 'parseInt', 'parseFloat', 'encodeURI', 'encodeURIComponent', 'decodeURI', 'decodeURIComponent', 'setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'requestAnimationFrame', 'cancelAnimationFrame', 'queueMicrotask', 'structuredClone', 'alert', 'confirm', 'prompt', 'atob', 'btoa', 'crypto', 'globalThis', 'arguments', 'defineProps', 'defineEmits', 'defineExpose', 'defineModel', 'defineOptions', 'defineSlots', 'withDefaults'])

/** 字符下标换算行号 */
function lineAt(text, index) {
  let line = 1
  for (let i = 0; i < index && i < text.length; i++) if (text[i] === '\n') line++
  return line
}

/**
 * 取 `defineProps({…})` 的**顶层键**，并返回「参数对象结束的位置」。
 *
 * 必须按大括号深度取：`columns: { type: Array, default: () => [{ name, nullable }] }`
 * 里嵌的 `nullable` 不是 prop，抓进来就会假报一片。
 */
function parseProps(body) {
  const m = /defineProps\s*\(\s*\{/.exec(body)
  if (!m) return { names: [], end: -1 }
  const names = []
  let i = m.index + m[0].length
  let depth = 1
  let expectKey = true
  while (i < body.length) {
    if (expectKey) {
      const km = /^\s*([A-Za-z_$][\w$]*)\s*[:(,]/.exec(body.slice(i))
      if (km) {
        names.push(km[1])
        i += km[0].length
        expectKey = false
        continue
      }
    }
    const c = body[i]
    if (c === '{' || c === '[' || c === '(') depth++
    else if (c === '}' || c === ']' || c === ')') {
      depth--
      if (depth === 0) return { names: [...new Set(names)], end: i }
    } else if (c === ',' && depth === 1) expectKey = true
    i++
  }
  return { names: [...new Set(names)], end: i }
}

function checkProps(files) {
  let scanned = 0
  for (const f of files) {
    if (!isVue(f)) continue
    // 顺序与去标签：onlyScript 会把 <script setup> 标签本身留着，不去掉会把标签名当标识符
    const script = stripComments(onlyScript(read(f), f)).replace(/<\/?script\b[^>]*>/g, '')
    if (!script.trim() || !script.includes('defineProps')) continue
    scanned++
    const body = blankStrings(script)
    const { names, end } = parseProps(body)
    if (!names.length || end < 0) continue

    // 局部声明（宽松口径：任何声明位置出现过就算已声明，避免误报）
    const declared = new Set()
    const add = (re) => {
      for (const m of body.matchAll(re)) {
        for (let k = 1; k < m.length; k++) {
          for (const n of String(m[k] || '').matchAll(/[A-Za-z_$][\w$]*/g)) declared.add(n[0])
        }
      }
    }
    add(/(?:^|[\s;{(])(?:const|let|var|function|class)\s+([A-Za-z_$][\w$]*)/g)
    add(/\bimport\s+([\s\S]*?)\s+from\b/g)
    add(/(?:const|let|var)\s*\{([^{}]*)\}/g)
    add(/\{([^{}]*)\}\s*=/g)
    add(/(?:\(([^()]*)\)|([A-Za-z_$][\w$]*))\s*=>/g)
    add(/\bfunction\s*[\w$]*\s*\(([^()]*)\)/g)
    add(/\bcatch\s*\(\s*([A-Za-z_$][\w$]*)/g)
    add(/\bfor\s*\(\s*(?:const|let|var)\s+([\s\S]*?)\s+of\b/g)

    // 只看 defineProps(…) **之后**的脚本：声明处那些键名是正常的，不能算使用
    const after = body.slice(end)
    for (const name of names) {
      if (JS_KEYWORDS.has(name) || JS_GLOBALS.has(name) || declared.has(name)) continue
      // 前面紧跟 \ 的当作正则字面量里的字（如 /\{database\}/），不算用 prop
      const re = new RegExp('(?<![.\\w$\\\\])' + name + '(?![\\w$])', 'g')
      const hit = re.exec(after)
      if (!hit) continue
      // 后面跟冒号的当对象键（{ database: db }），不是"用 prop"；简写 { database } 才算
      if (/^\s*:/.test(after.slice(hit.index + name.length))) continue
      // soft：解析到"正则字面量里的字/解构出的局部变量"还做不到完全精确，先只提示不判死
      warn(f, lineAt(body, end + hit.index), `prop「${name}」疑似在脚本里被当裸变量用（正确写法是 props.${name}）`, true)
    }
  }
  return scanned
}
// ---------------------------------------------------------------- 7. 进度框必须绑上状态

/**
 * `<TaskProgressDialog>` 的状态全靠 props 传进来，**漏绑不会报错**，只会让弹窗永远停在
 * 默认值上：进度条空着、phase 空、"取消"按钮点了没反应（事件没人接）。实测报上来的
 * "表预览界面导出点了没反应、取消也没反应"就是这个 —— 后端其实一直好好的。
 *
 * 所以：只要用了这个组件，就必须绑状态与取消事件。
 */
// phase 不强制：危险操作那个框只报 message + logs（:: 写成字面量的检查见下一条）
const TPD_REQUIRED = [':status', ':total', '@cancel']

function checkProgressDialog(files) {
  let checked = 0
  for (const f of files) {
    if (!isVue(f)) continue
    const text = read(f)
    const tags = text.match(/<TaskProgressDialog\b[\s\S]*?\/>/g) || []
    for (const tag of tags) {
      checked++
      const lack = TPD_REQUIRED.filter((k) => !tag.includes(k))
      if (lack.length) {
        const line = splitLines(text).findIndex((l) => l.includes('<TaskProgressDialog')) + 1
        warn(f, line, `<TaskProgressDialog> 少了 ${lack.join(' ')} —— 漏绑会让弹窗永远停在默认值（进度不动、取消没反应）`)
      }
    }
  }
  return checked
}
// ---------------------------------------------------------------- 8. 双冒号（`::attr` 一定是打错）

/**
 * `::target-name="x"` 里 Vue 把它当**字面量属性**（属性名就叫 `:target-name`），
 * prop 收不到值、也不报错 —— 表现是"标题里那个名字就是不显示"。实测就踩到过。
 */
function checkDoubleColon(files) {
  let hits = 0
  for (const f of files) {
    if (!isVue(f)) continue
    const lines = splitLines(read(f))
    lines.forEach((l, i) => {
      const m = /(?:^|\s)::([a-zA-Z][\w-]*)\s*=/.exec(l)
      if (m) {
        hits++
        warn(f, i + 1, `属性写成了双冒号 ::${m[1]} —— Vue 会当成字面量属性，接收方拿不到值（应为 :${m[1]}）`)
      }
    })
  }
  return hits
}
// ---------------------------------------------------------------- 主流程

const files = walk(SRC)
const dict = checkDict(files)
const tChecked = checkT(files)
const undef = checkProps(files)
const tpd = checkProgressDialog(files)
const dcol = checkDoubleColon(files)
const bundle = checkBundle()
const ver = checkVersions()
const dom = wantDom ? checkDom() : { skipped: '未开启（用 npm run guard:dom）' }

const green = (s) => `\x1b[32m${s}\x1b[0m`
const red = (s) => `\x1b[31m${s}\x1b[0m`

console.log('')
console.log(`  词典      zh ${dict.zh} / en ${dict.en}   ·   代码引用 ${dict.used} 个键`)
console.log(`  翻译函数  在 ${tChecked} 个有 t() 调用的文件里检查了导入与遮蔽`)
console.log(bundle.skipped ? '  产物      未构建，跳过（先 npm run build）' : `  产物      ${bundle.chunks} 个 chunk，断链 ${bundle.missing} 处`)
console.log(`  版本      Cargo.toml ${ver.ws} / tauri.conf.json ${ver.conf} / package.json ${ver.pkg}`)
console.log(dom.skipped ? `  首屏      跳过：${dom.skipped}` : `  首屏      渲染出 ${dom.textLen} 字：${dom.sample}…`)
console.log('')

const fmt = (p) => {
  const where = typeof p.file === 'string' && !p.file.startsWith(SRC) ? p.file : `${rel(p.file)}${p.line ? ':' + p.line : ''}`
  return `      ${where}  ${p.msg}`
}

if (suspects.length) {
  console.log(`  ⚠ 待人工确认 ${suspects.length} 条（所在文件有没被抹掉的花括号，深度判定不可靠）：`)
  for (const p of suspects.slice(0, 20)) console.log(fmt(p))
  if (suspects.length > 20) console.log(`      ……还有 ${suspects.length - 20} 条`)
  console.log('')
}

if (!problems.length) {
  console.log(green(suspects.length ? '  ✓ guard 通过（另有待确认项，见上）' : '  ✓ guard 通过'))
  process.exit(0)
}
console.log(red(`  ✗ guard 发现 ${problems.length} 个问题：`))
for (const p of problems.slice(0, 60)) console.log(fmt(p))
if (problems.length > 60) console.log(`      ……还有 ${problems.length - 60} 条`)
console.log('')
process.exit(1)
