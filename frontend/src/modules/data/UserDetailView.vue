<template>
  <div class="form-tab">
    <!-- 顶部标题栏 -->
    <div class="form-tab-header">
      <div class="header-title">
        <el-icon :size="16" class="header-icon"><User /></el-icon>
        <!-- 路径：数据源名称在前，中间用 `.` 连接（与编辑表单同一口径） -->
        <span class="header-text">{{ headPath }}</span>
      </div>
      <div class="header-actions">
        <!-- 用户详情**只读**：编辑入口已按需求下掉（编辑链路保留，随时可恢复） -->
      </div>
    </div>

    <!-- 主体：上详情 + 下 SQL 预览（与「编辑表结构」同一布局） -->
    <div class="form-tab-body">
      <!-- 左侧详情 -->
      <div class="form-area">
        <div v-if="loading" class="empty-state">
          <el-icon class="is-loading"><Loading /></el-icon> {{ $t('common.loading') }}
        </div>
        <div v-else-if="error" class="empty-state error">
          <el-icon><CircleClose /></el-icon> {{ error }}
        </div>
        <template v-else>
          <!-- 基本信息 -->
          <div class="form-card" v-if="detailItems.length">
            <div class="card-title">
              <div class="card-title-icon"><el-icon><InfoFilled /></el-icon></div>
              <span>{{ $t('udv.basicInfo') }}</span>
            </div>
            <div class="info-grid">
              <div v-for="item in detailItems" :key="item.key" class="info-row">
                <span class="info-label">{{ item.label }}</span>
                <span class="info-value" :class="item.cls">{{ item.value }}</span>
              </div>
            </div>
          </div>
        </template>
      </div>

      <!-- 分隔条：上下两块卡片之间那道缝，向上拖 = 预览变高，双击最大/最小 -->
      <div v-show="!sqlCollapsed" class="sql-resizer"
           :title="$t('tdet.resizerTitle')"
           @mousedown.prevent="onSqlResizeStart" @dblclick="onSqlResizeDblClick"></div>

      <!-- SQL 预览：在详情**下方**（原来是右侧 380px 固定栏），点标题行展开/收起 -->
      <div ref="sqlAreaRef" class="sql-area" :class="{ collapsed: sqlCollapsed }"
           :style="sqlCollapsed ? null : { height: sqlHeight + 'px' }">
        <div class="sql-head" @click="sqlCollapsed = !sqlCollapsed"
             :title="sqlCollapsed ? $t('tdet.expandSql') : $t('tdet.collapseSql')">
          <el-icon class="sql-fold" :class="{ folded: sqlCollapsed }"><CaretRight /></el-icon>
          <el-icon><DocumentCopy /></el-icon>
          <span>{{ $t('oft.preview') }}</span>
          <span class="sql-fold-tx">{{ sqlCollapsed ? $t('tdet.expand') : $t('tdet.collapse') }}</span>
        </div>
        <div v-show="!sqlCollapsed" class="sql-body">
          <pre><code>{{ fullSql || $t('udv.noSql') }}</code></pre>
        </div>
        <div class="sql-foot">
          <span class="sql-tip">{{ $t('udv.grantTip') }}</span>
          <div class="sql-foot-actions">
            <el-button size="small" text :icon="CopyDocument" @click="copyGrants">{{ $t('oft.copySql') }}</el-button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onBeforeUnmount } from 'vue'
import { ElMessage } from 'element-plus'
import { Loading, CircleClose, User, CopyDocument, EditPen, InfoFilled, DocumentCopy, CaretRight } from '@element-plus/icons-vue'
import { getUserInfo } from '../../api'
import { t } from '../../utils/i18n'
import { userDetailItems } from '../../utils/userFields'

const props = defineProps({
  conn: { type: Object, required: true },
  database: { type: String, default: '' },
  name: { type: String, required: true }
})

const emit = defineEmits(['edit'])

// 标题栏的路径：数据源名称在前 + `.` 连接
const headPath = computed(() => [props.conn?.name, info.value.name || props.name].filter(Boolean).join('.'))

const info = ref({})
const loading = ref(false)
const error = ref('')

// 标签表与「是 / 否」归一都抽到 utils/userFields.js 了 ——
// 编辑表单的「基本信息」用的是同一份（同一个字段在两处必须同名同义）。
const detailItems = computed(() => userDetailItems(info.value))

const grants = computed(() => {
  return info.value.grants || info.value.roles || info.value.permissions || []
})

// 完整 SQL = CREATE USER 语句 + GRANT 授权语句
const fullSql = computed(() => {
  const parts = []
  const cu = info.value.createUserSql || info.value.create_user_sql || ''
  if (cu && cu.trim()) parts.push(cu.trim())
  const g = grants.value.map(s => s.trim()).filter(s => s && !s.startsWith('--'))
  if (g.length) {
    if (parts.length) parts.push('')
    parts.push(...g)
  }
  return parts.join('\n')
})

// 请求代次：同一面板里快速切换用户时，先发的响应可能后到 —— 不校验就会把 A 用户的
// 属主/权限显示成 B 用户的（张冠李戴）。每次 load 自增，回来的不是最新代次就整段丢弃。
let loadSeq = 0
const load = async () => {
  // 缺连接时**不能静默返回**：那会渲染成一个「有标题、没内容」的空壳页，
  // 与「这个用户确实没有属性」无法区分 —— 这次就是这么误导人的
  // （根因在父组件的 openUserTab 没带 connId，已一并修掉）。
  if (!props.conn || !props.name) {
    error.value = t('udv.connUnknown')
    return
  }
  const seq = ++loadSeq
  loading.value = true
  error.value = ''
  try {
    const res = await getUserInfo(props.conn.id, props.database, props.name)
    if (seq !== loadSeq) return
    if (res.error) {
      error.value = res.error
    } else {
      info.value = res
    }
  } catch (e) {
    if (seq !== loadSeq) return
    error.value = t('odv.loadFailedDetail', { detail: (e.message || e) })
  } finally {
    if (seq === loadSeq) loading.value = false
  }
}

const copyGrants = async () => {
  const text = fullSql.value
  try {
    await navigator.clipboard.writeText(text || '')
    ElMessage.success(t('odv.copied'))
  } catch {
    ElMessage.warning(t('mv.copyManually'))
  }
}

const onEditClick = () => {
  // 带上本页自己的连接与库：父组件若用「当前选中的连接」会张冠李戴
  emit('edit', { name: props.name, kind: 'user', connId: props.conn?.id, database: props.database })
}

watch(() => [props.conn?.id, props.database, props.name], load, { immediate: true })

// ===== 底部 SQL 预览的折叠 / 拖拽（与「编辑表结构」同一套手感）=====
const SQL_H_DEFAULT = 260
const SQL_H_MIN = 0    // 最低可压到没有（只留分隔条，往上拖即可还原）
const FORM_MIN = 0     // 上方详情卡不预留高度：往上拖可以把预览拉到顶
const sqlAreaRef = ref(null)
const sqlCollapsed = ref(false)
const sqlHeight = ref(SQL_H_DEFAULT)
/** 高度上限量**所在容器的实际可用高度**，并给上方详情卡留出最小空间 */
const sqlMaxH = () => {
  const parent = sqlAreaRef.value?.parentElement
  const available = (parent ? parent.clientHeight : Math.round(window.innerHeight * 0.7)) - FORM_MIN
  return Math.max(SQL_H_MIN, available)
}
let sqlDrag = null
const onSqlResizeStart = (e) => {
  sqlDrag = { startY: e.clientY, startH: sqlHeight.value }
  document.body.style.cursor = 'row-resize'
  document.body.style.userSelect = 'none'
  window.addEventListener('mousemove', onSqlResizeMove)
  window.addEventListener('mouseup', onSqlResizeEnd)
}
const onSqlResizeMove = (e) => {
  if (!sqlDrag) return
  // 向上拖 = 变高：起始高度 + 鼠标上移的距离
  const next = sqlDrag.startH + (sqlDrag.startY - e.clientY)
  sqlHeight.value = Math.min(sqlMaxH(), Math.max(SQL_H_MIN, Math.round(next)))
}
const onSqlResizeEnd = () => {
  sqlDrag = null
  document.body.style.cursor = ''
  document.body.style.userSelect = ''
  window.removeEventListener('mousemove', onSqlResizeMove)
  window.removeEventListener('mouseup', onSqlResizeEnd)
}
/** 双击：最大 ↔ 最小 */
const onSqlResizeDblClick = () => {
  const max = sqlMaxH()
  sqlHeight.value = sqlHeight.value >= max - 8 ? SQL_H_MIN : max
}
onBeforeUnmount(onSqlResizeEnd)
</script>

<style scoped>
/* ===== 与 ObjectFormTab / UserForm 一致的顶层布局 ===== */
.form-tab {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--dc-bg-deep);
  /* 四周留 5px：卡片贴着面板边缘时，8px 圆角会被边线"压平"，看起来像没做圆角 */
  padding: 5px;
  box-sizing: border-box;
  gap: 5px;
}
/* 顶部标题栏：独立成一张小卡（与「编辑表结构」/编辑用户同一形态） */
.form-tab-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  background: var(--dc-bg-soft);
  box-shadow: 0 1px 3px rgba(16, 24, 40, .06), 0 1px 2px rgba(16, 24, 40, .04);
  flex-shrink: 0;
}
.header-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 600;
  color: var(--dc-text);
}
.header-icon { color: var(--dc-primary); }
.header-actions { display: flex; gap: 6px; }

.form-tab-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  min-height: 0;
}

/* 详情在上：自身**不滚动** —— 滚动交给内部的「基本信息」卡片，
   滚动条才会出现在卡片右缘内侧，而不是整块编辑区的最右边（用户反馈） */
.form-area {
  flex: 1;
  overflow: hidden;
  padding: 14px;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.empty-state {
  padding: 40px;
  text-align: center;
  color: var(--dc-text-dim);
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  font-size: 14px;
}
.empty-state.error { color: var(--dc-danger); }

/* ===== SQL 预览：在详情下方（原来右侧 380px 固定栏）=====
   宽度通栏、上边框替代左边框；高度由拖动/双击控制（内联 style），
   收起时高度交给内容（标题行 + 底部按钮行）。 */
.sql-area {
  /* 不设 width:100% —— flex 纵向布局里子项默认拉伸（stretch），
     自动等于「父宽 − 左右 margin」；写了 100% 反而会在 margin 之外再撑满整行，
     于是右侧溢出边距（实测贴边）。 */
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  background: var(--dc-bg-card);
  box-shadow: 0 1px 3px rgba(16, 24, 40, .06), 0 1px 2px rgba(16, 24, 40, .04);
  overflow: hidden;
  display: flex;
  flex-direction: column;
  flex: 0 0 auto;   /* 高度只认内联那一个（旧布局的 flex: 1 会在拖矮时把它拉大） */
  min-height: 0;
  margin: 0 0 8px;
}
.sql-area.collapsed { height: auto !important; }
/* 分隔条：本身就是上下两块卡片之间那道缝（6px + 透明底），悬停变主色 */
.sql-resizer { height: 6px; flex: 0 0 6px; cursor: row-resize; position: relative; z-index: 2; background: transparent; }
.sql-resizer::before {
  content: ''; position: absolute; left: 50%; top: 50%;
  transform: translate(-50%, -50%);
  width: 48px; height: 4px; border-radius: 3px;
  background: var(--dc-text-dim); opacity: .45;
  transition: background .15s ease, opacity .15s ease;
}
.sql-resizer:hover::before { background: var(--dc-primary); opacity: 1; }
.sql-foot-actions { display: flex; align-items: center; gap: 2px; flex-shrink: 0; }
.sql-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 10px 12px;
  font-size: 13px;
  color: var(--dc-text-dim);
  background: var(--dc-bg-soft);
  border-bottom: 1px solid var(--dc-border-soft);
  flex-shrink: 0;
  cursor: pointer;
  user-select: none;
}
.sql-head:hover { background: var(--dc-bg-hover); color: var(--dc-text); }
/* 折叠箭头：展开时朝下、收起时朝右 */
.sql-fold { transition: transform .18s ease; transform: rotate(90deg); }
.sql-fold.folded { transform: rotate(0deg); }
.sql-fold-tx { font-size: 12px; color: var(--dc-text-weak); }
/* 收起时把底部提示一并收掉，只留按钮 */
.sql-area.collapsed .sql-tip { display: none; }
.sql-body {
  flex: 1;
  overflow: auto;
  padding: 10px 12px;
  min-height: 0;
}
.sql-body pre { margin: 0; }
.sql-body code {
  font-family: "SF Mono", Consolas, monospace;
  font-size: 13px;
  line-height: 1.7;
  color: var(--dc-code-text);
  white-space: pre-wrap;
  word-break: break-all;
}
.sql-foot {
  padding: 10px 12px;
  border-top: 1px solid var(--dc-border-soft);
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
/* 底栏左侧提示：占满剩余宽度，把按钮推到右边 */
.sql-tip { flex: 1; font-size: 12px; color: var(--dc-text-dim); }

/* ===== 卡片（与 UserForm 统一风格） ===== */
.form-card {
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border-soft);
  border-radius: 10px;
  padding: 12px 14px;
  /* 占满编辑区并**在卡片内滚动**：滚动条贴着卡片右缘内侧，
     而不是飘在整块编辑区的最右边 */
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}
.card-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 14px;
  font-weight: 600;
  color: var(--dc-text);
  margin-bottom: 10px;
}
.card-title-icon {
  width: 26px; height: 26px; border-radius: 6px;
  background: rgba(79, 140, 255, .12);
  border: 1px solid rgba(79, 140, 255, .2);
  display: flex; align-items: center; justify-content: center;
  color: var(--dc-link);
  font-size: 14px;
}
/* 信息网格 */
.info-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 8px 16px;
}
.info-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 6px 0;
}
.info-label {
  font-size: 13px;
  color: var(--dc-text-dim);
  min-width: 80px;
  flex-shrink: 0;
  text-transform: uppercase;
  letter-spacing: .3px;
}
.info-value {
  font-size: 14px;
  color: var(--dc-text);
  font-family: "SF Mono", Consolas, monospace;
  word-break: break-all;
}
.info-value.allow { color: var(--dc-accent); }
.info-value.deny { color: var(--dc-text-dim); }
.info-value.yes { color: var(--dc-accent); }
.info-value.no { color: var(--dc-text-dim); }
</style>
