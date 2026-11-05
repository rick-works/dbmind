<template>
  <el-dialog v-model="visible" :title="$t('sqlq.tplVarTitle')" width="640" append-to-body
             :close-on-click-modal="false" class="tplvar-dialog">
    <div class="tv-lead">
      <el-icon class="tv-ico"><InfoFilled /></el-icon>
      <span>{{ $t('sqlq.tplVarLead') }}</span>
    </div>
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
import { Delete, Plus, InfoFilled } from '@element-plus/icons-vue'
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
    const name = String(r.name || '').trim().toLowerCase()
    if (!name) continue // 整行没填名字 = 顺手加多了的空行，直接丢弃
    if (!TPLVAR_NAME_RE.test(name)) { ElMessage.warning(t('sqlq.tplVarNameBad', { name: r.name || '' })); return }
    if (seen.has(name)) { ElMessage.warning(t('sqlq.tplVarDup', { name })); return }
    seen.add(name)
    out.push({ name, value: String(r.value || '').trim(), note: String(r.note || '').trim() })
  }
  writeTplVars(out)
  ElMessage.success(t('sqlq.tplVarSaved'))
  emit('saved')
  visible.value = false
}
</script>

<style scoped>
.tv-lead { display: flex; align-items: flex-start; gap: 8px; color: var(--dc-text-dim); font-size: 12px; line-height: 1.6; margin-bottom: 10px; }
.tv-ico { flex: 0 0 auto; margin-top: 3px; }
.tv-head, .tv-row { display: flex; align-items: center; gap: 8px; }
.tv-head { font-size: 12px; color: var(--dc-text-dim); padding: 0 2px 6px; }
.tv-rows { display: flex; flex-direction: column; gap: 8px; max-height: 46vh; overflow-y: auto; padding-right: 2px; }
.tv-c-name { width: 190px; flex: 0 0 auto; }
.tv-c-value { width: 170px; flex: 0 0 auto; }
.tv-c-note { flex: 1; min-width: 0; }
.tv-c-op { width: 32px; flex: 0 0 auto; text-align: right; }
.tv-empty { color: var(--dc-text-dim); font-size: 12px; padding: 18px 0; text-align: center; line-height: 1.7; }
</style>
