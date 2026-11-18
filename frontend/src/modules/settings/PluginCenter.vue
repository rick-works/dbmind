<!--
  插件中心（顶栏 → 插件中心，中央工作区页签）。

  ## 定位
  官网托管插件包，产品内一键安装。第一版只收**声明式**内容（manifest + 提示词），
  插件不带任何可执行文件 —— 后端解压时按后缀白名单拦截（见 api/plugins.rs 文件头）。

  技能**没有独立运行界面**：装好、启用后去 AI 助手的 Skills 里选中它，
  提示词拼进 system 后每一轮提问都由它生效。这里只管装 / 卸 / 启停 / 升级。

  ## 两条安装路径
  1. 「可安装」列表来自官网 plugins/index.json（拉不到不算错误，显示离线提示）；
  2. 「从文件安装」：开发者把 zip 发给用户，拖进来即可（本地开发/内网环境用）。
-->
<template>
  <div class="plugin-center">
    <!-- 说明 + 动作同一行：说明左对齐占满剩余宽度，按钮靠右（窄窗口自动换行） -->
    <div class="plg-top">
      <!-- 一行说完：是什么（声明式包）、怎么用（AI 助手的 Skills）、安全边界（不含可执行文件） -->
      <div class="plg-lead">{{ $t('plg.desc') }}</div>
      <div class="plg-bar">
        <button class="plg-btn" type="button" :disabled="loading" @click="reload">
          <el-icon><Refresh /></el-icon>{{ loading ? $t('plg.loading') : $t('plg.refresh') }}
        </button>
        <button class="plg-btn" type="button" :disabled="installing" @click="pickFile">
          <el-icon><Upload /></el-icon>{{ $t('plg.fromFile') }}
        </button>
      </div>
    </div>
    <input ref="fileRef" type="file" accept=".zip" class="plg-file" @change="onFile" />

    <!-- 已装 -->
    <div class="plg-section-title">
      {{ $t('plg.installed') }}<span class="plg-count">{{ installed.length }}</span>
    </div>
    <div v-if="!installed.length" class="plg-empty">{{ $t('plg.emptyInstalled') }}</div>
    <div v-else class="plg-grid">
      <div v-for="p in installed" :key="p.id" class="plg-card" :class="{ off: p.state === 'disabled' }">
        <div class="plg-card-head">
          <span class="plg-ic">{{ p.icon || '🧩' }}</span>
          <span class="plg-name" :title="p.id">{{ p.name }}</span>
          <span class="plg-ver">v{{ p.version }}</span>
        </div>
        <div class="plg-meta">{{ p.author || $t('plg.unknownAuthor') }} · {{ $t('plg.typeSkill') }}</div>
        <div class="plg-desc" :title="p.description">{{ p.description || '—' }}</div>
        <div class="plg-card-foot">
          <el-switch :model-value="p.state === 'enabled'" size="small" @change="(v) => toggle(p, v)" />
          <span class="plg-state">{{ p.state === 'enabled' ? $t('plg.enabled') : $t('plg.disabled') }}</span>
          <span class="plg-gap" />
          <button class="plg-btn sm danger" type="button" @click="remove(p)">{{ $t('plg.uninstall') }}</button>
        </div>
      </div>
    </div>

    <!-- 官网索引 -->
    <div class="plg-section-title">
      {{ $t('plg.available') }}<span class="plg-count">{{ available.length }}</span>
    </div>
    <div v-if="catalogError" class="plg-offline">{{ $t('plg.offline', { err: catalogError }) }}</div>
    <div v-else-if="!available.length" class="plg-empty">{{ $t('plg.emptyAvailable') }}</div>
    <div v-else class="plg-grid">
      <div v-for="c in available" :key="c.id" class="plg-card">
        <div class="plg-card-head">
          <span class="plg-ic">{{ c.icon || '🧩' }}</span>
          <span class="plg-name" :title="c.id">{{ loName(c.name) }}</span>
          <span class="plg-ver">v{{ c.version }}</span>
        </div>
        <div class="plg-meta">{{ c.author || $t('plg.unknownAuthor') }} · {{ $t('plg.typeSkill') }}</div>
        <div class="plg-desc">{{ loName(c.description) }}</div>
        <div class="plg-card-foot">
          <span class="plg-gap" />
          <button class="plg-btn sm primary" type="button" :disabled="!!installingId" @click="install(c)">
            {{ installingId === c.id ? $t('plg.installing')
              : (isUpgrade(c) ? $t('plg.upgrade', { ver: c.version }) : $t('plg.install')) }}
          </button>
        </div>
      </div>
    </div>

  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Refresh, Upload } from '@element-plus/icons-vue'
import { t, locale } from '../../utils/i18n'
import { listPlugins, getPluginCatalog, installPlugin, installPluginFile, setPluginEnabled, uninstallPlugin } from '../../api'

const loading = ref(false)
const installing = ref(false)
const installingId = ref('')
const installed = ref([])
const catalog = ref([])
const catalogError = ref('')
const fileRef = ref(null)

/**
 * 官网索引里的 name / description 是 `{ zh, en }` 双语对象，已装插件的清单由后端本地化过、是字符串。
 * 两种形态都从这一处取，模板里直接 `{{ c.name }}` 会把对象原样打印出来。
 */
const loName = (v) => {
  if (!v || typeof v !== 'object') return v || ''
  const cur = locale.value === 'en-US' ? v.en : v.zh
  return cur || v.zh || v.en || ''
}

/** 已装的 id：商店列表里要标成「已安装」而不是再给一次安装按钮 */
const installedIds = computed(() => new Set(installed.value.map((p) => p.id)))
/** 已装版本：索引里更高的版本要能「升级」——只按 id 过滤掉会把用户永久钉在旧版 */
const installedVer = computed(() => new Map(installed.value.map((p) => [p.id, p.version || '0'])))
/** x.y.z 比大小：逐段按数字比，非法段按 0（插件版本都是纯数字三段） */
const verNum = (v) => String(v || '').split('.').slice(0, 4)
  .reduce((acc, n) => acc * 1000 + (parseInt(n, 10) || 0), 0)
const isUpgrade = (c) => installedIds.value.has(c.id)
const available = computed(() => catalog.value.filter((c) => {
  const cur = installedVer.value.get(c.id)
  return !cur || verNum(c.version) > verNum(cur)
}))

const reload = async () => {
  loading.value = true
  try {
    const data = await listPlugins()
    installed.value = data.items || []
  } catch (e) {
    ElMessage.error(e?.response?.data?.message || String(e))
  } finally {
    loading.value = false
  }
  try {
    const cat = await getPluginCatalog()
    if (cat.ok) {
      catalog.value = cat.data?.plugins || []
      catalogError.value = ''
    } else {
      catalog.value = []
      catalogError.value = cat.error || t('plg.offlineUnknown')
    }
  } catch (e) {
    catalog.value = []
    catalogError.value = e?.message || String(e)
  }
  // 让 AI 面板的技能入口跟着变（装完/卸完不必等下次开下拉）
  window.dispatchEvent(new CustomEvent('dbmind-plugins-changed'))
}

const pickFile = () => fileRef.value?.click()

const onFile = async (ev) => {
  const file = ev.target.files?.[0]
  ev.target.value = ''
  if (!file) return
  installing.value = true
  try {
    const res = await installPluginFile(file)
    ElMessage.success(t('plg.installedOk', { name: res.name || res.id }))
    await reload()
  } catch (e) {
    ElMessage.error(e?.response?.data?.message || String(e))
  } finally {
    installing.value = false
  }
}

const install = async (item) => {
  const upgrading = isUpgrade(item)
  installingId.value = item.id
  try {
    await installPlugin({ id: item.id, url: item.download, sha256: item.sha256 })
    ElMessage.success(t(upgrading ? 'plg.upgraded' : 'plg.installedOk', { name: loName(item.name) }))
    await reload()
  } catch (e) {
    ElMessage.error(e?.response?.data?.message || String(e))
  } finally {
    installingId.value = ''
  }
}

const toggle = async (item, on) => {
  try {
    await setPluginEnabled(item.id, on)
    item.state = on ? 'enabled' : 'disabled'
    window.dispatchEvent(new CustomEvent('dbmind-plugins-changed'))
  } catch (e) {
    ElMessage.error(e?.response?.data?.message || String(e))
  }
}

const remove = async (item) => {
  try {
    await ElMessageBox.confirm(t('plg.removeConfirm', { name: item.name }), t('plg.uninstall'), {
      type: 'warning', confirmButtonText: t('plg.uninstall'), cancelButtonText: t('common.cancel')
    })
  } catch { return }
  try {
    await uninstallPlugin(item.id)
    ElMessage.success(t('plg.removed'))
    await reload()
  } catch (e) {
    ElMessage.error(e?.response?.data?.message || String(e))
  }
}

onMounted(reload)
</script>

<style scoped>
.plugin-center { padding-bottom: 4px; }
/* 弹窗里没有「面板标题」：说明与动作挤在同一行，纵向省一截，卡片能更快露出来 */
.plg-top { display: flex; align-items: center; gap: 14px; flex-wrap: wrap; margin: 0 0 8px; }
.plg-lead { flex: 1 1 320px; min-width: 0; font-size: 12.5px; line-height: 1.7; color: var(--dc-text-dim); }
.plg-bar { flex: 0 0 auto; display: flex; align-items: center; gap: 8px; margin-left: auto; }
.plg-file { display: none; }
.plg-btn { display: inline-flex; align-items: center; gap: 4px; height: 28px; padding: 0 10px; font-size: 12px;
  border: 1px solid var(--dc-border); border-radius: 6px; background: var(--dc-surface); color: var(--dc-text); cursor: pointer; }
.plg-btn:hover:not(:disabled) { border-color: var(--dc-primary); color: var(--dc-primary); }
.plg-btn:disabled { opacity: .5; cursor: not-allowed; }
.plg-btn.sm { height: 24px; padding: 0 8px; font-size: 12px; }
.plg-btn.primary { border-color: var(--dc-primary); color: var(--dc-primary); }
.plg-btn.danger:hover { border-color: #f56c6c; color: #f56c6c; }

.plg-section-title { display: flex; align-items: center; gap: 6px; margin: 18px 0 8px; font-size: 13px; font-weight: 600; }
.plg-count { font-size: 11px; font-weight: 400; color: var(--dc-text-weak); background: var(--dc-primary-wash);
  border-radius: 8px; padding: 0 6px; }
.plg-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 10px; }
.plg-card { border: 1px solid var(--dc-border); border-radius: 8px; padding: 10px 12px; background: var(--dc-surface);
  display: flex; flex-direction: column; gap: 6px; }
.plg-card.off { opacity: .6; }
.plg-card-head { display: flex; align-items: center; gap: 8px; }
/* 插件清单里的 emoji 图标：与标题同排，占位固定避免有无图标时标题跳动 */
.plg-ic { font-size: 15px; line-height: 1; flex-shrink: 0; }
.plg-name { font-size: 13px; font-weight: 600; }
.plg-ver { font-size: 11px; color: var(--dc-text-weak); }
.plg-meta { font-size: 11px; color: var(--dc-text-weak); }
.plg-desc { font-size: 12px; color: var(--dc-text-weak); line-height: 1.5; display: -webkit-box;
  -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; min-height: 36px; }
.plg-card-foot { display: flex; align-items: center; gap: 6px; margin-top: 2px; }
.plg-state { font-size: 11px; color: var(--dc-text-weak); }
.plg-gap { flex: 1; }
.plg-empty { font-size: 12px; color: var(--dc-text-weak); padding: 14px; border: 1px dashed var(--dc-border); border-radius: 8px; }
.plg-offline { font-size: 12px; color: #e6a23c; padding: 10px 12px; border: 1px dashed rgba(230, 162, 60, .5);
  border-radius: 8px; line-height: 1.6; word-break: break-all; }
</style>
