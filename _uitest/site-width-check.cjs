// 宽度体系核对：全站所有内容块必须落在同一条左右边线上（只有背景色通栏）
const { chromium } = require('playwright-core')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const URL = 'file:///d:/develop/workspace/product/DBMind/website/index.html'

const BLOCKS = [
  '.nav-inner', 'header .wrap', '.hero-shots .shots', '.marquee',
  '#features .sec-title', '#features .grid', '.more-strip',
  '.demo', '.stats', '.flow', '.safe', '.final .wrap',
]

// 量「内容边线」而不是 border-box：容器自带 24px padding，按内容框算才能和内部块直接比
const rects = (p, sels) => p.evaluate((sels) => sels.map((sel) => {
  const el = document.querySelector(sel)
  if (!el) return { sel, missing: true }
  const r = el.getBoundingClientRect()
  const cs = getComputedStyle(el)
  const pl = parseFloat(cs.paddingLeft) || 0
  const pr = parseFloat(cs.paddingRight) || 0
  return { sel, left: Math.round(r.left + pl + window.scrollX), right: Math.round(r.right - pr + window.scrollX), w: Math.round(r.width - pl - pr) }
}), sels)

;(async () => {
  const b = await chromium.launch({ channel: 'msedge', headless: true })
  let fail = 0
  const bad = (m) => { console.log('  FAIL ' + m); fail++ }

  for (const vw of [1400, 1024, 820]) {
    const p = await b.newPage({ viewport: { width: vw, height: 900 } })
    const errs = []
    p.on('pageerror', (e) => errs.push(String(e).slice(0, 160)))
    await p.goto(URL, { waitUntil: 'load' })
    await sleep(500)
    await p.evaluate(() => window.scrollTo(0, document.body.scrollHeight))
    await sleep(400)
    await p.evaluate(() => window.scrollTo(0, 0))
    await sleep(200)

    console.log(`\n=== viewport ${vw} ===`)
    const rows = await rects(p, BLOCKS)
    for (const r of rows) {
      if (r.missing) { bad(`${r.sel} 不存在`); continue }
      console.log(`  ${r.sel.padEnd(22)} L=${String(r.left).padStart(4)} R=${String(r.right).padStart(4)} W=${String(r.w).padStart(4)}`)
    }
    const ok = rows.filter((r) => !r.missing)
    const L = [...new Set(ok.map((r) => r.left))]
    const R = [...new Set(ok.map((r) => r.right))]
    L.length === 1 && R.length === 1
      ? console.log(`  全站内容边线唯一: PASS (${L[0]}..${R[0]}，宽 ${ok[0].w})`)
      : bad(`内容边线不一致 left=${L.join('/')} right=${R.join('/')}`)
    if (errs.length) bad('pageerror: ' + errs.join(' | '))
    await p.close()
  }

  // 版本独占视图（#releases 默认隐藏）
  const p2 = await b.newPage({ viewport: { width: 1400, height: 900 } })
  await p2.goto(URL, { waitUntil: 'load' })
  await p2.evaluate(() => { document.body.classList.add('version-only'); window.scrollTo(0, 0) })
  await sleep(500)
  const tl = (await rects(p2, ['.rel-timeline']))[0]
  console.log(`\n=== version-only ===\n  .rel-timeline          L=${tl.left} R=${tl.right} W=${tl.w}`)
  tl.left === 164 && tl.right === 1236
    ? console.log('  时间线与主栅格对齐: PASS')
    : bad(`时间线 ${tl.left}..${tl.right} 不符（应 164..1236）`)
  await p2.close()

  await b.close()
  console.log(fail ? `\n${fail} 项 FAIL` : '\n全部 PASS')
  process.exit(fail ? 1 : 0)
})().catch((e) => { console.error('ERR', e.message); process.exit(1) })
