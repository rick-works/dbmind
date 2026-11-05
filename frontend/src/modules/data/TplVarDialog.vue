<template>
  <el-dialog v-model="visible" width="min(680px, 94vw)" append-to-body align-center
             :close-on-click-modal="false" class="tplvar-dialog">
    <template #header>
      <div class="tv-title">
        <span class="tv-title-badge">{{ '\u003A' }}</span>
        <span class="tv-title-tx">{{ $t('sqlq.tplVarTitle') }}</span>
        <span class="tv-title-sub">{{ $t('sqlq.tplVarLead') }}</span>
      </div>
    </template>
    <div class="tv-card">
      <div class="tv-head" v-if="rows.length">
        <span class="tv-col tv-c-name">{{ $t('sqlq.tplVarName') }}</span>
        <span class="tv-col tv-c-value">{{ $t('sqlq.tplVarValue') }}</span>
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
    <div class="tv-hint">{{ $t('sqlq.tplVarHint') }}</div>
    <template #footer>
      <el-button :icon="Plus" @click="addRow">{{ $t('sqlq.tplVarAdd') }}</el-button>
      <el-button @click="visible = false">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" :disabled="!rows.length" @click="save">{{ $t('common.save') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { t } from '../../utils/i18n'
import { ElMessage } from 'element-plus'
import { Delete, Plus } from '@element-plus/icons-vue'
import { readTplVars, writeTplVars, TPLVAR_NAME_RE } from '../../utils/sqlTplVars'

const props = defineProps({ modelValue: { type: Boolean, default: false } })
const emit = defineEmits(['update:modelValue', 'saved'])
const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const rows = ref([])
watch(visible, (v) => {
  if (v) {
    // 打开时拷一份现库，编辑期间不动真身（取消不产生副作用）
    rows.value = readTplVars().map((r) => ({ name: r.name || '', value: r.value || '', note: r.note || '' }))
  }
})

const addRow = () => { rows.value.push({ name: '', value: '', note: '' }) }

const save = () => {
  const seen = new Set()
  const out = []
  for (const r of rows.value) {
    const name = String(r.name || '').trim()
    if (!name) continue // 整行没填名字 = 顺手加多了的空行，直接丢弃
    if (!TPLVAR_NAME_RE.test(name)) { ElMessage.warning(t('sqlq.tplVarNameBad', { name })); return }
    const key = name.toLowerCase()
    if (seen.has(key)) { ElMessage.warning(t('sqlq.tplVarDup', { name })); return }
    seen.add(key)
    out.push({ name, value: String(r.value || '').trim(), note: String(r.note || '').trim() })
  }
  writeTplVars(out)
  ElMessage.success(t('sqlq.tplVarSaved'))
  emit('saved')
  visible.value = false
}
</script>

<style scoped>
/* 标题：徽标 : + 主标题 + 一句话说明（说明弱化，替代原来占一整行的 lead 条） */
.tv-title { display: flex; align-items: baseline; gap: 8px; min-width: 0; padding-right: 24px; }
.tv-title-badge {
  flex: 0 0 auto; display: inline-flex; align-items: center; justify-content: center;
  width: 22px; height: 22px; border-radius: 6px; align-self: center;
  background: var(--dc-primary, #4c6ef5); color: #fff; font-weight: 600; font-size: 14px; line-height: 1;
}
.tv-title-tx { font-size: 15px; font-weight: 600; color: var(--dc-text); white-space: nowrap; }
.tv-title-sub { flex: 1; min-width: 0; font-size: 12px; color: var(--dc-text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
/* 卡片：圆角容器包住整张表，行间用分隔线而不是空隙 */
.tv-card {
  border: 1px solid var(--dc-border, rgba(255,255,255,.08)); border-radius: 10px;
  background: var(--dc-bg-soft, rgba(255,255,255,.02)); padding: 10px 12px 6px;
}
.tv-head, .tv-row { display: flex; align-items: center; gap: 8px; }
.tv-head { font-size: 12px; color: var(--dc-text-dim); padding: 0 2px 8px; border-bottom: 1px dashed var(--dc-border, rgba(255,255,255,.08)); margin-bottom: 2px; }
/* 高度自适应：vh 基准，窗口变矮列表跟着缩、内部滚动 */
.tv-rows { display: flex; flex-direction: column; max-height: min(46vh, 430px); overflow-y: auto; }
.tv-row { padding: 6px 0; border-bottom: 1px dashed var(--dc-border, rgba(255,255,255,.06)); }
.tv-row:last-child { border-bottom: none; }
.tv-c-name { width: 190px; flex: 0 0 auto; }
.tv-c-value { width: 180px; flex: 0 0 auto; }
.tv-c-note { flex: 1; min-width: 0; }
.tv-c-op { width: 32px; flex: 0 0 auto; text-align: right; }
.tv-empty { color: var(--dc-text-dim); font-size: 12px; padding: 20px 0; text-align: center; line-height: 1.7; }
.tv-hint { margin-top: 8px; font-size: 12px; color: var(--dc-text-dim); line-height: 1.6; }
</style>
