<template>
  <div class="cb">
    <div class="cb-toolbar">
      <el-select v-model="data.logic" size="small" style="width: 110px">
        <el-option value="AND" :label="$t('cb.all')" />
        <el-option value="OR" :label="$t('cb.any')" />
      </el-select>
      <el-button size="small" plain :icon="Plus" @click="addRow">{{ $t('cb.addCondition') }}</el-button>
      <span class="cb-count" v-if="data.items.length">{{ $t('cb.count', { n: data.items.length }) }}</span>
    </div>

    <el-empty v-if="!data.items.length" :description="$t('cb.empty')" :image-size="40" />
    <div v-else class="cb-rows">
      <div v-for="(it, i) in data.items" :key="i" class="cb-row">
        <el-select v-model="it.field" size="small" :placeholder="$t('dgen.colField')"
                   class="cb-field">
          <el-option v-for="c in columns" :key="c.name" :label="c.name" :value="c.name" />
        </el-select>
        <el-select v-model="it.op" size="small" class="cb-op">
          <el-option v-for="o in COND_OPS" :key="o.v" :label="$t(o.lKey)" :value="o.v" />
        </el-select>
        <template v-if="needValue(it.op)">
          <el-input v-model="it.value" size="small" maxlength="300" clearable
                    :placeholder="$t('cb.valuePlaceholder')" class="cb-val" />
        </template>
        <template v-if="it.op === 'between'">
          <el-input v-model="it.value2" size="small" maxlength="200" clearable
                    :placeholder="$t('cb.endValue')" class="cb-val2" />
        </template>
        <el-button type="danger" :icon="Delete" text size="small" @click="remove(i)" />
      </div>
    </div>

    <div v-if="sqlPreview" class="cb-preview">
      <span class="cb-preview-label">WHERE</span>
      <code>{{ sqlPreview }}</code>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { Plus, Delete } from '@element-plus/icons-vue'
import { COND_OPS, buildConditionSql, needValue } from '../utils/cond'
import { t } from '../utils/i18n'

const props = defineProps({
  columns: { type: Array, default: () => [] },
  modelValue: { type: Object, default: () => ({ logic: 'AND', items: [] }) }
})
const emit = defineEmits(['update:modelValue'])

const clone = o => JSON.parse(JSON.stringify(o || { logic: 'AND', items: [] }))

const commit = () => emit('update:modelValue', clone(data.value))
const data = ref(clone(props.modelValue))

// 外部（如父组件重置/回传）更新时才覆盖本地状态；值相同则跳过，避免双向回环
watch(() => props.modelValue, v => {
  const cur = JSON.stringify(data.value)
  const next = JSON.stringify(v || { logic: 'AND', items: [] })
  if (cur !== next) data.value = JSON.parse(next)
}, { deep: true })

// 任何内部变更（字段/操作符/值/逻辑/增删行）都同步给父组件，切换面板后不再回滚
watch(data, commit, { deep: true })

const sqlPreview = computed(() => buildConditionSql(data.value, props.columns))

const addRow = () => {
  data.value.items.push({ field: '', op: '=', value: '', value2: '' })
}
const remove = i => {
  data.value.items.splice(i, 1)
}
</script>

<style scoped>
.cb {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.cb-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
}
.cb-count { font-size: 11.5px; color: var(--dc-text-dim); }
.cb-rows {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.cb-row {
  display: flex;
  align-items: center;
  gap: 6px;
}
.cb-field { width: 160px; flex: none; }
.cb-op { width: 108px; flex: none; }
.cb-val { flex: 1; }
.cb-val2 { flex: 1; }
.cb-preview {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  background: rgba(79, 140, 255, .06);
  border: 1px dashed rgba(79, 140, 255, .3);
  border-radius: 8px;
  padding: 6px 10px;
  font-size: 13px;
}
.cb-preview-code, .cb-preview code {
  color: var(--dc-primary, var(--dc-primary));
  font-family: "Cascadia Code", Consolas, "JetBrains Mono", monospace;
  word-break: break-all;
  line-height: 1.6;
}
.cb-preview-label {
  flex: none;
  font-weight: 700;
  color: var(--dc-text-dim);
  font-size: 12px;
  padding-top: 1px;
}
</style>
