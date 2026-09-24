<template>
  <el-dialog v-model="visible" width="520px" :close-on-click-modal="false" append-to-body class="main-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Search /></el-icon></span>
        <span>{{ $t('ds.title') }}</span>
      </div>
    </template>
    <el-form size="small" @submit.prevent="doSearch">
      <el-form-item label="">
        <el-input v-model="keyword" clearable :prefix-icon="Search">
          <template #append>
            <el-button :icon="Search" @click="doSearch" />
          </template>
        </el-input>
      </el-form-item>
    </el-form>
    <div v-loading="loading" style="max-height:300px;overflow:auto;margin-top:8px;">
      <div v-if="!results.length && !loading" style="text-align:center;color:var(--dc-text-dim);padding:20px;">{{ $t('ds.empty') }}</div>
      <div v-for="r in results" :key="r.id" class="db-search-item" @click="openResult(r)">
        <div style="font-weight:600;font-size: 14px;">{{ r.label }}</div>
        <div style="font-size: 12px;color:var(--dc-text-dim);margin-top:2px;">{{ r.type }} · {{ r.db }}</div>
      </div>
    </div>
  </el-dialog>
</template>

<script setup>
// 在数据库中查找弹窗（从 MainView 拆出）：按关键字搜索库内表名 / 列名，点击结果跳转到对应表。
import { ref, computed, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { Search } from '@element-plus/icons-vue'
import { searchObjects } from '../../api'
import { t } from '../../utils/i18n'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  conn: { type: Object, default: null },
  database: { type: String, default: '' }
})
const emit = defineEmits(['update:modelValue', 'opentable'])

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const keyword = ref('')
const loading = ref(false)
const results = ref([])

// 打开弹窗时清空上次关键字与结果
watch(() => props.modelValue, (open) => {
  if (!open) return
  keyword.value = ''
  results.value = []
})

const doSearch = async () => {
  const db = props.database
  const kw = keyword.value.trim()
  if (!db || !kw) return
  loading.value = true
  results.value = []
  try {
    // 服务端一次批量搜索（表名 + 列名）：原来「取表列表 → 逐张表请求 columns」是 N 次 HTTP 往返
    // （几百张表的库就是几百次，每次都重新建连切库），现在固定 1 次请求
    const res = await searchObjects(props.conn?.id, db, kw)
    const list = []
    for (const tb of res?.tables || []) {
              list.push({ id: 'table:' + tb, label: tb, type: t('mv.catTable'), db, kind: 'table', table: tb })
            }
    for (const c of res?.columns || []) {
      list.push({ id: 'col:' + c.table + ':' + c.column, label: `${c.table}.${c.column}`, type: t('ds.col'), db, kind: 'table', table: c.table })
    }
    results.value = list
  } catch (e) {
    ElMessage.error(t('ds.searchFailed', { detail: (e?.message || e) }))
  }
  loading.value = false
}

const openResult = (r) => {
  visible.value = false
  if (r.kind === 'table') {
    emit('opentable', { table: r.table, db: r.db })
  }
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
.db-search-item {
  padding: 8px 10px; border-radius: 6px; cursor: pointer; transition: background .15s;
}
.db-search-item:hover { background: rgba(79, 140, 255, .12); }
</style>
