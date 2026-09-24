<template>
  <el-drawer v-model="visible" :title="title || $t('grd.title')" size="400px" direction="rtl"
             :append-to-body="true" class="grid-row-detail">
    <el-scrollbar class="rd-scroll">
      <div class="rd-table">
        <div v-for="c in colList" :key="c.name" class="rd-row">
          <div class="rd-key" :title="c.name">
            <span class="rd-name">{{ c.name }}</span>
            <span v-if="c.type" class="rd-type">{{ c.type }}</span>
          </div>
          <div class="rd-val" :class="{ 'is-null': isNull(row[c.name]) }" :title="isNull(row[c.name]) ? '' : String(row[c.name])">
            {{ isNull(row[c.name]) ? 'NULL' : String(row[c.name]) }}
          </div>
        </div>
      </div>
    </el-scrollbar>
  </el-drawer>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  visible: Boolean,
  // 行数据对象
  row: { type: Object, default: () => ({}) },
  // 列元信息：[{ name, type }]
  columns: { type: Array, default: () => [] },
  tableName: { type: String, default: '' },
  connType: { type: String, default: '' },
  title: { type: String, default: '' }
})
const emit = defineEmits(['update:visible'])
const visible = computed({
  get: () => props.visible,
  set: (v) => emit('update:visible', v)
})

const colList = computed(() => props.columns)
const isNull = (v) => v === null || v === undefined
</script>

<style scoped>
.rd-scroll { height: 100%; }
.rd-table { display: flex; flex-direction: column; }
.rd-row { display: flex; border-bottom: 1px solid var(--dc-border, #ebeef5); }
.rd-key {
  width: 42%; flex-shrink: 0; padding: 10px 12px; background: var(--dc-bg-soft, #f7f8fa);
  border-right: 1px solid var(--dc-border, #ebeef5);
  display: flex; flex-direction: column; gap: 2px;
}
.rd-name { font-weight: 600; color: var(--dc-text-strong, #303133); word-break: break-all; font-size: 14px; }
.rd-type { font-size: 12px; color: var(--dc-text-dim, #999); }
.rd-val {
  flex: 1; padding: 10px 12px; color: var(--dc-text, #454a50); word-break: break-all;
  white-space: pre-wrap; font-size: 14px; line-height: 1.6;
}
.rd-val.is-null { color: var(--dc-text-dim, #aaa); font-style: italic; }
</style>
