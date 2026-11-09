<template>
  <el-dialog v-model="visible" width="min(800px, 94vw)" append-to-body align-center
             :close-on-click-modal="false" class="tplvar-dialog">
    <template #header>
      <div class="tv-title">
        <span class="tv-title-badge"><el-icon><Key /></el-icon></span>
        <span class="tv-title-tx">{{ $t('sqlq.tplVarTitle') }}</span>
        <el-tooltip placement="bottom-start" effect="dark" popper-class="tv-tip">
          <template #content>{{ $t('sqlq.tplVarLead') }}</template>
          <el-icon class="tv-title-q"><QuestionFilled /></el-icon>
        </el-tooltip>
      </div>
    </template>
    <div class="tv-card">
      <div class="tv-head" v-if="rows.length">
        <span class="tv-col tv-c-name">{{ $t('sqlq.tplVarName') }}</span>
        <span class="tv-col tv-c-value">{{ $t('sqlq.tplVarValue') }}</span>
        <span class="tv-col tv-c-type">{{ $t('sqlq.tplVarType') }}</span>
        <span class="tv-col tv-c-note">{{ $t('sqlq.tplVarNote') }}</span>
        <span class="tv-col tv-c-op"></span>
      </div>
      <div class="tv-rows">
        <div class="tv-row" v-for="(r, i) in rows" :key="i">
          <span class="tv-col tv-c-name">
            <el-input v-model="r.name" :placeholder="$t('sqlq.tplVarNamePh')" clearable>
              <template #prepend>:</template>
            </el-input>
          </span>
          <span class="tv-col tv-c-value">
            <el-input v-model="r.value" :placeholder="$t('sqlq.tplVarValuePh')" clearable />
          </span>
          <span class="tv-col tv-c-type">
            <!-- auto：按 :var 前面的列元数据推断；显式声明优先于推断 -->
            <el-select v-model="r.type" size="small" class="tv-type">
              <el-option value="auto" :label="$t('sqlq.tplVarTypeAuto')" />
              <el-option value="string" :label="$t('sqlq.tplVarTypeString')" />
              <el-option value="number" :label="$t('sqlq.tplVarTypeNumber')" />
              <el-option value="bool" :label="$t('sqlq.tplVarTypeBool')" />
            </el-select>
          </span>
          <span class="tv-col tv-c-note">
            <el-input v-model="r.note" :placeholder="$t('sqlq.tplVarNotePh')" clearable />
          </span>
          <span class="tv-col tv-c-op">
            <el-button size="small" text type="danger" :icon="Delete" @click="rows.splice(i, 1)" />
          </span>
        </div>
        <div class="tv-empty" v-if="!rows.length">{{ $t('sqlq.tplVarEmpty') }}</div>
      </div>
    </div>
    <template #footer>
      <el-button :icon="Plus" @click="addRow">{{ $t('sqlq.tplVarAdd') }}</el-button>
      <el-button @click="visible = false">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" :disabled="!rows.length" :loading="saving" @click="save">{{ $t('common.save') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { t } from '../../utils/i18n'
import { ElMessage } from 'element-plus'
import { Delete, Plus, Key, QuestionFilled } from '@element-plus/icons-vue'
import { readTplVars, saveTplVars, ensureSqlVarStore, TPLVAR_NAME_RE } from '../../utils/sqlTplVars'

const props = defineProps({ modelValue: { type: Boolean, default: false } })
const emit = defineEmits(['update:modelValue', 'saved'])
const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const rows = ref([])
const saving = ref(false)
watch(visible, async (v) => {
  if (!v) return
  // 变量库在 dbmind.db 里（不是 localStorage）：打开时先确保拉过一次，再拷一份现库，
  // 编辑期间不动真身（取消不产生副作用）
  await ensureSqlVarStore()
  rows.value = readTplVars().map((r) => ({ name: r.name || '', value: r.value || '', note: r.note || '', type: r.type || 'auto' }))
})

const addRow = () => { rows.value.push({ name: '', value: '', note: '', type: 'auto' }) }

const save = async () => {
  const seen = new Set()
  const out = []
  for (const r of rows.value) {
    const name = String(r.name || '').trim()
    if (!name) continue // 整行没填名字 = 顺手加多了的空行，直接丢弃
    if (!TPLVAR_NAME_RE.test(name)) { ElMessage.warning(t('sqlq.tplVarNameBad', { name })); return }
    const key = name.toLowerCase()
    if (seen.has(key)) { ElMessage.warning(t('sqlq.tplVarDup', { name })); return }
    seen.add(key)
    out.push({ name, value: String(r.value || '').trim(), note: String(r.note || '').trim(), type: r.type || 'auto' })
  }
  if (saving.value) return
  saving.value = true
  try {
    await saveTplVars(out)
  } catch (e) {
    // 写库失败要明说（后端没起来 / 磁盘只读），不能像以前那样吞掉——
    // 用户以为存上了、下次打开又是空的
    const detail = e?.response?.data?.message || e?.message || ''
    ElMessage.error(t('sqlq.tplVarSaveFailed', { detail }))
    return
  } finally {
    saving.value = false
  }
  ElMessage.success(t('sqlq.tplVarSaved'))
  emit('saved')
  visible.value = false
}
</script>

<style scoped>
/* 标题：徽标（与工具条「变量」按钮同款 Key 图标）+ 标题 + 问号（说明全收进气泡） */
.tv-title { display: flex; align-items: center; gap: 8px; padding-right: 24px; }
.tv-title-badge {
  flex: 0 0 auto; display: inline-flex; align-items: center; justify-content: center;
  width: 22px; height: 22px; border-radius: 6px;
  background: var(--dc-primary, #4c6ef5); color: #fff; font-size: 13px; line-height: 1;
}
.tv-title-tx { font-size: 15px; font-weight: 600; color: var(--dc-text); white-space: nowrap; }
.tv-title-q { flex: 0 0 auto; font-size: 14px; color: var(--dc-text-dim); cursor: help; }
.tv-title-q:hover { color: var(--dc-primary, #4c6ef5); }
/* 卡片：圆角容器包住整张表，行间用分隔线而不是空隙 */
.tv-card {
  border: 1px solid var(--dc-border, rgba(255,255,255,.08)); border-radius: 10px;
  background: var(--dc-bg-soft, rgba(255,255,255,.02)); padding: 10px 12px 6px;
}
.tv-head, .tv-row { display: flex; align-items: center; gap: 8px; }
.tv-head { font-size: 12px; color: var(--dc-text-dim); padding: 0 2px 8px; border-bottom: 1px dashed var(--dc-border, rgba(255,255,255,.08)); margin-bottom: 2px; }
/* 高度自适应：vh 基准，窗口变矮列表跟着缩、内部滚动；滚动条不显示（滚动仍可用） */
.tv-rows { display: flex; flex-direction: column; max-height: min(46vh, 430px); overflow-y: auto; overflow-x: hidden; scrollbar-width: none; }
.tv-rows::-webkit-scrollbar { display: none; }
.tv-row { padding: 6px 0; border-bottom: 1px dashed var(--dc-border, rgba(255,255,255,.06)); }
.tv-row:last-child { border-bottom: none; }
.tv-c-name { width: 160px; flex: 0 0 auto; }
.tv-c-value { width: 150px; flex: 0 0 auto; }
.tv-c-type { width: 128px; flex: 0 0 auto; }
.tv-type { width: 100%; }
.tv-c-note { flex: 1; min-width: 0; }
.tv-c-op { width: 32px; flex: 0 0 auto; text-align: right; }
.tv-empty { color: var(--dc-text-dim); font-size: 12px; padding: 20px 0; text-align: center; line-height: 1.7; }
</style>

<style>
/* 问号气泡挂在 body 下（teleport），用全局样式给个合适的宽度 */
.tv-tip { max-width: 360px; line-height: 1.7; }
</style>
