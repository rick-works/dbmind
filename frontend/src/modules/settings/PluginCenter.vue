<!--
  插件中心（设置 → 插件）。

  ## 定位
  官网托管插件包，产品内一键安装。第一版只收**声明式**内容（manifest + 提示词），
  插件不带任何可执行文件 —— 后端解压时按后缀白名单拦截（见 api/plugins.rs 文件头）。

  ## 两条安装路径
  1. 「可安装」列表来自官网 plugins/index.json（拉不到不算错误，显示离线提示）；
  2. 「从文件安装」：开发者把 zip 发给用户，拖进来即可（本地开发/内网环境用）。

  ## 为什么运行结果不用 markdown 渲染
  技能输出是**给数据库人看的**（问题清单 / 改造 SQL / 风险），等宽字体 + 保留换行
  比渲染成 HTML 更好读，也省掉一层 XSS 面。
-->
<template>
  <div class="plugin-center">
    <div class="panel-title">{{ $t('plg.title') }}</div>
    <div class="panel-desc">{{ $t('plg.desc') }}</div>

    <div class="plg-bar">
      <button class="plg-btn" type="button" :disabled="loading" @click="reload">
        <el-icon><Refresh /></el-icon>{{ loading ? $t('plg.loading') : $t('plg.refresh') }}
      </button>
      <button class="plg-btn" type="button" :disabled="installing" @click="pickFile">
        <el-icon><Upload /></el-icon>{{ $t('plg.fromFile') }}
      </button>
      <input ref="fileRef" type="file" accept=".zip" class="plg-file" @change="onFile" />
      <span class="plg-dir" :title="dir">{{ $t('plg.dir') }}：{{ dir || '—' }}</span>
    </div>

    <!-- 已装 -->
    <div class="plg-section-title">
      {{ $t('plg.installed') }}<span class="plg-count">{{ installed.length }}</span>
    </div>
    <div v-if="!installed.length" class="plg-empty">{{ $t('plg.emptyInstalled') }}</div>
    <div v-else class="plg-grid">
      <div v-for="p in installed" :key="p.id" class="plg-card" :class="{ off: p.state === 'disabled' }">
        <div class="plg-card-head">
          <span class="plg-name" :title="p.id">{{ p.name }}</span>
          <span class="plg-ver">v{{ p.version }}</span>
        </div>
        <div class="plg-meta">{{ p.author || $t('plg.unknownAuthor') }} · {{ $t('plg.typeSkill') }}</div>
        <div class="plg-desc" :title="p.description">{{ p.description || '—' }}</div>
        <div class="plg-card-foot">
          <el-switch :model-value="p.state === 'enabled'" size="small" @change="(v) => toggle(p, v)" />
          <span class="plg-state">{{ p.state === 'enabled' ? $t('plg.enabled') : $t('plg.disabled') }}</span>
          <span class="plg-gap" />
          <button class="plg-btn sm" type="button" :disabled="p.state !== 'enabled'" @click="openRun(p)">
            {{ $t('plg.run') }}
          </button>
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
          <span class="plg-name" :title="c.id">{{ c.name }}</span>
          <span class="plg-ver">v{{ c.version }}</span>
        </div>
        <div class="plg-meta">{{ c.author || $t('plg.unknownAuthor') }} · {{ $t('plg.typeSkill') }}</div>
        <div class="plg-desc">{{ c.description }}</div>
        <div class="plg-card-foot">
          <span class="plg-gap" />
          <button class="plg-btn sm primary" type="button" :disabled="!!installingId" @click="install(c)">
            {{ installingId === c.id ? $t('plg.installing') : $t('plg.install') }}
          </button>
        </div>
      </div>
    </div>

    <!-- 运行技能：表单与调用逻辑与 AI 面板共用同一个组件（PluginSkillRun） -->
    <PluginSkillRun v-model="runOpen" :item="runItem" />
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Refresh, Upload } from '@element-plus/icons-vue'
import { t } from '../../utils/i18n'
import { listPlugins, getPluginCatalog, installPlugin, installPluginFile, setPluginEnabled, uninstallPlugin } from '../../api'
import PluginSkillRun from '../ai/PluginSkillRun.vue'

const loading = ref(false)
const installing = ref(false)
const installingId = ref('')
const dir = ref('')
const installed = ref([])
const catalog = ref([])
const catalogError = ref('')
const fileRef = ref(null)

/** 已装的 id：商店列表里要标成「已安装」而不是再给一次安装按钮 */
const installedIds = computed(() => new Set(installed.value.map((p) => p.id)))
const available = computed(() => catalog.value.filter((c) => !installedIds.value.has(c.id)))

const reload = async () => {
  loading.value = true
  try {
    const data = await listPlugins()
    installed.value = data.items || []
    dir.value = data.dir || ''
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
  installingId.value = item.id
  try {
    await installPlugin({ id: item.id, url: item.download, sha256: item.sha256 })
    ElMessage.success(t('plg.installedOk', { name: item.name }))
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

// ---------------------------------------------------------------- 运行

/** 只负责「点哪个技能」—— 表单、填参、调用都在 PluginSkillRun 里（与 AI 面板共用） */
const runOpen = ref(false)
const runItem = ref(null)
const openRun = (item) => {
  runItem.value = item
  runOpen.value = true
}

onMounted(reload)
</script>

<style scoped>
.plugin-center { padding-bottom: 20px; }
.plg-bar { display: flex; align-items: center; gap: 8px; margin: 12px 0 4px; flex-wrap: wrap; }
.plg-file { display: none; }
.plg-btn { display: inline-flex; align-items: center; gap: 4px; height: 28px; padding: 0 10px; font-size: 12px;
  border: 1px solid var(--dc-border); border-radius: 6px; background: var(--dc-surface); color: var(--dc-text); cursor: pointer; }
.plg-btn:hover:not(:disabled) { border-color: var(--dc-primary); color: var(--dc-primary); }
.plg-btn:disabled { opacity: .5; cursor: not-allowed; }
.plg-btn.sm { height: 24px; padding: 0 8px; font-size: 12px; }
.plg-btn.primary { border-color: var(--dc-primary); color: var(--dc-primary); }
.plg-btn.danger:hover { border-color: #f56c6c; color: #f56c6c; }
.plg-dir { font-size: 12px; color: var(--dc-text-weak); margin-left: auto; max-width: 46%; overflow: hidden;
  text-overflow: ellipsis; white-space: nowrap; }
.plg-section-title { display: flex; align-items: center; gap: 6px; margin: 18px 0 8px; font-size: 13px; font-weight: 600; }
.plg-count { font-size: 11px; font-weight: 400; color: var(--dc-text-weak); background: var(--dc-primary-wash);
  border-radius: 8px; padding: 0 6px; }
.plg-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 10px; }
.plg-card { border: 1px solid var(--dc-border); border-radius: 8px; padding: 10px 12px; background: var(--dc-surface);
  display: flex; flex-direction: column; gap: 6px; }
.plg-card.off { opacity: .6; }
.plg-card-head { display: flex; align-items: baseline; gap: 8px; }
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
