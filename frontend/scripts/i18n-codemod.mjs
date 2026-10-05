// 把界面里的中文硬编码反向映射回 i18n 键（AST 级，不用正则猜 .vue 的结构）。
//
// 为什么必须上 AST：这两个文件里有**嵌套模板**（`<template #header>`），
// 用"第一个 </template>"切边界必然切错，替换会落到属性中间 —— 试过两次正则版，两次构建失败。
//
// 覆盖的四种位置（每种都在 AST 给出的 loc 范围内替换）：
//   ① 文本节点        暂无筛选条件 → {{ $t('key') }}
//   ② 属性             title="取消" → :title="$t('key')"；:title / v-if / @click 里只换字面量
//   ③ {{ }} 插值       {{ a ? '执行选中' : '执行' }} → {{ a ? $t(k1) : $t(k2) }}
//   ④ 带插值的模板串   `删除选中行 (${x})` ↔ 词典 `删除选中行 ({n})` → t('key', { n: x })
//
// 实战踩过的两个坑（都写进注释了，别再试）：
//   · 模板内容起点必须 src.indexOf(tpl.content) 反查，不能用"标签结尾 +1"（错几个字符就插进属性里）
//   · INTERPOLATION 的 content.loc **本身就是花括号内部**，外层 {{ }} 不在里面 —— 再包一层必崩
//
// 用法：node scripts/i18n-codemod.mjs [--dry] <文件…>      （在 frontend/ 下运行）
import { readFileSync, writeFileSync } from 'node:fs'
import { parse as parseSFC } from '@vue/compiler-sfc'
import { parse as parseDOM, NodeTypes } from '@vue/compiler-dom'

const args = process.argv.slice(2)
const DRY = args.includes('--dry')
const files = args.filter((a) => !a.startsWith('--'))

const PLACEHOLDER = '\u0000'

// ---- 词典两张表：纯文本 / 带参数（带参数的记住参数名顺序，用来生成 t('key', { n: x })）
const dictText = readFileSync('src/locales/zh-CN.js', 'utf8')
const plain = new Map()
const withArgs = new Map()
for (const m of dictText.matchAll(/^ {2}'([^']+)':\s*(?:'((?:[^'\\]|\\.)*)'|"((?:[^"\\]|\\.)*)"),?$/gm)) {
  const key = m[1]
  const value = (m[2] ?? m[3] ?? '').replace(/\\'/g, "'")
  if (!/[\u4e00-\u9fa5]/.test(value)) continue
  if (/\{\w+\}/.test(value)) {
    const names = [...value.matchAll(/\{(\w+)\}/g)].map((x) => x[1])
    const norm = value.replace(/\{\w+\}/g, PLACEHOLDER)
    if (!withArgs.has(norm)) withArgs.set(norm, { key, names })
    continue
  }
  if (value.length > 60 || /[`$]/.test(value)) continue
  if (!plain.has(value)) plain.set(value, key)
}
const keyOf = (zh) => plain.get(zh)
const hasZh = (s) => /[\u4e00-\u9fa5]/.test(s)

/** '中文' / "中文" → wrap(key)，只换能整串反查到键的 */
function swapPlain(text, wrap) {
  let hit = 0
  const out = text
    .replace(/'([^'"\n]{1,60})'/g, (full, inner) => {
      if (!hasZh(inner)) return full
      const key = keyOf(inner)
      if (!key) return full
      hit++
      return wrap(key)
    })
    .replace(/"([^'"\n]{1,60})"/g, (full, inner) => {
      if (!hasZh(inner)) return full
      const key = keyOf(inner)
      if (!key) return full
      hit++
      return wrap(key)
    })
  return [out, hit]
}

/** `` `删除选中行 (${x})` `` → wrap(key, 'n: x')；词典归一后比对，参数名按序对应 */
function swapTemplates(text, wrap) {
  let hit = 0
  const out = text.replace(/`([^`\\]*)`/g, (full, body) => {
    if (!hasZh(body) || !body.includes('${')) return full
    const exprs = []
    const norm = body.replace(/\$\{([^{}]*)\}/g, (m, expr) => {
      exprs.push(expr.trim())
      return PLACEHOLDER
    })
    const found = withArgs.get(norm)
    if (!found || found.names.length !== exprs.length) return full
    hit++
    return wrap(found.key, found.names.map((n, i) => `${n}: ${exprs[i]}`).join(', '))
  })
  return [out, hit]
}

for (const file of files) {
  const src = readFileSync(file, 'utf8').replace(/\r\n/g, '\n')
  const { descriptor, errors } = parseSFC(src, { filename: file })
  if (errors?.length) {
    console.log('  ✗ ' + file + ' 解析失败：' + errors[0].message)
    continue
  }
  const edits = []
  let missed = 0
  const addEdit = (startAbs, endAbs, text) => edits.push({ start: startAbs, end: endAbs, text })

  // ---- 模板
  if (descriptor.template) {
    const tpl = descriptor.template
    const base = src.indexOf(tpl.content, tpl.loc.start.offset)
    if (base < 0) {
      console.log('  ✗ ' + file + ' 找不到模板内容起点')
      continue
    }
    const walk = (node) => {
      // ① 文本节点
      if (node.type === NodeTypes.TEXT && hasZh(node.content)) {
        const trimmed = node.content.trim()
        const key = keyOf(trimmed)
        if (key) {
          const at = node.content.indexOf(trimmed)
          addEdit(base + node.loc.start.offset, base + node.loc.end.offset,
            node.content.slice(0, at) + `{{ $t('${key}') }}` + node.content.slice(at + trimmed.length))
        } else missed++
      }
      // ③ {{ }} 插值：content.loc 是**花括号内部**，不要再包一层
      if (node.type === NodeTypes.INTERPOLATION && node.content?.loc) {
        const raw = node.content.loc.source
        const [n1, h1] = swapPlain(raw, (k) => `$t('${k}')`)
        const [n2, h2] = swapTemplates(n1, (k, a) => `$t('${k}', { ${a} })`)
        if (h1 + h2) addEdit(base + node.content.loc.start.offset, base + node.content.loc.end.offset, n2)
        else if (hasZh(raw)) missed++
      }
      // ② 属性
      for (const p of node.props || []) {
        const raw = p.loc?.source || ''
        if (!raw || !hasZh(raw)) continue
        if (p.type === NodeTypes.ATTRIBUTE && p.value && hasZh(p.value.content || '')) {
          const key = keyOf(p.value.content.trim())
          if (key) {
            addEdit(base + p.loc.start.offset, base + p.loc.end.offset, `:${p.name}="$t('${key}')"`)
            continue
          }
          missed++
        }
        const [n1, h1] = swapPlain(raw, (k) => `$t('${k}')`)
        const [n2, h2] = swapTemplates(n1, (k, a) => `$t('${k}', { ${a} })`)
        if (h1 + h2) addEdit(base + p.loc.start.offset, base + p.loc.end.offset, n2)
        else missed++
      }
      for (const c of node.children || []) walk(c)
    }
    walk(parseDOM(tpl.content))
  }

  // ---- <script setup>
  if (descriptor.scriptSetup) {
    const ss = descriptor.scriptSetup
    const start = src.indexOf(ss.content, ss.loc.start.offset)
    const [n1, h1] = swapPlain(ss.content, (k) => `t('${k}')`)
    const [n2, h2] = swapTemplates(n1, (k, a) => `t('${k}', { ${a} })`)
    if (h1 + h2) addEdit(start, start + ss.content.length, n2)
  }

  // ---- 从后往前应用，保证前面的偏移仍然有效
  edits.sort((a, b) => b.start - a.start)
  let out = src
  for (const e of edits) out = out.slice(0, e.start) + e.text + out.slice(e.end)

  // ---- 脚本里用了 t( 就保证有导入
  if (/\bt\('/.test(out) && !/^import \{[^}]*\bt\b[^}]*\} from '\.\.\/\.\.\/utils\/i18n'/m.test(out)) {
    if (/from '\.\.\/\.\.\/utils\/i18n'/.test(out)) {
      out = out.replace(/import \{([^}]*)\} from '\.\.\/\.\.\/utils\/i18n'/, (m, names) => (/\bt\b/.test(names) ? m : `import {${names}, t } from '../../utils/i18n'`))
    } else {
      out = out.replace(/^(import .*)$/m, `$1\nimport { t } from '../../utils/i18n'`)
    }
  }

  if (!DRY) writeFileSync(file, out.replace(/\n/g, '\r\n'), 'utf8')
  console.log('  ' + file.split('/').pop() + '：替换 ' + edits.length + ' 处，未匹配 ' + missed + ' 处' + (DRY ? '（dry-run）' : ''))
}
