<template>
  <!-- 关键：必须挂上 is-studio 类，否则 .ai-panel.is-studio 的 width:100% 永不命中；
       同时中央工作区形态下不再套用内联固定宽度（否则仍被锁成 420px 窄条）。 -->
  <div class="ai-panel" :class="{ 'is-studio': isStudio }"
       :style="isStudio ? null : { width: panelWidth + 'px' }">
    <div v-if="!isStudio" class="ai-resize-bar" :title="$t('ai.resizeTip')" @mousedown.prevent="startResize"></div>

    <!-- 头部：仅侧栏形态需要（品牌 + 关闭）。
         中央工作区形态由外层承载标题，这里不再重复一行，内容直接从顶部铺满。 -->
    <div v-if="!isStudio" class="ai-head">
      <div class="ai-brand">
        <span class="ai-brand-ic"><el-icon :size="15"><MagicStick /></el-icon></span>
        <span class="ai-brand-tx">
          <span class="ai-brand-name">{{ $t('empty.aiAssistant') }}</span>
          <span class="ai-brand-sub">{{ $t('ai.brandSub') }}</span>
        </span>
      </div>
      <div class="ai-head-right">
        <el-tooltip :content="$t('ai.closePanel')" placement="left">
          <button class="dc-btn-icon" @click="$emit('close')"><el-icon><Close /></el-icon></button>
        </el-tooltip>
      </div>
    </div>

    <div class="ai-main">
      <div class="ai-body">
        <!-- ===== 结果区：内容随当前 Skill 变化 ===== -->
        <div ref="chatBox" class="ai-view">
          <!-- 空态默认面板：首次打开 / 尚无结果时，展示数据源状态、技能入口与示例 -->
          <div v-if="showWelcome" class="ai-welcome">
            <div class="welcome-panel">
              <div class="welcome-hero">
                <span class="welcome-logo"><el-icon :size="18"><MagicStick /></el-icon></span>
                <div class="welcome-hero-tx">
                  <div class="welcome-title">{{ $t('ai.welcomeTitle') }}</div>
                  <div class="welcome-sub">{{ $t('ai.welcomeSub') }}</div>
                </div>
              </div>

              <!-- 功能介绍：替代原来的数据源状态条（数据源在下方工具条上已经能看到） -->
              <div class="welcome-block is-intro">
                <div class="welcome-section-title"><el-icon><MagicStick /></el-icon>{{ $t('ai.whatCanDo') }}</div>
                <div class="welcome-intro">
                  <div v-for="(f, i) in WELCOME_FEATURES" :key="i" class="intro-item">
                    <el-icon class="intro-ic"><component :is="f.icon" /></el-icon>
                    <div class="intro-tx">
                      <span class="intro-name">{{ f.name }}</span>
                      <span class="intro-desc">{{ f.desc }}</span>
                    </div>
                  </div>
                </div>
              </div>

              <div class="welcome-block">
                <div class="welcome-section-title"><el-icon><Star /></el-icon>{{ $t('ai.advancedSkills') }}</div>
                <div class="welcome-grid">
                  <button v-for="t in advancedTabs" :key="t.key" class="welcome-card" type="button"
                          :title="t.desc" @click="pickSkill(t.key)">
                    <el-icon class="welcome-card-ic"><component :is="t.icon" /></el-icon>
                    <span class="welcome-card-tx">
                      <span class="welcome-card-name">{{ t.label }}</span>
                      <span class="welcome-card-desc">{{ t.desc }}</span>
                    </span>
                  </button>
                </div>
              </div>

              <div class="welcome-block">
                <div class="welcome-section-title"><el-icon><ChatDotRound /></el-icon>{{ $t('ai.tryAsk') }}</div>
                <div class="welcome-examples">
                  <button v-for="(ex, i) in WELCOME_EXAMPLES" :key="i" class="welcome-example" type="button"
                          :title="$t('ai.askDirectly', { q: ex })" @click="useExample(ex)">
                    <span>{{ ex }}</span><el-icon><ArrowRight /></el-icon>
                  </button>
                </div>
              </div>
            </div>
          </div>

          <!-- 对话 / 智能分析：消息流 -->
          <template v-if="isChatSkill">
            <div v-for="(m, i) in messages" :key="i" class="msg" :class="m.role">
              <div class="msg-avatar">{{ m.role === 'user' ? $t('ai.me') : 'AI' }}</div>
              <div class="msg-bubble" :class="{ typing: m.role !== 'user' && !m.content && m.streaming }">
                <template v-if="m.role === 'user'"><div class="msg-text">{{ m.content }}</div></template>
                <!-- 流式回复尚未产出内容：显示动态三点，而不是一个空气泡 -->
                <template v-else-if="!m.content && m.streaming">
                  <span class="dot"></span><span class="dot"></span><span class="dot"></span>
                  <span v-if="waitingSec >= 3" class="wait">{{ $t('ai.waited', { s: waitingSec }) }}</span>
                </template>
                <!-- SQL 代码块的操作按钮由 markdown 渲染器生成（每段一个），这里做点击委托 -->
                <div v-else class="ai-markdown" v-html="renderMd(m.content, !m.streaming)"
                     @click="onMdSqlAction($event, m.content)"></div>
                <!-- 流式输出中：明确提示「还在生成」，避免用户误以为已经答完；可随时停下 -->
                <div v-if="m.role !== 'user' && m.streaming && m.content" class="streaming-hint">
                  <span class="dot"></span><span class="dot"></span><span class="dot"></span>
                  <span class="txt">{{ $t('ai.generating') }}</span>
                  <el-button size="small" text type="primary" class="stop-gen" @click="stopStream">{{ $t('ai.stopGen') }}</el-button>
                </div>
                <!-- 智能分析会真的执行只读查询：把工具调用轨迹显式列出来，
                     让「它查了库」这件事一眼可见，而不是黑盒给个结论 -->
                <div v-if="m.steps && m.steps.length" class="msg-steps">
                  <div class="msg-steps-head">
                    <el-icon><Cpu /></el-icon>{{ $t('ai.traceSteps', { n: m.steps.length }) }}
                  </div>
                  <div v-for="(s, si) in m.steps" :key="si" class="msg-step">
                    <el-icon><Cpu /></el-icon>
                    <span class="step-tool">{{ toolLabel(s.tool) }}</span>
                    <span class="step-args">{{ s.args }}</span>
                  </div>
                </div>
                <!-- 引用溯源：把「这次答话用了哪些团队资料」摊开。
                     能核对才敢信 —— 否则知识库到底有没有生效，用户只能靠猜。 -->
                <div v-if="m.sources && m.sources.length" class="msg-sources">
                  <div class="msg-src-head" @click="m.srcOpen = !m.srcOpen">
                    <el-icon><Collection /></el-icon>
                    <span>{{ $t('ai.citedSources', { n: m.sources.length }) }}</span>
                    <el-icon class="msg-src-caret" :class="{ open: m.srcOpen }"><ArrowDown /></el-icon>
                  </div>
                  <div v-if="m.srcOpen" class="msg-src-list">
                    <div v-for="(s, si) in m.sources" :key="si" class="msg-src">
                      <div class="msg-src-top">
                        <span class="msg-src-rank">#{{ si + 1 }}</span>
                        <span class="msg-src-bar"><i :style="{ width: srcPercent(s.score) }"></i></span>
                        <span class="msg-src-score">{{ Number(s.score || 0).toFixed(3) }}</span>
                        <!-- 点来源直接跳到知识库，落在那份资料所在的库上 -->
                        <span class="msg-src-src" :title="$t('ai.openSource')"
                              @click="emit('open-knowledge', s.kbId)">
                          {{ s.kbName }} · {{ s.docTitle }}<template v-if="s.index">{{ $t('ai.sourceIndex', { n: s.index }) }}</template>
                        </span>
                      </div>
                      <div class="msg-src-text">{{ s.text }}</div>
                    </div>
                  </div>
                </div>
                <!-- 用量脚注：这条回答实际消耗的 token。上游没回 usage 就**整块不显示** ——
                     宁可没有，也不要一个编出来的数字（那会让人误判成本）。 -->
                <div v-if="m.usage" class="msg-usage">
                  {{ $t('ai.usagePre') }}{{ fmtTokens(m.usage.totalTokens) }} tokens
                  <span class="msg-usage-sub">{{ $t('ai.usageIn', { n: fmtTokens(m.usage.promptTokens) }) }}{{ $t('ai.usageOut', { n: fmtTokens(m.usage.completionTokens) }) }}</span>
                </div>
              </div>
            </div>
            <div v-if="chatLoading" class="msg assistant">
              <div class="msg-avatar">AI</div>
              <div class="msg-bubble typing">
                <span class="dot"></span><span class="dot"></span><span class="dot"></span>
                <span v-if="waitingSec >= 3" class="wait">{{ $t('ai.waited', { s: waitingSec }) }}</span>
              </div>
            </div>
          </template>

          <!-- 高级技能（表健康巡检 / 表数据洞察）：统一渲染 Markdown 结论，并可导出文件 -->
          <template v-else-if="isAdvancedSkill">
            <div v-if="advResult" class="res-card">
              <el-dropdown class="res-export-float" trigger="click" placement="bottom-end"
                           popper-class="ai-model-dropdown"
                           @command="(f) => exportDoc(advResult, f)">
                <button class="res-export" type="button" :disabled="!!exporting" :title="$t('ai.exportAsFile')">
                  <el-icon><Download /></el-icon>{{ $t('qa.exportBtn') }}<el-icon class="res-export-caret"><ArrowDown /></el-icon>
                </button>
                <template #dropdown>
                  <div class="ai-dd-head">{{ $t('ai.exportAs') }}</div>
                  <el-dropdown-menu>
                    <el-dropdown-item command="md">
                      <el-icon class="skills-ic"><Notebook /></el-icon><span class="skills-name">{{ $t('ai.fmtMd') }}</span>
                    </el-dropdown-item>
                    <el-dropdown-item command="docx">
                      <el-icon class="skills-ic"><DocumentCopy /></el-icon><span class="skills-name">{{ $t('ai.fmtDocx') }}</span>
                    </el-dropdown-item>
                    <el-dropdown-item command="xlsx">
                      <el-icon class="skills-ic"><Grid /></el-icon><span class="skills-name">{{ $t('ai.fmtXlsx') }}</span>
                    </el-dropdown-item>
                    <el-dropdown-item command="pdf">
                      <el-icon class="skills-ic"><Notebook /></el-icon><span class="skills-name">{{ $t('ai.fmtPdf') }}</span>
                    </el-dropdown-item>
                  </el-dropdown-menu>
                </template>
              </el-dropdown>
              <div class="ai-markdown" v-html="renderMd(advResult, true)"
                   @click="onMdSqlAction($event, advResult)"></div>
            </div>
            <div v-else-if="advLoading" class="res-loading">
              <el-icon class="is-loading"><Loading /></el-icon>
              <span>{{ tab === 'patrol' ? $t('ai.patrolling') : $t('ai.analyzing') }}</span>
            </div>
          </template>

          <!-- 知识入库：体检 → 优化 → 建档 → 报告的多阶段流水线，实现见 KbIngestPanel.vue -->
          <template v-else-if="isKbIngest">
            <KbIngestPanel @open-knowledge="emit('open-knowledge')" />
          </template>

          <!-- 数据字典：Markdown 渲染 + 导出 Markdown / Word / Excel / PDF -->
          <template v-else-if="tab === 'dictionary'">
            <div v-if="dictResult" class="res-card">
              <!-- 导出按钮浮在卡片右上角，与文档标题同一行（标题由 Markdown 正文自带，这里不再重复） -->
              <el-dropdown class="res-export-float" trigger="click" placement="bottom-end"
                           popper-class="ai-model-dropdown"
                           @command="(f) => exportDoc(dictResult, f, $t('ai.tabDictionary'))">
                <button class="res-export" type="button" :disabled="!!exporting" :title="$t('ai.exportAsFile')">
                  <el-icon><Download /></el-icon>{{ $t('qa.exportBtn') }}<el-icon class="res-export-caret"><ArrowDown /></el-icon>
                </button>
                <template #dropdown>
                  <div class="ai-dd-head">{{ $t('ai.exportAs') }}</div>
                  <el-dropdown-menu>
                    <el-dropdown-item command="md">
                      <el-icon class="skills-ic"><Notebook /></el-icon><span class="skills-name">{{ $t('ai.fmtMd') }}</span>
                    </el-dropdown-item>
                    <el-dropdown-item command="docx">
                      <el-icon class="skills-ic"><DocumentCopy /></el-icon><span class="skills-name">{{ $t('ai.fmtDocx') }}</span>
                    </el-dropdown-item>
                    <el-dropdown-item command="xlsx">
                      <el-icon class="skills-ic"><Grid /></el-icon><span class="skills-name">{{ $t('ai.fmtXlsx') }}</span>
                    </el-dropdown-item>
                    <el-dropdown-item command="pdf">
                      <el-icon class="skills-ic"><Notebook /></el-icon><span class="skills-name">{{ $t('ai.fmtPdf') }}</span>
                    </el-dropdown-item>
                  </el-dropdown-menu>
                </template>
              </el-dropdown>
              <div class="ai-markdown" v-html="renderMd(dictResult, true)"
                   @click="onMdSqlAction($event, dictResult)"></div>
            </div>
            <div v-else-if="dictLoading" class="res-loading">
              <el-icon class="is-loading"><Loading /></el-icon><span>{{ $t('ai.generatingDict') }}</span>
            </div>
          </template>

        </div>

        <!-- ===== 统一输入框：Skills / 参数 / 模型 / 主按钮全部收在框内 ===== -->
        <div class="ai-composer">

          <!-- 斜杠命令浮层：在输入框里敲 / 直接唤起功能 -->
          <div v-if="slashOpen && slashFiltered.length" class="slash-menu">
            <div class="slash-head">{{ $t('ai.slashHead') }}</div>
            <template v-for="(c, i) in slashFiltered" :key="c.key">
              <div v-if="i === 0 || slashFiltered[i - 1].group !== c.group" class="slash-group">{{ c.group }}</div>
              <div
                class="slash-item"
                :class="{ active: i === slashIndex }"
                @mouseenter="slashIndex = i"
                @mousedown.prevent="pickSlash(c)"
              >
                <el-icon class="slash-ic"><component :is="c.icon" /></el-icon>
                <span class="slash-label">{{ c.label }}</span>
                <span class="slash-desc">{{ c.desc }}</span>
              </div>
            </template>
          </div>

          <!-- 输入域 + 工具条同属一个「框」，工具条贴在框内底部 -->
          <div class="composer-box">
            <el-input ref="inputRef" v-model="aiInput" type="textarea" :rows="3" resize="none" class="composer-textarea"
                      :placeholder="composePlaceholder"
                      @keydown="onComposeKeydown" @input="onComposeInput" />

            <div class="composer-bar">
              <!-- 左侧：Skills / 专属参数 / 上下文 / 模型（除发送外全部靠左排布） -->
              <!-- Skills：完全复用「模型」下拉的 el-dropdown，样式天然一致 -->
              <el-dropdown ref="skillDropRef" trigger="click" placement="top-start" popper-class="ai-model-dropdown"
                           @command="pickSkill" @visible-change="onSkillVisible">
                <button class="skill-btn" :class="{ open: skillsOpen }" type="button">
                  <el-icon class="skill-btn-ic"><component :is="currentSkill.icon" /></el-icon>
                  <span class="skill-btn-tx">{{ currentSkill.label }}</span>
                  <el-icon class="skill-btn-caret"><ArrowDown /></el-icon>
                </button>
                <template #dropdown>
                  <div class="ai-dd-head">{{ $t('ai.pickSkill') }}</div>
                  <el-dropdown-menu class="ai-model-menu">
                    <el-dropdown-item
                      v-for="t in tabs"
                      :key="t.key"
                      :command="t.key"
                      :class="{ active: tab === t.key }"
                      :title="t.desc"
                    >
                      <el-icon class="skills-ic"><component :is="t.icon" /></el-icon>
                      <span class="skills-name">{{ t.label }}</span>
                      <el-icon v-if="tab === t.key" class="skills-check"><Select /></el-icon>
                    </el-dropdown-item>
                  </el-dropdown-menu>
                </template>
              </el-dropdown>

              <!-- 上下文：同样复用「模型」下拉的 el-dropdown；树形数据拍平成下拉项 -->
              <!-- placement 用 top-start，但**必须**让 popper 自己避让视口：
                   上下文列表带层级缩进、项多且库名一长整块就很宽，贴着窗口右边时会整块铺到屏幕外。
                   preventOverflow 让它在视口内平移，flip 兜底上下翻转。 -->
              <el-dropdown ref="ctxDropRef" trigger="click" placement="top-start" popper-class="ai-model-dropdown"
                           :hide-on-click="false" @command="onCtxCommand" @visible-change="onCtxVisible"
                           :popper-options="{ modifiers: [
                             { name: 'preventOverflow', options: { boundary: 'viewport', padding: 8 } },
                             { name: 'flip', options: { fallbackPlacements: ['top-end', 'bottom-start', 'bottom-end'] } }
                           ] }">
                <button class="composer-ctx" :class="{ open: ctxOpen }" type="button">
                  <el-icon :size="12"><Coin /></el-icon>
                  <span class="composer-ctx-tx">{{ contextLabel || $t('ai.pickContext') }}</span>
                  <el-icon :size="12" class="composer-ctx-caret"><ArrowDown /></el-icon>
                </button>
                <template #dropdown>
                  <div class="ai-dd-head">{{ $t('ai.pickSource') }}</div>
                  <el-dropdown-menu class="ai-model-menu ctx-menu">
                    <el-dropdown-item
                      v-for="item in ctxFlatItems"
                      :key="item.value"
                      :command="item.value"
                      :class="{
                        'ctx-item': true,
                        active: ctxSelectedValue === item.value,
                        'is-parent': item.expandable,
                        'is-expanded': item.expanded,
                        ['lv-' + item.level]: true
                      }"
                      :style="{ paddingLeft: (10 + item.level * 16) + 'px' }"
                    >
                      <el-icon v-if="item.expandable" :size="10" class="ctx-caret" :class="{ expanded: item.expanded }">
                        <ArrowRight />
                      </el-icon>
                      <span v-else class="ctx-caret-spacer"></span>
                      <el-icon :size="13" class="ctx-level-ic"><component :is="ctxLevelIcon(item)" /></el-icon>
                      <span class="ctx-label">{{ item.label }}</span>
                      <el-icon v-if="ctxSelectedValue === item.value" class="skills-check"><Select /></el-icon>
                    </el-dropdown-item>
                    <el-dropdown-item v-if="!ctxFlatItems.length" disabled class="ctx-item ctx-empty">
                      {{ $t('ai.noConn') }}
                    </el-dropdown-item>
                  </el-dropdown-menu>
                </template>
              </el-dropdown>

              <!-- 知识库范围：控制这次提问检索哪些资料库。
                   原先是否注入完全由知识库侧的全局开关决定，问 A 系统的表却召回 B 系统的资料也无法控制 -->
              <el-dropdown ref="kbDropRef" trigger="click" placement="top-start" popper-class="ai-model-dropdown"
                           :hide-on-click="false" @command="onKbScopeCommand" @visible-change="onKbVisible">
                <button class="composer-ctx" :class="{ open: kbOpen }" type="button">
                  <el-icon :size="12"><component :is="kbScopeIcon" /></el-icon>
                  <span class="composer-ctx-tx">{{ kbScopeLabel }}</span>
                  <el-icon :size="12" class="composer-ctx-caret"><ArrowDown /></el-icon>
                </button>
                <template #dropdown>
                  <div class="ai-dd-head">{{ $t('ai.kbScope') }}</div>
                  <el-dropdown-menu class="ai-model-menu ctx-menu">
                    <el-dropdown-item command="all" :class="{ active: kbScope === 'all' }">
                      <el-icon :size="13"><Collection /></el-icon>
                      <span class="ctx-label">{{ $t('ai.kbAll') }}</span>
                      <el-icon v-if="kbScope === 'all'" class="skills-check"><Select /></el-icon>
                    </el-dropdown-item>
                    <el-dropdown-item command="none" :class="{ active: kbScope === 'none' }">
                      <el-icon :size="13"><Close /></el-icon>
                      <span class="ctx-label">{{ $t('ai.kbNone') }}</span>
                      <el-icon v-if="kbScope === 'none'" class="skills-check"><Select /></el-icon>
                    </el-dropdown-item>
                    <el-dropdown-item v-if="kbListItems.length" command="custom"
                                      :class="{ active: kbScope === 'custom' }" divided>
                      <el-icon :size="13"><Aim /></el-icon>
                      <span class="ctx-label">{{ $t('ai.kbPicked') }}</span>
                      <el-icon v-if="kbScope === 'custom'" class="skills-check"><Select /></el-icon>
                    </el-dropdown-item>
                    <template v-if="kbScope === 'custom'">
                      <el-dropdown-item v-for="k in kbListItems" :key="k.id"
                                        :command="'pick:' + k.id"
                                        :class="{ active: kbPicked.includes(k.id), 'ctx-sub': true }">
                        <el-icon v-if="kbPicked.includes(k.id)" class="skills-check"><Select /></el-icon>
                        <span v-else class="ctx-caret-spacer"></span>
                        <span class="ctx-label">{{ k.name }}</span>
                        <span class="ctx-sub-count">{{ $t('ai.kbChunks', { n: k.chunkCount }) }}</span>
                      </el-dropdown-item>
                    </template>
                  </el-dropdown-menu>
                </template>
              </el-dropdown>

              <!-- 模型选择：自定义下拉按钮（与「上下文」按钮同一实现）。
                   这里刻意不用 el-select —— 它的内部 selection 在「按内容定宽」的 flex 容器里会塌成 0 宽，
                   表现为只剩一个空的小方框。自定义按钮由内容撑开，宽度天然自适应。 -->
              <el-dropdown ref="modelDropRef" trigger="click" placement="top-start" popper-class="ai-model-dropdown"
                           @command="onPickModel" @visible-change="onModelVisible">
                <button class="composer-pick" type="button">
                  <el-icon :size="12"><Cpu /></el-icon>
                  <span class="composer-pick-tx">{{ currentModelLabel }}</span>
                  <el-icon :size="12" class="composer-pick-caret"><ArrowDown /></el-icon>
                </button>
                <template #dropdown>
                  <div class="ai-dd-head">{{ $t('ai.pickModel') }}</div>
                  <el-dropdown-menu class="ai-model-menu">
                    <el-dropdown-item command="auto" :class="{ active: !selectedModelId || selectedModelId === 'auto' }">
                      <span class="skills-name">{{ $t('ai.modelAuto') }}</span>
                      <el-icon v-if="!selectedModelId || selectedModelId === 'auto'" class="skills-check"><Select /></el-icon>
                    </el-dropdown-item>
                    <el-dropdown-item
                      v-for="m in aiModels"
                      :key="m.id"
                      :command="m.id"
                      :class="{ active: selectedModelId === m.id }"
                    >
                      <span class="skills-name">{{ m.name || m.model }}</span>
                      <el-icon v-if="selectedModelId === m.id" class="skills-check"><Select /></el-icon>
                    </el-dropdown-item>
                  </el-dropdown-menu>
                </template>
              </el-dropdown>

              <!-- 右侧：语音输入 + 主按钮（固定发送图标，不随技能变文案、不加 loading 转圈），等待回答时禁用发送 -->
              <div class="composer-right">
                <!-- 语音输入：点按说话，识别文本实时追加进输入框；仅 Web Speech API 可用时渲染 -->
                <button v-if="voiceUsable" class="voice-btn" :class="{ listening }" type="button"
                        :title="listening ? $t('ai.voiceStop') : $t('ai.voiceStart')" @click="toggleVoice">
                  <el-icon :size="14"><Microphone /></el-icon>
                </button>
                <el-button type="primary" class="composer-run" :disabled="busy || !canRun"
                           :title="runLabel" @click="runSkill">
                  <el-icon><Promotion /></el-icon>
                </el-button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- SQL 快捷验证：AI 给的 SQL 先看执行计划 / 试跑一次（不消耗 AI 调用） -->
    <SqlProbeDialog v-model="probeVisible" :sql="probeSql" :conn-id="ctxConnId"
                    :database="ctxEffectiveDatabase" :mode="probeMode" @insert="onProbeInsert" />
  </div>
</template>

<script setup>
import { ref, reactive, computed, nextTick, onMounted, onBeforeUnmount, watch } from 'vue'
import { saveBlobAs } from '../../utils/useExportTask'
import { ElMessage } from 'element-plus'
import { MagicStick, ChatDotRound, DocumentCopy, Close, Cpu, Notebook, Download, Grid, Star, Aim, DataAnalysis, Loading, ArrowDown, ArrowRight, Select, Coin, Connection, Promotion, Switch, Plus, Edit, Collection, Microphone } from '@element-plus/icons-vue'
import { aiChat, aiChatStream, aiAgent, aiDataDict, aiInsight, aiPatrol, aiWarmup, aiExportDoc, getAiConfig, listConnections, listDatabases, listSchemas, kbList, kbSearch } from '../../api'
import KbIngestPanel from './KbIngestPanel.vue'
import { renderMarkdown, extractCodeBlocks } from '../../utils/markdown'
import { t, te } from '../../utils/i18n'
import { schemaLevelOf } from '../../types'
import SqlProbeDialog from '../../common/SqlProbeDialog.vue'

const props = defineProps({
  conn: Object,
  database: String,
  /** 形态：panel=右侧窄面板；studio=中央工作区（左侧能力导航 + 大内容区） */
  mode: { type: String, default: 'panel' },
  /**
   * 外层「发送到 AI」带进来的载荷：`{ seq, text }`。
   *
   * <p>用 `seq`（自增序号）而不是 `text` 作为触发条件 —— 对**同一张表**连点两次
   * 「发送到 AI」时，文本一模一样，只比文本是不会触发 watch 的，面板会一动不动。
   *
   * <p>只有**中央工作区**那个实例会拿到这个 prop（右侧窄面板不接），
   * 否则右侧面板会一起被填上内容，之后 Ctrl/⌘+K 打开时里面是一句莫名其妙的问题。
   */
  pending: Object
})
/** 是否为「中央工作区」形态 */
const isStudio = computed(() => props.mode === 'studio')
/**
 * panel：斜杠命令要求打开某个面板（巡检 / 治理 / 设置），由外层接管
 * run-plan：「指令」页签解析出的可执行操作，交由外层执行（与命令面板同一套逻辑）
 */
const emit = defineEmits(['insert-sql', 'close', 'panel', 'run-plan', 'open-knowledge'])

const tab = ref('chat')

/**
 * Skills：把「能力」作为可切换的技能，由用户自己选择。
 * 定位：**只放对话替代不了的能力**。写 SQL、优化、诊断、改方言这类需求，
 * 直接对话就能做，因此不再单列；这里保留的是「要读真实表结构 / 要产出交付物」的技能：
 * 数据字典、表健康巡检（纯规则扫描、零 AI 消耗）、表数据洞察，
 * 三者的结果都能导出 Markdown / Word / Excel / PDF。
 */
// ⚠️ 必须是 computed：写死成普通数组只会在模块加载时求值一次，之后切语言不会跟着变
const tabs = computed(() => [
  { key: 'chat', label: t('ai.tabChat'), icon: ChatDotRound, desc: t('ai.tabChatDesc') },
  { key: 'agent', label: t('ai.tabAgent'), icon: Cpu, desc: t('ai.tabAgentDesc') },
  { key: 'dictionary', label: t('ai.tabDictionary'), icon: Notebook, desc: t('ai.tabDictionaryDesc') },
  { key: 'patrol', label: t('ai.tabPatrol'), icon: Star, desc: t('ai.tabPatrolDesc') },
  { key: 'insight', label: t('ai.tabInsight'), icon: Grid, desc: t('ai.tabInsightDesc') },
  { key: 'kbIngest', label: t('ai.tabKbIngest'), icon: Collection, desc: t('ai.tabKbIngestDesc') }
])

/** 当前选中的 Skill */
const currentSkill = computed(() => tabs.value.find(t => t.key === tab.value) || tabs.value[0])

const skillsOpen = ref(false)
/** 三个下拉互斥：每个下拉都持有 ref，以便其它下拉把它收起 */
const skillDropRef = ref(null)
const ctxDropRef = ref(null)
const modelDropRef = ref(null)
const closeSkillDrop = () => skillDropRef.value?.handleClose?.()
const closeCtxDrop = () => ctxDropRef.value?.handleClose?.()
const closeModelDrop = () => modelDropRef.value?.handleClose?.()
/** Skills 下拉展开/收起：同步状态并关闭其它下拉 */
const onSkillVisible = (visible) => {
  skillsOpen.value = visible
  if (!visible) return
  ctxDropRef.value?.handleClose?.()
  closeModelDrop()
}
/** 选择 Skill：切换能力（下拉自动收起） */
const pickSkill = (key) => { tab.value = key }

/** 对话类 Skill（走消息流，而不是一次性结果卡片） */
const isChatSkill = computed(() => tab.value === 'chat' || tab.value === 'agent')

/** 高级技能（结果统一渲染成 Markdown 分析报告，并支持导出文件） */
const isAdvancedSkill = computed(() => ['insight', 'patrol'].includes(tab.value))

/** 知识入库：不走对话流，是一条「上传 → 分析 → 确认」的独立流程 */
const isKbIngest = computed(() => tab.value === 'kbIngest')

// 切换技能时清掉上一技能的结论，避免串台
watch(tab, () => { advResult.value = '' })

/** 统一输入框：所有 Skill 共用，行为由当前 Skill 决定 */
const aiInput = ref('')

/** 各 Skill 的输入框提示语 */
const PLACEHOLDERS = computed(() => ({
  chat: t('ai.phChat'),
  agent: t('ai.phAgent'),
  dictionary: t('ai.phDictionary'),
  patrol: t('ai.phPatrol'),
  insight: t('ai.phInsight')
}))
const composePlaceholder = computed(() => PLACEHOLDERS.value[tab.value] || t('ai.inputPlaceholder'))

/** 当前数据上下文（连接 · 数据库 · 模式），显示在输入框工具条，让「AI 基于哪个库回答」一目了然 */
const contextLabel = computed(() => {
  const conn = (connList.value || []).find(c => String(c.id) === String(ctxConnId.value))
  const name = conn?.name || conn?.type || props.conn?.name || props.conn?.type || ''
  return [name, ctxDatabase.value, ctxSchema.value, ctxTable.value].filter(Boolean).join(' · ')
})

// 高级技能（诊断 / 优化 / 索引 / 洞察 / 巡检）共用一个 Markdown 结果
const advResult = ref('')
const advLoading = ref(false)

/** 输入框实例（点击示例后聚焦用） */
const inputRef = ref(null)

// ===== 知识库引用溯源 =====
// 后端会把召回片段拼进提示词，但接口不回传来源，前端无从得知「这次答话到底用了没用我的资料」。
// 这里在提问的同时并行跑一次同样的检索（同一套打分、同一份配置），把来源摆到回答下方。
// 只读取示，不参与回答生成，失败了也绝不打扰对话。
const kbCfg = ref(null)
/**
 * 知识库范围：**none=不检索（默认）**｜all=全部库｜custom=只在选中的库里检索。
 *
 * 默认改成「不检索」：检索会把库里的片段拼进提示词 —— 问「这张表怎么设计」这类问题时，
 * 无关资料既增加往返耗时，又容易把回答带偏（还会在回答下方摆一串"引用了哪些资料"）。
 * 需要的人自己点一下「全部资料库」即可，属于**用的时候开**，而不是默认开着。
 *
 * 这里**不做持久化**：每次打开助手都回到「不检索」，免得某次的临时选择被当成长期偏好。
 */
const kbScope = ref('none')
const kbPicked = ref([])
const kbOpen = ref(false)
const kbListItems = ref([])

const loadKbMeta = async () => {
  try {
    const d = await kbList()
    kbListItems.value = (d && d.items) || []
    kbCfg.value = (d && d.config) || null
  } catch (e) {
    kbListItems.value = []
    kbCfg.value = null
  }
}

const kbScopeLabel = computed(() => {
  if (kbScope.value === 'none') return t('ai.kbNone')
  if (kbScope.value === 'custom') {
    if (!kbPicked.value.length) return t('ai.kbPick')
    if (kbPicked.value.length === 1) {
      const k = kbListItems.value.find(x => x.id === kbPicked.value[0])
      return k ? k.name : t('ai.kbOne')
    }
    return t('ai.kbN', { n: kbPicked.value.length })
  }
  return kbListItems.value.length ? t('ai.kbAll') : t('ai.kbEmpty')
})

const kbScopeIcon = computed(() => kbScope.value === 'none' ? Close : Collection)

const kbDropRef = ref(null)

const onKbScopeCommand = (cmd) => {
  if (cmd === 'all' || cmd === 'none') { kbScope.value = cmd; return }
  if (cmd === 'custom') {
    kbScope.value = 'custom'
    // 首次切到「指定」时默认全选：从「全部」过来的人多半只是想排除掉某几个
    if (!kbPicked.value.length) kbPicked.value = kbListItems.value.map(k => k.id)
    return
  }
  if (typeof cmd === 'string' && cmd.startsWith('pick:')) toggleKbPick(cmd.slice(5))
}

/** 三个下拉互斥：同时铺开两层浮层会很乱 */
const onKbVisible = (v) => {
  kbOpen.value = v
  if (!v) return
  ctxDropRef.value?.handleClose?.()
  closeModelDrop()
}

const toggleKbPick = (id) => {
  kbScope.value = 'custom'
  const i = kbPicked.value.indexOf(id)
  if (i >= 0) kbPicked.value.splice(i, 1)
  else kbPicked.value.push(id)
}

/** 本次提问要检索哪些库：null = 全部；[] = 不检索 */
const kbIdsForQuery = () => {
  if (kbScope.value === 'none') return []
  if (kbScope.value === 'custom') return kbPicked.value.slice()
  return null
}

/** 能不能显示引用：总开关关了、或用户选了不检索，就别摆一副「用了资料」的样子 */
const kbCiteEnabled = computed(() => {
  if (kbScope.value === 'none') return false
  if (kbCfg.value && kbCfg.value.enabled === false) return false
  return true
})

/** 检索一次，拿回来源信息（失败静默：引用只是锦上添花，不值得打断对话） */
const recallSources = async (query) => {
  if (!kbCiteEnabled.value || !query) return []
  try {
    const res = await kbSearch({ query, kbIds: kbIdsForQuery(), topK: 0 })
    return (res && res.hits) || []
  } catch (e) {
    return []
  }
}

/** 相关度条：只做视觉参照，按 0~1 截断即可 */
const srcPercent = (score) => Math.max(4, Math.min(100, Math.round(Number(score || 0) * 100))) + '%'
/** 空态功能介绍：三句话说清它和「普通聊天」的区别 */
const WELCOME_FEATURES = computed(() => [
  { name: t('ai.featLibrary'), desc: t('ai.featLibraryDesc'), icon: Coin },
  { name: t('ai.featQuery'), desc: t('ai.featQueryDesc'), icon: Cpu },
  { name: t('ai.featDeliver'), desc: t('ai.featDeliverDesc'), icon: Download }
])
/** 空态示例：点一下就直接提问（省掉「填入 → 再回车」两步） */
const WELCOME_EXAMPLES = computed(() => [
  t('ai.ex1'),
  t('ai.ex2'),
  t('ai.ex3'),
  t('ai.ex4'),
  t('ai.ex5'),
  t('ai.ex6')
])
/** 点击示例：切到自由对话并立即发送 */
const useExample = (text) => {
  tab.value = 'chat'
  aiInput.value = text
  nextTick(() => runSkill())
}

/** 除「自由对话」外的能力（空态面板的能力入口） */
const advancedTabs = computed(() => tabs.value.filter(t => t.key !== 'chat'))

// 不保留历史对话：面板每次打开均为干净对话
const messages = ref([])

const chatLoading = ref(false)
/** 结果区滚动容器（新消息时滚到底） */
const chatBox = ref(null)

/**
 * Markdown 渲染结果缓存。
 * 流式回复时每个增量都会触发重渲染，若每条消息都重新解析 Markdown，
 * 长对话下开销会随消息数线性放大；这里按原文缓存 HTML，
 * 只有「正在增长的那一条」会真正重新解析，历史消息全部命中缓存。
 */
/** 等待模型返回的秒数：模型首字可能要等十几秒，用实时秒数告诉用户「在跑，不是卡死」 */
const waitingSec = ref(0)
let waitTimer = null
const startWait = () => {
  stopWait()
  waitingSec.value = 0
  waitTimer = setInterval(() => { waitingSec.value += 1 }, 1000)
}
const stopWait = () => {
  if (waitTimer) { clearInterval(waitTimer); waitTimer = null }
}
onBeforeUnmount(stopWait)

/** 进行中的流式对话（用户可点「停止生成」中断；已接收的内容保留并标注中断） */
let streamCtrl = null
const stopStream = () => {
  if (!streamCtrl) return
  const c = streamCtrl
  streamCtrl = null
  try { c.abort() } catch { /* ignore */ }
}

const mdCache = new Map()
/** markdown → HTML（带缓存）。sqlActions=true 时每个 SQL 代码块会带上操作按钮，
 *  与 extractCodeBlocks 的索引一一对应；流式过程中先不给按钮（内容还没稳定）。 */
const renderMd = (text, sqlActions = false) => {
  const src = text == null ? '' : String(text)
  const key = (sqlActions ? '1' : '0') + '\u0000' + src
  const hit = mdCache.get(key)
  if (hit !== undefined) return hit
  const html = renderMarkdown(src, { sqlActions })
  if (mdCache.size > 300) mdCache.clear()
  mdCache.set(key, html)
  return html
}

// ===== 斜杠命令：在输入框里敲 / 直接唤起功能，无需用文字描述需求 =====

/** 各能力页签的一句话说明（作为命令副标题） */
const TAB_DESC = computed(() => ({
  agent: t('ai.hintAgent'),
  insight: t('ai.hintInsight'),
  patrol: t('ai.hintPatrol'),
  dictionary: t('ai.hintDictionary'),
  kbIngest: t('ai.hintKbIngest')
}))

/**
 * 面板直达命令：本组件只负责发出意图，由外层 MainView 打开对应弹窗。
 * 注意这里**不列「命令面板」**——用户已经在 AI 助手内，
 * 再跳去命令面板是绕路（命令面板的价值在于「不打开助手也能全局唤起」）。
 */
const PANEL_CMDS = computed(() => [
  // 数据治理的四个页签直接点名，省掉「先打开治理再切页签」两步
  { key: 'sensitive', label: t('ai.cmdSensitive'), desc: t('ai.cmdSensitiveDesc'), icon: Aim },
  { key: 'capacity', label: t('ai.cmdCapacity'), desc: t('ai.cmdCapacityDesc'), icon: Grid },
  { key: 'quality', label: t('qa.rulesTitle'), desc: t('ai.cmdQualityDesc'), icon: Edit },
  { key: 'analysis', label: t('qa.title'), desc: t('ai.cmdAnalysisDesc'), icon: DataAnalysis },
  { key: 'compare', label: t('ai.cmdCompare'), desc: t('ai.cmdCompareDesc'), icon: Switch },
  { key: 'sync', label: t('ai.cmdSync'), desc: t('ai.cmdSyncDesc'), icon: Promotion },
  { key: 'newConn', label: t('ai.cmdNewConn'), desc: t('ai.cmdNewConnDesc'), icon: Plus },
  { key: 'editConn', label: t('ai.cmdEditConn'), desc: t('ai.cmdEditConnDesc'), icon: Edit }
])

/** 全部命令 = 技能切换（由 tabs 自动派生）+ 面板直达，按分组顺序排列 */
const slashCommands = computed(() => [
  ...tabs.filter(t => t.key !== 'chat').map(t => ({
    key: 'tab:' + t.key, label: t.label, desc: TAB_DESC.value[t.key] || '', icon: t.icon,
    group: t('ai.cmdGroupSkills'), action: 'tab', target: t.key
  })),
  ...PANEL_CMDS.value.map(c => ({ ...c, key: 'panel:' + c.key, group: t('ai.cmdGroupPanel'), action: 'panel', target: c.key }))
])

const slashOpen = ref(false)
const slashIndex = ref(0)

/** 仅当输入以 / 开头、且后面还没敲空格时进入命令模式（避免正常句子里带 / 时误触发） */
const onComposeInput = () => {
  slashOpen.value = /^\/\S*$/.test(aiInput.value)
  if (slashOpen.value) slashIndex.value = 0
}

// ===== 语音输入：浏览器 Web Speech API（Chrome / Edge 内置语音识别，零依赖、无需后端与密钥） =====
const voiceSupported = !!(window.SpeechRecognition || window.webkitSpeechRecognition)
// Electron 的 Chromium 默认未配置语音服务密钥，识别必然失败 —— 桌面版直接不显示语音按钮
const voiceInDesktop = navigator.userAgent.toLowerCase().includes('electron')
// 仅当 API 存在且非桌面版时才展示按钮：不可用时不留按钮
const voiceUsable = voiceSupported && !voiceInDesktop
const listening = ref(false)
let recognizer = null
let voiceBase = ''   // 开始识别时输入框已有的内容，识别文本追加在其后
let voiceFinal = ''  // 已确定的识别片段（interim 之外的部分，逐段累积）
const stopVoice = () => {
  listening.value = false
  if (recognizer) { try { recognizer.stop() } catch (_) { /* 已停止则忽略 */ } recognizer = null }
}
const toggleVoice = () => {
  if (listening.value) { stopVoice(); return }
  const SR = window.SpeechRecognition || window.webkitSpeechRecognition
  if (!SR) return
  const rec = new SR()
  recognizer = rec
  rec.lang = 'zh-CN'
  rec.continuous = true      // 持续识别，支持长句
  rec.interimResults = true  // 中间结果实时上屏，像打字一样有反馈
  voiceBase = aiInput.value ? aiInput.value.replace(/\s+$/, '') + ' ' : ''
  voiceFinal = ''
  rec.onresult = (e) => {
    let fin = '', interim = ''
    for (let i = e.resultIndex; i < e.results.length; i++) {
      const t = e.results[i][0].transcript
      if (e.results[i].isFinal) fin += t; else interim += t
    }
    voiceFinal += fin
    aiInput.value = voiceBase + voiceFinal + interim
    onComposeInput() // 与手动输入同一路径：维护斜杠命令浮层状态
  }
  rec.onerror = (e) => {
    // no-speech / aborted 属正常暂停，onend 会续上，不必打扰用户
    if (e.error === 'no-speech' || e.error === 'aborted') return
    if (e.error === 'not-allowed' || e.error === 'service-not-allowed') {
      ElMessage.error(t('ai.micDenied'))
    } else {
      ElMessage.warning(t('ai.voiceUnavailable', { detail: e.error }))
    }
    stopVoice()
  }
  rec.onend = () => {
    // 识别完一段后 Chrome 会自动结束；只要用户没有主动停止就续上，支持连续说话
    if (listening.value && recognizer === rec) {
      try { rec.start() } catch (_) { listening.value = false }
    }
  }
  try { rec.start(); listening.value = true } catch (_) { ElMessage.warning(t('ai.voiceStartFailed')) }
}
onBeforeUnmount(stopVoice)

/** 按关键词过滤命令（匹配名称 / 说明 / key） */
const slashFiltered = computed(() => {
  if (!slashOpen.value) return []
  const kw = aiInput.value.slice(1).trim().toLowerCase()
  if (!kw) return slashCommands.value
  return slashCommands.value.filter(c =>
    c.label.toLowerCase().includes(kw)
    || c.desc.toLowerCase().includes(kw)
    || c.key.toLowerCase().includes(kw))
})

/** 执行选中的命令 */
const pickSlash = (cmd) => {
  if (!cmd) return
  slashOpen.value = false
  aiInput.value = ''
  if (cmd.action === 'tab') {
    tab.value = cmd.target
    ElMessage.success(t('ai.switchedTo', { name: cmd.label }))
  } else {
    emit('panel', cmd.target)
  }
}

/** 输入框键盘：命令浮层开启时优先响应 ↑↓ / Enter / Esc；否则 Enter 执行当前 Skill */
const onComposeKeydown = (e) => {
  if (slashOpen.value && slashFiltered.value.length) {
    const n = slashFiltered.value.length
    if (e.key === 'ArrowDown') { e.preventDefault(); slashIndex.value = (slashIndex.value + 1) % n; return }
    if (e.key === 'ArrowUp') { e.preventDefault(); slashIndex.value = (slashIndex.value - 1 + n) % n; return }
    if (e.key === 'Escape') { e.preventDefault(); slashOpen.value = false; return }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      pickSlash(slashFiltered.value[slashIndex.value])
      return
    }
  }
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault()
    runSkill()
  }
}

/** 供外层（如 Ctrl/⌘+K）唤起：直接切到「自由对话」——泛化指令都交给对话处理 */
const openCommand = () => { tab.value = 'chat' }

/**
 * 供外层把一条问题带进面板：切到对话页签并填好输入，**不自动发送**——
 * 既不替用户花掉一次调用，也让他把问题补完整。
 *
 * 注：原先对象树右键的「在 AI 助手里继续问」入口已移除，目前暂无调用方；
 * 作为面板的公开能力保留，后续若再需要「带上下文进面板」可直接复用。
 */
const openWith = (text) => {
  tab.value = 'chat'
  aiInput.value = text || ''
  nextTick(() => inputRef.value?.focus?.())
}

defineExpose({ openCommand, openWith })

/**
 * 已处理过的载荷序号。
 *
 * <p>放在模块作用域（不是 setup 内）是刻意的：页签切走再切回来，组件会**重新挂载**，
 * setup 里的变量会被重置，那样"上一次发送的问题"会把这期间用户敲进去的内容再覆盖一遍。
 *
 * <p>配合 `immediate: true`：面板**首次挂载时**载荷可能已经在了
 * （外层是先设载荷、再开页签，此刻组件还没创建），非 immediate 的 watch 收不到。
 */
let handledPendingSeq = 0
const applyPending = () => {
  const p = props.pending
  if (!p?.seq || p.seq === handledPendingSeq) return
  handledPendingSeq = p.seq
  // 聚焦表跟着载荷走；载荷里没有就清掉，免得上一次的表名留在工具条上
  ctxTable.value = p.table || ''
  if (p.text) openWith(p.text)
}
watch(() => props.pending?.seq, applyPending, { immediate: true })

const aiModels = ref([])
const selectedModelId = ref('')

/** 模型下拉按钮上显示的文案（auto / 未指定时显示 Auto） */
const currentModelLabel = computed(() => {
  if (!selectedModelId.value || selectedModelId.value === 'auto') return 'Auto'
  const m = aiModels.value.find(x => x.id === selectedModelId.value)
  return (m && (m.name || m.model)) || 'Auto'
})

/** 选择模型：命令即模型 id（auto 为后端自动挑选） */
const onPickModel = (cmd) => { if (cmd) selectedModelId.value = cmd }

/** 模型下拉展开时收起另两个下拉（同一时刻只开一个） */
const onModelVisible = (visible) => {
  if (!visible) return
  closeSkillDrop()
  closeCtxDrop()
}

/** 结果导出中（Markdown / Word / Excel / PDF），用于禁用导出按钮 */
const exporting = ref('')
const connList = ref([])

const loadConnList = async () => {
  try { connList.value = (await listConnections()) || [] } catch (e) { /* ignore */ }
}

// ===== 上下文（连接 / 数据库 / 模式）：工具条上可逐级选择 =====
const ctxConnId = ref('')
const ctxDatabase = ref('')
const ctxSchema = ref('')
/**
 * 右键「发送到 AI」带进来的**聚焦表**（只在上下文标签上多显示一级）。
 *
 * <p>为什么不并进 `ctxEffectiveDatabase`：后端所有 AI 接口认的都是「连接 + 库」，
 * 表不是查询参数 —— 表名本来就在提问文本里。这里多显示一级，是为了消除
 * 「问题里写 dify.orders、工具条却只到 dify」这种看着像选错了库的错觉。
 */
const ctxTable = ref('')
const ctxOpen = ref(false)
/** 上下文下拉实例（与 Skills、模型下拉互斥） */
const ctxFlatItems = ref([])
const ctxCache = ref(new Map())

/** 当前上下文所选连接（用于判断是否需要模式层级、显示连接名）；
 *  连接列表尚未加载完时回退到父组件传入的连接，避免 schema 判断依赖加载时序 */
const ctxConn = computed(() => (connList.value || []).find(c => String(c.id) === String(ctxConnId.value)) || props.conn)
/** 空态面板展示的连接名 */
const ctxConnName = computed(() => ctxConn.value?.name || ctxConn.value?.type || t('ai.noConnSelected'))
/** 该连接类型是否需要模式层级（SQL Server / PostgreSQL / Kingbase 等） */
const ctxNeedSchema = computed(() => {
  const t = ctxConn.value?.type
  return !!t && schemaLevelOf(t) === 'schema'
})
/** 传给后端的库标识：模式层级类型用「库.模式」复合串，其余类型原样 */
const ctxEffectiveDatabase = computed(() => {
  if (!ctxDatabase.value) return ''
  if (ctxNeedSchema.value && ctxSchema.value) return `${ctxDatabase.value}.${ctxSchema.value}`
  return ctxDatabase.value
})
/** 当前选中项（取最深一层）对应的下拉 value，用于高亮 */
const ctxSelectedValue = computed(() => {
  if (ctxSchema.value) return 'sc:' + ctxSchema.value
  if (ctxDatabase.value) return 'db:' + ctxDatabase.value
  if (ctxConnId.value) return 'conn:' + ctxConnId.value
  return ''
})
/** 层级图标：连接 / 数据库 / 模式 */
const ctxLevelIcon = (item) => (item.level === 0 ? Connection : item.level === 1 ? Coin : Grid)

// 数据源变化时后台「预热」表结构上下文：
// 冷缓存下首次构建要跑十几次元数据/样例查询（实测可达 18s），提前热好，
// 用户真正提问时首字就只需等模型本身。
let warmTimer = null
const warmCtx = () => {
  if (!ctxConnId.value) return
  clearTimeout(warmTimer)
  warmTimer = setTimeout(() => {
    aiWarmup({ connectionId: ctxConnId.value, database: ctxEffectiveDatabase.value }).catch(() => { /* 预热失败不影响使用 */ })
  }, 300)
}
watch([ctxConnId, ctxEffectiveDatabase], warmCtx)
onBeforeUnmount(() => clearTimeout(warmTimer))

/** 上下文下拉展开/收起：同步状态、关闭其它下拉、首次打开时加载连接列表；
 *  展开后把当前已选的层级也一并展开出来，方便确认「AI 用的就是左侧选的库」 */
const onCtxVisible = async (visible) => {
  ctxOpen.value = visible
  if (!visible) return
  closeSkillDrop()
  closeModelDrop()
  if (ctxFlatItems.value.length === 0) await loadCtxConnections()
  await revealCtxSelection()
}

/** 展开下拉中指定 value 的项（已展开则跳过） */
const expandCtxByValue = async (value) => {
  const idx = findCtxIndex(value)
  if (idx < 0 || ctxFlatItems.value[idx].expanded) return
  await expandCtxItem(idx)
}

/** 把当前选中的连接 / 库在下拉里逐级展开，便于看到并确认当前上下文 */
const revealCtxSelection = async () => {
  if (!ctxConnId.value) return
  await expandCtxByValue('conn:' + ctxConnId.value)
  if (ctxDatabase.value) await expandCtxByValue('db:' + ctxDatabase.value)
}

/**
 * 与父组件传入的「当前连接 / 库」保持同步（左侧树点库、编辑器切换库都会走到这里）。
 * 两个要点：
 * 1) 必须 immediate：面板挂载时父组件往往已经有值（例如先点库、再打开 AI 页签），
 *    只在「变化时」同步会让这里永远只显示连接名而没有库；
 * 2) schema 层级类型（SQL Server / PostgreSQL / Kingbase）下，父组件的 database 是
 *    「库.模式」复合串，要拆成库 + 模式两层，否则会拼出「库.模式.模式」。
 */
const syncCtxFromProps = () => {
  ctxConnId.value = props.conn?.id || ''
  const raw = props.database || ''
  const type = props.conn?.type || ctxConn.value?.type || ''
  const dot = raw.indexOf('.')
  if (raw && dot > 0 && schemaLevelOf(type) === 'schema') {
    ctxDatabase.value = raw.slice(0, dot)
    ctxSchema.value = raw.slice(dot + 1)
  } else {
    ctxDatabase.value = raw
    ctxSchema.value = ''
  }
}
watch([() => props.conn?.id, () => props.database], syncCtxFromProps, { immediate: true })

/** 去掉层级前缀（conn: / db: / sc:） */
const ctxUnwrap = (v) => String(v || '').replace(/^(conn|db|sc):/, '')

/** 构造上下文下拉项对象 */
const makeCtxItem = (value, label, level, expandable, parentValue) => ({
  value, label, level, expandable, expanded: false, loading: false, parentValue
})

/** 加载顶层连接列表 */
const loadCtxConnections = async () => {
  if (!connList.value.length) await loadConnList()
  ctxFlatItems.value = (connList.value || []).map(c => makeCtxItem('conn:' + c.id, c.name || c.id, 0, true, null))
}

/** 指定连接是否还需要模式层级 */
const connNeedsSchema = (connId) => {
  const conn = (connList.value || []).find(c => String(c.id) === String(connId))
  return schemaLevelOf(conn?.type || '') === 'schema'
}

/** 根据 value 找到当前在下拉列表中的索引 */
const findCtxIndex = (value) => ctxFlatItems.value.findIndex(i => i.value === value)

/** 查找指定项的所有子项在当前列表中的范围 [start, end) */
const findCtxChildrenRange = (idx) => {
  const parentLevel = ctxFlatItems.value[idx].level
  let end = idx + 1
  while (end < ctxFlatItems.value.length && ctxFlatItems.value[end].level > parentLevel) end++
  return { start: idx + 1, end }
}

/** 折叠某一项：移除其所有子孙项 */
const collapseCtxItem = (idx) => {
  const item = ctxFlatItems.value[idx]
  const { start, end } = findCtxChildrenRange(idx)
  if (end > start) ctxFlatItems.value.splice(start, end - start)
  item.expanded = false
}

/** 异步加载某一项的子节点：连接 → 数据库；数据库 → 模式（仅模式层级类型） */
const loadCtxChildren = async (item) => {
  const cached = ctxCache.value.get(item.value)
  if (cached) return cached
  let children = []
  try {
    if (item.level === 0) {
      const connId = ctxUnwrap(item.value)
      const dbs = await listDatabases(connId)
      const needSchema = connNeedsSchema(connId)
      children = (dbs || []).map(d => makeCtxItem('db:' + d, String(d), 1, needSchema, item.value))
    } else if (item.level === 1) {
      const connId = ctxUnwrap(item.parentValue)
      const db = ctxUnwrap(item.value)
      const raw = await listSchemas(connId, db)
      children = (raw || [])
        .map(s => (typeof s === 'string' ? s : (s.name || s.schema || '')))
        .filter(Boolean)
        .map(s => makeCtxItem('sc:' + s, String(s), 2, false, item.value))
    }
  } catch (e) { /* ignore */ }
  ctxCache.value.set(item.value, children)
  return children
}

/** 展开某一项：加载并插入子节点，返回是否确实存在子节点 */
const expandCtxItem = async (idx) => {
  const item = ctxFlatItems.value[idx]
  item.loading = true
  const children = await loadCtxChildren(item)
  item.loading = false
  if (children.length) {
    ctxFlatItems.value.splice(idx + 1, 0, ...children)
    item.expanded = true
  }
  return children.length > 0
}

/** 根据当前项及父链设置上下文 */
const selectCtxByItem = (item) => {
  const chain = []
  let cur = item
  while (cur) {
    chain.unshift(cur)
    if (!cur.parentValue) break
    cur = ctxFlatItems.value.find(i => i.value === cur.parentValue)
  }
  ctxConnId.value = ctxUnwrap(chain[0]?.value)
  ctxDatabase.value = ctxUnwrap(chain[1]?.value)
  ctxSchema.value = ctxUnwrap(chain[2]?.value)
  // 用户**主动**在工具条上换了上下文：之前那个"聚焦表"不再适用，清掉。
  // 不清的话会出现「已经切到别的库了，提问还被"只基于表 X 回答"绑着」。
  // （右键「发送到 AI」那条路不走这里 —— 它直接设 ctxTable，见 applyPending）
  ctxTable.value = ''
}

/** 处理上下文下拉命令：
 * 可展开项既设置当前层级上下文，又展开/折叠下级；
 * 叶子项设置上下文并关闭下拉。 */
const onCtxCommand = async (value) => {
  const idx = findCtxIndex(value)
  if (idx < 0) return
  const item = ctxFlatItems.value[idx]
  selectCtxByItem(item)
  if (item.expandable) {
    if (item.expanded) { collapseCtxItem(idx); return }
    const hasChildren = await expandCtxItem(idx)
    // 兜底：理论上模式层级类型必能取到模式，若无下级则直接收起
    if (!hasChildren) closeCtxDrop()
    return
  }
  closeCtxDrop()
}

// 数据字典（输入统一取自 aiInput）
const dictLoading = ref(false)
const dictResult = ref('')

const loadAiModels = async () => {
  try {
    const cfg = await getAiConfig()
    if (cfg && Array.isArray(cfg.models)) {
      aiModels.value = cfg.models
      // 默认使用 Auto 自动选择模式（后端自动挑选最优可用模型，失败自动切换）
      if (!selectedModelId.value) selectedModelId.value = 'auto'
    }
  } catch (e) { /* ignore */ }
}

onMounted(() => { loadAiModels() })

// ===== 面板宽度拖拽 =====
const MIN_W = 300, MAX_W = 800
const panelWidth = ref(parseInt(localStorage.getItem('dbmind-panel-width') || '420', 10) || 420)

// 拖拽期间挂在 window 上的监听：组件若在拖拽中途被卸载（关页签 / 切形态），
// 只在 mouseup 里移除就会残留（闭包还持有组件），这里留一个卸载兜底
let resizeCleanup = null
const startResize = (e) => {
  const startX = e.clientX
  const startW = panelWidth.value
  const onMove = (ev) => {
    let w = startW + (startX - ev.clientX)
    w = Math.min(MAX_W, Math.max(MIN_W, w))
    panelWidth.value = w
  }
  const onUp = () => {
    window.removeEventListener('mousemove', onMove)
    window.removeEventListener('mouseup', onUp)
    resizeCleanup = null
    localStorage.setItem('dbmind-panel-width', String(panelWidth.value))
  }
  window.addEventListener('mousemove', onMove)
  window.addEventListener('mouseup', onUp)
  resizeCleanup = onUp
}
onBeforeUnmount(() => {
  if (resizeCleanup) resizeCleanup()
  stopStream() // 卸载时中断进行中的流式请求，别让它在后台继续读
})

/** 巡检结果 → Markdown（用表格承载，导出的 Excel 才是规整的明细表） */
const renderPatrol = (res) => {
  const issues = res.issues || []
  const head = t('ai.patrolHead', { scanned: res.scanned || 0, total: res.totalTables || 0 }) +
    t('ai.patrolFound', { high: res.high || 0, medium: res.medium || 0, low: res.low || 0 })
  if (!issues.length) return t('ai.patrolNoIssue', { head })
  // 级别名：字典里有就用译文，没有就原样（后端以后新增的级别不会显示成键名）
const LEVELS = { high: 'ai.lv.high', medium: 'ai.lv.medium', low: 'ai.lv.low', info: 'ai.lv.info' }
const levelLabel = (key) => (key && te(LEVELS[key])) ? t(LEVELS[key]) : (key || '')
  const esc = (s) => String(s == null ? '' : s).replace(/\|/g, '\\|').replace(/\n/g, ' ')
  const rows = issues.map(i => '| ' + [
    esc(i.table), esc(i.column && i.column !== '-' ? i.column : ''),
    esc(levelLabel(i.level)), esc(i.message)
  ].join(' | ') + ' |')
  return t('ai.patrolTitle', { head })
    + t('ai.patrolTableHead') + rows.join('\n')
}

/** 高级技能统一执行：表健康巡检 / 表数据洞察（结果都是 Markdown，可直接导出文件） */
const runAdvancedSkill = async () => {
  const text = aiInput.value.trim()
  if (tab.value === 'insight' && !text) { ElMessage.warning(t('ai.needTableName')); return }
  if (!ctxConnId.value) { ElMessage.warning(t('ai.needConn')); return }
  advLoading.value = true
  advResult.value = ''
  try {
    if (tab.value === 'insight') {
      const res = await aiInsight({
        connectionId: ctxConnId.value, database: ctxEffectiveDatabase.value,
        table: text, question: '', modelId: selectedModelId.value
      })
      if (!res?.success) throw new Error(res?.message || t('ai.analyzeFailed'))
      advResult.value = (res.content || '') +
        (res.stats ? t('ai.statsSection', { stats: res.stats }) : '')
    } else if (tab.value === 'patrol') {
      // 不传 maxTables：后端按「整库」扫描（上限 500 张）
      const res = await aiPatrol({
        connectionId: ctxConnId.value, database: ctxEffectiveDatabase.value
      })
      if (!res?.success) throw new Error(res?.message || t('ai.patrolFailed'))
      advResult.value = renderPatrol(res)
    }
  } catch (e) {
    ElMessage.error(e?.message || e?.toString?.() || t('ai.runFailed'))
  }
  advLoading.value = false
}

/**
 * 聚焦表时给提问补一句范围约束 —— 「选中了表就基于表回答」（用户明确要求）。
 *
 * <p>只在 `ctxTable` 有值时生效（右键表 →「发送到 AI」带进来的那一级）；
 * 没聚焦表时**一个字都不加**，普通对话保持原样。
 *
 * <p>加在 `prompt` 上、而不是拼进输入框：输入框里显示的始终是用户自己写的问题。
 * 拼进去的话，用户会看到一行自己没敲过的字，想删还得手动删。
 *
 * <p>**不做去重**：预置问题里的"请分析表 dify.orders：…"是**任务**，
 * 这句是**范围**，两句各司其职；去重反而会让右键进来的那次丢掉范围约束。
 */
/** token 数的展示格式：千分位（1234 → 1,234）。缺字段显示 —，**不显示 0**（0 与"没给"是两回事）。 */
const fmtTokens = (n) => (n === 0 || n) ? Number(n).toLocaleString() : '—'

const withTableScope = (text) => {
  const tbl = String(ctxTable.value || '').trim()
  if (!tbl) return text
  const full = ctxEffectiveDatabase.value ? ctxEffectiveDatabase.value + '.' + tbl : tbl
  return text + t('ai.scopePrefix', { table: full }) + t('ai.scopeSuffix')
}

const sendChat = async () => {
  const text = aiInput.value.trim()
  if (!text || chatLoading.value) return
  // 「智能分析」页签走 Agent（带只读工具），其余走普通对话（优先流式）
  const useAgent = tab.value === 'agent'
  // 多轮上下文：本次提问之前的对话轮次。
  // 只带最近 8 条（约 4 轮）——历史越长，请求体越大、模型首字越慢，收益却很低
  const history = messages.value
    .filter(m => (m.role === 'user' || m.role === 'assistant') && m.content)
    .slice(-8)
    .map(m => ({ role: m.role, content: m.content }))
  messages.value.push({ role: 'user', content: text })
  aiInput.value = ''
  chatLoading.value = true
  startWait()
  scrollBottom()
  // 与回答并行跑一次检索拿来源：不阻塞、失败也不影响对话，只是把「用了什么资料」摆出来
  const srcPromise = recallSources(text)
  const payload = {
    // 聚焦表时带上范围约束：Agent 与普通对话（流式 / 回退）都走这一个 payload，
    // 在这里套一层即可全覆盖
    prompt: withTableScope(text),
    connectionId: ctxConnId.value,
    database: ctxEffectiveDatabase.value,
    modelId: selectedModelId.value,
    history,
    // 知识库范围：null=全部库，[]=本次不注入，[...]=只在这些库里找。
    // 必须随请求发给后端，只在界面上控制等于没控制。
    kbIds: kbIdsForQuery()
  }
  try {
    if (useAgent) {
      const res = await aiAgent(payload)
      if (res && res.success) {
        const m = reactive({ role: 'assistant', content: res.content, steps: res.steps || [], sources: [], usage: res.usage || null })
        messages.value.push(m)
        srcPromise.then(s => { m.sources = s })
      } else {
        messages.value.push({ role: 'assistant', content: '❌ ' + (res?.message || t('ai.requestFailed')) })
      }
    } else {
      // 流式优先：逐段追加；不可用时回退普通请求。
      // 两个关键点（否则会「后端早返回完，界面还在慢慢磨」）：
      // 1) msg 必须是 reactive 代理：push 进数组的原始对象再直接改属性不会触发重渲染，
      //    之前全靠 waitingSec 秒表每秒顺带触发一次，结束时 streaming=false 甚至永远不生效；
      // 2) 分片极细（实测约 1~3 字一片，一条回答上百片），按 60ms 合批刷新，
      //    避免每片都全量重解析 Markdown + 重建 innerHTML 把主线程打满。
      const msg = reactive({ role: 'assistant', content: '', streaming: true })
      messages.value.push(msg)
      chatLoading.value = false
      let acc = ''
      let flushTimer = null
      const flush = () => {
        flushTimer = null
        if (msg.content === acc) return
        msg.content = acc
        scrollBottom()
      }
      const finalFlush = () => {
        if (flushTimer !== null) { clearTimeout(flushTimer); flushTimer = null }
        flush()
      }
      try {
        streamCtrl = new AbortController()
        await aiChatStream(payload, (d) => {
          acc += d
          if (flushTimer !== null) return
          flushTimer = setTimeout(flush, 60)
        }, { signal: streamCtrl.signal, onUsage: (u) => { msg.usage = u } })
        finalFlush()
        if (!acc) throw new Error('empty-stream')
        msg.streaming = false
        streamCtrl = null
        scrollBottom()
      } catch (e) {
        streamCtrl = null
        finalFlush()
        if (acc) {
          // 已收到部分内容：保留并标注中断，避免界面一直停在「生成中」
          msg.streaming = false
          msg.content = acc + '\n\n> ⏱ ' + t('ai.interrupted', { detail: (e?.message || t('ai.responseInterrupted')) })
          scrollBottom()
        } else {
          messages.value.pop()
          chatLoading.value = true
          const res = await aiChat(payload)
          if (res && res.success) {
            const m = reactive({ role: 'assistant', content: res.content, sources: [], usage: res.usage || null })
            messages.value.push(m)
            srcPromise.then(s => { m.sources = s })
          } else {
            messages.value.push({ role: 'assistant', content: '❌ ' + (res?.message || t('ai.requestFailed')) })
          }
        }
      }
    }
  } catch (e) {
    messages.value.push({ role: 'assistant', content: '❌ ' + (e?.message || e?.toString?.() || t('ai.requestFailed')) })
  }
  chatLoading.value = false
  stopWait()
  scrollBottom()
}

const scrollBottom = () => nextTick(() => {
  if (chatBox.value) chatBox.value.scrollTop = chatBox.value.scrollHeight
})

const copySql = async (text) => {
  try { await navigator.clipboard.writeText(text); ElMessage.success(t('ai.copied')) } catch (e) { ElMessage.error(t('sqlq.copyFailed')) }
}

/** Agent 工具名 → 中文标签：轨迹里显示中文，用户才知道它「真的查了库」 */
// 工具名只有代码（list_tables 这类）：前端按代码查字典覆盖，收录过的走译文、
// 没收录的（后端以后新增的工具）原样显示，不会把键名甩到界面上
const toolLabel = (ty) => (ty && te('ai.tool.' + ty)) ? t('ai.tool.' + ty) : ty

/** SQL 快捷验证弹窗：plan = 看执行计划，run = 试跑取前 100 行（两者都不消耗 AI 调用） */
const probeVisible = ref(false)
const probeMode = ref('run')
const probeSql = ref('')
/** 弹窗里点「插入编辑器」：只写入不自动执行，跑不跑由用户决定 */
const onProbeInsert = (text) => {
  probeVisible.value = false
  emit('insert-sql', text, false)
}

/**
 * markdown 里每个 SQL 代码块的操作按钮走事件委托：
 * v-html 出来的内容无法绑定 Vue 事件，只能靠 data-sql-act / data-sql-idx 分发。
 * 索引与 renderMarkdown(src, { sqlActions: true }) 生成的按钮严格对应，
 * 所以这里用同一份原文重新抽取 SQL。
 */
const onMdSqlAction = (e, md) => {
  const btn = e.target instanceof Element ? e.target.closest('[data-sql-act]') : null
  if (!btn) return
  e.preventDefault()
  e.stopPropagation()
  const block = extractCodeBlocks(md)[Number(btn.dataset.sqlIdx)]
  const text = block ? block.code : ''
  if (!text) return
  const act = btn.dataset.sqlAct
  if (act === 'run' || act === 'plan') {
    // 先验证再用：只读护栏在后端，这里只负责把结果摊给用户看
    probeMode.value = act === 'plan' ? 'plan' : 'run'
    probeSql.value = text
    probeVisible.value = true
    return
  }
  if (act === 'copy') copySql(text)
  else emit('insert-sql', text, true) // 第二个参数 true：插入编辑器后直接执行
}

const doDataDict = async () => {
  if (!ctxConnId.value) { ElMessage.warning(t('ai.needConn')); return }
  dictLoading.value = true
  dictResult.value = ''
  try {
    // 中英文逗号都当分隔符：中文输入法下很容易打出「，」，对用户来说和「,」是一回事（这是输入解析，不是界面文案）
  const tables = aiInput.value.split(/[,，\s]+/).map(s => s.trim()).filter(Boolean)
    const res = await aiDataDict({
      connectionId: ctxConnId.value,
      database: ctxEffectiveDatabase.value,
      tables,
      modelId: selectedModelId.value
    })
    if (res && res.success) dictResult.value = res.content || ''
    else ElMessage.error(res?.message || t('ai.genFailed'))
  } catch (e) {
    ElMessage.error(e?.message || e?.toString?.() || t('ai.genFailed'))
  }
  dictLoading.value = false
}

// 导出走统一的「用户选位置」（原来这里自己造 <a download>，只能落到下载目录）

/** axios 以 blob 接收时，错误响应体也是 Blob，这里还原后端的 message */
const blobErrMsg = async (e) => {
  const data = e?.response?.data
  if (data instanceof Blob) {
    try {
      const text = await data.text()
      const json = JSON.parse(text)
      if (json && json.message) return json.message
    } catch (ignore) { /* 不是 JSON 就退回通用提示 */ }
  }
  return e?.message || ''
}

/**
 * 把当前结果导出成文件：md / docx / xlsx / pdf。
 * 转换放在后端（Excel 需要真正的结构化表格、PDF 需要内嵌中文字体），前端只负责触发下载。
 */
const exportDoc = async (markdown, format, nameHint) => {
  if (!markdown || exporting.value) return
  exporting.value = format
  try {
    const title = nameHint || currentSkill.value.label
    const base = [title, ctxDatabase.value || ctxConnName.value].filter(Boolean).join('_')
    const blob = await aiExportDoc({ title, markdown, format, fileName: base })
    const saved = await saveBlobAs(blob, base + '.' + format)
    if (saved.canceled) return
    ElMessage.success(t('ai.exportedFmt', { fmt: format.toUpperCase() }))
  } catch (e) {
    ElMessage.error(await blobErrMsg(e) || t('qa.exportFailed'))
  }
  exporting.value = ''
}

// ===== 统一执行入口与状态（按当前 Skill 分派） =====

/** 任一 Skill 正在执行 → 主按钮 loading */
const busy = computed(() =>
  chatLoading.value || advLoading.value || dictLoading.value)

/** 是否已有结果（对话消息 / 字典 / 高级技能任一） */
const hasResult = computed(() =>
  messages.value.length > 0 || !!dictResult.value || !!advResult.value)
/** 空态默认面板：无结果且不在执行中时展示。
 *  知识入库自带完整的引导界面，再叠一层欢迎页只会把它的入口挤到下面去。 */
const showWelcome = computed(() => !hasResult.value && !busy.value && !isKbIngest.value)

/** 主按钮文案随 Skill 变化 */
const RUN_LABEL = computed(() => ({
  chat: t('ai.runChat'), agent: t('ai.runAgent'), dictionary: t('ai.runDictionary'), patrol: t('ai.runPatrol'), insight: t('ai.runInsight')
}))
const runLabel = computed(() => RUN_LABEL.value[tab.value] || t('ai.runChat'))

/** 是否可执行：数据字典 / 巡检允许留空（整库） */
const canRun = computed(() => {
  if (busy.value) return false
  if (tab.value === 'dictionary' || tab.value === 'patrol') return !!ctxConnId.value
  return !!aiInput.value.trim()
})

/** 统一执行：按当前 Skill 分派到对应实现 */
const runSkill = async () => {
  if (!canRun.value) return
  switch (tab.value) {
    case 'chat':
    case 'agent': await sendChat(); break
    case 'insight':
    case 'patrol': await runAdvancedSkill(); break
    case 'dictionary': await doDataDict(); break
  }
}

onMounted(() => { scrollBottom(); loadConnList(); warmCtx(); loadKbMeta() })
</script>

<style scoped>
.ai-panel { height: 100%; display: flex; flex-direction: column; position: relative; overflow: visible; background: var(--dc-bg-sidebar, var(--dc-bg-soft)); }
/* 中央工作区形态：占满主区，由 .ai-main 横向拆成「导航 | 内容」。
   注意：外层（MainView）给 .ai-panel 加了 flex-shrink:0 与 border-left（那是给右侧栏用的），
   这里必须显式覆盖，否则放进 Tab 后宽度撑不开、还多一条左边线。 */
.ai-panel.is-studio {
  width: 100% !important;
  flex: 1 1 auto;
  min-width: 0;
  height: 100%;
  border-left: none;
  overflow: hidden;
  /* 沉浸形态：与外层工作区同底色，去掉头部后不会有「多一块」的割裂感 */
  background: var(--dc-bg-deep);
}
.ai-main { flex: 1; min-height: 0; display: flex; flex-direction: column; }

/* 当前 Skill 按钮：工具条内统一样式（描边框 + 无底，悬浮/展开时主色高亮） */
.skill-btn {
  display: inline-flex; align-items: center; gap: 5px; flex-shrink: 0;
  height: 28px; padding: 0 10px; border: 1px solid var(--dc-border); border-radius: 8px; cursor: pointer;
  background: transparent;
  color: var(--dc-text); font-size: 13px; font-family: inherit;
  transition: background .15s ease, color .15s ease, border-color .15s ease;
}
.skill-btn:hover { border-color: var(--dc-primary); color: var(--dc-primary); }
.skill-btn.open { border-color: var(--dc-primary); color: var(--dc-primary); background: var(--dc-primary-wash); }
.skill-btn-ic { color: var(--dc-primary); }
/* 与其它控件（上下文 / 模型下拉）保持一致：常规字重，不要加粗 */
.skill-btn-tx { font-weight: 400; }
.skill-btn-caret { font-size: 13px; color: var(--dc-text-weak); transition: transform .18s ease; }
.skill-btn.open .skill-btn-caret { transform: rotate(180deg); }

/* 下拉弹层样式（小标题 / 条目 / 选中态 / 数据源层级的图标与引导线）统一放在
   src/styles/index.css：弹层被 teleport 到 body，写在本组件的 scoped 块里根本命不中。 */

/* 拖拽条 */
.ai-resize-bar { position: absolute; left: -3px; top: 0; bottom: 0; width: 6px; cursor: col-resize; z-index: 20; transition: background .15s; }
.ai-resize-bar:hover, .ai-resize-bar.active { background: var(--dc-primary-glow); }

/* 头部 */
.ai-head {
  display: flex; justify-content: space-between; align-items: center;
  height: 46px; padding: 0 12px 0 14px;
  background: var(--dc-bg-input); position: relative;
}
.ai-head::after { content: ''; position: absolute; left: 0; right: 0; bottom: 0; height: 1px; background: var(--dc-border); }
.ai-brand { display: flex; align-items: center; gap: 9px; min-width: 0; }
.ai-brand-ic {
  width: 28px; height: 28px; border-radius: 8px; flex-shrink: 0;
  display: inline-flex; align-items: center; justify-content: center;
  color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-purple));
  box-shadow: 0 3px 10px var(--dc-primary-glow);
}
.ai-brand-tx { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
.ai-brand-name {
  font-size: 14px; font-weight: 700; line-height: 1.15; letter-spacing: .2px;
  background: linear-gradient(90deg, var(--dc-primary), var(--dc-purple));
  -webkit-background-clip: text; -webkit-text-fill-color: transparent;
}
.ai-brand-sub { font-size: 11px; color: var(--dc-text-weak); line-height: 1.15; }
.ai-head-right { display: flex; align-items: center; gap: 6px; min-width: 0; }
/* 当前数据上下文（连接 · 库 · 表）：可点击的层级选择入口，与其它控件同规格 */
.composer-ctx {
  display: inline-flex; align-items: center; gap: 5px; flex-shrink: 0;
  height: 28px; max-width: 300px; padding: 0 10px;
  border-radius: 8px; cursor: pointer;
  background: transparent; border: 1px solid var(--dc-border);
  color: var(--dc-text); font-size: 13px; font-family: inherit;
  transition: border-color .15s ease, color .15s ease, background .15s ease;
}
.composer-ctx:hover { border-color: var(--dc-primary); color: var(--dc-primary); }
.composer-ctx.open { border-color: var(--dc-primary); color: var(--dc-primary); background: var(--dc-primary-wash); }
.composer-ctx :deep(.el-icon) { color: currentColor; opacity: .75; }
.composer-ctx-tx { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.composer-ctx-caret { transition: transform .18s ease; }
.composer-ctx.open .composer-ctx-caret { transform: rotate(180deg); }



/* 主体 */
.ai-body { flex: 1; min-height: 0; display: flex; flex-direction: column; overflow: hidden; }

/* ===== 结果区：内容随当前 Skill 变化 ===== */
/* 统一的面板：空态引导、对话消息、各技能结果都装在这块有底色/描边的卡片里；
   外边距与「新建脚本」界面（10px）一致，底部留 0，与输入框的 10px 上留白合成一段 10px 间距 */
.ai-view {
  flex: 1; min-height: 0;
  /* 纵向滚动；横向裁掉溢出（宽代码块自身可横向滚动），避免内容把消息行顶出面板 */
  overflow-y: auto; overflow-x: hidden;
  margin: 10px 10px 0;
  padding: 14px;
  border-radius: 8px;
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border);
  display: flex; flex-direction: column; gap: 14px;
}

/* ===== 空态默认面板 ===== */
/* 内容整体限宽 + 垂直居中：宽屏下不再贴顶、下半屏大片留白 */
.ai-welcome {
  flex: 1; min-height: 0;
  display: flex; align-items: center; justify-content: center;
}
.welcome-panel {
  width: 100%; max-width: 1000px; margin: 0 auto;
  /* 四段（标题 / 能做什么 / 高级技能 / 试试这样问）之间统一用同一个间距，避免有的松有的紧 */
  display: flex; flex-direction: column; gap: 28px;
}
.welcome-hero { display: flex; align-items: center; gap: 11px; }
.welcome-logo {
  width: 40px; height: 40px; border-radius: 10px; flex-shrink: 0;
  display: inline-flex; align-items: center; justify-content: center;
  color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-purple));
  box-shadow: 0 6px 16px var(--dc-primary-glow);
}
.welcome-hero-tx { min-width: 0; }
.welcome-title { font-size: 16px; font-weight: 700; color: var(--dc-text); }
.welcome-sub { margin-top: 2px; font-size: 13px; color: var(--dc-text-weak); line-height: 1.55; }
/* 功能介绍：3 条并排（图标 + 名称 + 一行说明），说清它和普通聊天的区别 */
.welcome-intro { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 8px; }
.intro-item {
  display: flex; align-items: flex-start; gap: 9px;
  padding: 10px 11px; border-radius: 10px;
  /* 底色与「高级技能」卡片对齐（--dc-bg-card + 描边），不再用更深的内嵌层 */
  background: var(--dc-bg-card); border: 1px solid var(--dc-border);
}
.intro-ic {
  flex-shrink: 0; width: 26px; height: 26px; border-radius: 8px;
  display: inline-flex; align-items: center; justify-content: center;
  font-size: 14px; color: var(--dc-primary); background: var(--dc-primary-wash);
}
.intro-tx { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.intro-name { font-size: 13px; font-weight: 600; }
.intro-desc { font-size: 12px; line-height: 1.45; color: var(--dc-text-weak); }
.welcome-block { display: flex; flex-direction: column; gap: 8px; }
.welcome-section-title {
  display: flex; align-items: center; gap: 5px;
  font-size: 11.5px; font-weight: 600; letter-spacing: .3px; color: var(--dc-text-weak);
}
.welcome-section-title .el-icon { font-size: 14px; color: var(--dc-primary); }
/* 技能入口：容器上限 1000px + 200px 最小列宽 → 常见宽度下 4 列（8 个技能刚好 4×4 两行），
   窄面板自动降成 3 / 2 / 1 列 */
.welcome-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 10px; }
/* 技能卡片的底色是**这一页的基准**：「能做什么」与「试试这样问」都对齐到它（见下）。
   它与外层 .ai-view 同色（--dc-bg-card），靠一圈描边成形 —— 观感是"平的描边卡"，
   而不是压在深色底上的实心块。 */
.welcome-card {
  display: flex; align-items: flex-start; gap: 10px; text-align: left;
  padding: 12px 13px; border-radius: 10px; cursor: pointer; font-family: inherit;
  background: var(--dc-bg-card); border: 1px solid var(--dc-border); color: var(--dc-text);
  transition: border-color .15s ease, background .15s ease, transform .15s ease, box-shadow .15s ease;
}
.welcome-card:hover {
  border-color: var(--dc-primary); background: var(--dc-primary-wash);
  transform: translateY(-1px); box-shadow: 0 4px 12px var(--dc-primary-glow);
}
.welcome-card-ic {
  flex-shrink: 0; width: 28px; height: 28px; border-radius: 8px;
  display: inline-flex; align-items: center; justify-content: center;
  font-size: 15px; color: var(--dc-primary); background: var(--dc-primary-wash);
}
.welcome-card:hover .welcome-card-ic { background: var(--dc-bg-card); }
.welcome-card-tx { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
.welcome-card-name { font-size: 13px; font-weight: 600; }
/* 描述最多两行：卡片更饱满，也避免长描述把列宽撑歪 */
.welcome-card-desc {
  font-size: 12px; line-height: 1.5; color: var(--dc-text-weak);
  display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;
}
/* 示例：按网格铺开、点一下直接提问（箭头靠右对齐，整块更整齐） */
.welcome-examples { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 8px; }
.welcome-example {
  display: flex; align-items: center; gap: 8px;
  /* 底色与圆角都对齐「高级技能」卡片（原来底色是更深的内嵌层、圆角 8px） */
  padding: 9px 11px; border-radius: 10px; cursor: pointer; font-family: inherit; text-align: left;
  background: var(--dc-bg-card); border: 1px solid var(--dc-border); color: var(--dc-text); font-size: 11.5px;
  transition: border-color .15s ease, color .15s ease, background .15s ease;
}
.welcome-example .el-icon { margin-left: auto; font-size: 13px; color: var(--dc-text-weak); transition: color .15s ease; }
.welcome-example:hover { border-color: var(--dc-primary); color: var(--dc-primary); background: var(--dc-primary-wash); }
.welcome-example:hover .el-icon { color: var(--dc-primary); }

/* 结果卡片（非对话类 Skill 的产物）：在结果面板底色上用 input 底以区分层次。
   卡片内不再放重复的小标题（Markdown 正文自带），导出按钮改为浮在右上角 */
.res-card {
  position: relative;
  background: var(--dc-bg-input); border: 1px solid var(--dc-border);
  border-radius: 8px; padding: 12px 14px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, .05);
  animation: fadeIn .28s ease;
}
.res-sql {
  background: var(--dc-bg-deep);
  border: 1px solid var(--dc-border); border-left: 3px solid var(--dc-primary);
  border-radius: 8px; padding: 12px 14px;
  font-family: "SF Mono", ui-monospace, Consolas, monospace;
  font-size: 13px; color: var(--dc-link); white-space: pre-wrap;
  max-height: 320px; overflow: auto; line-height: 1.65;
}
/* 「导出」菜单（Markdown / Word / Excel / PDF 自选）：绝对定位到卡片右上角，
   与 Markdown 文档标题同一行；标题右侧留出按钮宽度，避免长标题压到按钮下面 */
.res-export-float { position: absolute; top: 10px; right: 12px; z-index: 2; }
.res-card :deep(.ai-markdown > :first-child) { padding-right: 96px; }
.res-export {
  display: inline-flex; align-items: center; gap: 4px; flex-shrink: 0;
  height: 24px; padding: 0 9px; border-radius: 7px; cursor: pointer; font-family: inherit;
  border: 1px solid var(--dc-border); background: var(--dc-bg-card); color: var(--dc-text);
  font-size: 11.5px;
  transition: color .15s ease, border-color .15s ease, background .15s ease;
}
.res-export:hover:not(:disabled) {
  color: var(--dc-primary); border-color: var(--dc-primary); background: var(--dc-primary-wash);
}
.res-export:disabled { opacity: .55; cursor: default; }
.res-export .el-icon { font-size: 14px; }
.res-export-caret { font-size: 12px; color: var(--dc-text-weak); }
.res-loading {
  display: flex; align-items: center; justify-content: center; gap: 8px;
  padding: 26px 0; color: var(--dc-text-dim); font-size: 13px;
}
/* 对话消息 */
.msg { display: flex; gap: 10px; min-width: 0; animation: fadeIn .2s ease; }
.msg.user { flex-direction: row-reverse; }
.msg-avatar {
  width: 28px; height: 28px; border-radius: 8px; flex-shrink: 0;
  display: flex; align-items: center; justify-content: center;
  font-size: 12px; font-weight: 700; letter-spacing: .3px;
}
.msg.user .msg-avatar { background: linear-gradient(135deg, var(--dc-primary), var(--dc-purple)); color: var(--dc-on-primary); box-shadow: 0 2px 8px var(--dc-primary-glow); }
.msg.assistant .msg-avatar { background: var(--ai-bubble-bg); color: var(--dc-purple); border: 1px solid var(--dc-border); }
/* 用量脚注：贴在回答末尾，字号小一档、颜色淡一档 —— 它是"备注"，不该抢正文的视线 */
.msg-usage { margin-top: 8px; padding-top: 6px; border-top: 1px dashed var(--dc-border); font-size: 12px; color: var(--dc-text); opacity: .6; }
.msg-usage-sub { opacity: .85; }
.msg-bubble {
  position: relative; min-width: 0;
  max-width: min(820px, calc(100% - 40px));
  padding: 11px 14px; border-radius: 12px;
  font-size: 14px; line-height: 1.8;
  transition: box-shadow .18s ease, border-color .18s ease;
}
/* 用户气泡：主色与面板底按比例混合出的柔和蓝（不透明，保证箭头同色无缝），
   比纯主色实底更内敛；不支持 color-mix 时回退为半透明主色 */
.msg.user { --user-bubble-bg: var(--dc-primary-soft); }
@supports (background: color-mix(in srgb, #fff, #000)) {
  .msg.user { --user-bubble-bg: color-mix(in srgb, var(--dc-primary) 26%, var(--dc-bg-card)); }
}
.msg.user .msg-bubble { background: var(--user-bubble-bg); border: none; border-top-right-radius: 4px; color: var(--dc-text); }
.msg.user .msg-text { color: var(--dc-text); }
/* 助手气泡：底色由「主题文字色」按低比例混入卡片底得到，深色下变亮、浅色下变淡。
   关键是要跟内嵌块（代码块 / 表格 / 轨迹）拉开两级层次，避免灰糊成一片：
   - 深色：气泡略亮于面板，内嵌块更暗；
   - 浅色：气泡只带极淡冷灰（原来的 9% 偏灰发闷），内嵌块用中灰。
   描边 + 极轻阴影让气泡在 .ai-view 面板上「浮」起来。 */
.msg.assistant {
  --ai-bubble-bg: var(--dc-bg-hover);
  --ai-inset-bg: var(--dc-bg-soft);
  --ai-bubble-shadow: 0 2px 8px rgba(0, 0, 0, .22);
}
@supports (background: color-mix(in srgb, #fff, #000)) {
  .msg.assistant {
    --ai-bubble-bg: color-mix(in srgb, var(--dc-text) 8%, var(--dc-bg-card));
    --ai-inset-bg: color-mix(in srgb, #000 22%, var(--ai-bubble-bg));
  }
  html[data-theme="light"] .msg.assistant {
    --ai-bubble-bg: color-mix(in srgb, var(--dc-text) 5%, var(--dc-bg-card));
    --ai-inset-bg: color-mix(in srgb, var(--dc-text) 11%, var(--dc-bg-card));
    --ai-bubble-shadow: 0 1px 2px rgba(15, 23, 42, .05), 0 3px 10px rgba(15, 23, 42, .04);
  }
}
.msg.assistant .msg-bubble {
  background: var(--ai-bubble-bg);
  border: 1px solid var(--dc-border);
  border-top-left-radius: 4px;
  box-shadow: var(--ai-bubble-shadow, none);
}
/* ---- 微信风格小箭头 ---- */
.msg.assistant .msg-bubble::before {
  content: ''; position: absolute; top: 12px; left: -8px;
  width: 0; height: 0; border-style: solid;
  border-width: 7px 8px 7px 0;
  border-color: transparent var(--dc-border) transparent transparent;
}
.msg.assistant .msg-bubble::after {
  content: ''; position: absolute; top: 13px; left: -6px;
  width: 0; height: 0; border-style: solid;
  border-width: 6px 7px 6px 0;
  border-color: transparent var(--ai-bubble-bg) transparent transparent;
}
.msg.user .msg-bubble::before {
  content: ''; position: absolute; top: 13px; right: -6px;
  width: 0; height: 0; border-style: solid;
  border-width: 6px 0 6px 7px;
  border-color: transparent transparent transparent var(--user-bubble-bg);
}

.msg-text { white-space: pre-wrap; word-break: break-word; }
/* 消息内代码块：底色比气泡深一档（--ai-inset-bg），靠底色而不是边框拉开层次，
   去掉描边后块内更干净、也不与气泡边框「线条打架」。
   注意 v-html 渲染出来的 <pre> 没有 scoped 属性，必须用 :deep() 才能命中 */
.msg.assistant .ai-markdown :deep(pre) {
  background: var(--ai-inset-bg);
  border-color: transparent;
  border-radius: 8px;
  line-height: 1.65;
}
.typing { display: flex; align-items: center; gap: 4px; padding: 14px 16px; }
.typing .dot { width: 6px; height: 6px; border-radius: 8px; background: var(--dc-text-dim); animation: dc-blink 1.2s infinite; }
.typing .dot:nth-child(2) { animation-delay: .2s; }
.typing .dot:nth-child(3) { animation-delay: .4s; }
.typing .wait { margin-left: 6px; font-size: 12px; color: var(--dc-text-dim); }
/* 流式进行中的底部提示：三个跳动小点 + 「生成中…」 */
.streaming-hint {
  display: flex; align-items: center; gap: 4px;
  margin-top: 8px; padding-top: 8px;
  border-top: 1px dashed var(--dc-border);
}
.streaming-hint .dot {
  width: 5px; height: 5px; border-radius: 8px;
  background: var(--dc-primary);
  animation: dc-blink 1.2s infinite;
}
.streaming-hint .dot:nth-child(2) { animation-delay: .2s; }
.streaming-hint .dot:nth-child(3) { animation-delay: .4s; }
.streaming-hint .txt { margin-left: 4px; font-size: 12px; color: var(--dc-text-dim); }
@keyframes dc-blink { 0%, 100% { opacity: .3; } 50% { opacity: 1; } }

/* ===== 指令 Skill 的操作卡片（原命令面板能力）===== */
.cmd-plan { display: flex; flex-direction: column; }
/* 操作卡片：左侧强调边，需确认的转红 */
.cmd-action {
  display: flex; align-items: flex-start; gap: 10px;
  padding: 10px 12px; border-radius: 8px; margin-bottom: 8px;
  border: 1px solid var(--dc-border); border-left-width: 3px; border-left-color: var(--dc-primary);
  background: var(--dc-bg-soft); transition: background .16s ease;
}
.cmd-action:hover { background: var(--dc-bg-card); }
.cmd-action.danger { border-left-color: var(--dc-danger); }
.cmd-badge {
  flex-shrink: 0; font-size: 12px; font-weight: 600; color: var(--dc-primary);
  background: var(--dc-primary-wash); border-radius: 8px; padding: 2px 8px; line-height: 1.6;
}
.cmd-badge.danger { color: var(--dc-danger); background: rgba(245, 108, 108, .12); }
.cmd-body { flex: 1; min-width: 0; }
.cmd-title { font-size: 13px; color: var(--dc-text); line-height: 1.65; }
.cmd-sql {
  display: block; margin-top: 6px; font-size: 11.5px; color: var(--dc-link);
  background: var(--dc-bg-deep); border-radius: 8px; padding: 5px 8px;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  font-family: 'SF Mono', ui-monospace, Consolas, monospace;
}
.cmd-warn { flex-shrink: 0; font-size: 12px; color: var(--dc-warning, #e6a23c); }
/* ===== 统一输入框（Skills / 参数 / 模型 / 主按钮都收在框内） =====
   同时作为斜杠命令与 Skills 浮层的定位参照 */
.ai-composer {
  position: relative; flex-shrink: 0;
  /* 与「新建脚本」界面保持一致的左右留白 */
  padding: 10px;
}
/* 斜杠命令浮层：贴在输入框上方，不遮挡正在输入的内容。
   命令已扩到 18 条，高度上限放开到视口的 70%（最高 620px），尽量一屏看全；
   但用 vh 兜底，窗口很矮时不会顶出可视区 */
.slash-menu {
  position: absolute; left: 14px; right: 14px; bottom: calc(100% - 4px); z-index: 20;
  max-height: min(70vh, 620px); overflow: auto; padding: 6px;
  background: var(--dc-bg-card); border: 1px solid var(--dc-border);
  border-radius: 8px; box-shadow: 0 10px 28px rgba(0, 0, 0, .18);
  animation: slashIn .14s ease;
}
@keyframes slashIn { from { opacity: 0; transform: translateY(4px); } to { opacity: 1; transform: none; } }
.slash-head {
  font-size: 10.5px; color: var(--dc-text-weak);
  padding: 4px 8px 6px; letter-spacing: .3px;
}
/* 分组小标题（技能 / 快捷提问 / 面板） */
.slash-group {
  padding: 7px 9px 4px; font-size: 10.5px; font-weight: 600;
  letter-spacing: .3px; color: var(--dc-text-weak);
}
.slash-group:not(:first-of-type) { border-top: 1px solid var(--dc-border-soft); margin-top: 4px; }
.slash-item {
  display: flex; align-items: center; gap: 8px;
  padding: 7px 9px; border-radius: 8px; cursor: pointer;
  transition: background .13s ease;
}
.slash-item.active { background: var(--dc-primary-wash); }
.slash-ic { flex-shrink: 0; color: var(--dc-text-dim); }
.slash-item.active .slash-ic { color: var(--dc-primary); }
.slash-label { flex-shrink: 0; font-size: 13px; font-weight: 600; color: var(--dc-text); }
.slash-desc {
  margin-left: auto; font-size: 12px; color: var(--dc-text-weak);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
/* 输入框本体与工具条同属一个「框」：整体一个大圆角容器，工具条贴在框内底部。
   overflow:hidden 让框内悬浮底色、文本域背景都跟随圆角裁切，不出现直角外溢。 */
.composer-box {
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  background: var(--dc-bg-input);
  box-shadow: 0 1px 2px rgba(0, 0, 0, .04);
  /* 不要 overflow: hidden —— 工具条上的「上下文 / 模型」浮层是框内元素、向上弹出，
     加了裁切后会只剩下半截（看起来像没有框）。框内元素本身都是透明底，无需裁切。 */
  overflow: visible;
  transition: border-color .2s, box-shadow .2s;
}
.composer-box:focus-within { border-color: var(--dc-primary); box-shadow: 0 0 0 3px var(--dc-primary-wash); }
/* 文本域：彻底清掉 Element 自带的 1px 内边框与 4px 圆角
   （否则它会在文字区底部露出一条两端带圆角的线） */
.composer-textarea :deep(.el-textarea__inner) {
  resize: none;
  background: transparent !important;
  border: none !important;
  box-shadow: none !important;
  border-radius: 0 !important;
  color: var(--dc-text); padding: 12px 16px 8px;
  font-size: 13px; line-height: 1.65;
}
.composer-textarea :deep(.el-textarea__inner:focus) { box-shadow: none !important; outline: none; }
.composer-textarea :deep(.el-textarea__inner::placeholder) { color: var(--dc-text-weak); }
.composer-textarea :deep(.el-textarea) { border: none !important; box-shadow: none !important; background: transparent !important; }

/* 工具条：除主按钮外全部靠左，主按钮单独推到右侧。
   用一条平直的分隔线与文字区分开（两端直角、贯穿整框，不带圆角） */
.composer-bar {
  display: flex; align-items: center; gap: 8px;
  padding: 7px 10px 8px;
  border-top: 1px solid var(--dc-border);
}
.composer-param { flex-shrink: 0; width: 150px; }
.composer-param :deep(.el-input__wrapper) {
  background: var(--dc-bg-deep); border-radius: 8px;
  box-shadow: 0 0 0 1px var(--dc-border) inset; min-height: 28px;
}
.composer-param :deep(.el-input__inner) { font-size: 13px; color: var(--dc-text); }
/* 固定发送图标按钮：与工具条其它控件同高（28px），圆形、图标居中 */
.composer-run { margin-left: auto; flex-shrink: 0; border-radius: 8px; width: 28px; height: 28px; padding: 0; }
.composer-run :deep(.el-icon) { font-size: 15px; }
/* 右侧操作组：语音输入 + 发送整体靠右（margin-left:auto 收拢在一处） */
.composer-right { margin-left: auto; display: inline-flex; align-items: center; gap: 8px; flex-shrink: 0; }
.composer-right .composer-run { margin-left: 0; }
/* 语音输入按钮：与发送按钮同规格的圆角图标钮；识别中红色 + 呼吸光圈提示 */
.voice-btn {
  width: 28px; height: 28px; padding: 0; flex-shrink: 0;
  display: inline-flex; align-items: center; justify-content: center;
  border-radius: 8px; cursor: pointer;
  background: transparent; border: 1px solid var(--dc-border);
  color: var(--dc-text); font-family: inherit;
  transition: border-color .15s ease, color .15s ease, background .15s ease;
}
.voice-btn:hover { border-color: var(--dc-primary); color: var(--dc-primary); }
.voice-btn.listening {
  border-color: #f56c6c; color: #f56c6c;
  animation: voice-pulse 1.2s ease-in-out infinite;
}
@keyframes voice-pulse {
  0%, 100% { box-shadow: 0 0 0 0 rgba(245, 108, 108, .35); }
  50% { box-shadow: 0 0 0 4px rgba(245, 108, 108, .12); }
}
/* 模型选择：自定义下拉按钮，与「上下文」按钮同规格。
   不用 el-select —— 其内部 selection 会塌陷导致内容不可见；自定义按钮由内容撑开、宽度自适应。 */
.composer-pick {
  display: inline-flex; align-items: center; gap: 5px; flex-shrink: 0;
  height: 28px; max-width: 180px; padding: 0 10px;
  border-radius: 8px; cursor: pointer;
  background: transparent; border: 1px solid var(--dc-border);
  color: var(--dc-text); font-size: 13px; font-family: inherit;
  transition: border-color .15s ease, color .15s ease, background .15s ease;
}
.composer-pick:hover { border-color: var(--dc-primary); color: var(--dc-primary); }
.composer-pick :deep(.el-icon) { color: currentColor; opacity: .75; }
.composer-pick-tx { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.composer-pick-caret { transition: transform .18s ease; }
/* el-dropdown 包裹层在 flex 工具条里保持原宽，不抢空间 */
.composer-bar :deep(.el-dropdown) { flex-shrink: 0; line-height: 1; }

/* 目标连接（跨库转换参数）：沿用 el-select，宽度由 .composer-param 固定，
   不再做任何宽度覆盖（避免 selection 塌陷导致内容不可见）。 */
.ai-plain-select :deep(.el-select__wrapper) {
  min-height: 28px; height: 28px; padding: 0 10px;
  border-radius: 8px;
  background-color: transparent !important;
  box-shadow: 0 0 0 1px var(--dc-border) inset !important;
  transition: box-shadow .15s ease;
}
.ai-plain-select :deep(.el-select__wrapper:hover) { box-shadow: 0 0 0 1px var(--dc-primary) inset !important; }
.ai-plain-select :deep(.el-select__wrapper.is-focused) {
  box-shadow: 0 0 0 1px var(--dc-primary) inset, 0 0 0 3px var(--dc-primary-wash) !important;
}
.ai-plain-select :deep(.el-select__selected-item),
.ai-plain-select :deep(.el-select__placeholder) { font-size: 13px !important; color: var(--dc-text) !important; padding: 0 !important; }
.ai-plain-select :deep(.el-select__caret) { color: var(--dc-text-weak); }
@keyframes fadeIn { from { opacity: 0; transform: translateY(6px); } to { opacity: 1; transform: translateY(0); } }

/* Agent 工具调用轨迹 */
.msg-steps { margin-top: 10px; border-top: 1px dashed var(--dc-border); padding-top: 8px; display: flex; flex-direction: column; gap: 5px; }
.msg-steps-head {
  display: flex; align-items: center; gap: 5px; margin-bottom: 2px;
  font-size: 12px; font-weight: 600; color: var(--dc-text-dim);
}
.msg-steps-head .el-icon { color: var(--dc-purple); font-size: 13px; }
.msg-step { display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--dc-text-dim); overflow: hidden; background: var(--ai-inset-bg, var(--dc-bg-soft)); border-radius: 8px; padding: 3px 8px; }
.msg-step .el-icon { color: var(--dc-purple); flex-shrink: 0; }
.step-tool { color: var(--dc-link); font-weight: 600; flex-shrink: 0; }
.step-args { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

/* ===== 引用溯源：回答下方那条「引用了 N 段团队资料」 ===== */
.msg-sources {
  margin-top: 8px; border-radius: 8px;
  background: var(--ai-inset-bg, var(--dc-bg-soft));
  border: 1px solid var(--dc-border);
}
.msg-src-head {
  display: flex; align-items: center; gap: 6px; padding: 6px 9px;
  font-size: 12px; color: var(--dc-text-mid); cursor: pointer; user-select: none;
}
.msg-src-head .el-icon:first-child { color: var(--dc-purple); }
.msg-src-caret { margin-left: auto; transition: transform .15s ease; }
.msg-src-caret.open { transform: rotate(180deg); }
.msg-src-list { padding: 0 9px 8px; display: flex; flex-direction: column; gap: 6px; }
.msg-src {
  padding: 7px 8px; border-radius: 6px;
  background: var(--dc-bg-card); border: 1px solid var(--dc-border);
}
.msg-src-top { display: flex; align-items: center; gap: 6px; }
.msg-src-rank { font-size: 10.5px; color: var(--dc-text-dim); font-variant-numeric: tabular-nums; }
.msg-src-bar {
  flex: 0 0 44px; height: 3px; border-radius: 2px;
  background: var(--dc-bg-hover); overflow: hidden;
}
.msg-src-bar i { display: block; height: 100%; background: var(--dc-purple); border-radius: 2px; }
.msg-src-score { font-size: 10.5px; color: var(--dc-text-dim); font-variant-numeric: tabular-nums; }
.msg-src-src {
  font-size: 10.5px; color: var(--dc-text-mid); cursor: pointer;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;
  border-bottom: 1px dashed transparent;
}
.msg-src-src:hover { color: var(--dc-primary); border-bottom-color: currentColor; }
.msg-src-text {
  margin-top: 4px; font-size: 12px; line-height: 1.7; color: var(--dc-text-dim);
  display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden;
}
/* 知识库下拉里每个库后面的块数 */
.ctx-sub-count { margin-left: auto; font-size: 10.5px; color: var(--dc-text-dim); }

</style>
