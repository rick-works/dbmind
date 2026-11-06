/**
 * 原生 `title` 的「快速提示」接管。
 *
 * <p>项目里有近 300 处 `:title="…"`（表格单元格、树节点、按钮、页签…）。浏览器自带的
 * 悬停提示要等约 1 秒才弹，体感就是「放上去半天没反应」，而且样式跟着系统走、不可控。
 *
 * <p>这里**不动模板**（改 279 处必然引入布局与绑定问题），改成运行时事件委托：
 * 鼠标进入带 `title` 的元素时先把属性摘掉（不摘的话浏览器随后还会弹它自己那个），
 * 50ms 后用全局浮层显示；离开 / 按下 / 滚动 / 失焦时收起，并把 `title` 还回去
 * （Vue 认为值没变不会再 patch，不还回去下次悬停就读不到内容了）。
 *
 * <p><b>关键：`title` 的还原必须挂在「已摘掉 title 的元素」上，而不是「正在显示浮层的元素」上。</b>
 * 前者比后者早 -- 鼠标停不到 50ms 就移走时浮层还没显示，但 title 已经摘了；如果只按
 * 显示中的元素还原，这次就永远不还，第二次悬停读不到 title，表现为「只弹一次」。
 *
 * <p>只管原生 `title`：`el-tooltip` 的内容放在 popper 里、触发元素上没有 title 属性，
 * 天然不冲突。浮层 `pointer-events:none`，不会自己抢走鼠标事件。
 */

/** 悬停多久后弹出。50ms 足够滤掉「鼠标扫过」的误触，又快到几乎无感。 */
const SHOW_DELAY = 50
/** 浮层与元素的间距（含箭头） */
const OFFSET = 10
/** 浮层最大宽度：再宽就换行，避免横穿半个屏幕 */
const MAX_WIDTH = 420
/** 箭头尺寸 */
const ARROW = 8
/**
 * 气泡配色与 el-tooltip 共用同一组主题变量（见 styles/index.css 的 --dc-tip-*），
 * 深浅主题各自成立；变量缺失时退回原来的深色，保证仍然可用。
 */
const BG = 'var(--dc-tip-bg, rgba(48,49,51,.97))'
const FG = 'var(--dc-tip-fg, #fff)'
const SHADOW = 'var(--dc-tip-shadow, 0 8px 24px rgba(0,0,0,.3))'

let layer = null
let timer = null
/** 已经摘掉 title 的元素（可能还在等待显示，也可能正在显示） */
let armedEl = null
/** 元素 -> 被我们摘掉的 title 原文 */
const stashed = new WeakMap()
let installed = false

const ensureLayer = () => {
  if (layer && document.body.contains(layer)) return layer
  layer = document.createElement('div')
  layer.id = 'dbmind-fast-tip'
  layer.setAttribute('role', 'tooltip')
  layer.style.cssText = [
    'position:fixed',
    'z-index:4000',
    'pointer-events:none',
    'display:none',
    'box-sizing:border-box',
    `max-width:${MAX_WIDTH}px`,
    `background:${BG}`,
    `color:${FG}`,
    'border-radius:6px',
    'padding:7px 11px',
    'font-size:12px',
    'line-height:1.5',
    'font-family:inherit',
    'white-space:pre-line',
    'overflow-wrap:anywhere',
    'word-break:break-word',
    `box-shadow:${SHADOW}`,
    'opacity:0',
    'transform:translateY(2px)',
    'transition:opacity .12s ease,transform .12s ease'
  ].join(';')

  const arrow = document.createElement('i')
  arrow.className = 'ft-arrow'
  arrow.style.cssText = [
    'position:absolute',
    `width:${ARROW}px`,
    `height:${ARROW}px`,
    'background:inherit',
    'transform:rotate(45deg)',
    'border-radius:1px'
  ].join(';')

  const text = document.createElement('span')
  text.className = 'ft-text'

  layer.appendChild(arrow)
  layer.appendChild(text)
  document.body.appendChild(layer)
  return layer
}

/** 定位：默认在元素下方，空间不够翻到上方，左右夹在视口内，箭头始终指向元素中心 */
const place = (el) => {
  const box = el.getBoundingClientRect()
  const node = ensureLayer()
  const arrow = node.querySelector('.ft-arrow')
  node.style.left = '0px'
  node.style.top = '0px'
  node.style.display = 'block'

  const width = node.offsetWidth
  const height = node.offsetHeight
  const viewW = window.innerWidth
  const viewH = window.innerHeight

  let top = box.bottom + OFFSET
  let above = false
  if (top + height > viewH - 4) {
    const candidate = box.top - OFFSET - height
    if (candidate >= 4) {
      top = candidate
      above = true
    } else {
      top = Math.max(4, viewH - height - 4)
    }
  }

  let left = box.left + box.width / 2 - width / 2
  left = Math.min(Math.max(4, left), Math.max(4, viewW - width - 4))

  node.style.left = `${left}px`
  node.style.top = `${top}px`

  // 箭头指向元素中心（浮层被夹到边界时中心并不对齐，所以单独算一次）
  const centerX = box.left + box.width / 2
  const arrowLeft = Math.min(Math.max(8, centerX - left - ARROW / 2), Math.max(8, width - ARROW - 8))
  arrow.style.left = `${arrowLeft}px`
  if (above) {
    arrow.style.top = 'auto'
    arrow.style.bottom = `-${ARROW / 2}px`
    node.style.transform = 'translateY(-2px)'
  } else {
    arrow.style.bottom = 'auto'
    arrow.style.top = `-${ARROW / 2}px`
    node.style.transform = 'translateY(2px)'
  }
}

const hide = () => {
  if (timer) {
    clearTimeout(timer)
    timer = null
  }
  if (layer) {
    layer.style.display = 'none'
    layer.style.opacity = '0'
  }
  const el = armedEl
  armedEl = null
  if (!el) return
  const text = stashed.get(el)
  stashed.delete(el)
  // 还回去：元素已销毁就不用管了（WeakMap 会自己回收）
  if (text != null && document.body.contains(el)) el.setAttribute('title', text)
}

const show = (el) => {
  const text = stashed.get(el)
  if (!text) return
  const node = ensureLayer()
  node.querySelector('.ft-text').textContent = text
  place(el)
  // 下一帧再淡入，否则 opacity 过渡不会触发
  requestAnimationFrame(() => {
    if (armedEl !== el) return
    node.style.opacity = '1'
    node.style.transform = 'translateY(0)'
  })
}

const targetElement = (node) => {
  if (node instanceof Element) return node
  return node && node.parentElement instanceof Element ? node.parentElement : null
}

const onOver = (event) => {
  const el = targetElement(event.target)
  if (!el || el === armedEl) return
  const text = el.getAttribute('title')
  // 鼠标在同一片提示区域内的子节点之间移动：别把提示收掉，也别反复弹
  if (armedEl && armedEl.contains(el) && !text) return
  if (armedEl) hide()
  if (!text || !text.trim()) return
  stashed.set(el, text)
  // 摘掉原生 title，否则浏览器会在 1 秒后再弹一个自己的
  el.removeAttribute('title')
  armedEl = el
  timer = setTimeout(() => {
    timer = null
    if (armedEl === el) show(el)
  }, SHOW_DELAY)
}

const onOut = (event) => {
  if (!armedEl) return
  const to = event.relatedTarget
  // 在元素内部从一个子节点移到另一个子节点，不算离开
  if (to instanceof Node && armedEl.contains(to)) return
  hide()
}

/** 装一次即可（HMR 会重复执行，用 installed 兜住） */
export const installFastTooltip = () => {
  if (installed || typeof document === 'undefined') return
  installed = true
  // 捕获阶段：Element 的组件内部偶有 stopPropagation，冒泡会被吞掉
  document.addEventListener('mouseover', onOver, true)
  document.addEventListener('mouseout', onOut, true)
  // 这些场景下浮层会「挂」在原地，主动收起来
  document.addEventListener('mousedown', hide, true)
  document.addEventListener('wheel', hide, { capture: true, passive: true })
  window.addEventListener('scroll', hide, true)
  window.addEventListener('resize', hide)
  window.addEventListener('blur', hide)
}

export default installFastTooltip
