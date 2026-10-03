<template>
  <div class="kbs">
    <!-- 左栏：库列表 -->
    <aside class="kbs-side">
      <div class="kbs-side-head">
        <span class="kbs-side-title">
          {{ $t('kbx.kbTitle') }}
          <span v-if="items.length" class="kbs-side-count">{{ items.length }}</span>
        </span>
        <el-button size="small" text :icon="Plus" :title="$t('kbx.newKb')" @click="startWizard" />
      </div>

      <el-input v-model="filter" size="small" clearable :placeholder="$t('ks.searchKb')"
                :prefix-icon="Search" class="kbs-side-search" />

      <div class="kbs-side-list">
        <div v-for="kb in filtered" :key="kb.id" class="kbs-item"
             :class="{ active: kb.id === activeId && !wizard }" @click="selectKb(kb)">
          <el-icon class="kbs-item-icon"><Collection /></el-icon>
          <div class="kbs-item-body">
            <div class="kbs-item-name">{{ kb.name }}</div>
            <div class="kbs-item-meta">{{ $t('ks.kbMeta', { docs: kb.docCount, chunks: kb.chunkCount }) }}</div>
          </div>
        </div>
        <div v-if="!filtered.length" class="kbs-side-empty">{{ sideEmptyText }}</div>
      </div>

      <div class="kbs-side-foot">
        <el-tooltip placement="top" :content="$t('ks.injectTip')">
          <span class="kbs-foot-label"><el-icon><MagicStick /></el-icon>{{ $t('ks.inject') }}</span>
        </el-tooltip>
        <el-switch v-model="cfg.enabled" size="small" @change="saveCfg" />
      </div>
    </aside>

    <!-- 右栏 -->
    <section class="kbs-main" ref="mainRef">
      <!-- ==================== 新建向导（三步，一气呵成）==================== -->
      <div v-if="wizard" class="kbs-wizard">
        <el-steps :active="wizardStep - 1" finish-status="success" align-center class="kbs-steps">
          <el-step :title="$t('ks.step1')" :description="$t('ks.step1Desc')" />
          <el-step :title="$t('ks.step2')" :description="$t('ks.step2Desc')" />
          <el-step :title="$t('ks.step3')" :description="$t('ks.step3Desc')" />
        </el-steps>

        <!-- 步骤 1：只负责选文件（名称/备注到最后一步弹窗再填） -->
        <div v-if="wizardStep === 1" class="kbs-step">
          <div class="kbs-drop" :class="{ over: dragOver }" @click="pickFile"
               @dragover.prevent="dragOver = true"
               @dragleave.prevent="dragOver = false"
               @drop.prevent="onDrop">
            <el-icon class="kbs-drop-icon"><UploadFilled /></el-icon>
            <div class="kbs-drop-title">{{ reading ? $t('kbx.reading') : $t('ks.dropHere') }}</div>
            <div class="kbs-drop-desc">{{ $t('ks.dropDesc') }}</div>
          </div>

          <div v-if="!pending.length" class="kbs-empty">{{ $t('ks.noFilesYet') }}</div>
          <div v-for="(p, i) in pending" :key="i" class="kbs-doc">
            <div class="kbs-doc-row">
              <el-icon class="kbs-doc-icon"><Document /></el-icon>
              <div class="kbs-doc-main">
                <el-input v-model="p.title" size="small" :placeholder="$t('s2k.titleLabel')" class="kbs-title-input" />
                <div class="kbs-doc-meta">{{ readable(p.text.length) }}<template v-if="p.source"> · {{ p.source }}</template></div>
              </div>
              <el-button size="small" type="danger" plain :icon="Delete" :title="$t('kbx.remove')" @click="pending.splice(i, 1)" />
            </div>
          </div>

          <div class="kbs-wizard-foot">
            <div class="kbs-foot-inner">
              <el-button class="kbs-foot-cancel" @click="cancelWizard">{{ $t('common.cancel') }}</el-button>
              <el-button type="primary" @click="goStep(2)">{{ $t('ks.next') }}</el-button>
            </div>
          </div>
        </div>

        <!-- 步骤 2：分段与索引 -->
        <div v-else-if="wizardStep === 2" class="kbs-step">
          <KbSettingsForm :config="cfg" :models="models" :preview-text="pending[0]?.text || ''"
                          :docs="pendingPreviewSources"
                          @dirty="dirty = true" @previewed="onPreviewed" ref="settingsRef" />
          <div class="kbs-wizard-foot">
            <div class="kbs-foot-inner">
              <el-button class="kbs-foot-cancel" @click="cancelWizard">{{ $t('common.cancel') }}</el-button>
              <el-button @click="goStep(1)">{{ $t('ks.prev') }}</el-button>
              <el-button type="primary" @click="goStep(3)">{{ $t('ks.next') }}</el-button>
            </div>
          </div>
        </div>

        <!-- 步骤 3：确认 -->
        <div v-else class="kbs-step">
          <div class="kbs-card">
            <div class="kbs-card-title">{{ $t('ks.aboutToCreate') }}</div>
            <div class="kbs-sum">
              <div class="kbs-sum-item"><span>{{ $t('ks.statDocs') }}</span><b>{{ $t('ks.docsAndChars', { n: pending.length, chars: readable(pendingChars) }) }}</b></div>
              <div class="kbs-sum-item">
                <span>{{ $t('ks.estChunks') }}</span>
                <b>{{ preview?.count != null ? $t('ks.estChunksValue', { n: preview.count }) : $t('ks.autoChunk') }}</b>
              </div>
              <div class="kbs-sum-item">
                <span>{{ $t('ks.chunkParams') }}</span>
                <b>{{ chunkSummary }}</b>
              </div>
              <div class="kbs-sum-item">
                <span>{{ $t('kbx.planIndex') }}</span>
                <b>{{ qualityLabel }} · {{ indexLabel(cfg.indexMode) }}{{ cfg.buildVector ? (cfg.embedModelId ? $t('ks.embedFixed') : $t('ks.embedAuto')) : $t('ks.localZero') }}</b>
              </div>
              <div v-if="cfg.buildVector" class="kbs-hint">
                {{ $t('ks.vectorCostTip') }}
              </div>
            </div>
          </div>
          <div class="kbs-wizard-foot">
            <div class="kbs-foot-inner">
              <el-button class="kbs-foot-cancel" @click="cancelWizard">{{ $t('common.cancel') }}</el-button>
              <el-button @click="goStep(2)">{{ $t('ks.prev') }}</el-button>
              <el-button type="primary" @click="openNameDialog">{{ $t('ks.create') }}</el-button>
            </div>
          </div>
        </div>
      </div>

      <!-- ==================== 库详情 ==================== -->
      <template v-else-if="active">
        <header class="kbs-head">
          <div class="kbs-head-icon"><el-icon><Collection /></el-icon></div>
          <div class="kbs-head-main">
            <div class="kbs-head-name">
              <span>{{ active.name }}</span>
              <el-button size="small" text :icon="EditPen" :title="$t('ks.editTitleBtn')" @click="openEditDialog" />
              <!-- 库级操作：只留图标，鼠标悬停给说明；竖线把「删除」单独隔开 -->
              <span class="kbs-head-ops">
                <el-button size="small" text :icon="Search" :title="$t('ks.recallTitle')"
                           @click="recallOpen = true" />
                <el-button size="small" text :icon="Setting" :title="$t('ks.settingsTitle')"
                           @click="openSettings" />
                <el-button size="small" text :icon="Refresh" :title="$t('ks.reindex')"
                           :disabled="!cfg.buildVector" @click="doReindex" />
                <span class="kbs-head-ops-sep"></span>
                <el-button size="small" text :icon="Delete" :title="$t('ks.deleteKb')" class="kbs-head-del"
                           @click="removeKb" />
              </span>
            </div>
            <div class="kbs-head-meta">
              {{ $t('ks.kbMetaFull', { docs: active.docCount, chunks: active.chunkCount, chars: readable(active.charCount) }) }}
              {{ $t('ks.indexLine', { quality: qualityLabel, index: indexLabel(indexMode) }) }}
            </div>
            <!-- 备注是自由文本，单独一行且限两行：塞进统计行会把上面的信息挤没 -->
            <div v-if="active.description" class="kbs-head-desc" :title="active.description">
              {{ active.description }}
            </div>
          </div>
        </header>

        <!-- 主体：导入区 + 资料列表 -->
        <div class="kbs-drop kbs-drop-sm" :class="{ over: dragOver, busy: importing }" @click="pickFile"
             @dragover.prevent="dragOver = true"
             @dragleave.prevent="dragOver = false"
             @drop.prevent="onDrop">
          <el-icon class="kbs-drop-icon"><UploadFilled /></el-icon>
          <div class="kbs-drop-title">{{ importing ? $t('ks.importing') : $t('ks.dropHereMulti') }}</div>
        </div>
        <div v-if="indexMode !== 'keyword' && !vectorsReady" class="kbs-warn">
          <span>{{ $t('ks.needVectorHint', { index: indexLabel(indexMode) }) }}</span>
          <el-button size="small" type="primary" plain :loading="reindexing" @click="doReindex">{{ $t('ks.reindexNow') }}</el-button>
        </div>

        <div class="kbs-list-head">
          <span class="kbs-list-title">{{ $t('ks.allDocs') }}</span>
          <span class="kbs-count">{{ docs.length }}</span>
          <span class="kbs-list-spacer"></span>
          <!-- 资料多时才给搜索：只有一两份文件时，多一个输入框纯属噪音 -->
          <el-input v-if="docs.length > 5" v-model="docFilter" size="small" clearable
                    :placeholder="$t('ks.searchDocs')" :prefix-icon="Search" class="kbs-doc-search" />
        </div>
        <div v-if="!docs.length" class="kbs-empty">{{ $t('ks.noDocs') }}</div>
        <div v-else-if="!filteredDocs.length" class="kbs-empty">{{ $t('ks.noDocMatch', { kw: docFilter }) }}</div>

        <div v-for="d in filteredDocs" :key="d.id" class="kbs-doc">
          <!-- 点整行展开分块：行里只留一个删除图标，不再挂下拉菜单 -->
          <div class="kbs-doc-row" @click="toggleChunks(d)"
               :title="chunkDocId === d.id ? $t('ks.collapseChunks') : $t('ks.viewChunks')">
            <el-icon class="kbs-doc-icon"><Document /></el-icon>
            <div class="kbs-doc-main">
              <div class="kbs-doc-title">{{ d.title }}</div>
              <div class="kbs-doc-meta">
                {{ $t('ks.docMeta', { chunks: d.chunkCount, chars: d.charText }) }}<template v-if="d.source && d.source !== d.title">{{ $t('ks.sourceLine', { name: d.source }) }}</template>
                <span v-if="cfg.buildVector" class="kbs-tag" :class="{ ok: d.hasVector }">
                  {{ d.hasVector ? $t('ks.hasVector') : $t('ks.noVector') }}
                </span>
              </div>
            </div>
            <el-button size="small" text :icon="Delete" :title="$t('ks.deleteDoc')" class="kbs-doc-del"
                       @click.stop="removeDoc(d)" />
          </div>

          <div v-if="chunkDocId === d.id" class="kbs-chunks">
            <div v-if="chunksLoading" class="kbs-empty">{{ $t('ks.loadingChunks') }}</div>
            <template v-else-if="chunkData">
              <div class="kbs-chunks-head">
                {{ $t('kbs.previewStats', { chunks: chunkData.chunkCount, avg: chunkData.avgChars }) }}
                <span class="kbs-hint">{{ $t('ks.overlapHint') }}</span>
              </div>
              <div v-for="c in chunkData.chunks.slice(0, chunkLimit)" :key="c.index" class="kbs-chunk">
                <div class="kbs-chunk-head">
                  <span class="kbs-chunk-no">#{{ c.index }}</span>
                  <span class="kbs-chunk-size">{{ $t('kbx.chars', { n: c.charCount }) }}</span>
                </div>
                <div class="kbs-chunk-text">{{ c.text }}</div>
              </div>
              <div v-if="chunkData.chunks.length > chunkLimit" class="kbs-more">
                <el-button size="small" plain @click="chunkLimit += 20">
                  {{ $t('ks.moreChunks', { n: chunkData.chunks.length - chunkLimit }) }}
                </el-button>
              </div>
            </template>
          </div>
        </div>
      </template>

      <!-- 空态 -->
      <div v-else class="kbs-blank">
        <el-icon class="kbs-blank-icon"><Reading /></el-icon>
        <div class="kbs-blank-title">{{ $t('ks.blankTitle') }}</div>
        <div class="kbs-blank-desc">
          {{ $t('ks.blankDescA') }}<b>{{ $t('ks.blankDescB') }}</b>{{ $t('ks.blankDescC') }}<br />
          {{ $t('ks.blankDescD') }}
        </div>
        <el-button type="primary" :icon="Plus" @click="startWizard">{{ $t('kbx.newKb') }}</el-button>
      </div>
    </section>

    <!-- 编辑名称 / 备注：备注用多行文本框（描述通常是一两句话，单行输入框看不全） -->
    <el-dialog v-model="editOpen" :title="$t('ks.editTitle')" width="480px" :append-to-body="true"
               :close-on-click-modal="false">
      <div class="kbs-field">
        <label>{{ $t('ks.name') }}</label>
        <el-input v-model="editForm.name" maxlength="60" show-word-limit :placeholder="$t('kbx.kbName')" />
      </div>
      <div class="kbs-field">
        <label>{{ $t('ks.note') }}</label>
        <el-input v-model="editForm.description" type="textarea" :rows="4" resize="none"
                  :placeholder="$t('s2k.descPlaceholder')" />
      </div>
      <template #footer>
        <el-button @click="editOpen = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="busy" @click="submitEdit">{{ $t('common.save') }}</el-button>
      </template>
    </el-dialog>

    <!-- 召回测试：库级操作，弹窗呈现，看完关掉不影响列表 -->
    <el-dialog v-model="recallOpen" :title="$t('ks.recallTitle')" width="680px" top="6vh" :append-to-body="true">
      <div class="rc">
        <!-- 测哪个库、怎么测：一句话交代清楚，细节收进问号 -->
        <div class="rc-head">
          <span>{{ $t('ks.recallIn', { name: (active?.name || $t('ks.currentKb')) }) }}</span>
          <el-tooltip placement="top"
                      :content="$t('ks.recallTip')">
            <el-icon class="q"><QuestionFilled /></el-icon>
          </el-tooltip>
        </div>

        <div class="rc-box">
          <div class="rc-input">
            <el-input v-model="query" :placeholder="recallPlaceholder" clearable @keyup.enter="runSearch" />
            <el-button type="primary" :loading="searching" :disabled="!query.trim()"
                       @click="runSearch">{{ $t('ks.recall') }}</el-button>
            <el-tooltip placement="top"
                        :content="$t('ks.tryAnswerTip')">
              <el-button :loading="aiAnswering" :disabled="!query.trim()" @click="runAiAnswer">
                <el-icon><MagicStick /></el-icon>{{ $t('ks.tryAnswer') }}
              </el-button>
            </el-tooltip>
          </div>
          </div>

          <!-- 该问什么：库名和资料清单看不出这个库能干什么，直接给几个它答得出的问题 -->
      <div class="rc-outline">
        <div v-if="!outline && !outlining" class="rc-outline-cta">
          <a class="rc-outline-go" @click="runOutline">
            <el-icon><MagicStick /></el-icon>{{ $t('ks.outline') }}
          </a>
        </div>
        <div v-else-if="outlining" class="rc-outline-loading">
          <el-icon class="is-loading"><Loading /></el-icon>{{ $t('ks.outlining') }}
        </div>
        <template v-else>
          <div v-if="outline.summary" class="rc-outline-sum">{{ outline.summary }}</div>
          <div v-if="outline.questions && outline.questions.length" class="rc-outline-qs">
            <div v-for="(q, i) in outline.questions" :key="'q' + i" class="rc-q"
                 :title="$t('ks.recallDirect', { q })" @click="useOutlineQuestion(q)">
              <el-icon><Search /></el-icon><span>{{ q }}</span>
            </div>
          </div>
          <div v-if="outline.terms && outline.terms.length" class="rc-outline-terms">
            <span v-for="(t, i) in outline.terms" :key="'t' + i" class="rc-term"
                  @click="useOutlineQuestion(t)">{{ t }}</span>
          </div>
        </template>
      </div>

      <!-- AI 试答：片段沾边但答不到点上的情况很常见，光看命中判断不了「好不好用」 -->
          <div v-if="aiAnswering || aiAnswer" class="rc-answer">
          <div class="rc-answer-head">
            <el-icon><MagicStick /></el-icon>
            <span>{{ $t('ks.answerTitle') }}</span>
            <span class="rc-answer-scope">{{ $t('ks.answerScope', { name: (active?.name || $t('ks.currentKb')) }) }}</span>
          </div>
          <div v-if="aiAnswering" class="rc-answer-loading">
            <el-icon class="is-loading"><Loading /></el-icon>{{ $t('ks.answering') }}
          </div>
          <div v-else class="rc-answer-body">{{ aiAnswer }}</div>
          </div>

        <div v-if="searched" class="rc-stat">
          <template v-if="hits.length">{{ $t('ks.hitCount', { n: hits.length }) }}</template>
          <template v-else>{{ $t('ks.noHit') }}</template>
        </div>

        <div v-if="!searched && !searching" class="rc-blank">
          {{ $t('ks.recallEmptyHint') }}
        </div>

        <div v-if="searched && !hits.length" class="kbs-empty">
          {{ $t('ks.recallEmptyAdvice') }}
        </div>
      <div v-for="(h, i) in hits" :key="i" class="kbs-hit">
        <div class="kbs-hit-head">
          <span class="kbs-hit-rank">#{{ i + 1 }}</span>
          <span class="kbs-hit-bar"><i :style="{ width: hitPercent(h.score) }"></i></span>
          <span class="kbs-hit-score">{{ Number(h.score || 0).toFixed(3) }}</span>
          <span class="kbs-hit-src">{{ $t('ks.hitSrc', { doc: h.docTitle, n: h.index }) }}</span>
        </div>
        <div class="kbs-hit-text" :class="{ fold: !expanded[i] && h.text.length > 220 }">{{ h.text }}</div>
        <el-button v-if="h.text.length > 220" size="small" text type="primary"
                   class="kbs-hit-more" @click="expanded[i] = !expanded[i]">
          {{ expanded[i] ? $t('ks.collapse') : $t('ks.expandFull') }}
        </el-button>
      </div>
      </div>
      </el-dialog>

    <!-- 分段与索引设置：同样用弹窗，宽度留足给「左配置 / 右预览」 -->
    <el-dialog v-model="settingsOpen" :title="$t('ks.settingsTitle')" width="min(1120px, 94vw)" top="5vh"
               :append-to-body="true">
      <KbSettingsForm :config="cfg" :models="models" :preview-text="previewText"
                      :kb-id="activeId" :docs="docs"
                      @dirty="dirty = true" />
      <template #footer>
        <div class="kbs-drawer-foot">
          <el-button :disabled="!dirty" @click="resetSettings">{{ $t('ks.discardChanges') }}</el-button>
          <el-button v-if="cfg.buildVector" :loading="reindexing" @click="doReindex">{{ $t('ks.reindex') }}</el-button>
          <el-button type="primary" :loading="savingCfg" :disabled="!dirty" @click="saveSettings">{{ $t('ks.saveSettings') }}</el-button>
        </div>
      </template>
    </el-dialog>

    <!-- 最后一步才问名称：资料与参数都已定好，这一步只剩"叫它什么"；
         确认后按「创建 → 保存设置 → 导入」的顺序一次做完 -->
    <el-dialog v-model="nameDialog" :title="$t('ks.createTitle')" width="480px" append-to-body
               :close-on-click-modal="false">
      <!-- 与「编辑知识库」保持同一套字段结构：label 在上、输入框在下 -->
      <div class="kbs-field">
        <label>{{ $t('ks.name') }}</label>
        <el-input v-model="draft.name" maxlength="60" show-word-limit :placeholder="$t('kbx.kbName')"
                  @keyup.enter="confirmCreate" />
      </div>
      <div class="kbs-field">
        <label>{{ $t('ks.note') }}</label>
        <el-input v-model="draft.description" type="textarea" :rows="4" resize="none"
                  :placeholder="$t('s2k.descPlaceholder')" />
      </div>
      <template #footer>
        <el-button :disabled="creating" @click="nameDialog = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="creating" @click="confirmCreate">
          {{ creating ? $t('ks.creating') : $t('ks.createAndImport') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 全局唯一：放进 v-for 会被 Vue 收集成数组，click 就失效了 -->
    <input ref="fileInput" type="file" multiple
           accept=".txt,.md,.markdown,.csv,.tsv,.json,.sql,.log,.yml,.yaml,.xml,.html,.properties,.conf"
           style="display:none" @change="onFile" />
  </div>
</template>

<script setup>
import { ref, computed, watch, nextTick, onMounted } from 'vue'
import { t, locale } from '../../utils/i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus, Search, EditPen, Delete, UploadFilled, Document, MagicStick, Setting, Refresh, Collection, QuestionFilled, Loading , Reading } from '@element-plus/icons-vue'
import KbSettingsForm from './KbSettingsForm.vue'

const props = defineProps({
  /** 从别处跳过来时要选中的知识库（用完通知外层清空，免得下次进来又强制切过去） */
  focusKbId: { type: String, default: '' }
})
const emit = defineEmits(['focus-done'])
import {
  kbList, kbCreate, kbRename, kbDelete, kbDocs, kbDocChunks, kbImportDoc,
  kbDeleteDoc, kbSearch, kbSaveConfig, kbReindex, getAiConfig, probeEmbedModels,
  aiChat, aiKbOutline
} from '../../api'

const items = ref([])
const models = ref([])
/** 全局配置（分段 + 索引 + 检索）：后端只有一份，所有库共用 */
const cfg = ref({
  enabled: true, topK: 4, minScore: 0,
  chunkSize: 2600, chunkOverlap: 130, separator: '\\n####', minChunkChars: 50,
  indexMode: 'keyword', buildVector: false, embedModelId: '', hybridWeight: 0.5, ngram: true,
  // 分段模式与预处理（对应 Dify 的分组）
  chunkMode: 'general', parentMode: 'paragraph',
  parentSize: 2600, parentOverlap: 0, parentSeparator: '\\n\\n',
  childSize: 512, childOverlap: 50, childSeparator: '\\n',
  cleanWhitespace: true, stripLinks: false
})
const activeId = ref('')

// 外部要求聚焦某个库（例如从对话的引用点过来）：列表已加载就直接切过去
watch(() => props.focusKbId, (id) => {
  if (!id) return
  if (items.value.some(k => k.id === id)) {
    activeId.value = id
    emit('focus-done')
  }
})

/** 库级操作（召回测试 / 分段与索引设置）用抽屉承载，主区始终留给资料列表 */
const recallOpen = ref(false)
const settingsOpen = ref(false)
const filter = ref('')
const busy = ref(false)
const listLoading = ref(true)
const loadFailed = ref(false)
const dirty = ref(false)
const savingCfg = ref(false)
const reindexing = ref(false)

// 向导
const mainRef = ref(null)
const wizard = ref(false)
const wizardStep = ref(1)
const draft = ref({ name: '', description: '' })
const pending = ref([])
const preview = ref(null)
const creating = ref(false)
const settingsRef = ref(null)

// 详情
/** 编辑名称 / 备注（弹窗：备注是描述性文字，用多行文本框） */
const editOpen = ref(false)
const editForm = ref({ name: '', description: '' })
const docs = ref([])
/** 库内资料搜索：资料多于 5 份时才显示输入框，少的时候不添乱 */
const docFilter = ref('')
const importing = ref(false)
const dragOver = ref(false)
const reading = ref(false)
const fileInput = ref(null)

const previewText = ref('')
const indexMode = ref('keyword')
const vectorsReady = ref(true)

// 分块预览
const chunkDocId = ref('')
const chunkData = ref(null)
const chunksLoading = ref(false)
const chunkLimit = ref(10)

// 召回
const query = ref('')
const hits = ref([])
const searched = ref(false)
const searching = ref(false)
const expanded = ref({})
/** AI 试答：让模型只拿当前库的资料回答，用来判断这个库够不够用 */
const aiAnswer = ref('')
const aiAnswering = ref(false)
/** 库概要：装了什么、能问什么、有哪些关键词 —— 光看库名很难判断该拿它问什么 */
const outline = ref(null)
const outlining = ref(false)

const runOutline = async () => {
  if (!activeId.value || outlining.value) return
  outlining.value = true
  try {
    const res = await aiKbOutline({ kbId: activeId.value })
    if (res?.success === false) throw new Error(res.message || t('ks.summarizeFailed'))
    outline.value = res
  } catch (e) {
    ElMessage.error(e?.message || t('ks.summarizeFailed'))
  }
  outlining.value = false
}

/** 点推荐问题：填进召回框并直接召回，省掉手打一遍 */
const useOutlineQuestion = (q) => {
  query.value = q
  runSearch()
}

/**
 * 新建向导里的预览素材：就是第 1 步选的那几份文件。
 *
 * <p>它们自带正文，不用像库内资料那样再拉一次分块；给个合成 id 让下拉能定位即可。
 */
const pendingPreviewSources = computed(() =>
  (pending.value || []).map((d, i) => ({
    id: 'pd' + i,
    title: d.title || d.source || t('ks.untitledN', { n: i + 1 }),
    text: d.text || ''
  }))
)

const active = computed(() => items.value.find(k => k.id === activeId.value) || null)
const filtered = computed(() => {
  const q = filter.value.trim().toLowerCase()
  if (!q) return items.value
  return items.value.filter(k =>
    (k.name || '').toLowerCase().includes(q) || (k.description || '').toLowerCase().includes(q))
})
const filteredDocs = computed(() => {
  const q = docFilter.value.trim().toLowerCase()
  if (!q) return docs.value
  return docs.value.filter(d =>
    (d.title || '').toLowerCase().includes(q) || (d.source || '').toLowerCase().includes(q))
})
/** 左栏空态：加载中 / 加载失败 / 确实还没有，是三件不同的事，不该都写成「还没有知识库」 */
const sideEmptyText = computed(() => {
  if (listLoading.value) return t('ks.kbTipLoading')
  if (loadFailed.value) return t('ks.kbTipFailed')
  return items.value.length ? t('ks.kbTipNoMatch') : t('ks.kbTipNone')
})
const recallPlaceholder = computed(() =>
  active.value ? t('ks.askExample', { name: active.value.name }) : t('ks.askPlaceholder'))
const pendingChars = computed(() => pending.value.reduce((n, p) => n + (p.text?.length || 0), 0))
/** 步骤一只需要有资料：名称挪到最后一步的弹窗里，不再拦在这一步 */
const canStep1 = computed(() => pending.value.length > 0)
const step1Tip = computed(() => pending.value.length
  ? t('ks.docsReady', { n: pending.value.length, chars: readable(pendingChars.value) })
  : t('ks.needOneFile'))

/**
 * 切换步骤：先校验，再滚回顶部。
 * 不滚顶部的话，内容高度一变视口还停在下方，会让人以为"按钮点了没反应"。
 */
const goStep = async (n) => {
  if (n >= 2 && !canStep1.value) {
    ElMessage.warning(step1Tip.value || t('ks.needMore'))
    return
  }
  wizardStep.value = n
  await nextTick()
  if (mainRef.value) mainRef.value.scrollTop = 0
}

const readable = (n) => {
  const v = n || 0
  // 中文习惯「千字 / 万字」、英文习惯「K / M」：同一个数量级两种语言的写法不同
  if (locale.value === 'en-US') {
    return v < 1000000 ? t('ks.charsK', { v: (v / 1000).toFixed(1) }) : t('ks.charsM', { v: (v / 1000000).toFixed(1) })
  }
  return v < 10000 ? t('ks.charsK', { v: (v / 1000).toFixed(1) }) : t('ks.charsW', { v: (v / 10000).toFixed(1) })
}
/** 与设置面板同一套叫法：keyword 在界面上叫「全文检索」 */
const indexLabel = (m) => ({ vector: t('kbs.rmVector'), hybrid: t('kbs.rmHybrid') }[m] || t('kbs.rmKeyword'))
/** 索引方式的两层：高质量 = 导入时建向量；经济 = 只建关键词 */
const qualityLabel = computed(() => (cfg.value.buildVector ? t('kbs.highQuality') : t('kbs.economy')))
/** 向导摘要里用一句话说清当前分段方案（长度一律按字符数，与设置面板同口径） */
const chunkSummary = computed(() => {
  const c = cfg.value
  if (c.chunkMode === 'parentChild') {
    const parent = c.parentMode === 'full' ? t('ks.parentFull') : t('ks.parentSize', { n: c.parentSize })
    return t('ks.chunkSummary', { parent, n: c.childSize })
      + (c.childSeparator ? t('ks.childBySep', { sep: c.childSeparator }) : '')
  }
  return t('ks.genericSummary', { n: c.chunkSize, o: c.chunkOverlap })
    + (c.separator ? t('ks.bySep', { sep: c.separator }) : t('ks.byBlank'))
    + (c.minChunkChars ? t('ks.dropShort', { n: c.minChunkChars }) : '')
})
const hitPercent = (score) => {
  const max = Math.max(...hits.value.map(h => Number(h.score) || 0), 0.0001)
  return Math.max(6, Math.round((Number(score) / max) * 100)) + '%'
}
const onPreviewed = (data) => { preview.value = data }

// ---------- 加载 ----------

const loadList = async () => {
  try {
    const d = await kbList()
    items.value = (d && d.items) || []
    loadFailed.value = false
    if (d && d.config) applyConfig(d.config)
    if (activeId.value && !items.value.some(k => k.id === activeId.value)) activeId.value = ''
    if (!activeId.value && items.value.length) activeId.value = items.value[0].id
    if (activeId.value) await loadDocs()
  } catch (e) {
    // 后端未装配 / 请求失败：标记出来，界面不能停在「还没有知识库」这种误导文案上
    loadFailed.value = true
  } finally {
    listLoading.value = false
  }
}

/** 后端配置 → 本地表单（缺字段用默认值补齐，兼容老配置） */
const applyConfig = (c) => {
  cfg.value = {
    ...cfg.value, ...c,
    chunkSize: c.chunkSize || 2600,
    chunkOverlap: c.chunkOverlap == null ? 130 : c.chunkOverlap,
    separator: c.separator || '',
    minChunkChars: c.minChunkChars || 0,
    indexMode: c.indexMode || 'keyword',
    buildVector: c.buildVector === true,
    embedModelId: c.embedModelId || '',
    hybridWeight: c.hybridWeight == null ? 0.5 : c.hybridWeight,
    ngram: c.ngram !== false,
    chunkMode: c.chunkMode || 'general',
    parentMode: c.parentMode || 'paragraph',
    parentSize: c.parentSize || 2600,
    parentOverlap: c.parentOverlap || 0,
    parentSeparator: c.parentSeparator || '\\n\\n',
    childSize: c.childSize || 512,
    childOverlap: c.childOverlap == null ? 50 : c.childOverlap,
    childSeparator: c.childSeparator || '\\n',
    cleanWhitespace: c.cleanWhitespace !== false,
    stripLinks: c.stripLinks === true
  }
  indexMode.value = cfg.value.indexMode
}

/** AI 服务里已配置的模型（向量化要从里面选） */
const loadModels = async () => {
  try {
    const d = await getAiConfig()
    models.value = (d && d.models) || []
  } catch (e) { models.value = [] }
  // 「能不能做向量化」由后端自己判断（按名称识别，判不出的实调一次 /embeddings），
  // 不再要求用户去设置里打勾。先让下拉可用，探测结果回来再修正。
  try {
    const probe = await probeEmbedModels()
    const cap = (probe && probe.capability) || {}
    models.value = models.value.map(m =>
      cap[m.id] == null ? m : { ...m, supportEmbedding: cap[m.id] })
  } catch (e) { /* 探测不可用时，沿用后端按名称给出的判断 */ }
}

const selectKb = async (kb) => {
  if (wizard.value) wizard.value = false
  if (activeId.value === kb.id) return
  activeId.value = kb.id
  chunkDocId.value = ''
  chunkData.value = null
  docFilter.value = ''
  hits.value = []
  searched.value = false
  expanded.value = {}
  dirty.value = false
  await loadDocs()
}

const loadDocs = async () => {
  if (!activeId.value) return
  try {
    const d = await kbDocs({ kbId: activeId.value })
    docs.value = (d && d.docs) || []
    if (d && d.indexMode) indexMode.value = d.indexMode
    vectorsReady.value = d ? d.vectorsReady !== false : true
  } catch (e) { docs.value = [] }
}

// ---------- 向导 ----------

const startWizard = () => {
  wizard.value = true
  wizardStep.value = 1
  draft.value = { name: '', description: '' }
  pending.value = []
  preview.value = null
}

const closeWizard = () => {
  wizard.value = false
  pending.value = []
  preview.value = null
}

/** 退出向导：已经选好资料时先确认 —— 一次误点把清单清掉，代价太大 */
const cancelWizard = async () => {
  if (pending.value.length) {
    try {
      await ElMessageBox.confirm(t('ks.exitConfirm'), t('ks.exitTitle'),
        { type: 'warning', confirmButtonText: t('ks.exit'), cancelButtonText: t('ks.keepCreating') })
    } catch { return }
  }
  closeWizard()
}

/** 读文件进清单：向导里加进「待导入」，详情页里直接导入 */
const ingest = async (files) => {
  const list = Array.from(files || []).filter(f => f && f.size > 0)
  if (!list.length) return
  if (wizard.value) {
    reading.value = true
    for (const f of list) {
      try {
        const text = await f.text()
        pending.value.push({ title: f.name.replace(/\.[^.]+$/, ''), source: f.name, text })
      } catch (e) { ElMessage.error(t('ks.readFailed', { name: f.name, detail: (e?.message || e) })) }
    }
    reading.value = false
    if (list.length === 1) previewText.value = pending.value[0]?.text || ''
    return
  }
  if (!activeId.value) return
  importing.value = true
  let done = 0
  const failed = []
  for (const f of list) {
    try {
      const text = await f.text()
      const res = await kbImportDoc({
        kbId: activeId.value, title: f.name.replace(/\.[^.]+$/, ''), source: f.name, text
      })
      if (!res?.success) throw new Error(res?.message || t('kbx.importFailed'))
      done++
    } catch (e) { failed.push(t('ks.fileFailed', { name: f.name, detail: (e?.message || e) })) }
  }
  await Promise.all([loadList(), loadDocs()])
  importing.value = false
  if (done) ElMessage.success(t('ks.importedDocs', { n: done }))
  if (failed.length) ElMessage.warning(t('ks.notImported', { n: failed.length, list: failed.slice(0, 2).join(t('ks.semi')) }))
}

const onFile = async (ev) => {
  await ingest(ev?.target?.files)
  if (ev?.target) ev.target.value = ''
}
const onDrop = async (ev) => {
  dragOver.value = false
  if (importing.value) return
  await ingest(ev?.dataTransfer?.files)
}

/** 详情页用拖拽 / 选择文件导入；这里只保留向导里的「加入清单」 */
/** 打开选择器：正在导入时忽略点击，免得又叠一批进来 */
const pickFile = () => {
  if (importing.value || reading.value) return
  fileInput.value?.click?.()
}

const nameDialog = ref(false)

/** 最后一步：先在弹窗里确认名称，再「创建 → 保存设置 → 导入」 */
const openNameDialog = () => {
  if (!pending.value.length) { ElMessage.warning(t('ks.pickAtLeastOne')); return }
  draft.value = { name: '', description: '' }
  nameDialog.value = true
}

const confirmCreate = async () => {
  if (!draft.value.name.trim()) { ElMessage.warning(t('kbx.needName')); return }
  await finishWizard()
}

const finishWizard = async () => {
  creating.value = true
  try {
    const res = await kbCreate({ name: draft.value.name, description: draft.value.description })
    if (!res?.success) throw new Error(res?.message || t('kbx.createFailed'))
    const kbId = res.item?.id || res.kbId || res.info?.id
    if (!kbId) throw new Error(t('kbx.noKbId'))

    // 先把参数落盘，再导入 —— 否则导入用的是旧参数，切出来的块和预览对不上
    await saveCfgSilently()
    let done = 0
    const failed = []
    for (const p of pending.value) {
      try {
        const r = await kbImportDoc({ kbId, title: p.title, source: p.source, text: p.text })
        if (!r?.success) throw new Error(r?.message || t('kbx.importFailed'))
        done++
      } catch (e) { failed.push(t('ks.fileFailed', { name: p.title, detail: (e?.message || e) })) }
    }
    nameDialog.value = false
    closeWizard()
    await loadList()
    activeId.value = kbId
    await loadDocs()
    if (failed.length) {
      ElMessage.warning(t('ks.createdNotImported', { n: failed.length, list: failed.slice(0, 2).join(t('ks.semi')) }))
    } else {
      ElMessage.success(t('ks.createdImported', { n: done }))
    }
  } catch (e) { ElMessage.error(e?.message || t('kbx.createFailed')) }
  creating.value = false
}

// ---------- 库操作 ----------

const openEditDialog = () => {
  editForm.value = { name: active.value?.name || '', description: active.value?.description || '' }
  editOpen.value = true
}

const submitEdit = async () => {
  if (!editForm.value.name.trim()) { ElMessage.warning(t('ks.nameRequired')); return }
  busy.value = true
  try {
    const res = await kbRename({ kbId: activeId.value, ...editForm.value })
    if (!res?.success) throw new Error(res?.message || t('s2k.saveFailed'))
    editOpen.value = false
    await loadList()
    ElMessage.success(t('ks.saved'))
  } catch (e) { ElMessage.error(e?.message || t('s2k.saveFailed')) }
  busy.value = false
}

/**
 * 打开设置面板。
 *
 * <p>自动把当前库第一份资料的正文带进预览 —— 调分段参数时看到的就是自己的数据，
 * 不用再手动复制一段进来。素材上限 5 万字：既能反映真实切分，又不至于把长文档整个搬过来。
 */
const openSettings = async () => {
  dirty.value = false
  settingsOpen.value = true
  const first = docs.value[0]
  if (!first || !activeId.value) return
  try {
    const d = await kbDocChunks({ kbId: activeId.value, docId: first.id })
    // 优先用入库时存的原文：拿分块拼回去会因为相邻块重叠而失真，跟新建库时的预览对不上。
    // 老资料没有原文，只能回退到拼接。
    const text = d?.raw || (d?.chunks || []).map(c => c.text).join('\n\n')
    if (text.trim()) previewText.value = text.slice(0, 50000)
  } catch (e) { /* 取不到就算了：仍可在资料行点「查看分块」把素材带出来 */ }
}

const removeKb = async () => {
  const kb = active.value
  if (!kb) return
  try {
    await ElMessageBox.confirm(
      t('ks.deleteKbBody', { name: kb.name, docs: kb.docCount, chunks: kb.chunkCount }),
      t('ks.deleteKb'), { type: 'warning', confirmButtonText: t('common.delete'), cancelButtonText: t('common.cancel') })
  } catch { return }
  try {
    const res = await kbDelete({ kbId: kb.id })
    if (!res?.success) throw new Error(res?.message || t('ks.deleteFailed'))
    activeId.value = ''
    docs.value = []
    await loadList()
    ElMessage.success(t('mv.deleted'))
  } catch (e) { ElMessage.error(e?.message || t('ks.deleteFailed')) }
}

// ---------- 配置 ----------

const configPayload = () => ({
  enabled: cfg.value.enabled, topK: cfg.value.topK, minScore: cfg.value.minScore,
  chunkSize: cfg.value.chunkSize, chunkOverlap: cfg.value.chunkOverlap, separator: cfg.value.separator,
  minChunkChars: cfg.value.minChunkChars, indexMode: cfg.value.indexMode,
  buildVector: cfg.value.buildVector, embedModelId: cfg.value.embedModelId,
  hybridWeight: cfg.value.hybridWeight, ngram: cfg.value.ngram,
  chunkMode: cfg.value.chunkMode, parentMode: cfg.value.parentMode,
  parentSize: cfg.value.parentSize, parentOverlap: cfg.value.parentOverlap,
  parentSeparator: cfg.value.parentSeparator,
  childSize: cfg.value.childSize, childOverlap: cfg.value.childOverlap,
  childSeparator: cfg.value.childSeparator,
  cleanWhitespace: cfg.value.cleanWhitespace, stripLinks: cfg.value.stripLinks
})

const saveCfgSilently = async () => {
  const res = await kbSaveConfig(configPayload())
  if (res?.config) applyConfig(res.config)
  dirty.value = false
}

const saveCfg = async () => {
  try {
    await saveCfgSilently()
    ElMessage.success(cfg.value.enabled ? t('ks.injectOn') : t('ks.injectOff'))
  } catch (e) { ElMessage.error(e?.message || t('s2k.saveFailed')) }
}

const saveSettings = async () => {
  savingCfg.value = true
  try {
    await saveCfgSilently()
    // 只有「建向量 + 检索确实用向量」时，旧资料的向量才谈得上需要重算
    const needVector = cfg.value.buildVector && cfg.value.indexMode !== 'keyword'
    const noEmbed = cfg.value.buildVector && !models.value.some(m => m.supportEmbedding)
    if (noEmbed) {
      // 先说清楚：高质量但没有可用的嵌入模型，导入时不会生成向量，检索会退化成关键词
      ElMessage.warning(t('ks.savedNoEmbed'))
    } else {
      ElMessage.success(needVector
        ? t('ks.savedNeedReindex')
        : t('ks.savedForNew'))
    }
    await loadDocs()
  } catch (e) { ElMessage.error(e?.message || t('s2k.saveFailed')) }
  savingCfg.value = false
}

const resetSettings = async () => {
  dirty.value = false
  await loadList()
}

const doReindex = async () => {
  if (!activeId.value) return
  reindexing.value = true
  try {
    const res = await kbReindex({ kbId: activeId.value })
    if (res?.success === false) throw new Error(res.message || t('ks.reindexFailed'))
    await loadDocs()
    ElMessage.success(t('ks.reindexed', { n: (res.chunks || 0) }))
  } catch (e) { ElMessage.error(e?.message || t('ks.reindexFailed')) }
  reindexing.value = false
}

// ---------- 文档 ----------

const toggleChunks = async (doc) => {
  if (chunkDocId.value === doc.id) { chunkDocId.value = ''; chunkData.value = null; return }
  chunkDocId.value = doc.id
  chunkData.value = null
  chunkLimit.value = 10
  chunksLoading.value = true
  try {
    const d = await kbDocChunks({ kbId: activeId.value, docId: doc.id })
    if (d?.success === false) throw new Error(d.message || t('ks.loadFailed'))
    chunkData.value = d
    previewText.value = d.chunks?.[0]?.text || previewText.value
  } catch (e) { ElMessage.error(e?.message || t('ks.loadChunksFailed')) }
  chunksLoading.value = false
}

const removeDoc = async (doc) => {
  try {
    await ElMessageBox.confirm(t('ks.deleteDocBody', { name: doc.title }), t('ks.deleteDoc'), { type: 'warning' })
  } catch { return }
  try {
    const res = await kbDeleteDoc({ kbId: activeId.value, docId: doc.id })
    if (!res?.success) throw new Error(res?.message || t('ks.deleteFailed'))
    if (chunkDocId.value === doc.id) { chunkDocId.value = ''; chunkData.value = null }
    await Promise.all([loadList(), loadDocs()])
    ElMessage.success(t('mv.deleted'))
  } catch (e) { ElMessage.error(e?.message || t('ks.deleteFailed')) }
}

const runSearch = async () => {
  if (!activeId.value || !query.value.trim()) return
  searching.value = true
  expanded.value = {}
  aiAnswer.value = ''
  try {
    // topK 传 0 = 交给后端按问题类型自适应，和对话注入走同一套策略：
    // 这个弹窗的定位是「预览 AI 会看到什么」，两边规则不一致就等于白测
    const res = await kbSearch({ query: query.value, kbIds: [activeId.value], topK: 0 })
    hits.value = (res && res.hits) || []
    searched.value = true
  } catch (e) { ElMessage.error(e?.message || t('ks.recallFailed')) }
  searching.value = false
}

/**
 * 让 AI 基于当前库作答。
 *
 * <p>光看命中片段判断不了「这个库好不好用」—— 片段沾边但答不到点上的情况很常见。
 * 这里把范围锁死在当前库，让 AI 只拿这个库的资料回答，一眼就能看出它到底够不够用。
 */
const runAiAnswer = async () => {
  if (!activeId.value || !query.value.trim() || aiAnswering.value) return
  aiAnswering.value = true
  aiAnswer.value = ''
  try {
    const res = await aiChat({
      prompt: query.value,
      connectionId: '',
      database: '',
      kbIds: [activeId.value]
    })
    if (res?.success === false) throw new Error(res.message || t('ks.answerFailed'))
    aiAnswer.value = res?.content || t('ks.emptyAnswer')
  } catch (e) {
    ElMessage.error(e?.message || t('ks.answerFailed'))
  }
  aiAnswering.value = false
}

onMounted(async () => {
  await Promise.all([loadList(), loadModels()])
})
</script>

<style scoped>
/* 知识库工作台：左栏库列表 + 右栏（向导 / 详情） */
.kbs { display: flex; height: 100%; min-height: 0; background: var(--dc-bg); color: var(--dc-text); }

/* ---- 左栏 ---- */
.kbs-side {
  width: 240px; flex-shrink: 0; display: flex; flex-direction: column; min-height: 0;
  border-right: 1px solid var(--dc-border); background: var(--dc-bg-soft);
}
.kbs-side-head { display: flex; align-items: center; justify-content: space-between; padding: 13px 12px 9px; }
.kbs-side-title {
  display: inline-flex; align-items: center; gap: 6px;
  font-size: 14px; font-weight: 600; color: var(--dc-text-strong); letter-spacing: .3px;
}
.kbs-side-count {
  font-size: 10.5px; font-weight: 500; color: var(--dc-text-dim); background: var(--dc-bg-hover);
  border-radius: 9px; padding: 1px 7px; letter-spacing: 0;
}
.kbs-side-search { padding: 0 12px 9px; }
.kbs-side-list { flex: 1; overflow: auto; padding: 0 8px 8px; }
.kbs-item {
  position: relative; display: flex; align-items: center; gap: 9px; padding: 9px 10px;
  border-radius: 8px; cursor: pointer; margin-bottom: 3px;
  transition: background .15s ease;
}
.kbs-item:hover { background: var(--dc-bg-hover); }
.kbs-item.active { background: var(--dc-primary-wash); }
/* 选中态再加一道左竖条，扫一眼就知道当前在哪个库 */
.kbs-item.active::before {
  content: ''; position: absolute; left: 0; top: 8px; bottom: 8px;
  width: 2px; border-radius: 0 2px 2px 0; background: var(--dc-primary);
}
.kbs-item-icon { color: var(--dc-text-dim); flex-shrink: 0; font-size: 15px; }
.kbs-item.active .kbs-item-icon { color: var(--dc-primary); }
.kbs-item-body { min-width: 0; flex: 1; }
.kbs-item-name { font-size: 13px; color: var(--dc-text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.kbs-item.active .kbs-item-name { color: var(--dc-primary); font-weight: 600; }
.kbs-item-meta { font-size: 12px; color: var(--dc-text-dim); margin-top: 3px; }
.kbs-side-empty { padding: 16px 10px; font-size: 13px; color: var(--dc-text-dim); text-align: center; }
.kbs-side-foot {
  display: flex; align-items: center; justify-content: space-between;
  padding: 10px 14px; border-top: 1px solid var(--dc-border);
}
.kbs-foot-label { display: inline-flex; align-items: center; gap: 6px; font-size: 13px; color: var(--dc-text-dim); }

/* ---- 右栏 ---- */
.kbs-main { flex: 1; min-width: 0; min-height: 0; overflow: auto; padding: 16px 20px 24px; }
.kbs-head {
  display: flex; align-items: flex-start; gap: 12px;
  padding-bottom: 12px; border-bottom: 1px solid var(--dc-border);
}
.kbs-head-icon {
  width: 36px; height: 36px; border-radius: 10px; flex-shrink: 0;
  display: inline-flex; align-items: center; justify-content: center;
  background: var(--dc-primary-wash); color: var(--dc-primary); font-size: 18px;
}
.kbs-head-main { min-width: 0; flex: 1; }
/* 名称后的库级操作：方形图标按钮，悬停浮出底色；删除单独隔开且常显红 */
.kbs-head-ops { display: inline-flex; align-items: center; gap: 2px; margin-left: 10px; }
.kbs-head-ops :deep(.el-button) {
  width: 26px; height: 26px; padding: 0; border-radius: 6px; color: var(--dc-text-dim);
}
.kbs-head-ops :deep(.el-button:hover) { background: var(--dc-bg-hover); color: var(--dc-text); }
.kbs-head-ops :deep(.el-button.is-disabled) { color: var(--dc-text-weak); opacity: .5; }
.kbs-head-ops-sep { width: 1px; height: 14px; background: var(--dc-border); margin: 0 5px; }
.kbs-head-ops :deep(.kbs-head-del) { color: var(--el-color-danger); }
.kbs-head-ops :deep(.kbs-head-del:hover) {
  background: var(--el-color-danger-light-9); color: var(--el-color-danger);
}
/* 弹窗里的字段 */
.kbs-field { display: flex; flex-direction: column; gap: 6px; margin-bottom: 14px; }
.kbs-field > label { font-size: 13px; color: var(--dc-text-mid); }
/* 输入控件统一走填充式（与设置弹窗里的数字框同一形制） */
.kbs :deep(.el-input__wrapper), .kbs :deep(.el-textarea__inner) {
  border-radius: 8px; background: var(--dc-bg-soft); box-shadow: none;
}
.kbs :deep(.el-input__wrapper.is-focus), .kbs :deep(.el-textarea__inner:focus) {
  box-shadow: 0 0 0 1px var(--dc-primary) inset;
}
.kbs-head-name { display: flex; align-items: center; gap: 4px; font-size: 16px; font-weight: 600; color: var(--dc-text-strong); }
.kbs-head-name > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.kbs-head-meta { font-size: 13px; color: var(--dc-text-dim); margin-top: 4px; }
/* 备注：最多两行、超出省略，鼠标悬停看全文 */
.kbs-head-desc {
  margin-top: 5px; font-size: 13px; color: var(--dc-text-dim); line-height: 1.7;
  display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical;
  overflow: hidden; overflow-wrap: anywhere;
}


/* ---- 向导：占满内容区的可用高度。
   高度链一路传下去，两栏内部自己滚，页面就不会再出现最右侧那条整页滚动条 ---- */
.kbs-wizard {
  max-width: 1180px; margin: 0 auto; padding-top: 16px; height: 100%;
  box-sizing: border-box; display: flex; flex-direction: column;
}
.kbs-steps { flex-shrink: 0; margin: 2px 0 26px; }
.kbs-steps :deep(.el-step__title) { font-size: 14px; }
.kbs-steps :deep(.el-step__description) { font-size: 11.5px; }
.kbs-step { flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 12px; }
/* 设置面板吃掉剩余高度（内部自己滚），操作条保持固定 */
.kbs-step > .kbset { flex: 1; min-height: 0; }
/* 说明文字做成一条浅底提示，不再是一段悬空的灰字 */
.kbs-step-hint {
  line-height: 1.85; padding: 9px 12px; border-radius: 8px;
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border-soft);
}
/* 操作条吸底：不画分隔线，改用向上渐隐的背景把按钮与滚动内容分开（横线在滚动时会割裂内容）；
   底部只留 10px，按钮就落在离滚动区底部 10px 的位置 */
.kbs-wizard-foot {
  position: sticky; bottom: 0; z-index: 2; flex-shrink: 0;
  margin: 24px -20px -24px; padding: 16px 20px 10px;
  background: linear-gradient(to top, var(--dc-bg) 72%, transparent);
}
.kbs-foot-inner {
  max-width: 1180px; margin: 0 auto;
  display: flex; align-items: center; justify-content: flex-end; gap: 12px;
}

/* 设置弹窗底部操作条（在 dialog 的 footer 插槽里，分隔线由 dialog 自己画） */
.kbs-drawer-foot { display: flex; align-items: center; justify-content: flex-end; gap: 10px; }

.kbs-sum { display: flex; flex-direction: column; }
.kbs-sum-item {
  display: flex; gap: 12px; align-items: baseline;
  padding: 9px 0; font-size: 13px; border-bottom: 1px dashed var(--dc-border-soft);
}
.kbs-sum-item:last-child { border-bottom: none; }
.kbs-sum-item > span { color: var(--dc-text-dim); min-width: 68px; flex-shrink: 0; }
.kbs-sum-item > b { color: var(--dc-text-strong); font-weight: 600; }

/* 上传区：圆底图标 + hover 主色，明确"这里是能拖/能点的" */
.kbs-drop {
  display: flex; flex-direction: column; align-items: center; gap: 8px;
  padding: 30px 16px; border: 1px dashed var(--dc-border-strong); border-radius: 12px;
  background: var(--dc-bg-soft); cursor: pointer; text-align: center;
  transition: border-color .15s ease, background .15s ease, transform .15s ease;
}
.kbs-drop:hover { transform: translateY(-1px); }
.kbs-drop .kbs-drop-icon {
  font-size: 22px; color: var(--dc-primary);
  width: 44px; height: 44px; border-radius: 50%;
  display: inline-flex; align-items: center; justify-content: center;
  background: var(--dc-primary-wash);
}
.kbs-drop-sm { padding: 20px 16px; }
.kbs-drop:hover, .kbs-drop.over { border-color: var(--dc-primary); background: var(--dc-primary-wash); }
/* 导入中：区里文字已换成「正在导入…」，光标也交代一声 */
.kbs-drop.busy { cursor: progress; }
.kbs-drop-sm .kbs-drop-icon { font-size: 20px; }
.kbs-drop-title { font-size: 14px; color: var(--dc-text); }
.kbs-drop-desc { font-size: 11.5px; color: var(--dc-text-dim); line-height: 1.7; }
.kbs-drop-desc b { color: var(--dc-text-mid); }

.kbs-card {
  border: 1px solid var(--dc-border); border-radius: 10px;
  background: var(--dc-bg-soft); padding: 12px 14px; margin: 12px 0;
}
.kbs-card-title { font-size: 14px; font-weight: 600; color: var(--dc-text-strong); margin-bottom: 8px; }

/* 标题 / 计数 / 搜索 / 操作排一行：用 center —— baseline 会让按钮与标题错位 */
.kbs-list-head {
  display: flex; align-items: center; gap: 8px;
  margin: 18px 0 8px; padding-bottom: 6px; border-bottom: 1px solid var(--dc-border);
}
.kbs-list-title { font-size: 14px; font-weight: 600; color: var(--dc-text-strong); }
.kbs-count { font-size: 12px; color: var(--dc-text-dim); font-variant-numeric: tabular-nums; }
/* 占位把后面的东西推到右边：有没有搜索框，「…」都贴右、也不会挤到计数上 */
.kbs-list-spacer { flex: 1; }
.kbs-doc-search { width: 176px; }
.kbs-empty {
  padding: 18px 14px; text-align: center; font-size: 13px; color: var(--dc-text-dim);
  border: 1px dashed var(--dc-border); border-radius: 8px; background: var(--dc-bg-soft);
}
.kbs-warn {
  display: flex; align-items: center; gap: 6px; flex-wrap: wrap;
  margin: 10px 0; padding: 8px 12px; font-size: 13px; color: var(--dc-text-mid);
  border: 1px solid var(--dc-border-strong); border-radius: 8px; background: var(--dc-bg-hover);
}

/* 资料行 */
.kbs-doc {
  border: 1px solid var(--dc-border); border-radius: 10px; background: var(--dc-bg-soft);
  margin-bottom: 8px; overflow: hidden;
  transition: border-color .15s ease, background .15s ease;
}
.kbs-doc:hover { border-color: var(--dc-border-hover); }
/* 资料行整行可点（展开分块），行尾只留一个红色删除图标，鼠标进到行里才全亮 */
.kbs-doc-row { display: flex; align-items: center; gap: 10px; padding: 10px 12px; cursor: pointer; }
.kbs-doc-row :deep(.kbs-doc-del) { color: var(--el-color-danger); opacity: .65; transition: opacity .15s ease; }
.kbs-doc:hover .kbs-doc-row :deep(.kbs-doc-del) { opacity: 1; }
.kbs-doc-icon { color: var(--dc-text-dim); flex-shrink: 0; }
.kbs-doc-main { flex: 1; min-width: 0; }
.kbs-doc-title { font-size: 13px; color: var(--dc-text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.kbs-doc-meta { font-size: 12px; color: var(--dc-text-dim); margin-top: 2px; }
.kbs-title-input { margin-bottom: 4px; }
.kbs-tag {
  margin-left: 6px; font-size: 10.5px; color: var(--dc-text-dim);
  background: var(--dc-bg-hover); border-radius: 9px; padding: 1px 6px;
}
.kbs-tag.ok { color: var(--dc-primary); background: var(--dc-primary-wash); }

/* 分块 */
.kbs-chunks { border-top: 1px dashed var(--dc-border); padding: 10px 12px 12px; background: var(--dc-bg); }
.kbs-chunks-head { font-size: 11.5px; color: var(--dc-text-dim); margin-bottom: 8px; }
.kbs-chunks-head b { color: var(--dc-primary); }
.kbs-chunk {
  border: 1px solid var(--dc-border-soft); border-radius: 8px;
  background: var(--dc-bg-card); padding: 8px 10px; margin-bottom: 6px;
}
.kbs-chunk-head { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; }
.kbs-chunk-no {
  font-size: 12px; color: var(--dc-primary); background: var(--dc-primary-wash);
  border-radius: 4px; padding: 1px 6px; font-variant-numeric: tabular-nums;
}
.kbs-chunk-size { font-size: 12px; color: var(--dc-text-dim); }
.kbs-chunk-text {
  font-size: 13px; color: var(--dc-text-mid); line-height: 1.75;
  white-space: pre-wrap; word-break: break-word; max-height: 160px; overflow: auto;
}
.kbs-more { text-align: center; margin-top: 6px; }

/* ---- 召回测试弹窗 ---- */
.rc-head { display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--dc-text-mid); }
.rc-head b { color: var(--dc-text-strong); }
/* 输入区合成一整块：问题、按钮、当前检索参数在一块里，看着才知道在测什么 */
.rc-box {
  margin-top: 10px; padding: 12px;
  border: 1px solid var(--dc-border); border-radius: 10px; background: var(--dc-bg-soft);
}
.rc-input { display: flex; align-items: center; gap: 8px; }
.rc-input > .el-input { flex: 1; }
.rc-input :deep(.el-button) { flex-shrink: 0; }
.rc-stat { margin-top: 12px; font-size: 13px; color: var(--dc-text-dim); }
.rc-stat b { color: var(--dc-primary); }
.rc-blank {
  margin-top: 12px; padding: 24px 16px; text-align: center; font-size: 13px;
  color: var(--dc-text-dim); line-height: 1.8;
  border: 1px dashed var(--dc-border); border-radius: 10px; background: var(--dc-bg-soft);
}
/* 字段名旁的问号（与设置面板同一形制） */
.q { font-size: 13px; color: var(--dc-text-weak); cursor: help; flex-shrink: 0; }
.q:hover { color: var(--dc-text-mid); }

/* ---- 召回测试里的「能问什么」 ---- */
.rc-outline { margin: 10px 0; }
.rc-outline-cta { display: flex; }
.rc-outline-go {
  display: inline-flex; align-items: center; gap: 5px;
  font-size: 13px; color: var(--dc-primary); cursor: pointer;
}
.rc-outline-go:hover { text-decoration: underline; }
.rc-outline-loading {
  display: flex; align-items: center; gap: 6px;
  font-size: 13px; color: var(--dc-text-dim);
}
.rc-outline-sum {
  font-size: 13px; color: var(--dc-text); line-height: 1.8;
  padding: 8px 10px; border-radius: 8px;
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border);
}
.rc-outline-qs { display: flex; flex-direction: column; gap: 5px; margin-top: 8px; }
.rc-q {
  display: flex; align-items: center; gap: 6px; padding: 6px 10px;
  font-size: 13px; color: var(--dc-text-mid); cursor: pointer;
  border-radius: 8px; background: var(--dc-bg-soft); border: 1px solid var(--dc-border);
  transition: border-color .15s ease, color .15s ease;
}
.rc-q:hover { border-color: var(--dc-primary); color: var(--dc-primary); }
.rc-q .el-icon { flex-shrink: 0; font-size: 13px; }
.rc-outline-terms { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 8px; }
.rc-term {
  font-size: 12px; padding: 2px 9px; border-radius: 9px; cursor: pointer;
  background: var(--dc-bg-hover); color: var(--dc-text-mid);
}
.rc-term:hover { background: var(--dc-primary-wash); color: var(--dc-primary); }

/* ---- 召回测试里的 AI 试答 ---- */
.rc-answer {
  margin: 10px 0 12px; border-radius: 10px; overflow: hidden;
  background: var(--dc-primary-wash); border: 1px solid var(--dc-border);
}
.rc-answer-head {
  display: flex; align-items: center; gap: 6px; padding: 8px 11px;
  font-size: 13px; font-weight: 600; color: var(--dc-text-strong);
  border-bottom: 1px dashed var(--dc-border);
}
.rc-answer-head .el-icon { color: var(--dc-primary); }
.rc-answer-scope { margin-left: auto; font-size: 10.5px; font-weight: 400; color: var(--dc-text-dim); }
.rc-answer-loading {
  display: flex; align-items: center; gap: 6px; padding: 14px 11px;
  font-size: 13px; color: var(--dc-text-dim);
}
.rc-answer-body {
  padding: 10px 12px; font-size: 13px; line-height: 1.8; color: var(--dc-text);
  white-space: pre-wrap; word-break: break-word;
}

.kbs-hit {
  border: 1px solid var(--dc-border); border-radius: 10px; background: var(--dc-bg-soft);
  padding: 10px 12px; margin-top: 10px;
}
.kbs-hit-head { display: flex; align-items: center; gap: 8px; margin-bottom: 8px; flex-wrap: wrap; }
.kbs-hit-rank { font-size: 12px; color: var(--dc-text-dim); font-variant-numeric: tabular-nums; }
.kbs-hit-bar { width: 90px; height: 5px; border-radius: 3px; background: var(--dc-bg-hover); overflow: hidden; }
.kbs-hit-bar i { display: block; height: 100%; background: var(--dc-primary); }
.kbs-hit-score { font-size: 11.5px; color: var(--dc-primary); font-variant-numeric: tabular-nums; }
.kbs-hit-src { font-size: 12px; color: var(--dc-text-dim); margin-left: auto; }
.kbs-hit-text {
  font-size: 13px; color: var(--dc-text-mid); line-height: 1.8;
  white-space: pre-wrap; word-break: break-word;
}
.kbs-hit-text.fold { max-height: 110px; overflow: hidden; }
.kbs-hit-more { margin-top: 2px; padding-left: 0; }
.kbs-hit-more { margin-top: 4px; }

.kbs-hint { font-size: 11.5px; color: var(--dc-text-dim); line-height: 1.7; }
.kbs-hint b { color: var(--dc-text-mid); }

/* 空态 */
.kbs-blank {
  height: 100%; display: flex; flex-direction: column; align-items: center;
  justify-content: center; gap: 10px; text-align: center; padding: 40px 20px;
}
.kbs-blank-icon { font-size: 34px; color: var(--dc-border-strong); }
.kbs-blank-title { font-size: 14px; font-weight: 600; color: var(--dc-text-mid); }
.kbs-blank-desc { font-size: 13px; color: var(--dc-text-dim); line-height: 1.9; max-width: 420px; }
.kbs-blank-desc b { color: var(--dc-text-mid); }
</style>
