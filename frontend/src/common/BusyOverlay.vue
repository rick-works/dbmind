<template>
  <teleport to="body">
    <div v-if="visible" class="busy-mask" @click.self="noop">
      <div class="busy-card">
        <span class="busy-spin" :class="{ 'is-idle': canceling }" />
        <div class="busy-text">{{ title }}</div>
        <div class="busy-sub">{{ subtitle || $t('busy.elapsed', { t: elapsedText }) }}</div>
        <el-button v-if="cancelable" size="small" :disabled="canceling" @click="$emit('cancel')">
          {{ canceling ? $t('tpd.canceling') : $t('common.cancel') }}
        </el-button>
      </div>
    </div>
  </teleport>
</template>

<script setup>
// 通用「进行中」遮罩：用于导出 / 导入 / 执行脚本等无法上报百分比的耗时操作。
// 相比 ElLoading 多两点：显示实时用时（让用户知道没卡死）+ 提供取消入口（避免长时间停不掉）。
import { ref, watch, onBeforeUnmount } from 'vue'
import { t } from '../utils/i18n'

const props = defineProps({
  visible: { type: Boolean, default: false },
  title: { type: String, default: () => t('busy.working') },
  subtitle: { type: String, default: '' },
  cancelable: { type: Boolean, default: true },
  // 已发出取消请求、等待后端确认（避免重复点击）
  canceling: { type: Boolean, default: false }
})
defineEmits(['cancel'])

const elapsed = ref(0)
let timer = null
watch(() => props.visible, (v) => {
  if (timer) { clearInterval(timer); timer = null }
  if (!v) return
  const start = Date.now()
  elapsed.value = 0
  timer = setInterval(() => { elapsed.value = Date.now() - start }, 200)
}, { immediate: true })
onBeforeUnmount(() => { if (timer) clearInterval(timer) })

const elapsedText = ref('')
watch(elapsed, (ms) => {
  elapsedText.value = ms < 1000 ? ms + ' ms' : (ms / 1000).toFixed(1) + ' s'
}, { immediate: true })

const noop = () => {}
</script>

<style scoped>
.busy-mask {
  position: fixed;
  inset: 0;
  z-index: 4200;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.45);
  backdrop-filter: blur(2px);
}
.busy-card {
  min-width: 260px;
  padding: 20px 24px;
  border-radius: 10px;
  background: var(--dc-bg-raised);
  border: 1px solid var(--dc-border);
  box-shadow: var(--dc-shadow-md, 0 8px 24px rgba(0, 0, 0, 0.25));
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}
.busy-spin {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  border: 2px solid var(--dc-border-strong);
  border-top-color: var(--dc-primary);
  animation: busy-rotate 0.8s linear infinite;
}
.busy-spin.is-idle { animation-duration: 1.8s; }
.busy-text { font-size: 14px; color: var(--dc-text); }
.busy-sub { font-size: 13px; color: var(--dc-text-dim); }
@keyframes busy-rotate { to { transform: rotate(360deg); } }
</style>
