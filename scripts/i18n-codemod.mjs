// 把界面里的中文硬编码反向映射回 i18n 键（AST 级，不再用正则改 .vue）。
//
// 为什么必须上 AST：这两个文件里有**嵌套模板**（`<template #header>`），
// 用"第一个 </template>"切边界必然切错，正则替换就会落到属性上 —— 试了两次都构建失败。
// 这一版的做法：
//   · @vue/compiler-dom 解析模板 → 只在 **属性节点 / 文本节点** 的 loc 范围内动手
//   · script setup 只在 **它自己的 loc 范围**内动手（边界由 SFC 描述符给出，不猜）
//   · 替换一律**从后往前**做，保证前面的偏移仍然有效
//
// 用法：node scripts/i18n-codemod.mjs [--dry] <文件…>
import { readFileSync, writeFileSync } from 'node:fs'
import { parse as parseSFC } from '@vue/compiler-sfc'
import { parse as parseDOM, NodeTypes } from '@vue/compiler-dom'

const args = process.argv.slice(2)
const DRY = args.includes('--dry')
const files = args.filter((a) => !a.startsWith('--'))

// ---- 词典：中文 -> 键（短、纯中文、无占位符的才当锚点）
const dictText = readFileSync('frontend/src/locales/zh-CN.js', 'utf8')
const map = new Map()
for (const m of dictText.matchAll(/^ {2}'([^']+)':\s*(?:'((?:[^'\\]|\\.)*)'|"((?:[^"\\]|\\.)*)"),?$/gm)) {
  const value = (m[2] ?? m[3] ?? '').replace(/\\'/g, "'")
  if (!/[\u4e00-\u9fa5]/.test(value)) continue
  if (value.length > 60 || /[{}`$]/.test(value)) continue
  if (!map.has(value)) map.set(value, m[1])
}
const keyOf = (zh) => map.get(zh)

const hasZh = (s) => /[\u4e00-\u9fa5]/.test(s)
// 在给定文本里把 '中文' / "中文" 换成 $t('key')（仅当整串能反查到键）
function swapQuoted(text, wrap) {
  let hit = 0
  const out = text.replace(/'([^'"\n]{1,60})'/g, (full, inner) => {
    if (!hasZh(inner)) return full
    const key = keyOf(inner)
    if (!key) return full
    hit++
    return wrap(key)
  }).replace(/"([^'"\n]{1,60})"/g, (full, inner) => {
    if (!hasZh(inner)) return full
    const key = keyOf(inner)
    if (!key) return full
    hit++
    return wrap(key)
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
  const edits = [] // {start,end,text}
  let missed = 0

  // ---- 模板
  if (descriptor.template) {
    const tpl = descriptor.template
    const openEnd = src.indexOf('>', tpl.loc.start.offset)
    const base = openEnd + 1
    const ast = parseDOM(tpl.content)
    const walk = (node) => {
      // 文本节点：整段（除首尾空白）能反查到键 → {{ $t('key') }}
      if (node.type === NodeTypes.TEXT && hasZh(node.content)) {
        const trimmed = node.content.trim()
        const key = keyOf(trimmed)
        if (key) {
          const lead = node.content.slice(0, node.content.indexOf(trimmed))
          const tail = node.content.slice(node.content.indexOf(trimmed) + trimmed.length)
          edits.push({ start: base + node.loc.start.offset, end: base + node.loc.end.offset, text: lead + `{{ $t('${key}') }}` + tail })
        } else missed++
      }
      // 属性节点
      for (const p of node.props || []) {
        const raw = p.loc?.source || ''
        if (!raw || !hasZh(raw)) continue
        if (p.type === NodeTypes.ATTRIBUTE && p.value && hasZh(p.value.content || '')) {
          const key = keyOf(p.value.content.trim())
          if (key) {
            edits.push({ start: base + p.loc.start.offset, end: base + p.loc.end.offset, text: `:${p.name}="$t('${key}')"` })
            continue
          }
          missed++
        }
        // 其余（v-bind / v-if / @click 等）只替换里面能反查到的字符串字面量
        const [next, hit] = swapQuoted(raw, (k) => `$t('${k}')`)
        if (hit) edits.push({ start: base + p.loc.start.offset, end: base + p.loc.end.offset, text: next })
        else missed++
      }
      for (const c of node.children || []) walk(c)
    }
    walk(ast)
  }

  // ---- <script setup>
  if (descriptor.scriptSetup) {
    const ss = descriptor.scriptSetup
    // 只在脚本自己的范围内替换（边界由 SFC 描述符给出，不猜）
    // loc 覆盖整块含标签；用 content 在源里的位置反推
    const start = src.indexOf(ss.content, ss.loc.start.offset)
    const [next, hit] = swapQuoted(ss.content, (k) => `t('${k}')`)
    if (hit) edits.push({ start, end: start + ss.content.length, text: next })
  }

  // ---- 从后往前应用
  edits.sort((a, b) => b.start - a.start)
  let out = src
  for (const e of edits) out = out.slice(0, e.start) + e.text + out.slice(e.end)

  // ---- 脚本里用到了 t( 就保证有导入
  if (/\bt\('/.test(out) && !/^import \{[^}]*\bt\b[^}]*\} from '\.\.\/\.\.\/utils\/i18n'/m.test(out)) {
    if (/from '\.\.\/\.\.\/utils\/i18n'/.test(out)) {
      out = out.replace(/import \{([^}]*)\} from '\.\.\/\.\.\/utils\/i18n'/, (m, names) => (/\bt\b/.test(names) ? m : `import {${names}, t } from '../../utils/i18n'`))
    } else {
      out = out.replace(/^(import .*)$/m, `$1\nimport { t } from '../../utils/i18n'`)
    }
  }

  if (!DRY) writeFileSync(file, out.replace(/\n/g, '\r\n'), 'utf8')
  console.log('  ' + file.split('/').pop() + '：AST 替换 ' + edits.length + ' 处，未匹配 ' + missed + ' 处' + (DRY ? '（dry-run）' : ''))
}
