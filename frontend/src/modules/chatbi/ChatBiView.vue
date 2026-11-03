<template>
  <div class="chatbi">
    <!-- 顶部：数据源 + 操作 -->
    <header class="cb-head">
      <div class="cb-brand">
        <el-icon class="cb-logo"><DataAnalysis /></el-icon>
        <div class="cb-brand-txt">
          <div class="cb-title">智能问数</div>
          <div class="cb-sub">一句话问出数据，自动出图与结论</div>
        </div>
      </div>
      <div class="cb-src">
        <ConnectionSelect
          v-model="connId"
          :connections="connOptions"
          :width-min="170"
          :width-max="320"
          filterable
          placeholder="选择连接"
          class="cb-conn"
          @change="onConnChange"
        />
        <el-select
          v-model="database"
          class="cb-db"
          size="small"
          filterable
          placeholder="选择库"
          :loading="dbLoading"
          @change="onDbChange"
        >
          <el-option v-for="db in dbOptions" :key="db" :label="db" :value="db" />
        </el-select>
        <el-tooltip content="只执行只读查询（SELECT），不会改动任何数据" placement="bottom">
          <span class="cb-guard"><el-icon><Lock /></el-icon>只读</span>
        </el-tooltip>
        <el-button size="small" :icon="RefreshLeft" @click="resetChat">新对话</el-button>
      </div>
    </header>

    <div class="cb-body">
      <!-- 左侧：示例 / 常用 -->
      <aside class="cb-side">
        <div class="side-block">
          <div class="side-title">
            <span>试试这样问</span>
            <el-button link size="small" :loading="exLoading" @click="loadExamples">换一批</el-button>
          </div>
          <div v-if="!examples.length" class="side-empty">选择连接后给出示例</div>
          <button v-for="(ex, i) in examples" :key="i" class="ex-item" :title="ex.question" @click="ask(ex.question)">
            <el-icon><ChatLineSquare /></el-icon>
            <span>{{ ex.question }}</span>
          </button>
        </div>

        <div class="side-block">
          <div class="side-title">
            <span>常用问数</span>
            <span class="side-count">{{ favorites.length }}</span>
          </div>
          <div v-if="!favorites.length" class="side-empty">问出满意的结果后点「收藏」</div>
          <div v-for="(fav, i) in favorites" :key="fav.id" class="fav-item">
            <span class="fav-q" :title="fav.question" @click="ask(fav.question)">{{ fav.question }}</span>
            <el-icon class="fav-del" @click="removeFavorite(i)"><Close /></el-icon>
          </div>
        </div>

        <div class="side-hint">
          <el-icon><InfoFilled /></el-icon>
          <span>问数只读取数据；图表可切换类型、导出图片，SQL 可一键送到编辑器。</span>
        </div>
      </aside>

      <!-- 主区：对话 -->
      <main ref="scrollRef" class="cb-main">
        <div v-if="!messages.length" class="cb-hero">
          <el-icon class="hero-icon"><DataAnalysis /></el-icon>
          <h3>想知道什么，直接问</h3>
          <p>例如「上个月各渠道的销售额和环比」「订单量最多的 10 个商品」「最近 7 天每天的支付成功率」</p>
          <div class="hero-chips">
            <button v-for="(s, i) in heroHints" :key="i" class="hero-chip" @click="ask(s)">{{ s }}</button>
          </div>
        </div>

        <div v-for="msg in messages" :key="msg.id" class="msg" :class="`msg-${msg.role}`">
          <!-- 用户 -->
          <div v-if="msg.role === 'user'" class="bubble-user">{{ msg.text }}</div>

          <!-- 助手：过程与结果都交给 AskAnswer —— 流式时它就是「执行过程」（一步步点亮），
               跑完自动合并成一行。不再另做一套按经验时间推进的假进度条 -->
          <div v-else class="answer">
            <el-alert v-if="msg.error" :title="msg.error" type="error" :closable="false" show-icon />

            <!-- 需要澄清 -->
            <div v-else-if="msg.answer && msg.answer.intent === 'clarify'" class="clarify">
              <div class="clarify-title"><el-icon><QuestionFilled /></el-icon>还需要你补充一点信息</div>
              <div class="clarify-text">{{ msg.answer.clarify }}</div>
              <div v-if="msg.answer.followups?.length" class="chips">
                <button v-for="(f, i) in msg.answer.followups" :key="i" class="chip" @click="ask(f)">{{ f }}</button>
              </div>
            </div>

            <!-- 回答渲染统一交给 AskAnswer（与 AI 助手对话流里那份是同一实现） -->
            <AskAnswer v-else-if="msg.answer" :answer="msg.answer" @ask="ask"
                       @insert-sql="emit('insert-sql', $event)" />
          </div>
        </div>
      </main>
    </div>

    <!-- 输入 -->
    <footer class="cb-input">
      <el-input
        v-model="question"
        type="textarea"
        :rows="2"
        resize="none"
        :placeholder="inputPlaceholder"
        :disabled="busy"
        @keydown.enter.ctrl.prevent="ask()"
        @keydown.enter.meta.prevent="ask()"
      />
      <div class="input-side">
        <el-button type="primary" :loading="busy" :icon="Promotion" @click="ask()">问一下</el-button>
        <span class="input-tip">Ctrl + Enter 发送</span>
      </div>
    </footer>
  </div>
</template>

<script setup>
// 智能问数（ChatBI）主界面。
//
// 交互上只有一条主线：**一句话 → 一张图 + 一句结论**。刻意不做「先选表、再配维度度量」
// 那套 BI 拖拽：用户想说的是业务问题，不是字段名。
//
// 几个决定：
// 1. 一次问答 = 一次 HTTP（后端把"选表→写 SQL→执行→定图表"合成一次），前端不做重试编排 ——
//    中间态越多，出错时的提示越难写清楚。
// 2. 图表类型可**本地切换**：后端只给推荐的类型与字段映射，切换时用同一份结果集重画，
//    不需要再问一次模型（省钱且即时）。
// 3. 结果集留在消息里，SQL/明细都能再展开 —— 问数最怕"只给张图，数对不上没法查"。
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue'
import {
  DataAnalysis, Lock, RefreshLeft, ChatLineSquare, Close, InfoFilled,
  QuestionFilled, Star, Promotion,
} from '@element-plus/icons-vue'
import { ElMessage } from 'element-plus'
import ConnectionSelect from '../../common/ConnectionSelect.vue'
import AskAnswer from './AskAnswer.vue'
import { mergeStep } from './askSteps.js'
import { listConnections, listDatabases, aiAskStream, aiAskExamples } from '../../api/index.js'

const props = defineProps({
  /** 打开页签时带过来的连接 / 库（从树上右键问数、或表详情跳过来） */
  conn: { type: Object, default: null },
  database: { type: String, default: '' },
})
const emit = defineEmits(['insert-sql'])

const FAV_KEY = 'dbmind.chatbi.favorites'
const SRC_KEY = 'dbmind.chatbi.source'

const connOptions = ref([])
const connId = ref('')
const database = ref('')
const dbOptions = ref([])
const dbLoading = ref(false)
const examples = ref([])
const exLoading = ref(false)
const favorites = ref(loadFavorites())
const messages = ref([])
const question = ref('')
const busy = ref(false)
const scrollRef = ref(null)

let seq = 0

const uid = () => `m${++seq}_${Date.now().toString(36)}`

const inputPlaceholder = computed(() =>
  connId.value ? '问点什么，比如：上个月各渠道的销售额和环比' : '先选择左侧的连接与库')

const heroHints = [
  '最近 30 天每天的订单量趋势',
  '各地区的销售额排名 Top 10',
  '本月各产品线的收入占比',
  '下单最多的 10 个客户',
]

/* ---------------- 数据源 ---------------- */

const loadConnections = async () => {
  try {
    const list = await listConnections()
    connOptions.value = Array.isArray(list) ? list : []
    const saved = readJson(SRC_KEY, null)
    const preset = props.conn?.id || saved?.connId
    // 预置来源 > 上次用的 > 第一条；不能空着，空着就没有示例问题可给
    const hit = connOptions.value.find((c) => c.id === preset) || connOptions.value[0]
    if (hit) {
      connId.value = hit.id
      await onConnChange(hit.id, props.database || saved?.database || '')
    }
  } catch (e) {
    ElMessage.error(e?.message || '读取连接失败')
  }
}

const onConnChange = async (id, prefer) => {
  database.value = ''
  dbOptions.value = []
  examples.value = []
  const conn = (id && typeof id === 'object' ? id : connOptions.value.find((c) => c.id === id)) || null
  const target = conn?.id || connId.value
  if (!target) return
  connId.value = target
  dbLoading.value = true
  try {
    const list = await listDatabases(target)
    const names = (Array.isArray(list) ? list : [])
      .map((db) => (typeof db === 'string' ? db : db?.name))
      .filter(Boolean)
    dbOptions.value = names
    const want = prefer || conn?.database || ''
    database.value = names.includes(want) ? want : (names[0] || '')
  } catch {
    // 列库失败不弹错：连接本身可能只是没起，示例问题会空着，用户仍能看到原因
    dbOptions.value = []
  } finally {
    dbLoading.value = false
  }
  persistSource()
  loadExamples()
}

const onDbChange = () => {
  persistSource()
  loadExamples()
}

const persistSource = () => {
  try {
    localStorage.setItem(SRC_KEY, JSON.stringify({ connId: connId.value, database: database.value }))
  } catch { /* 隐私模式忽略 */ }
}

const loadExamples = async () => {
  if (!connId.value || exLoading.value) return
  exLoading.value = true
  try {
    const res = await aiAskExamples(connId.value, database.value)
    examples.value = (res?.examples || []).slice(0, 8)
  } catch {
    examples.value = []
  } finally {
    exLoading.value = false
  }
}

/* ---------------- 提问 ---------------- */

const historyFor = () => {
  const list = []
  for (const msg of messages.value.slice(-6)) {
    if (msg.role === 'user') list.push({ role: 'user', content: msg.text })
    else if (msg.answer?.sql) {
      list.push({ role: 'assistant', content: `${msg.answer.summary || ''}\n${msg.answer.sql}` })
    }
  }
  return list.slice(-4)
}

const ask = async (text) => {
  const q = String(text ?? question.value ?? '').trim()
  if (!q || busy.value) return
  if (!connId.value) {
    ElMessage.warning('请先选择连接')
    return
  }
  messages.value.push({ id: uid(), role: 'user', text: q })
  // 流式占位：先挂一个「只有步骤、没有结论」的回答对象，AskAnswer 据此只渲染执行过程；
  // 后端每推一步就补一行，收到完整回答后整体替换（streaming 标记随之消失 → 自动合并收起）
  const answer = reactive({
    id: uid(), role: 'assistant', error: '',
    answer: {
      streaming: true, intent: 'query', question: q,
      summary: '', sql: '', columns: [], columnTypes: [], rows: [], rowCount: 0,
      charts: [], insights: [], followups: [], steps: [],
    },
  })
  messages.value.push(answer)
  question.value = ''
  busy.value = true
  scrollToBottom()

  const history = historyFor()
  try {
    const res = await aiAskStream(
      { connectionId: connId.value, database: database.value, question: q, history, lang: 'zh' },
      (step) => {
        mergeStep(answer.answer.steps, step)
        scrollToBottom()
      },
    )
    answer.answer = res
  } catch (e) {
    answer.error = e?.message || '问数失败，请稍后重试'
    answer.answer = null
  } finally {
    busy.value = false
    scrollToBottom()
    nextTick(() => scrollToBottom())
  }
}

const scrollToBottom = () => {
  nextTick(() => {
    const el = scrollRef.value
    if (el) el.scrollTop = el.scrollHeight
  })
}

const resetChat = () => {
  messages.value = []
  question.value = ''
}

/* ---------------- 收藏 ---------------- */

function loadFavorites() {
  const list = readJson(FAV_KEY, [])
  return Array.isArray(list) ? list : []
}
function readJson(key, fallback) {
  try {
    const raw = localStorage.getItem(key)
    return raw ? JSON.parse(raw) : fallback
  } catch {
    return fallback
  }
}
const saveFavorites = () => {
  try { localStorage.setItem(FAV_KEY, JSON.stringify(favorites.value.slice(0, 30))) } catch { /* ignore */ }
}
const favorite = (text) => {
  const q = String(text || '').trim()
  if (!q) return
  if (favorites.value.some((f) => f.question === q)) {
    ElMessage.info('已经在常用里了')
    return
  }
  favorites.value.unshift({ id: uid(), question: q })
  saveFavorites()
  ElMessage.success('已收藏')
}
const removeFavorite = (index) => {
  favorites.value.splice(index, 1)
  saveFavorites()
}

watch(() => props.conn, () => { if (props.conn?.id) { connId.value = props.conn.id; onConnChange(props.conn.id, props.database) } })

// 只换了库、连接没变的情况（AI 助手技能入口「智能问数」带上下文过来、或顶栏菜单再点一次）：
// conn 的 watch 不会触发，这里补一条，否则页签切过来了、库还是上一次的。
watch(() => props.database, (db) => {
  if (!db || db === database.value) return
  // 库还没加载完时也记下来：onConnChange 拉完清单会按 database 兜底选中
  database.value = db
  onDbChange()
})

onMounted(loadConnections)
</script>

<style scoped>
.chatbi { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--dc-bg); color: var(--dc-text); }

/* 顶部 */
.cb-head {
  display: flex; align-items: center; gap: 16px;
  padding: 10px 16px; border-bottom: 1px solid var(--dc-border);
  background: var(--dc-bg-sidebar);
}
.cb-brand { display: flex; align-items: center; gap: 10px; }
.cb-logo { font-size: 22px; color: var(--dc-primary); }
.cb-title { font-size: 14px; font-weight: 600; line-height: 1.2; }
.cb-sub { font-size: 11px; color: var(--dc-text-2); }
.cb-src { display: flex; align-items: center; gap: 8px; margin-left: auto; }
.cb-guard {
  display: inline-flex; align-items: center; gap: 4px;
  font-size: 11px; color: var(--dc-text-2);
  border: 1px solid var(--dc-border); border-radius: 10px; padding: 2px 8px;
}

/* 主体 */
.cb-body { display: flex; flex: 1; min-height: 0; }
.cb-side {
  width: 250px; flex: none; border-right: 1px solid var(--dc-border);
  background: var(--dc-bg-sidebar); padding: 12px; overflow-y: auto;
}
.side-block { margin-bottom: 18px; }
.side-title {
  display: flex; align-items: center; justify-content: space-between;
  font-size: 12px; color: var(--dc-text-2); margin-bottom: 8px;
}
.side-count { font-size: 11px; opacity: .7; }
.side-empty { font-size: 11px; color: var(--dc-text-2); opacity: .7; padding: 6px 2px; }
.ex-item {
  display: flex; align-items: flex-start; gap: 6px; width: 100%;
  text-align: left; background: transparent; border: 0; cursor: pointer;
  padding: 7px 8px; border-radius: 6px; color: var(--dc-text);
  font-size: 12px; line-height: 1.45;
}
.ex-item:hover { background: var(--dc-bg-card); }
.ex-item .el-icon { color: var(--dc-primary); margin-top: 2px; flex: none; }
.ex-item span { overflow: hidden; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; }
.fav-item { display: flex; align-items: center; gap: 4px; padding: 4px 6px; border-radius: 6px; }
.fav-item:hover { background: var(--dc-bg-card); }
.fav-q {
  flex: 1; font-size: 12px; cursor: pointer; overflow: hidden;
  white-space: nowrap; text-overflow: ellipsis; color: var(--dc-text);
}
.fav-del { cursor: pointer; color: var(--dc-text-2); opacity: 0; }
.fav-item:hover .fav-del { opacity: 1; }
.side-hint {
  display: flex; gap: 6px; font-size: 11px; line-height: 1.6;
  color: var(--dc-text-2); border-top: 1px solid var(--dc-border); padding-top: 10px;
}

/* 对话区 */
.cb-main { flex: 1; min-width: 0; overflow-y: auto; padding: 18px 22px 8px; }
.cb-hero { max-width: 620px; margin: 40px auto; text-align: center; color: var(--dc-text-2); }
.hero-icon { font-size: 40px; color: var(--dc-primary); }
.cb-hero h3 { margin: 12px 0 6px; font-size: 17px; color: var(--dc-text); }
.cb-hero p { font-size: 12px; line-height: 1.7; }
.hero-chips { display: flex; flex-wrap: wrap; gap: 8px; justify-content: center; margin-top: 16px; }
.hero-chip {
  border: 1px solid var(--dc-border); background: var(--dc-bg-card); color: var(--dc-text);
  border-radius: 14px; padding: 5px 12px; font-size: 12px; cursor: pointer;
}
.hero-chip:hover { border-color: var(--dc-primary); color: var(--dc-primary); }

.msg { margin-bottom: 18px; }
.msg-user { display: flex; justify-content: flex-end; }
.bubble-user {
  max-width: 78%; background: var(--dc-primary); color: #fff;
  padding: 8px 12px; border-radius: 10px 10px 2px 10px; font-size: 13px; line-height: 1.6;
  white-space: pre-wrap; word-break: break-word;
}
.answer { border: 1px solid var(--dc-border); background: var(--dc-bg-card); border-radius: 10px; padding: 12px 14px; }

/* 输入 */
.cb-input {
  display: flex; align-items: flex-end; gap: 10px;
  padding: 10px 16px 12px; border-top: 1px solid var(--dc-border); background: var(--dc-bg-sidebar);
}
.cb-input :deep(.el-textarea__inner) {
  background: var(--dc-bg-card); color: var(--dc-text);
  border-color: var(--dc-border); font-size: 13px;
}
.input-side { display: flex; flex-direction: column; align-items: flex-end; gap: 4px; }
.input-tip { font-size: 10px; color: var(--dc-text-2); }
</style>
