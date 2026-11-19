<template>
  <i class="nav-ic" :data-icon="name" :style="sizeStyle" aria-hidden="true"></i>
</template>

<script setup>
import { computed } from 'vue'

// 顶栏右侧那一组图标（2026-11-19 用户提供的一套 SVG，放 src/assets/nav/）。
//
// **为什么用 CSS mask 而不是内联 path**：这套图是 iconfont 导出的多色形状，颜色写死在
// path 上（#272636 的深灰、#3CBAFF 的品牌蓝……）。深色主题下深灰几乎看不见、浅色主题下
// 又要另一套色值 —— 内联就得给每个图标维护两套。遮罩把「形状」当模板、颜色统一取
// currentColor，于是深浅主题、悬停、激活态全都跟着顶栏文字色走，一份资源到底。
//
// 尺寸跟「设置 → 通用 → 界面尺寸 → 顶栏右侧图标」走（--dc-nav-icon），与原来
// `.top-icon-btn .el-icon { font-size: var(--dc-nav-icon) }` 同一档。
// 非顶栏场景（菜单项、空态大图）传 size：「1em」跟随所在容器的字号，
// 「32px」这类固定值用于空态插画。
const props = defineProps({
  name: {
    type: String,
    required: true,
    validator: (v) => [
      'home', 'github', 'knowledge', 'tasks', 'refresh', 'update', 'settings',
      // 主题与语言各两枚：按当前解析出的主题 / 当前语言选择，见 MainView 顶栏
      'theme-day', 'theme-night', 'lang-zh', 'lang-en',
    ].includes(v),
  },
  size: { type: [String, Number], default: '' },
})

const sizeStyle = computed(() => {
  const s = typeof props.size === 'number' ? `${props.size}px` : String(props.size || '').trim()
  return s ? { width: s, height: s } : null
})
</script>

<style>
.nav-ic {
  display: block;
  flex: none;
  width: var(--dc-nav-icon, 16px);
  height: var(--dc-nav-icon, 16px);
  background-color: currentColor;
  -webkit-mask-repeat: no-repeat;
  mask-repeat: no-repeat;
  -webkit-mask-position: center;
  mask-position: center;
  -webkit-mask-size: contain;
  mask-size: contain;
}
/* 检查更新：检查中 / 下载中原地转，代替原来换成 Loading 字形 */
.nav-ic.nav-ic-spin {
  animation: nav-ic-spin 1.1s linear infinite;
}
@keyframes nav-ic-spin {
  to { transform: rotate(360deg); }
}

.nav-ic[data-icon='home'] {
  -webkit-mask-image: url('../assets/nav/home.svg');
  mask-image: url('../assets/nav/home.svg');
  /* 小房子是宽扁图形（1200×1024，宽度本来就已经满幅）：在正方形框里只能长到 82% 高，
     跟旁边一圈圆图标比小一号。加宽元素让「按宽度适配」时高度也涨上来（顶栏按钮 30px，
     18.4px 的图标左右还剩 5px）。其余图标已由 viewBox 收到墨迹外接框，无需额外处理。 */
  width: calc(var(--dc-nav-icon, 16px) * 1.15);
  height: var(--dc-nav-icon, 16px);
}
.nav-ic[data-icon='github'] {
  -webkit-mask-image: url('../assets/nav/github.svg');
  mask-image: url('../assets/nav/github.svg');
}
.nav-ic[data-icon='knowledge'] {
  -webkit-mask-image: url('../assets/nav/knowledge.svg');
  mask-image: url('../assets/nav/knowledge.svg');
}
.nav-ic[data-icon='tasks'] {
  -webkit-mask-image: url('../assets/nav/tasks.svg');
  mask-image: url('../assets/nav/tasks.svg');
}
.nav-ic[data-icon='refresh'] {
  -webkit-mask-image: url('../assets/nav/refresh.svg');
  mask-image: url('../assets/nav/refresh.svg');
}
/* 主题：浅色主题显示太阳（白天）、深色主题显示月亮（晚上）—— 图标说明「现在是哪一种」 */
.nav-ic[data-icon='theme-day'] {
  -webkit-mask-image: url('../assets/nav/theme-day.svg');
  mask-image: url('../assets/nav/theme-day.svg');
}
.nav-ic[data-icon='theme-night'] {
  -webkit-mask-image: url('../assets/nav/theme-night.svg');
  mask-image: url('../assets/nav/theme-night.svg');
}
/* 语言：中文界面显示「中」、英文界面显示「e」 */
.nav-ic[data-icon='lang-zh'] {
  -webkit-mask-image: url('../assets/nav/lang-zh.svg');
  mask-image: url('../assets/nav/lang-zh.svg');
}
.nav-ic[data-icon='lang-en'] {
  -webkit-mask-image: url('../assets/nav/lang-en.svg');
  mask-image: url('../assets/nav/lang-en.svg');
}
.nav-ic[data-icon='update'] {
  -webkit-mask-image: url('../assets/nav/update.svg');
  mask-image: url('../assets/nav/update.svg');
}
.nav-ic[data-icon='settings'] {
  -webkit-mask-image: url('../assets/nav/settings.svg');
  mask-image: url('../assets/nav/settings.svg');
}
</style>
