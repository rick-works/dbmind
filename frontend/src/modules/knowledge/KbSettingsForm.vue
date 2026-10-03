<template>
  <div class="kbset">
    <div class="kbset-grid">
      <!-- 左：配置 -->
      <div class="kbset-col">
        <!-- 分段模式：参数就长在各自模式的卡片里，选哪个模式、改的就是哪套参数 -->
        <section class="sec">
          <div class="sec-title">
            <span>{{ $t('kbs.chunkMode') }}</span>
            <el-button size="small" text @click="restoreDefaults">{{ $t('kbs.restoreDefaults') }}</el-button>
          </div>
          <div class="mode-opts">
            <!-- 通用 -->
            <div class="mode-opt" :class="{ active: cfg.chunkMode === 'general' }"
                 @click="pickValue('chunkMode', 'general')">
              <div class="mode-opt-head">
                <span class="mode-opt-name">{{ $t('kbs.general') }}</span>
                <el-tooltip placement="top" :content="$t('kbs.generalTip')">
                  <el-icon class="q"><QuestionFilled /></el-icon>
                </el-tooltip>
                <el-icon v-if="cfg.chunkMode === 'general'" class="mode-opt-check"><CircleCheck /></el-icon>
              </div>
              <div class="mode-opt-desc">{{ $t('kbs.generalDesc') }}</div>

              <!-- 通用参数：和「通用」放在一起，不选它就不展开 -->
              <div v-if="cfg.chunkMode === 'general'" class="mode-opt-body" @click.stop>
                <div class="pgrid">
                  <div class="pitem">
                    <label>{{ $t('kbs.separatorLabel') }}</label>
                    <el-input v-model="cfg.separator" :placeholder="$t('kbs.separatorPlaceholder')" clearable @change="emit('dirty')" />
                  </div>
                  <div class="pitem">
                    <label>{{ $t('kbs.maxLenLabel') }}</label>
                    <div class="num-wrap">
                      <el-input-number v-model="cfg.chunkSize" :min="80" :max="4000" :step="100"
                                       controls-position="right" @change="emit('dirty')" />
                      <span class="num-unit">characters</span>
                    </div>
                  </div>
                  <div class="pitem">
                    <label>{{ $t('kbs.overlapLabel') }}</label>
                    <div class="num-wrap">
                      <el-input-number v-model="cfg.chunkOverlap" :min="0" :max="2000" :step="10"
                                       controls-position="right" @change="emit('dirty')" />
                      <span class="num-unit">characters</span>
                    </div>
                  </div>
                  <div class="pitem">
                    <label>{{ $t('kbs.minCharsLabel') }}</label>
                    <div class="num-wrap">
                      <el-input-number v-model="cfg.minChunkChars" :min="0" :max="2000" :step="10"
                                       controls-position="right" @change="emit('dirty')" />
                      <span class="num-unit">characters</span>
                    </div>
                  </div>
                </div>
              </div>
            </div>

            <!-- 父子分段 -->
            <div class="mode-opt" :class="{ active: cfg.chunkMode === 'parentChild' }"
                 @click="pickValue('chunkMode', 'parentChild')">
              <div class="mode-opt-head">
                <span class="mode-opt-name">{{ $t('kbs.parentChild') }}</span>
                <el-tooltip placement="top"
                            :content="$t('kbs.parentChildTip')">
                  <el-icon class="q"><QuestionFilled /></el-icon>
                </el-tooltip>
                <span class="mode-opt-tag">{{ $t('kbs.recommended') }}</span>
                <el-icon v-if="cfg.chunkMode === 'parentChild'" class="mode-opt-check"><CircleCheck /></el-icon>
              </div>
              <div class="mode-opt-desc">{{ $t('kbs.parentChildDesc') }}</div>

              <div v-if="cfg.chunkMode === 'parentChild'" class="mode-opt-body" @click.stop>
                <div class="field">
                  <label>{{ $t('kbs.parentLabel') }}</label>
                  <div class="mode-opts mode-opts-row">
                    <div v-for="p in parentOptions" :key="p.value" class="mode-opt"
                         :class="{ active: cfg.parentMode === p.value }" @click="pickValue('parentMode', p.value)">
                      <div class="mode-opt-head">
                        <span class="mode-opt-name">{{ p.label }}</span>
                        <el-icon v-if="cfg.parentMode === p.value" class="mode-opt-check"><CircleCheck /></el-icon>
                      </div>
                      <div class="mode-opt-desc">{{ p.desc }}</div>
                    </div>
                  </div>
                </div>

                <div class="pgrid">
                  <div v-if="cfg.parentMode === 'paragraph'" class="pitem">
                    <label>{{ $t('kbs.parentSepLabel') }}</label>
                    <el-input v-model="cfg.parentSeparator" placeholder="\n\n" clearable @change="emit('dirty')" />
                  </div>
                  <div v-if="cfg.parentMode === 'paragraph'" class="pitem">
                    <label>{{ $t('kbs.parentMaxLabel') }}</label>
                    <div class="num-wrap">
                      <el-input-number v-model="cfg.parentSize" :min="80" :max="6000" :step="100"
                                       controls-position="right" @change="emit('dirty')" />
                      <span class="num-unit">characters</span>
                    </div>
                  </div>
                  <div class="pitem">
                    <label>{{ $t('kbs.childSepLabel') }}</label>
                    <el-input v-model="cfg.childSeparator" placeholder="\n" clearable @change="emit('dirty')" />
                  </div>
                  <div class="pitem">
                    <label>{{ $t('kbs.childMaxLabel') }}</label>
                    <div class="num-wrap">
                      <el-input-number v-model="cfg.childSize" :min="80" :max="4000" :step="50"
                                       controls-position="right" @change="emit('dirty')" />
                      <span class="num-unit">characters</span>
                    </div>
                  </div>
                  <div class="pitem">
                    <label>{{ $t('kbs.minCharsLabel') }}</label>
                    <div class="num-wrap">
                      <el-input-number v-model="cfg.minChunkChars" :min="0" :max="2000" :step="10"
                                       controls-position="right" @change="emit('dirty')" />
                      <span class="num-unit">characters</span>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </section>

        <!-- 文本预处理规则 -->
        <section class="sec">
          <div class="sec-title">{{ $t('kbs.preprocess') }}</div>
          <div class="check-list">
            <el-checkbox v-model="cfg.cleanWhitespace" @change="emit('dirty')">
              {{ $t('kbs.preSpace') }}
            </el-checkbox>
            <el-checkbox v-model="cfg.stripLinks" @change="emit('dirty')">
              {{ $t('kbs.preUrl') }}
            </el-checkbox>
          </div>
        </section>

        <!-- 索引方式：先定「要不要花成本建向量」，再定具体检索方式 -->
        <section class="sec">
          <div class="sec-title">{{ $t('kbs.indexMode') }}</div>
          <div class="q-opts">
            <div class="q-opt" :class="{ active: highQuality }" @click="pickQuality('high')">
              <div class="q-opt-icon high"><el-icon><Sunny /></el-icon></div>
              <div class="q-opt-body">
                <div class="q-opt-head">
                  <span class="q-opt-name">{{ $t('kbs.highQuality') }}</span>
                  <el-tooltip placement="top"
                              :content="$t('kbs.highQualityTip')">
                    <el-icon class="q"><QuestionFilled /></el-icon>
                  </el-tooltip>
                  <span class="q-opt-tag">{{ $t('kbs.recommended') }}</span>
                </div>
                <div class="q-opt-desc">{{ $t('kbs.highQualityDesc') }}</div>
              </div>
            </div>
            <div class="q-opt" :class="{ active: !highQuality }" @click="pickQuality('economy')">
              <div class="q-opt-icon econ"><el-icon><Wallet /></el-icon></div>
              <div class="q-opt-body">
                <div class="q-opt-head">
                  <span class="q-opt-name">{{ $t('kbs.economy') }}</span>
                  <el-tooltip placement="top"
                              :content="$t('kbs.economyTip')">
                    <el-icon class="q"><QuestionFilled /></el-icon>
                  </el-tooltip>
                </div>
                <div class="q-opt-desc">{{ $t('kbs.economyDesc') }}</div>
              </div>
            </div>
          </div>

          <template v-if="highQuality">
            <div class="field">
              <label>
                {{ $t('kbs.embeddingModel') }}
                <el-tooltip placement="top"
                            :content="$t('kbs.embeddingTip')">
                  <el-icon class="q"><QuestionFilled /></el-icon>
                </el-tooltip>
              </label>
              <!-- 不支持向量化的模型置灰：选中它只会在导入时白报一次错 -->
              <el-select v-model="cfg.embedModelId" :placeholder="embedPlaceholder" clearable
                         style="width:100%" @change="emit('dirty')">
                <el-option v-for="m in models" :key="m.id" :value="m.id"
                           :label="m.name || m.model || m.id"
                           :disabled="!m.supportEmbedding">
                  <span class="opt-row">
                    <span>{{ m.name || m.model || m.id }}</span>
                    <span v-if="!m.supportEmbedding" class="opt-no">{{ $t('kbs.noEmbedding') }}</span>
                  </span>
                </el-option>
              </el-select>
            </div>
          </template>
        </section>

        <!-- {{ $t('kbs.retrievalSettings') }}：选中哪张卡，参数就展开在哪张卡里 -->
        <section class="sec">
          <div class="sec-title">
            <span class="sec-name">
              {{ $t('kbs.retrievalSettings') }}
              <el-tooltip placement="top"
                          :content="$t('kbs.retrievalTip')">
                <el-icon class="q"><QuestionFilled /></el-icon>
              </el-tooltip>
            </span>
          </div>

          <!-- 经济模式：检索方式固定，作为只读卡片展示 -->
          <div v-if="!highQuality" class="r-opt active is-static">
            <div class="r-opt-icon"><el-icon><Grid /></el-icon></div>
            <div class="r-opt-body">
              <div class="r-opt-head"><span class="r-opt-name">{{ $t('kbs.inverted') }}</span></div>
              <div class="r-opt-desc">{{ $t('kbs.invertedDesc') }}</div>
            </div>
          </div>

          <template v-else>
            <div v-for="r in retrieveOptions" :key="r.value" class="r-opt"
                 :class="{ active: cfg.indexMode === r.value }" @click="pickValue('indexMode', r.value)">
              <div class="r-opt-icon"><el-icon><component :is="r.icon" /></el-icon></div>
              <div class="r-opt-body">
                <div class="r-opt-head">
                  <span class="r-opt-name">{{ r.label }}</span>
                  <el-tooltip placement="top" :content="r.tip">
                    <el-icon class="q"><QuestionFilled /></el-icon>
                  </el-tooltip>
                  <span v-if="r.tag" class="r-opt-tag">{{ r.tag }}</span>
                </div>
                <div class="r-opt-desc">{{ r.desc }}</div>

                <div v-if="cfg.indexMode === r.value" class="r-opt-in" @click.stop>
                  <div v-if="r.value === 'hybrid'" class="weight-row">
                    <div class="weight-title">{{ $t('kbs.weightTitle') }}</div>
                    <!-- 滑轨左右两段着色 + 两端可精确输入：拖也行、直接填数也行 -->
                    <el-slider v-model="cfg.hybridWeight" :min="0" :max="1" :step="0.05"
                               :show-tooltip="false" class="weight-slider"
                               :style="{ '--vw': vectorPct + '%' }" @change="emit('dirty')" />
                    <div class="weight-sides">
                      <label>
                        <span class="weight-side-label">{{ $t('kbs.semantic') }}</span>
                        <div class="num-wrap weight-num">
                          <el-input-number v-model="vectorPct" :min="0" :max="100" :step="5"
                                           controls-position="right" />
                          <span class="num-unit">%</span>
                        </div>
                      </label>
                      <label>
                        <span class="weight-side-label">{{ $t('kbs.keyword') }}</span>
                        <div class="num-wrap weight-num">
                          <el-input-number v-model="kwPct" :min="0" :max="100" :step="5"
                                           controls-position="right" />
                          <span class="num-unit">%</span>
                        </div>
                      </label>
                    </div>
                  </div>

                </div>
              </div>
            </div>
          </template>

          <!-- 召回条数：两种索引方式都在用它（关键词模式同样按这个数注入片段），
               所以不能挂在某一张检索方式的卡里 —— 那样选「经济」就既看不见也改不了 -->
          <div class="r-opt active is-static">
            <div class="r-opt-icon"><el-icon><Grid /></el-icon></div>
            <div class="r-opt-body">
              <div class="r-opt-head"><span class="r-opt-name">{{ $t('kbs.topK') }}</span></div>
              <div class="r-opt-desc">
                {{ $t('kbs.topKDesc') }}
                {{ $t('kbs.topKDesc2') }}
                {{ $t('kbs.topKDesc3') }}
              </div>
              <div class="r-opt-in">
                <div class="pgrid">
                  <div class="pitem">
                    <label>
                      Top K
                      <el-tooltip placement="top"
                                  :content="$t('kbs.topKTip', { n: cfg.topK })">
                        <el-icon class="q"><QuestionFilled /></el-icon>
                      </el-tooltip>
                    </label>
                    <div class="num-wrap">
                      <el-input-number v-model="cfg.topK" :min="1" :max="20" controls-position="right"
                                       @change="emit('dirty')" />
                      <span class="num-unit">chunks</span>
                    </div>
                    <el-slider v-model="cfg.topK" :min="1" :max="20" :show-tooltip="false"
                               @change="emit('dirty')" />
                  </div>
                  <!-- {{ $t('kbs.scoreThreshold') }}只对向量/混合有意义：关键词走的是 BM25 原始分，量纲对不上 -->
                  <div v-if="highQuality" class="pitem">
                    <label>
                      {{ $t('kbs.scoreThreshold') }}
                      <el-tooltip :content="$t('kbs.scoreThresholdTip')" placement="top">
                        <el-icon class="q"><QuestionFilled /></el-icon>
                      </el-tooltip>
                      <el-switch v-model="scoreOn" size="small" class="pitem-switch" />
                    </label>
                    <div v-if="scoreOn" class="num-wrap">
                      <el-input-number v-model="cfg.minScore" :min="0" :max="1" :step="0.05" :precision="2"
                                       controls-position="right" @change="emit('dirty')" />
                      <span class="num-unit">score</span>
                    </div>
                    <el-slider v-if="scoreOn" v-model="cfg.minScore" :min="0" :max="1" :step="0.05"
                               :show-tooltip="false" @change="emit('dirty')" />
                  </div>
                </div>
              </div>
            </div>
          </div>

        </section>
      </div>

      <!-- 右：预览（调参数时立刻看结果） -->
      <div class="kbset-col kbset-preview">
        <div class="sec-title">
          <span>{{ $t('kbs.previewTitle') }}</span>
          <el-button size="small" type="primary" plain :loading="previewing"
                     :disabled="!previewText.trim()" @click="runPreview">
            {{ previewData ? $t('kbs.rePreview') : $t('kbs.previewChunks') }}
          </el-button>
        </div>
        <!-- 素材来源：库里有多份资料时必须能挑，否则调参数时永远只看到第一份的效果 -->
        <div v-if="previewDocs.length" class="preview-src">
          <span class="preview-src-label">{{ $t('kbs.previewSource') }}</span>
          <el-select v-model="previewDocId" size="small" :loading="loadingDocText"
                     :placeholder="$t('kbs.pickDoc')" style="flex:1" @change="switchPreviewDoc">
            <el-option v-for="d in previewDocs" :key="d.id" :value="d.id"
                       :label="d.chunkCount != null ? $t('kbs.docOption', { name: (d.title || d.source || $t('kbs.untitled')), n: d.chunkCount }) : (d.title || d.source || $t('kbs.untitled'))" />
          </el-select>
        </div>
        <div v-if="previewData" class="preview-stat">
          {{ $t('kbs.previewStats', { chunks: previewData.count, avg: previewData.avgChars }) }}
          <template v-if="previewData.parentCount">{{ $t('kbs.parentCount', { n: previewData.parentCount }) }}</template>
          <template v-if="previewData.dropped">{{ $t('kbs.dropped', { n: previewData.dropped }) }}</template>
          <template v-if="previewData.truncated">{{ $t('kbs.truncated', { n: previewData.items.length }) }}</template>
          <template v-if="previewData.cleanedChars">{{ $t('kbs.cleanedChars', { n: previewData.cleanedChars }) }}</template>
        </div>


        <div v-if="previewing" class="preview-stat">{{ $t('kbs.splitting') }}</div>
        <div class="kbset-body">
          <div v-for="c in (previewData?.items || [])" :key="c.index" class="chunk">
            <div class="chunk-head">
              <span class="chunk-no">#{{ c.index }}</span>
              <span class="chunk-size">{{ $t('kbx.chars', { n: c.charCount }) }}</span>
              <span v-if="c.parentIndex" class="chunk-parent">{{ $t('kbs.parentOf', { n: c.parentIndex }) }}</span>
            </div>
            <div class="chunk-text">{{ c.text }}</div>
          </div>
          <div v-if="!previewData && !previewing" class="preview-empty">
            <div>{{ $t('kbs.noDocsPreview') }}</div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, markRaw } from 'vue'
import { ElMessage } from 'element-plus'
import {
  CircleCheck, Sunny, Wallet, Grid, Connection, QuestionFilled, Document
} from '@element-plus/icons-vue'
import { kbPreview, kbDocChunks } from '../../api'
import { t } from '../../utils/i18n'

const props = defineProps({
  /** 配置对象（分段 + 预处理 + 索引 + 检索）。父组件持有，这里直接改字段并 emit('dirty') 通知保存 */
  config: { type: Object, required: true },
  /** 可选模型列表（用于向量化选型） */
  models: { type: Array, default: () => [] },
  /** 预览用的初始文本（向导里带上待导入资料） */
  previewText: { type: String, default: '' },
  /** 当前库 id + 预览素材清单：多份资料时要能挑一份来预览。
   *  两种来源共用这个 prop：库内资料（只有 id，正文要现拉）与新建向导里的待导入文件（自带 text） */
  kbId: { type: String, default: '' },
  docs: { type: Array, default: () => [] }
})
const emit = defineEmits(['dirty', 'previewed'])

const cfg = props.config
// **分隔正则给默认值**：留空的语义（按空行）太隐晦，用户不该被要求会写正则（真机反馈）。
// 父组件传进来的 config 里 separator 为空时，直接用推荐值填上（可改可清）。
if (cfg && !cfg.separator) cfg.separator = '\\n####'
const previewText = ref(props.previewText || '')
const previewData = ref(null)
/** 预览哪一份资料（库里有几份时可以切换，不然永远只能看到第一份） */
const previewDocId = ref('')
const loadingDocText = ref(false)

const previewDocs = computed(() => props.docs || [])

// 素材到位后默认选中第一份：下拉空着会让人以为「没得选」
watch(previewDocs, (list) => {
  if (!previewDocId.value && list.length) previewDocId.value = list[0].id
}, { immediate: true })

/**
 * 换一份素材当预览对象。
 *
 * <p>待导入的文件自带正文，直接用；已在库里的资料只有 id，得取回分块拼成全文。
 * 两条路都走同一个入口，界面才不用分两套。
 */
const switchPreviewDoc = async (docId) => {
  previewDocId.value = docId
  if (docId === '' || docId == null) return
  const picked = previewDocs.value.find(d => d.id === docId)
  loadingDocText.value = true
  try {
    let text = picked?.text || ''
    if (!text.trim()) {
      if (!props.kbId) { ElMessage.warning(t('kbs.noSourceText')); loadingDocText.value = false; return }
      const res = await kbDocChunks({ kbId: props.kbId, docId })
      const blocks = (res && res.chunks) || []
      text = blocks.map(c => c.text || '').join('\n\n')
    }
    if (!text.trim()) { ElMessage.warning(t('kbs.noPreviewText')); loadingDocText.value = false; return }
    previewText.value = text
    previewData.value = null
    await runPreview()
  } catch (e) {
    ElMessage.error(t('kbs.readSourceFailed', { detail: (e?.message || e) }))
  }
  loadingDocText.value = false
}


const previewing = ref(false)

// 分段长度直接按「字符」填写，与后端分块引擎同单位。
// 不再做 token 换算：界面写 2000、实际切 2600 字这种偏差很难向用户解释。

/** {{ $t('kbs.scoreThreshold') }}开关：关掉等价于 minScore = 0（有分就收） */
const scoreOn = computed({
  get: () => (cfg.minScore || 0) > 0,
  set: (v) => { cfg.minScore = v ? 0.3 : 0; emit('dirty') }
})

/**
 * 预览素材由父组件给（库里第一份资料 / 向导里选的文件）。
 *
 * <p>拿到素材就自动试切一次 —— 参数面板和预览并排放在这儿，就是要「边调边看」，
 * 还得先点一下按钮才出结果，就少了那口气。immediate 顺便覆盖首次挂载。
 */
watch(() => props.previewText, async (v) => {
  previewText.value = v || ''
  previewData.value = null
  if (previewText.value.trim()) await runPreview()
}, { immediate: true })

/**
 * 分段相关的默认值（与后端 defaultConfig 保持一致）。
 * 「恢复默认」用它，也当作推荐起点：不确定怎么填时就用这套。
 */
const DEFAULTS = {
  chunkSize: 2600, chunkOverlap: 130, separator: '\\n####', minChunkChars: 50,
  chunkMode: 'general', parentMode: 'paragraph',
  parentSize: 2600, parentOverlap: 0, parentSeparator: '\\n\\n',
  childSize: 512, childOverlap: 50, childSeparator: '\\n'
}

/** 只恢复分段参数：索引方式与{{ $t('kbs.retrievalSettings') }}是用户有意的选择，不该被一键抹掉 */
const restoreDefaults = () => {
  Object.assign(cfg, DEFAULTS)
  emit('dirty')
  ElMessage.success(t('kbs.defaultsRestored'))
}

// ⚠️ 必须是 computed：静态数组只在模块加载时求值一次，之后切语言不会跟着变
const parentOptions = computed(() => [
  { value: 'paragraph', label: t('kbs.parentParagraph'), desc: t('kbs.parentParagraphDesc') },
  { value: 'full', label: t('kbs.parentFull'), desc: t('kbs.parentFullDesc') }
])

/**
 * 高质量模式下的检索方式。
 *
 * <p>三张卡都成立：高质量说的是「资料已经向量化过」，检索用不用向量是另一件事 ——
 * 选「全文检索」就是建了向量但检索仍走关键词（以后想切向量不用重新导入）。
 */
const retrieveOptions = computed(() => [
  {
    value: 'keyword', label: t('kbs.rmKeyword'), icon: markRaw(Document),
    desc: t('kbs.rmKeywordDesc'),
    tip: t('kbs.rmKeywordTip')
  },
  {
    value: 'vector', label: t('kbs.rmVector'), icon: markRaw(Grid),
    desc: t('kbs.rmVectorDesc'),
    tip: t('kbs.rmVectorTip')
  },
  {
    value: 'hybrid', label: t('kbs.rmHybrid'), tag: t('kbs.recommended'), icon: markRaw(Connection),
    desc: t('kbs.rmHybridDesc'),
    tip: t('kbs.rmHybridTip')
  }
])

/** 索引方式的两层：「高质量」= 导入时建向量；「经济」= 只建关键词，不花调用成本 */
const lastHigh = ref('hybrid')
const highQuality = computed(() => cfg.buildVector === true)
/** 关键词权重（%）：混合检索里 hybridWeight 存的就是它，可拖可填 */
const kwPct = computed({
  get: () => Math.round((cfg.hybridWeight ?? 0.5) * 100),
  set: (v) => { cfg.hybridWeight = Math.min(1, Math.max(0, (v || 0) / 100)); emit('dirty') }
})
/** 语义（向量）权重 = 100 - 关键词权重：两边恒为一，改哪边都一样 */
const vectorPct = computed({
  get: () => 100 - kwPct.value,
  set: (v) => { kwPct.value = 100 - (v || 0) }
})
/** 配置里标了「支持向量化」的模型（含按名字自动识别的） */
const embedCapable = computed(() => (props.models || []).filter(m => m.supportEmbedding))
/** 一个能用的都没有时，把话写在占位符里 —— 比在下面挂一行说明安静得多 */
const embedPlaceholder = computed(() => embedCapable.value.length
  ? t('kbs.embedPlaceholderFirst')
  : t('kbs.embedPlaceholderNone'))

/**
 * 把指向「不支持向量化」模型的选择清掉。
 *
 * <p>它可能是早先存进配置的，也可能是探测后才判定不支持的。留着它，导入时会白报一次错，
 * 用户还以为是自己的问题 —— 不如直接取消选择，让占位符把原因说清楚。
 */
watch(() => props.models, (list) => {
  const id = cfg.embedModelId
  if (!id) return
  const m = (list || []).find(x => x.id === id)
  if (m && !m.supportEmbedding) cfg.embedModelId = ''
}, { immediate: true })

/**
 * 切换「高质量 / 经济」。
 *
 * <p>切到经济时记住原来选的是哪种检索方式，切回来原样恢复 —— 否则用户只是想省一次
 * 调用，回来发现选择被重置，还得重新挑一遍。
 */
const pickQuality = (q) => {
  if (q === 'economy') {
    if (cfg.indexMode !== 'keyword') lastHigh.value = cfg.indexMode
    cfg.buildVector = false
    cfg.indexMode = 'keyword'
  } else {
    cfg.buildVector = true
    cfg.indexMode = lastHigh.value || 'hybrid'
  }
  emit('dirty')
}

const pickValue = (field, v) => {
  cfg[field] = v
  emit('dirty')
}

const runPreview = async () => {
  if (!previewText.value.trim()) { ElMessage.warning(t('kbs.previewTextMissing')); return }
  previewing.value = true
  try {
    const res = await kbPreview({
      text: previewText.value,
      chunkSize: cfg.chunkSize,
      chunkOverlap: cfg.chunkOverlap,
      separator: cfg.separator,
      minChunkChars: cfg.minChunkChars,
      chunkMode: cfg.chunkMode,
      parentMode: cfg.parentMode,
      parentSize: cfg.parentSize,
      parentOverlap: cfg.parentOverlap,
      parentSeparator: cfg.parentSeparator,
      childSize: cfg.childSize,
      childOverlap: cfg.childOverlap,
      childSeparator: cfg.childSeparator,
      cleanWhitespace: cfg.cleanWhitespace,
      stripLinks: cfg.stripLinks
    })
    if (res?.success === false) throw new Error(res.message || t('kbs.previewFailed'))
    previewData.value = res
    emit('previewed', res)
  } catch (e) { ElMessage.error(e?.message || t('kbs.previewFailed')) }
  previewing.value = false
}

defineExpose({ runPreview })
</script>

<style scoped>
.kbset { min-width: 0; height: 100%; }

/* 预览素材选择：跟在标题右边，不额外占一整行高度 */
.preview-src {
  display: flex; align-items: center; gap: 8px;
  margin: 8px 0 10px;
}
.preview-src-label { font-size: 11.5px; color: var(--dc-text-dim); flex-shrink: 0; }

/* 左配置 / 右预览：两栏等宽等高、同样式面板，右栏默认铺满 */
.kbset-grid {
  display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: 26px; align-items: stretch; height: 100%;
}
.kbset-col {
  display: flex; flex-direction: column; gap: 18px; min-width: 0;
  border: 1px solid var(--dc-border); border-radius: 10px;
  background: var(--dc-bg-soft); padding: 16px 18px;
  /* 两栏各自滚动：滚轮停在哪栏就滚哪栏 */
  height: 100%; max-height: calc(100vh - 220px); min-height: 300px; overflow: auto;
}
/* 右栏：标题固定，结果区吃满剩余高度（输入框已去掉，素材自动带出） */
.kbset-preview { overflow: hidden; display: flex; flex-direction: column; }

.sec { border-top: 1px dashed var(--dc-border); padding-top: 12px; }
.kbset-col > .sec:first-child { border-top: none; padding-top: 0; }
/* 分组标题带主色竖条：几块配置扫一眼就能分开 */
.sec-title {
  display: flex; align-items: center; justify-content: space-between; gap: 8px;
  font-size: 13px; font-weight: 600; color: var(--dc-text-mid); margin-bottom: 12px;
  padding-left: 8px; border-left: 2px solid var(--dc-primary);
}
.field { display: flex; flex-direction: column; gap: 6px; margin-bottom: 10px; }
.field > label { display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--dc-text); }
/* 下拉框与文本框同一形制，别一个描边一个填充 */
.field :deep(.el-select__wrapper) {
  border-radius: 8px; background: var(--dc-bg-soft); box-shadow: none;
}
.field :deep(.el-select__wrapper.is-focused) { box-shadow: 0 0 0 1px var(--dc-primary) inset; }
.hint { font-size: 11.5px; color: var(--dc-text-dim); line-height: 1.7; }
.hint code {
  font-family: "SF Mono", ui-monospace, monospace; font-size: 12px;
  background: var(--dc-bg-hover); border-radius: 3px; padding: 0 4px;
}
/* 说明文字删掉后，各项之间靠间距分隔 */
.check-list { display: flex; flex-direction: column; gap: 10px; }

/* 通用/父子、段落/全文：同一套可选卡片 */
.mode-opts { display: flex; flex-direction: column; gap: 10px; margin-bottom: 12px; }
.mode-opts-row { flex-direction: row; }
.mode-opts-row .mode-opt { flex: 1 1 0; }
.mode-opt {
  border: 1px solid var(--dc-border); border-radius: 10px; background: var(--dc-bg-card);
  padding: 10px 12px; cursor: pointer; transition: border-color .15s ease, background .15s ease;
}
.mode-opt:hover { border-color: var(--dc-border-strong); }
.mode-opt.active { border-color: var(--dc-primary); background: var(--dc-primary-wash); }
.mode-opt-head { display: flex; align-items: center; gap: 8px; }
.mode-opt-name { font-size: 14px; font-weight: 600; color: var(--dc-text-strong); }
.mode-opt-tag {
  font-size: 10.5px; color: var(--dc-text-dim); background: var(--dc-bg-hover);
  border-radius: 9px; padding: 1px 7px;
}
.mode-opt-check { margin-left: auto; color: var(--dc-primary); }
.mode-opt-desc { font-size: 11.5px; color: var(--dc-text-dim); line-height: 1.7; margin-top: 4px; }
/* 展开在卡片里的参数区：与卡片同属一个语义块，用虚线顶边分出层级、并取消卡片的指针样式 */
.mode-opt-body {
  margin-top: 10px; padding: 12px 12px 4px;
  border-top: 1px dashed var(--dc-border); cursor: default;
}
.mode-opt.active .mode-opt-body { border-top-color: var(--dc-border-strong); }
/* 参数区两列：短输入（分隔符、数字）不该被拉成一整行 */
.pgrid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px 18px; }
.pitem { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
/* label 行固定高度：右侧带开关的字段（{{ $t('kbs.scoreThreshold') }}）不会把下面的数字框顶下去，
   同一行的 Top K 也就始终和它对齐 */
.pitem > label {
  display: flex; align-items: center; gap: 6px; height: 22px;
  font-size: 13px; color: var(--dc-text-mid);
}
.pitem-switch { margin-left: auto; }
/* 开关关掉时（这一列没有数字框）也占住高度，两列保持同一水平线 */
.pitem > .num-wrap { min-height: 32px; }
.pitem-ctl { display: flex; align-items: center; gap: 6px; }
.pitem > .el-input { width: 100%; }
/* 输入框走 Dify 那套：填充式（浅底、无描边）+ 大圆角，聚焦时才浮出描边 */
.pitem :deep(.el-input__wrapper) {
  border-radius: 8px; background: var(--dc-bg-soft); box-shadow: none;
}
.pitem :deep(.el-input__wrapper.is-focus) { box-shadow: 0 0 0 1px var(--dc-primary) inset; }
.pitem :deep(.el-input__inner) { font-variant-numeric: tabular-nums; }
/* 箭头列单独一块：用分隔线断开，不再和数字挤在一起 */
.pitem :deep(.el-input-number.is-controls-right .el-input-number__increase),
.pitem :deep(.el-input-number.is-controls-right .el-input-number__decrease) {
  border-left: 1px solid var(--dc-border);
}
.pitem :deep(.el-input-number.is-controls-right .el-input-number__increase) { border-radius: 0 8px 0 0; }
.pitem :deep(.el-input-number.is-controls-right .el-input-number__decrease) { border-radius: 0 0 8px 0; }
/* 多行文本框同款形制：填充式、无描边、聚焦才浮出边 */
:deep(.el-textarea__inner) {
  border-radius: 8px; background: var(--dc-bg-soft); box-shadow: none;
}
:deep(.el-textarea__inner:focus) { box-shadow: 0 0 0 1px var(--dc-primary) inset; }

/* 数字框：单位收进框内 —— 数字靠左、单位靠右、箭头上下一列（Dify 的 stepper 形制） */
.num-wrap { position: relative; width: 100%; }
.num-wrap :deep(.el-input-number) { width: 100%; }
.num-wrap :deep(.el-input-number .el-input__inner) { text-align: left; padding-left: 10px; }
.num-unit {
  position: absolute; top: 50%; right: 40px; transform: translateY(-50%);
  font-size: 13px; color: var(--dc-text-dim); pointer-events: none; white-space: nowrap;
}
/* ---- 索引方式：并排的「高质量 / 经济」卡片 ---- */
.q-opts { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; margin-bottom: 10px; }
.q-opt {
  display: flex; gap: 10px; padding: 11px 12px; cursor: pointer;
  border: 1px solid var(--dc-border); border-radius: 10px; background: var(--dc-bg-card);
  transition: border-color .15s ease, background .15s ease;
}
.q-opt:hover { border-color: var(--dc-border-strong); }
.q-opt.active { border-color: var(--dc-primary); background: var(--dc-primary-wash); }
.q-opt-icon {
  flex: 0 0 34px; height: 34px; border-radius: 8px; font-size: 18px;
  display: flex; align-items: center; justify-content: center;
}
.q-opt-icon.high { background: rgba(245, 158, 11, .14); color: #f59e0b; }
.q-opt-icon.econ { background: rgba(59, 130, 246, .14); color: #3b82f6; }
.q-opt-body { min-width: 0; }
.q-opt-head { display: flex; align-items: center; gap: 6px; }
.q-opt-name { font-size: 14px; font-weight: 600; color: var(--dc-text-strong); }
.q-opt-tag {
  font-size: 10.5px; color: var(--dc-text-dim); background: var(--dc-bg-hover);
  border-radius: 9px; padding: 1px 7px;
}
.q-opt-desc { margin-top: 4px; font-size: 11.5px; color: var(--dc-text-dim); line-height: 1.7; }

/* 分组标题里的「文字 + 问号」：两者作为标题的左半部分整体排布 */
.sec-name { display: inline-flex; align-items: center; gap: 6px; }
/* 下拉项：能选的正常显示，不能选的标一句原因 */
.opt-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.opt-no { font-size: 12px; color: var(--dc-text-weak); }

/* ---- {{ $t('kbs.retrievalSettings') }}：卡片选中即在卡内展开参数 ---- */
.r-opt {
  display: flex; gap: 10px; padding: 10px 12px; margin-bottom: 10px; cursor: pointer;
  border: 1px solid var(--dc-border); border-radius: 10px; background: var(--dc-bg-card);
  transition: border-color .15s ease, background .15s ease;
}
.r-opt:last-of-type { margin-bottom: 0; }
.r-opt:hover { border-color: var(--dc-border-strong); }
.r-opt.active { border-color: var(--dc-primary); background: var(--dc-primary-wash); }
.r-opt.is-static { cursor: default; }
.r-opt-icon {
  flex: 0 0 28px; height: 28px; border-radius: 8px; font-size: 15px;
  display: flex; align-items: center; justify-content: center;
  background: var(--dc-bg-hover); color: var(--dc-text-mid);
}
.r-opt.active .r-opt-icon { background: var(--dc-primary-wash); color: var(--dc-primary); }
.r-opt-body { min-width: 0; flex: 1; }
.r-opt-head { display: flex; align-items: center; gap: 6px; }
.r-opt-name { font-size: 14px; font-weight: 600; color: var(--dc-text-strong); }
/* 标签与分段模式、索引方式用的是同一套（灰底），只有文字不同 */
.r-opt-tag {
  font-size: 10.5px; color: var(--dc-text-dim); background: var(--dc-bg-hover);
  border-radius: 9px; padding: 1px 7px;
}
.r-opt-desc { margin-top: 4px; font-size: 11.5px; color: var(--dc-text-dim); line-height: 1.7; }
.r-opt-in {
  margin-top: 10px; padding: 12px 12px 4px;
  border-top: 1px dashed var(--dc-border); cursor: default;
}
.r-opt.active .r-opt-in { border-top-color: var(--dc-border-strong); }
/* 权重设置：滑轨左段（语义）右段（关键词）两段着色，两端各配一个可输入的数字框 */
.weight-row { margin-bottom: 12px; }
.weight-title { font-size: 13px; color: var(--dc-text-mid); margin-bottom: 4px; }
.weight-slider :deep(.el-slider__runway) {
  background: linear-gradient(to right,
    var(--dc-primary) 0 var(--vw, 50%),
    var(--dc-border) var(--vw, 50%) 100%);
}
/* 已选进度条不另着色，否则会盖掉上面的两段分界 */
.weight-slider :deep(.el-slider__bar) { background: transparent; }
.weight-sides { display: flex; align-items: center; gap: 14px; }
.weight-sides > label { display: inline-flex; align-items: center; gap: 6px; }
.weight-side-label { font-size: 13px; color: var(--dc-text-dim); }
.weight-num { width: 104px; }
/* 滑块与数字框成组：上下贴紧一点 */
.r-opt-in :deep(.el-slider) { margin: 2px 0 0; }
.r-opt-in :deep(.el-slider__runway) { margin: 10px 4px; }
/* 字段名旁的问号：不抢视线，但点得到 */
.q { font-size: 13px; color: var(--dc-text-weak); cursor: help; flex-shrink: 0; }
.q:hover { color: var(--dc-text-mid); }

/* 右栏预览 */
.kbset-body {
  flex: 1; min-height: 0; overflow: auto;
  margin-right: -18px; padding-right: 18px; padding-bottom: 4px;
}
.preview-stat { font-size: 11.5px; color: var(--dc-text-dim); margin: 8px 0; }
.preview-stat b { color: var(--dc-primary); }


/* 空态占满整栏：这块就是预览区，不该是贴在顶部的一小条 */
.preview-empty {
  height: 100%; min-height: 260px; display: flex; align-items: center; justify-content: center;
  text-align: center; font-size: 13px; color: var(--dc-text-dim); line-height: 1.9;
  border: 1px dashed var(--dc-border); border-radius: 10px; background: var(--dc-bg-card);
}
.preview-empty > div { max-width: 260px; }
.chunk {
  border: 1px solid var(--dc-border-soft); border-radius: 8px;
  background: var(--dc-bg-card); padding: 8px 10px; margin-bottom: 6px;
}
.chunk-head { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; flex-wrap: wrap; }
.chunk-no {
  font-size: 12px; color: var(--dc-primary); background: var(--dc-primary-wash);
  border-radius: 4px; padding: 1px 6px; font-variant-numeric: tabular-nums;
}
.chunk-size { font-size: 12px; color: var(--dc-text-dim); }
.chunk-parent { font-size: 12px; color: var(--dc-text-dim); margin-left: auto; }
.chunk-text {
  font-size: 13px; color: var(--dc-text-mid); line-height: 1.75;
  white-space: pre-wrap; word-break: break-word; max-height: 150px; overflow: auto;
}

/* 窄屏（或设置面板里）自动堆叠：左右挤在一起反而更难用，改回跟随页面滚动 */
@media (max-width: 1080px) {
  .kbset-grid { grid-template-columns: minmax(0, 1fr); }
  .kbset-col { height: auto; max-height: none; overflow: visible; }
  .kbset-preview { overflow: visible; }
  .kbset-body { overflow: visible; }
}
</style>
