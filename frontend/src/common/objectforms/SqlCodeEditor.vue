<template>
  <div class="sql-code-editor">
    <div class="sql-code-bar">
      <span class="sql-code-bar-tip">{{ $t('sce.barTip') }}</span>
      <el-tooltip :content="$t('sce.formatTip')" placement="top" :show-after="200">
        <el-button
          class="sql-code-fmt"
          size="small"
          text
          :icon="Brush"
          :loading="formatting"
          :disabled="!value.trim()"
          @click.stop="doFormat"
        >{{ $t('sce.format') }}</el-button>
      </el-tooltip>
    </div>
    <div class="sql-code-main">
      <VueMonacoEditor
        v-if="monacoReady"
        ref="editorRef"
        v-model:value="value"
        :theme="editorTheme"
        language="sql"
        :options="editorOptions"
        class="sql-code-monaco"
        @mount="onEditorMount"
      />
      <div v-else class="sql-code-loading">{{ $t('sce.loading') }}</div>
      <div v-if="!value && placeholder" class="sql-code-placeholder">{{ placeholder }}</div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { t } from '../../utils/i18n'
import { ElMessage } from 'element-plus'
import { Brush } from '@element-plus/icons-vue'
import VueMonacoEditor from '@guolao/vue-monaco-editor'
import { ensureMonaco } from '../../utils/monaco'
import { getEditorSettings } from '../../utils/settings'
import { getResolvedTheme, onResolvedThemeChange } from '../../utils/theme'
import { formatSql, connDialectOf } from '../../utils/sqlFormat'

const props = defineProps({
  modelValue: { type: String, default: '' },
  // 数据库连接类型码（如 MYSQL / POSTGRESQL），用于自动选择 SQL 格式化方言
  connType: { type: String, default: '' },
  placeholder: { type: String, default: '' },
  // 用于编辑器自动补全的表名列表
  tables: { type: Array, default: () => [] },
  // 用于编辑器自动补全的列名：{ 表名: [列名, ...] }
  columns: { type: Object, default: () => ({}) }
})

const emit = defineEmits(['update:modelValue', 'mount'])

const value = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const editorRef = ref(null)
// Monaco 约 2.6MB，改为按需加载：加载完成前先渲染占位，避免拖慢首屏
const monacoReady = ref(false)
ensureMonaco().then(() => { monacoReady.value = true })
  .catch((e) => ElMessage.error(t('sce.loadFailed', { detail: (e && e.message ? e.message : e) })))
const editorInstance = ref(null)
const formatting = ref(false)
const providerDisposables = []

const editorSettings = getEditorSettings()
const editorTheme = ref(getResolvedTheme() === 'dark' ? 'vs-dark' : 'vs-light')
let offEditorTheme = null

// 常用 SQL 关键字（补全列表保持精简，避免大列表拖慢 Monaco）
const SQL_KEYWORDS = [
  'SELECT', 'FROM', 'WHERE', 'JOIN', 'LEFT', 'RIGHT', 'INNER', 'ON', 'GROUP', 'BY',
  'ORDER', 'HAVING', 'LIMIT', 'OFFSET', 'INSERT', 'INTO', 'VALUES', 'UPDATE', 'SET',
  'DELETE', 'CREATE', 'TABLE', 'ALTER', 'DROP', 'AS', 'AND', 'OR', 'NOT', 'IN', 'EXISTS',
  'BETWEEN', 'LIKE', 'IS', 'NULL', 'DISTINCT', 'UNION', 'ALL', 'CASE', 'WHEN', 'THEN',
  'ELSE', 'END', 'COUNT', 'SUM', 'AVG', 'MIN', 'MAX', 'ASC', 'DESC'
]

const editorOptions = {
  automaticLayout: true,
  fontSize: editorSettings.fontSize,
  mouseWheelZoom: true,
  minimap: { enabled: editorSettings.minimap },
  scrollBeyondLastLine: false,
  wordWrap: editorSettings.wordWrap ? 'on' : 'off',
  tabSize: editorSettings.tabSize,
  lineNumbers: editorSettings.lineNumbers ? 'on' : 'off',
  lineNumbersMinChars: 2,
  lineDecorationsWidth: 0,
  // 当前编辑行不做任何高亮：失焦时 'line'/'all' 会把当前行画成一个边框（用户不要这个框）
  renderLineHighlight: 'none',
  suggest: { preview: true, showKeywords: true, showSnippets: true },
  quickSuggestions: { other: true, comments: false, strings: false },
  acceptSuggestionOnEnter: 'on',
  snippetSuggestions: 'bottom',
  fixedOverflowWidgets: true,
  folding: true,
  foldingHighlight: true,
  bracketPairColorization: { enabled: true },
  padding: { top: 8, bottom: 8 },
  contextmenu: false,
  rulers: [],
  overviewRulerLanes: 0,
  overviewRulerBorder: false,
  hideCursorInOverviewRuler: true,
  renderWhitespace: 'none',
  matchBrackets: 'always',
  smoothScrolling: true,
  // 对象表单里格式化由按钮手动触发，关闭自动格式化，避免触发全局 formatting provider 造成循环/卡顿
  formatOnPaste: false,
  formatOnType: false,
  scrollbar: {
    vertical: 'auto',
    horizontal: 'auto',
    useShadows: false,
    verticalHasArrows: false,
    horizontalHasArrows: false,
    verticalScrollbarSize: 8,
    horizontalScrollbarSize: 8,
    arrowSize: 0
  }
}

const onEditorMount = (editor, monaco) => {
  editorInstance.value = editor
  if (monaco) {
    providerDisposables.push(
      monaco.languages.registerCompletionItemProvider('sql', {
        triggerCharacters: ['.'],
        provideCompletionItems: (model, position) => {
          const word = model.getWordUntilPosition(position)
          const range = {
            startLineNumber: position.lineNumber,
            startColumn: word.startColumn,
            endLineNumber: position.lineNumber,
            endColumn: word.endColumn
          }
          const prefix = model.getValueInRange({
            startLineNumber: position.lineNumber,
            startColumn: 1,
            endLineNumber: position.lineNumber,
            endColumn: word.startColumn
          })
          const inputWord = (word.word || '').toUpperCase()
          const matches = (label) => !inputWord || String(label).toUpperCase().startsWith(inputWord)
          const kind = monaco.languages.CompletionItemKind
          const suggestions = []

          // SQL 关键字（精简列表，避免大列表拖慢 suggest widget）
          for (const kw of SQL_KEYWORDS) {
            if (matches(kw)) {
              suggestions.push({ label: kw, kind: kind.Keyword, insertText: kw, range, sortText: '0' + kw })
            }
          }
          // 表名
          for (const t of props.tables || []) {
            if (matches(t)) {
              suggestions.push({ label: t, kind: kind.Class, insertText: t, range, sortText: '1' + t })
            }
          }
          // 列名：在 "表名." 后触发
          const dotMatch = prefix.match(/(?:^|\s)([a-zA-Z_][\w]*)\.\s*$/)
          if (dotMatch) {
            const table = dotMatch[1]
            const cols = props.columns?.[table] || []
            for (const c of cols) {
              suggestions.push({ label: c, kind: kind.Field, insertText: c, range, sortText: '2' + c })
            }
          }
          return { suggestions }
        }
      })
    )
  }
  emit('mount', editor)
}

const doFormat = () => {
  const text = value.value
  if (!text || !text.trim()) return
  formatting.value = true
  // 异步执行格式化，让 loading 状态先渲染，也避免在点击事件栈中长时间阻塞主线程
  requestAnimationFrame(() => {
    try {
      const dialect = connDialectOf(props.connType)
      const formatted = formatSql(text, getEditorSettings(), dialect)
      value.value = formatted
      ElMessage.success(t('sce.formatDone'))
    } catch (e) {
      ElMessage.error(t('sce.formatFailed', { detail: (e?.message || e?.toString?.() || t('common.unknownError')) }))
    } finally {
      formatting.value = false
    }
  })
}

onMounted(() => {
  offEditorTheme = onResolvedThemeChange((r) => {
    editorTheme.value = r === 'dark' ? 'vs-dark' : 'vs-light'
  })
})

onUnmounted(() => {
  if (offEditorTheme) offEditorTheme()
  providerDisposables.forEach((d) => d.dispose())
  providerDisposables.length = 0
})
</script>

<style scoped>
.sql-code-editor {
  position: relative;
  flex: 1;
  min-height: 160px;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--dc-border-soft);
  border-radius: 8px;
  overflow: hidden;
  background: var(--dc-bg-code);
}
.sql-code-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 2px 6px 2px 10px;
  border-bottom: 1px solid var(--dc-border-soft);
  background: var(--dc-bg-soft);
}
.sql-code-bar-tip {
  font-size: 12px;
  color: var(--dc-text-dim);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.sql-code-fmt {
  color: var(--dc-text-dim);
  flex-shrink: 0;
}
.sql-code-fmt:not(.is-disabled):hover {
  color: var(--dc-primary);
  background: var(--dc-bg-hover);
}
.sql-code-main {
  position: relative;
  flex: 1;
  min-height: 120px;
}
.sql-code-main :deep(.vue-monaco-editor) {
  height: 100%;
  border-radius: 0;
}
.sql-code-loading {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--dc-text-dim);
  font-size: 13px;
}
.sql-code-placeholder {
  position: absolute;
  top: 10px;
  left: 14px;
  right: 14px;
  z-index: 5;
  pointer-events: none;
  color: var(--dc-text-dim);
  font-family: "SF Mono", Consolas, monospace;
  font-size: 13px;
  line-height: 1.7;
  white-space: pre-wrap;
  opacity: .85;
}
</style>

