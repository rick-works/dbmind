<template>
  <el-dialog :model-value="modelValue" width="min(880px, 94vw)" top="8vh" append-to-body
             :close-on-click-modal="!importing" :show-close="!importing"
             @update:model-value="$emit('update:modelValue', $event)"
             class="ic-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Upload /></el-icon></span>
        <span>{{ $t('ci.title') }}</span>
        <el-tag v-if="source" size="small" effect="plain" class="ic-src">{{ sourceLabel(source) }}</el-tag>
      </div>
    </template>

    <!-- 第一步：选文件 -->
    <div v-if="!parsed.length" class="ic-drop" @click="pickFile">
      <el-icon :size="30"><FolderOpened /></el-icon>
      <div class="ic-drop-t"> {{ $t('ci.pick') }} </div>
      <div class="ic-drop-s">{{ $t('ci.pickHint') }}</div>
      <div class="ic-srcs">
        <el-tag v-for="s in IMPORT_SOURCES" :key="s" size="small" effect="plain">{{ sourceLabel(s) }}</el-tag>
      </div>
    </div>

    <!-- 第二步：预览 + 勾选 -->
    <template v-else>
      <div class="ic-head">
        <span class="ic-file" :title="fileName">{{ fileName }}</span>
        <span class="ic-count">{{ checkedIds.length }} / {{ importable.length }}</span>
        <span v-if="group" class="ic-group">{{ $t('ci.intoGroup', { group }) }}</span>
        <el-input v-model="keyword" :prefix-icon="Search" clearable class="ic-search" :placeholder="$t('ci.search')" />
      </div>

      <!-- 同名连接怎么处理：三种方式由用户选，重命名时可自改后缀 -->
      <div class="ic-dup">
        <span class="ic-dup-t">{{ $t('ci.dupTitle') }}</span>
        <el-radio-group v-model="dupMode" size="small" class="ic-dup-mode">
          <el-radio-button value="skip">{{ $t('ci.dupSkip') }}</el-radio-button>
          <el-radio-button value="overwrite">{{ $t('ci.dupOverwrite') }}</el-radio-button>
          <el-radio-button value="rename">{{ $t('ci.dupRename') }}</el-radio-button>
        </el-radio-group>
        <template v-if="dupMode === 'rename'">
          <span class="ic-dup-t">{{ $t('ci.dupSuffix') }}</span>
          <el-input v-model="dupSuffix" size="small" class="ic-dup-suffix"
                    :style="dupSuffixStyle" :placeholder="$t('ci.dupSuffixDefault')" />
        </template>
        <span v-if="dupCount" class="ic-dup-hit">{{ $t('ci.dupHit', { n: dupCount }) }}</span>
      </div>

      <div class="ic-list">
        <el-checkbox v-for="c in visibleRows" :key="c._uid"
                     :model-value="checkedIds.includes(c._uid)"
                     :disabled="!c.type"
                     @change="(v) => toggle(c._uid, v)">
          <DbLogo :type="c.type || 'MYSQL'" :size="16" class="ic-logo" />
          <span class="ic-name">{{ c.name }}</span>
          <!-- 类型由左侧图标表示；只有认不出类型时才用文字说明（否则这一行是禁用的，得给原因） -->
          <span v-if="!c.type" class="ic-type">{{ $t('ci.typeUnknown') }}</span>
          <span class="ic-meta" v-if="c.host">{{ c.host }}<template v-if="c.port">:{{ c.port }}</template></span>
          <span class="ic-meta" v-else-if="c.filePath">{{ c.filePath }}</span>
          <el-tag v-if="isDup(c.name)" size="small" type="warning" effect="plain" class="ic-dup-tag">{{ $t('ci.dupTag') }}</el-tag>
          <el-tag size="small" :type="c.pwd === PWD_INCLUDED ? 'success' : 'info'" effect="plain" class="ic-pwd">
            {{ c.pwd === PWD_INCLUDED ? $t('ci.pwdYes') : (c.pwd === PWD_ENCRYPTED ? $t('ci.pwdEnc') : $t('ci.pwdNo')) }}
          </el-tag>
        </el-checkbox>
        <el-empty v-if="!visibleRows.length" :image-size="60" :description="$t('common.noMatch')" />
      </div>

      <!-- 文件级提示放列表下方（原来这里写死一句「口令需补填」，正常导入时是噪声，已删）；
           真正需要当心的提醒（如 Navicat 口令读不出来）占用这一行 -->
      <div v-for="w in warnings" :key="w" class="ic-warn">
        <el-icon :size="13"><InfoFilled /></el-icon><span>{{ w }}</span>
      </div>
    </template>

    <template #footer>
      <el-button :disabled="importing" @click="close">{{ $t('common.cancel') }}</el-button>
      <el-button v-if="!parsed.length" type="primary" @click="pickFile">{{ $t('ci.pick') }}</el-button>
      <el-button v-else type="primary" :disabled="!willImport" :loading="importing" @click="doImport">
        {{ $t('ci.importN', { n: willImport }) }}
      </el-button>
    </template>

    <input ref="fileRef" type="file" accept=".json,.ncx,.xml,application/json,text/xml" style="display:none" @change="onFile" />
  </el-dialog>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { Upload, Search, FolderOpened, InfoFilled } from '@element-plus/icons-vue'
import { ElMessage } from 'element-plus'
import { t } from '../utils/i18n'
import { saveConnection, listConnections } from '../api'
import { sanitizeConnection } from '../utils/connTransfer'
import DbLogo from './DbLogo.vue'
import {
  IMPORT_SOURCES, sourceLabel, parseImportFile,
  PWD_INCLUDED, PWD_ENCRYPTED
} from '../utils/connImporters'

/**
 * 导入连接配置：DBMind / Navicat / DBeaver 三种文件都从这里进。
 *
 * 之所以要"预览"这一步，而不是选完文件直接落库：外部工具的类型/字段跟 DBMind 不是一一对应，
 * 有些条目会认不出类型（这些直接禁用勾选），有些拿不到口令（导入后要补填）。
 * 静默导入一堆连不上的连接，比导入失败更难排查。
 */
const props = defineProps({
  modelValue: Boolean,
  /** 导入到哪个分组（树里在某个分组右键进来时带），空 = 保持文件里的分组 */
  group: { type: String, default: '' }
})
const emit = defineEmits(['update:modelValue', 'imported'])

const fileRef = ref(null)
const raw = ref([])
const source = ref('')
const fileName = ref('')
const warnings = ref([])
const keyword = ref('')
const checkedIds = ref([])
const importing = ref(false)
/** 已存连接（只用来判「同名」：名称 → 记录，重名以第一条为准） */
const existing = ref([])
/** 同名处理方式：skip 跳过 / overwrite 覆盖 / rename 重命名（加后缀） */
const dupMode = ref('rename')
/** 重命名用的后缀，默认「新」，用户可以自己改 */
const dupSuffix = ref('')
let uid = 0

/** 只保留能落库的条目（类型识别出来的）；认不出类型的照样显示，但不可勾选 */
const importable = computed(() => raw.value.filter((c) => c.type))
const parsed = computed(() => raw.value)

const existingByName = computed(() => {
  const map = new Map()
  for (const c of existing.value) map.set(String(c.name || '').trim(), c)
  return map
})
const isDup = (name) => existingByName.value.has(String(name || '').trim())
/** 文件里有几条与已存连接同名（只算勾选中的，界面提示用） */
const dupCount = computed(() => raw.value.filter(
  (c) => c.type && checkedIds.value.includes(c._uid) && isDup(c.name)
).length)
/** 按当前同名策略，实际会落库的条数（跳过模式下要扣掉同名的） */
const willImport = computed(() => {
  const list = raw.value.filter((c) => c.type && checkedIds.value.includes(c._uid))
  if (dupMode.value !== 'skip') return list.length
  return list.filter((c) => !isDup(c.name)).length
})

/** 后缀输入框跟着内容走：默认一个字就是个窄框（写死 96px 时旁边空一大片，看着怪） */
const dupSuffixStyle = computed(() => {
  const text = String(dupSuffix.value || '') || t('ci.dupSuffixDefault')
  let w = 0
  for (const ch of text) w += /[\u2E80-\u9FFF\uFF00-\uFFEF]/.test(ch) ? 13 : 8
  return { width: Math.min(240, Math.max(56, w + 32)) + 'px' }
})

const visibleRows = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  if (!kw) return raw.value
  return raw.value.filter((c) => String(c.name || '').toLowerCase().includes(kw)
    || String(c.host || '').toLowerCase().includes(kw)
    || String(c.type || '').toLowerCase().includes(kw))
})

const pickFile = () => {
  if (!fileRef.value) return
  fileRef.value.value = ''
  fileRef.value.click()
}

const reset = () => {
  raw.value = []
  source.value = ''
  fileName.value = ''
  warnings.value = []
  keyword.value = ''
  checkedIds.value = []
  dupMode.value = 'rename'
  dupSuffix.value = t('ci.dupSuffixDefault')
}

const close = () => { if (!importing.value) { emit('update:modelValue', false) } }

/** 拉一遍已存连接：同名判定 + 覆盖时要用到它的 id。失败就当作「没有同名」 */
const loadExisting = async () => {
  try {
    existing.value = (await listConnections()) || []
  } catch (e) {
    existing.value = []
  }
}

watch(() => props.modelValue, (v) => { if (v) { reset(); loadExisting() } })

const onFile = async (e) => {
  const file = e.target.files && e.target.files[0]
  if (!file) return
  let text = ''
  try {
    text = await file.text()
  } catch (err) {
    ElMessage.error(t('ci.readFailed'))
    return
  }
  const res = parseImportFile(text, file.name)
  if (res.error) {
    ElMessage.error(res.error)
    return
  }
  raw.value = res.list.map((c) => ({ ...c, _uid: ++uid }))
  source.value = res.source
  fileName.value = file.name
  warnings.value = res.warnings || []
  checkedIds.value = importable.value.map((c) => c._uid)
}

const toggle = (id, checked) => {
  const set = new Set(checkedIds.value)
  if (checked) set.add(id)
  else set.delete(id)
  checkedIds.value = raw.value.filter((c) => c.type && set.has(c._uid)).map((c) => c._uid)
}

/**
 * 落库 payload：口令只在"文件里真的带了明文"时才写进去，其它情况留空（空 = 不覆盖/未设置）
 *
 * `targetName` / `targetId` 由同名策略给：重命名时是加过后缀的新名字，
 * 覆盖时是被覆盖那条连接的 id（带 id 保存 = 更新，口令留空 = 沿用旧口令）。
 */
const payloadOf = (item, targetName = '', targetId = '') => {
  const payload = sanitizeConnection(item)
  delete payload.pwd
  delete payload._uid
  payload.id = targetId || ''
  if (targetName) payload.name = targetName
  payload.password = item.pwd === PWD_INCLUDED ? (item.password || '') : ''
  if (!payload.sshPassword) payload.sshPassword = ''
  // 后端只认数字端口（`as_u64`），字符串会被静默丢掉退回默认端口
  for (const key of ['port', 'sshPort']) {
    if (payload[key] != null && payload[key] !== '') payload[key] = Number(payload[key]) || undefined
    else delete payload[key]
  }
  if (props.group) payload.group = props.group
  return payload
}

/** 重命名：后缀一直追加到不撞名为止（后缀可能被用户清空，所以兜底用默认值 + 序号） */
const renamedFor = (name, suffix, used) => {
  const sfx = suffix.trim() || t('ci.dupSuffixDefault')
  let next = name + sfx
  let guard = 0
  while (used.has(next) && guard < 20) {
    next += sfx
    guard++
  }
  if (used.has(next)) next = name + sfx + (Date.now() % 1000)
  return next
}

const doImport = async () => {
  const list = raw.value.filter((c) => c.type && checkedIds.value.includes(c._uid))
  if (!list.length) return
  importing.value = true
  const byName = existingByName.value
  // 已经被占用的名字：已存连接 + 本次已导入的（文件里自己重名也要分开）
  const used = new Set(byName.keys())
  let ok = 0
  let skipped = 0
  const failed = []
  for (const item of list) {
    const name = String(item.name || '').trim()
    const hit = byName.get(name)
    let targetName = ''
    let targetId = ''
    if (hit) {
      if (dupMode.value === 'skip') {
        skipped++
        continue
      }
      if (dupMode.value === 'overwrite') {
        targetId = hit.id
      } else {
        targetName = renamedFor(name, dupSuffix.value, used)
        used.add(targetName)
      }
    } else {
      used.add(name)
    }
    try {
      await saveConnection(payloadOf(item, targetName, targetId))
      ok++
    } catch (err) {
      failed.push(t('mv.failItem', { name, detail: (err && err.message) || t('common.unknownError') }))
    }
  }
  importing.value = false
  emit('imported', { ok, failed, skipped })
  loadExisting()
  if (failed.length) {
    ElMessage.warning(t(skipped ? 'ci.doneFailedSkipped' : 'ci.doneFailed', { ok, skipped, failed: failed.length }))
    console.warn('导入失败的连接：', failed)
    return
  }
  if (!ok && skipped) {
    // 全被跳过：留在弹窗里，方便换个处理方式再导一次
    ElMessage.info(t('ci.doneAllSkipped', { n: skipped }))
    return
  }
  if (skipped) ElMessage.success(t('ci.doneSkipped', { ok, skipped }))
  else ElMessage.success(t('ci.done', { n: ok }))
  emit('update:modelValue', false)
}
</script>

<style scoped>
.dlg-title { display: flex; align-items: center; gap: 8px; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}
.ic-src { margin-left: 4px; }
.ic-drop {
  display: flex; flex-direction: column; align-items: center; gap: 6px;
  padding: 52px 16px; border: 1px dashed var(--dc-bg-hover); border-radius: 12px;
  background: var(--dc-bg-soft); cursor: pointer; transition: all .18s ease;
  color: var(--dc-text-dim);
}
.ic-drop:hover { border-color: var(--dc-primary); background: var(--dc-bg-hover); }
.ic-drop-t { font-size: 14px; font-weight: 600; color: var(--dc-text); }
.ic-drop-s { font-size: 12px; color: var(--dc-text-dim); text-align: center; line-height: 1.6; }
.ic-srcs { display: flex; gap: 6px; margin-top: 6px; }
.ic-head { display: flex; align-items: center; gap: 10px; padding-bottom: 8px; border-bottom: 1px solid var(--dc-bg-hover); }
.ic-file { font-size: 13px; color: var(--dc-text); max-width: 240px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ic-count { font-size: 12px; color: var(--dc-text-dim); }
.ic-search { width: 200px; margin-left: auto; }
/* 同名连接处理方式：窄窗口下允许折行，别把弹窗撑横 */
.ic-dup {
  display: flex; align-items: center; flex-wrap: wrap; gap: 6px 10px;
  padding-top: 8px; font-size: 12px; color: var(--dc-text-dim);
}
.ic-dup-t { flex: none; color: var(--dc-text-mid); }
.ic-dup-mode { flex: none; }
.ic-dup-suffix { flex: none; }
/* 后缀框是按内容算宽的窄框，文字再左对齐就会贴死左边、右边空一块 —— 居中看着才对称 */
.ic-dup-suffix :deep(.el-input__inner) { text-align: center; }
.ic-dup-hit { min-width: 0; margin-left: auto; color: var(--dc-warning); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ic-dup-tag { flex: none; }
.ic-warn {
  display: flex; align-items: flex-start; gap: 6px; margin-top: 8px;
  font-size: 12px; line-height: 1.6; color: var(--dc-warning);
}
/* 图标盒做成「正好一行文字高」再内部居中：跟首行文字对齐（原来 flex-start 让图标顶在
   行盒顶端，看着比文字高半行）；提示是多行时图标也不会掉到中间 */
.ic-warn > .el-icon { flex: none; height: 1.6em; }
/* 高度自适应：body 摊成 flex 列，列表吃掉剩余高度 */
:global(.el-dialog.ic-dialog) {
  display: flex; flex-direction: column;
  max-height: calc(100vh - 16vh);
}
:global(.el-dialog.ic-dialog .el-dialog__body) {
  flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column;
}
.ic-head, .ic-dup, .ic-warn, .ic-drop { flex: none; }
.ic-list {
  flex: 1 1 auto; min-height: 96px;
  overflow-y: auto; overflow-x: hidden; margin-top: 6px;
}
.ic-list :deep(.el-checkbox) { display: flex; align-items: center; height: 32px; margin-right: 0; width: 100%; }
.ic-list :deep(.el-checkbox__label) { display: flex; align-items: center; gap: 8px; overflow: hidden; flex: 1; }
.ic-logo { flex: none; }
.ic-name { font-size: 13px; color: var(--dc-text); max-width: 300px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ic-type { font-size: 12px; color: var(--dc-text-dim); }
.ic-meta { font-size: 12px; color: var(--dc-text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ic-pwd { margin-left: auto; flex: none; }
/* 目标分组：放头部的计数后面（原来跟「口令需补填」那句一起挤在底部） */
.ic-group { min-width: 0; color: var(--dc-text-mid); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
