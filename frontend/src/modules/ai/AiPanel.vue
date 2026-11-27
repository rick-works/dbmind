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
        <span class="ai-brand-ic"><AiMark :size="15" /></span>
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
      <!-- 左栏「记录」：只有中央工作区（studio）摆得下 —— 侧边 420px 窄面板再切一栏就没对话区了。
           一个会话一行：点开续聊、悬浮出删除；顶部 + 号新建。 -->
      <aside v-if="isStudio" class="ai-hist" :class="{ 'is-collapsed': !histOpen }">
        <!-- 展开态：标题 + 新建 + 收起 -->
        <div v-if="histOpen" class="hist-top">
          <span class="hist-title">{{ $t('ai.history') }}</span>
          <button class="hist-new" type="button" :title="$t('ai.newThread')" @click="onHistoryNew">
            <el-icon :size="13"><Plus /></el-icon>
          </button>
          <button class="hist-toggle" type="button" :title="$t('ai.historyCollapse')" @click="toggleHist">
            <el-icon :size="13"><ArrowLeft /></el-icon>
          </button>
        </div>
        <!-- 收起态：只剩竖排标题 + 两个图标，对话区因此变宽 -->
        <div v-else class="hist-top is-col">
          <button class="hist-toggle" type="button" :title="$t('ai.historyExpand')" @click="toggleHist">
            <el-icon :size="13"><ArrowRight /></el-icon>
          </button>
          <button class="hist-new" type="button" :title="$t('ai.newThread')" @click="onHistoryNew">
            <el-icon :size="13"><Plus /></el-icon>
          </button>
          <span class="hist-vlabel">{{ $t('ai.history') }}</span>
        </div>
        <div v-show="histOpen" v-loading="histLoading" class="hist-list">
          <!-- 问数（本地）与对话（服务端）合成一份列表：长得一样、按时间新的在前，
               点哪条就把技能自动切到对应的类型（问数 → 智能问数，会话 → 自由对话） -->
          <button
            v-for="it in histItems"
            :key="it.kind + ':' + it.id"
            type="button"
            class="hist-item"
            :class="{ active: it.kind === 'thread' && it.raw.id === chatThreadId }"
            :data-kind="it.kind"
            :title="it.title"
            @click="it.kind === 'ask' ? openAskHistory(it.raw) : openThread(it.raw)"
          >
            <span class="hist-item-tx">{{ it.title }}</span>
            <span class="hist-item-day">{{ it.day }}</span>
            <span
              v-if="it.kind === 'thread'"
              class="hist-ren"
              :title="$t('ai.historyRename')"
              @click.stop="renameThread(it.raw)"
            >
              <el-icon :size="12"><Edit /></el-icon>
            </span>
            <span
              class="hist-del"
              :title="it.kind === 'thread' ? $t('ai.historyDelete') : '删除'"
              @click.stop="it.kind === 'ask' ? deleteAskHistory(it.raw) : deleteThread(it.raw)"
            >
              <el-icon :size="12"><Delete /></el-icon>
            </span>
          </button>
          <div v-if="!histItems.length && !histLoading" class="hist-empty">{{ $t('ai.historyEmpty') }}</div>
        </div>
      </aside>

      <div class="ai-body">
        <!-- ===== 结果区：内容随当前 Skill 变化 ===== -->
        <div ref="chatBox" class="ai-view">
          <!-- 空态默认面板：首次打开 / 尚无结果时，展示数据源状态、技能入口与示例 -->
          <div v-if="showWelcome" class="ai-welcome">
            <div class="welcome-panel">
              <div class="welcome-hero">
                <span class="welcome-logo"><AiMark :size="18" /></span>
                <div class="welcome-hero-tx">
                  <div class="welcome-title">{{ $t('ai.welcomeTitle') }}</div>
                  <div class="welcome-sub">{{ $t('ai.welcomeSub') }}</div>
                </div>
              </div>

              <!-- 功能介绍：替代原来的数据源状态条（数据源在下方工具条上已经能看到） -->
              <div class="welcome-block is-intro">
                <div class="welcome-section-title"><AiMark />{{ $t('ai.whatCanDo') }}</div>
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
              <!-- 助手头像用机器人标记（原来是「AI」两个字） -->
              <div class="msg-avatar">
                <AiMark v-if="m.role !== 'user'" :size="19" />
                <template v-else>{{ $t('ai.me') }}</template>
              </div>
              <!-- 气泡 + 下方操作条：按钮放在**气泡外面**，悬停时才出现，
                   不撑高气泡、也不挤在气泡里（用户要求「在框下面显示按钮」） -->
              <div class="msg-main">
              <div class="msg-bubble" :class="{ typing: m.role !== 'user' && !hasBody(m) && m.streaming }">
                <!-- 用户气泡：**技能标签跟着消息走** —— 这条是按哪个技能问的，一眼看见 -->
                <template v-if="m.role === 'user'">
                  <!-- 编辑态：改完直接重发，这条之后的问答全部作废重来 -->
                  <div v-if="editingIndex === i" class="msg-edit">
                    <el-input v-model="editingText" type="textarea" :rows="5" resize="none"
                              class="msg-edit-input" @keydown.enter.exact.prevent="submitEdit" />
                    <div class="msg-edit-acts">
                      <el-button size="small" text @click="cancelEdit">{{ $t('common.cancel') }}</el-button>
                      <el-button size="small" type="primary" :disabled="!editingText.trim() || generating"
                                 @click="submitEdit">{{ $t('ai.resend') }}</el-button>
                    </div>
                  </div>
                  <div v-else class="msg-text">
                    <span v-for="sk in (m.skills || [])" :key="sk.id || sk.name" class="msg-skill-tag">
                      <span class="msg-skill-ic">
                        <template v-if="sk.icon || iconOfSkill(sk.name)">{{ sk.icon || iconOfSkill(sk.name) }}</template>
                        <MagicStick v-else />
                      </span>
                      <span class="msg-skill-tx">{{ sk.name }}</span>
                    </span>
                    <span class="msg-body">{{ m.content }}</span>
                  </div>
                </template>
                <!-- 流式回复尚未产出内容：显示动态三点，而不是一个空气泡 -->
                <template v-else-if="!hasBody(m) && m.streaming">
                  <!-- 文案在前、跳动的三点在后（用户：「前面三个点挪后面去」）；
                       文案本身不带省略号，否则末尾又多出三个点 -->
                  <span class="wait-tx">{{ $t('ai.thinkingNow') }}</span>
                  <span class="dot"></span><span class="dot"></span><span class="dot"></span>
                  <span v-if="waitingSec >= 3" class="wait">{{ $t('ai.waited', { s: waitingSec }) }}</span>
                </template>
                <!-- 智能问数：一条回答 = 结论 + 图表 + 明细（与对话同一条消息流，不开新页签） -->
                <div v-else-if="m.askAnswer" class="msg-ask">
                  <AskAnswer :answer="m.askAnswer" @ask="runAskInChat" @insert-sql="emit('insert-sql', $event)" />
                </div>
                <!-- 深度思考：一条**只往下长**的时间线。
                     文字永远在它出现的位置往后写；这一轮如果去调工具，同一块原地变灰（思考），
                     工具行接在下面 —— 不回填、不挪位（用户反馈「怎么跑上面输出去了」）。 -->
                <div v-else-if="m.timeline && m.timeline.length" class="ai-timeline"
                     @click="onMdSqlAction($event, m.content)">
                  <!-- 每一步一个圆角块：本步的叙述 + 本步的工具都装在同一个块里，
                       块与块之间留空 —— 不然①和②会连成一片（用户：「是不是跑一起去了」） -->
                  <div v-for="(g, gi) in stepGroups(m)" :key="gi" class="tl-step"
                       :class="{ 'is-solo': g.solo }">
                    <template v-for="{ p, pi } in g.parts" :key="pi">
                      <div v-if="p.kind === 'text'" class="tl-text ai-markdown"
                           v-html="renderMd(p.text, !m.streaming)"></div>
                      <!-- 思考块也走 markdown：这一轮去调工具时，前面的步骤叙述会原地变灰，
                           里面照样有表格 / 代码块 / 提示卡 —— 按纯文本渲染就会露出 `|` 和 ``` 原文 -->
                      <div v-else-if="p.kind === 'think'" class="tl-think ai-markdown"
                           v-html="renderMd(p.text, false)"></div>
                      <!-- 工具调用：只留「工具名 + 耗时」，点箭头才展开参数与结果。
                           摘要（表名 / SQL 片段）放在展开体里 —— 挤在标题行上又长又乱。 -->
                      <div v-else-if="p.kind === 'tool'" class="tl-tool-wrap"
                           :class="{ 'is-open': p.open, 'is-run': p.running, 'is-bad': p.ok === false }">
                        <button type="button" class="tl-tool" @click="p.open = !p.open">
                          <el-icon class="tl-tool-caret">
                            <ArrowDown v-if="p.open" />
                            <ArrowRight v-else />
                          </el-icon>
                          <el-icon class="tl-tool-ic" :class="{ 'is-spin': p.running }">
                            <Loading v-if="p.running" />
                            <Cpu v-else />
                          </el-icon>
                          <span class="tl-tool-name">{{ toolLabel(p.tool) }}</span>
                          <span v-if="p.durationMs" class="tl-tool-ms">{{ p.durationMs }}ms</span>
                        </button>
                        <AiToolDetail v-if="p.open" :step="p" class="tl-tool-detail" />
                      </div>
                      <!-- 变更单：**落在它出现的位置**，不固定堆在气泡底部 ——
                           否则审批后续跑，新吐的文字会跑到上一轮卡片上面（看着像顺序错乱）。 -->
                      <AiChangeCard v-else-if="p.kind === 'change'" :card="p.change"
                                    :conn-name="connNameOf(p.change)" :busy="changeBusy === p.change.id"
                                    @allow="runChange(p.change, 'allow')" @reject="runChange(p.change, 'reject')"
                                    @skip="skipChange(p.change)" @undo="runChange(p.change, 'undo')"
                                    @unskip="p.change.skipped = false" />
                      <!-- 审批后接着做的分隔线：只留一条细线，不要文字 -->
                      <div v-else-if="p.kind === 'divider'" class="tl-sep"></div>
                    </template>
                  </div>
                </div>
                <!-- SQL 代码块的操作按钮由 markdown 渲染器生成（每段一个），这里做点击委托 -->
                <div v-else class="ai-markdown" v-html="renderMd(m.content, !m.streaming)"
                     @click="onMdSqlAction($event, m.content)"></div>
                <!-- 技能结果消息（数据字典 / 数据洞察）保留导出入口：与对话一致的形态，但能落文件 -->
                <div v-if="m.exportMd" class="msg-export">
                  <el-dropdown trigger="click" placement="bottom-end" popper-class="ai-model-dropdown"
                               @command="(f) => exportDoc(m.exportMd, f, m.exportTitle || '')">
                    <button class="msg-export-btn" type="button" :disabled="!!exporting" :title="$t('ai.exportAsFile')">
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
                        <el-dropdown-item command="pdf" disabled>
                          <el-icon class="skills-ic"><Notebook /></el-icon><span class="skills-name" :title="$t('ai.pdfHint')">{{ $t('ai.fmtPdf') }}</span>
                        </el-dropdown-item>
                      </el-dropdown-menu>
                    </template>
                  </el-dropdown>
                </div>
                <!-- 流式输出中：明确提示「还在生成」，避免用户误以为已经答完；可随时停下。
                     一条安静的状态行：「生成中」+ 三个跳动的点（点跟在字后面，
                     文字本身不再带省略号 —— 否则「生成中…•••」重复），
                     右侧一个淡红的胶囊「■ 停止」 -->
                <div v-if="m.role !== 'user' && m.streaming && hasBody(m)" class="streaming-hint">
                  <span class="txt">{{ $t('ai.generating') }}</span>
                  <span class="dot"></span><span class="dot"></span><span class="dot"></span>
                  <button type="button" class="sh-stop" :title="$t('ai.stopGen')" @click="stopStream">
                    <span class="sh-stop-sq" />
                    <span class="sh-stop-tx">{{ $t('ai.stopShort') }}</span>
                  </button>
                </div>
                <!-- 服务端提示（预算降级等）：挂在回答下面，不混进正文 -->
                <div v-if="m.notice" class="msg-notice">{{ m.notice }}</div>
                <!-- 变更单：AI 对数据库的任何增删改都出一张单，**就在这段对话里**给三个动作：
                     允许（一步执行）/ 拒绝（这张不要）/ 跳过（这次不执行，留着以后在「记录」里再定）。
                     正常情况卡片已经落在时间线的位置上（见上），这里只补「没进时间线」的那几张
                     （流里丢了 change 帧、时间线为空的老消息），否则卡片会整张消失。 -->
                <div v-if="orphanChanges(m).length" class="msg-changes">
                  <AiChangeCard v-for="c in orphanChanges(m)" :key="c.id" :card="c"
                                :conn-name="connNameOf(c)" :busy="changeBusy === c.id"
                                @allow="runChange(c, 'allow')" @reject="runChange(c, 'reject')"
                                @skip="skipChange(c)" @undo="runChange(c, 'undo')"
                                @unskip="c.skipped = false" />
                </div>
                <!-- 等审批：出一张单就停下来等人点，点了（批准或拒绝）会自动接着往下做 ——
                     不是把剩下的步骤闷在后台，而是把后续的思考和结论继续吐在这段对话里 -->
                <div v-if="m.waiting && m.jobId" class="msg-await">
                  <el-icon><Loading /></el-icon>
                  <span>{{ $t('ai.awaitApprove') }}</span>
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
                <!-- 用量脚注：与 SQL 查询里 AI 弹窗、问数卡片共用同一个组件（common/AiUsageNote.vue），
                     上游没回 usage 就整块不渲染；全项目统一右对齐 -->
                <AiUsageNote :usage="m.usage" align="right" />
              </div>
              <!-- 操作条：气泡下方一排小图标（请求记录 / 编辑后重发 / 复制），悬停才出现 -->
              <div v-if="m.role === 'user' && editingIndex !== i" class="msg-tools">
                <!-- 请求记录：看这句**实际发给模型的内容**（system + 历史 + 提问）。
                     与「编辑 / 复制」同一排、放在它们左侧（用户定位）；
                     快照按次记，点开是列表，默认选最新一次 -->
                <button class="msg-tool" type="button" :title="$t('ai.reqLogTip')" @click="reqLogOpen = true">
                  <el-icon><List /></el-icon>
                </button>
                <button class="msg-tool" type="button" :title="$t('ai.editMsg')"
                        :disabled="generating" @click="startEdit(i)">
                  <el-icon><Edit /></el-icon>
                </button>
                <button class="msg-tool" type="button" :title="$t('ai.copyMsg')" @click="copyMsg(m)">
                  <el-icon><DocumentCopy /></el-icon>
                </button>
              </div>
              </div>
            </div>
            <div v-if="showTyping" class="msg assistant is-typing">
              <div class="msg-avatar"><AiMark :size="19" /></div>
              <div class="msg-main">
                <div class="msg-bubble typing">
                  <span class="wait-tx">{{ $t('ai.thinkingNow') }}</span>
                  <span class="dot"></span><span class="dot"></span><span class="dot"></span>
                  <span v-if="waitingSec >= 3" class="wait">{{ $t('ai.waited', { s: waitingSec }) }}</span>
                </div>
              </div>
            </div>
          </template>

          <template v-else-if="isKbIngest">
            <KbIngestPanel @open-knowledge="emit('open-knowledge')" />
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
                <span v-if="typeof c.icon === 'string'" class="slash-ic slash-emoji">{{ c.icon }}</span>
                <el-icon v-else class="slash-ic"><component :is="c.icon" /></el-icon>
                <span class="slash-label">{{ c.label }}</span>
                <span class="slash-desc">{{ c.desc }}</span>
              </div>
            </template>
          </div>

          <!-- 输入域 + 工具条同属一个「框」，工具条贴在框内底部 -->
          <div class="composer-box" ref="composerBoxEl">
            <!-- 选中的技能：**只有选了才出现**（默认什么都没选 = 自由对话）。
                 内置能力选中时显示当前那一个（× 回到自由对话）；插件技能可多个叠加 -->
            <div v-if="activeSkill || pickedSkills.length" class="picked-skills">
              <span v-if="activeSkill" class="picked-skill" :title="activeSkill.desc">
                <span class="picked-skill-ic"><component :is="activeSkill.icon" /></span>
                <span class="picked-skill-tx">{{ activeSkill.label }}</span>
                <el-icon class="picked-skill-x" :title="$t('ai.skillRemove')" @click="backToFreeChat"><Close /></el-icon>
              </span>
              <span v-for="s in pickedSkills" :key="s.id" class="picked-skill" :title="s.description">
                <span class="picked-skill-ic">
                  <template v-if="s.icon || iconOfSkill(s.name)">{{ s.icon || iconOfSkill(s.name) }}</template>
                  <MagicStick v-else />
                </span>
                <span class="picked-skill-tx">{{ s.name }}</span>
                <el-icon class="picked-skill-x" :title="$t('ai.skillRemove')" @click="unpickSkill(s.id)"><Close /></el-icon>
              </span>
            </div>
            <el-input ref="inputRef" v-model="aiInput" type="textarea" :rows="3" resize="none" class="composer-textarea"
                      :placeholder="composePlaceholder"
                      @keydown="onComposeKeydown" @input="onComposeInput" />

            <div class="composer-bar" ref="composerBarEl">
              <!-- 左侧：Skills / 专属参数 / 上下文 / 模型（除发送外全部靠左排布） -->
              <!-- 深度思考（原「自主排查 / 深挖」）：交给智能体自己查库、出变更单。
                   它是模式入口、永远不收成图标，所以提示用 .always：光看「深度思考」四个字
                   看不出它跟普通问答差在哪 -->
              <button class="composer-ctx mate-btn" :class="{ open: agentMode }" type="button" data-g="7"
                      v-fast-tip.always="() => $t('ai.agentModeTip')" @click="agentMode = !agentMode">
                <el-icon :size="12"><Aim /></el-icon>
                <span class="composer-ctx-tx">{{ $t('ai.agentMode') }}</span>
              </button>
              <!-- 记事（原「同事」）：记录 + 记忆。角标只统计 alert（值班真发现问题），
                   不把「等你确认变更单」也标红 —— 那会变成天天有红点 -->
              <button class="composer-ctx mate-btn" type="button" data-g="6"
                      v-fast-tip.always="() => $t('ai.mateTip')" @click="openMate('records')">
                <el-icon :size="12"><Bell /></el-icon>
                <span class="composer-ctx-tx">{{ $t('ai.mate') }}</span>
                <span v-if="alertCount" class="mate-badge">{{ alertCount }}</span>
              </button>
              <!-- Skills：按钮文案恒定「Skills」，点开是普通条目列表（没有开关）。
                   点某个技能 = 选中它：该技能随后显示在输入框里，之后提问就按它作答；
                   再点同一个技能 = 取消，输入框回到什么都没选的自由对话 -->
              <el-dropdown ref="skillDropRef" data-g="5" trigger="click" placement="top-start" popper-class="ai-model-dropdown"
                           @command="pickSkill" @visible-change="onSkillVisible">
                <button class="skill-btn" :class="{ open: skillsOpen }" type="button"
                        v-fast-tip.always="() => $t('ai.pickSkill')">
                  <!-- 与 AI 面板品牌/「问数」等入口统一用自绘 AiMark，别混用 MagicStick（那是插件中心的图标） -->
                  <span class="skill-btn-ic"><AiMark /></span>
                  <span class="skill-btn-tx">{{ $t('ai.skills') }}</span>
                  <el-icon class="skill-btn-caret"><ArrowDown /></el-icon>
                </button>
                <template #dropdown>
                  <div class="ai-dd-head">{{ $t('ai.pickSkill') }}</div>
                  <el-dropdown-menu class="ai-model-menu skill-menu">
                    <el-dropdown-item disabled class="ai-dd-group">{{ $t('ai.builtinSkills') }}</el-dropdown-item>
                    <el-dropdown-item v-for="s in builtinSkills" :key="s.key" :command="s.key"
                                      :class="{ active: tab === s.key }" :title="s.desc">
                      <el-icon class="skills-ic"><component :is="s.icon" /></el-icon>
                      <span class="skills-name">{{ s.label }}</span>
                      <el-icon v-if="tab === s.key" class="skills-check"><Select /></el-icon>
                    </el-dropdown-item>
                    <template v-if="pluginItems.length">
                      <el-dropdown-item disabled class="ai-dd-group">{{ $t('ai.pluginSkills') }}</el-dropdown-item>
                      <el-dropdown-item v-for="p in pluginItems" :key="p.command" :command="p.command"
                                        :class="{ active: p.active }" :title="p.desc">
                        <span v-if="p.icon" class="skills-ic skill-emoji">{{ p.icon }}</span>
                        <el-icon v-else class="skills-ic"><MagicStick /></el-icon>
                        <span class="skills-name">{{ p.name }}</span>
                        <el-icon v-if="p.active" class="skills-check"><Select /></el-icon>
                      </el-dropdown-item>
                    </template>
                    <el-dropdown-item v-else disabled class="ai-dd-group">{{ $t('ai.pluginSkillsEmpty') }}</el-dropdown-item>
                  </el-dropdown-menu>
                </template>
              </el-dropdown>

              <!-- 上下文：同样复用「模型」下拉的 el-dropdown；树形数据拍平成下拉项 -->
              <!-- placement 用 top-start，但**必须**让 popper 自己避让视口：
                   上下文列表带层级缩进、项多且库名一长整块就很宽，贴着窗口右边时会整块铺到屏幕外。
                   preventOverflow 让它在视口内平移，flip 兜底上下翻转。 -->
              <el-dropdown ref="ctxDropRef" data-g="4" trigger="click" placement="top-start" popper-class="ai-model-dropdown"
                           :hide-on-click="false" @command="onCtxCommand" @visible-change="onCtxVisible"
                           :popper-options="{ modifiers: [
                             { name: 'preventOverflow', options: { boundary: 'viewport', padding: 8 } },
                             { name: 'flip', options: { fallbackPlacements: ['top-end', 'bottom-start', 'bottom-end'] } }
                           ] }">
                <button class="composer-ctx" :class="{ open: ctxOpen }" type="button"
                        v-fast-tip="() => contextLabel || $t('ai.pickContext')">
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
                      <!-- 连接行用数据库类型 logo（与连接对话框/数据源弹窗同一套图标），分组/库/模式仍用线性图标 -->
                      <DbLogo v-if="item.value.startsWith('conn:')" class="ctx-db-logo" :type="ctxConnType(item)" :size="14" />
                      <el-icon v-else :size="13" class="ctx-level-ic"><component :is="ctxLevelIcon(item)" /></el-icon>
                      <span class="ctx-label">{{ item.label }}</span>
                      <el-icon v-if="ctxSelectedValue === item.value" class="skills-check"><Select /></el-icon>
                    </el-dropdown-item>
                    <el-dropdown-item v-if="!ctxFlatItems.length" disabled class="ctx-item ctx-empty">
                      {{ $t('ai.noConn') }}
                    </el-dropdown-item>
                  </el-dropdown-menu>
                </template>
              </el-dropdown>

              <!-- 聚焦表（多选）：独立的表选择器 —— 深藏在数据源下拉的第三/四层里根本用不了
                   （用户反馈「把表挪到外面来显示」）：顶层入口 + 搜索框 + 勾选列表。
                   没选库时禁用；列表懒加载，键是 连接|库(库.模式)。 -->
              <el-dropdown ref="tbDropRef" data-g="3" trigger="click" placement="top-start" popper-class="ai-model-dropdown"
                           :hide-on-click="false" @visible-change="onTbVisible">
                <button class="composer-ctx" :class="{ open: tbOpen }" type="button"
                        :disabled="!ctxEffectiveDatabase" v-fast-tip="() => ctxTables.length ? ctxTables.join('、') : (tbLabel || $t('ai.pickTables'))">
                  <el-icon :size="12"><Tickets /></el-icon>
                  <span class="composer-ctx-tx">{{ tbLabel }}</span>
                  <span v-if="ctxTables.length" class="composer-tb-badge">{{ ctxTables.length }}</span>
                  <el-icon :size="12" class="composer-ctx-caret"><ArrowDown /></el-icon>
                </button>
                <template #dropdown>
                  <div class="ai-dd-head">{{ $t('ai.pickTables') }}</div>
                  <div class="tb-search">
                    <el-input v-model="tbKeyword" size="small" clearable :prefix-icon="Search"
                              :placeholder="$t('ai.tbSearch')" />
                  </div>
                  <el-dropdown-menu class="ai-model-menu ctx-menu tb-menu">
                    <li v-for="name in tbFiltered" :key="name"
                        class="ctx-item ctx-tb-row"
                        :class="{ 'is-checked': ctxTables.includes(name) }"
                        @click.stop="toggleCtxTable(name)">
                      <el-icon :size="13" class="ctx-level-ic"><component :is="Tickets" /></el-icon>
                      <span class="ctx-label">{{ name }}</span>
                      <el-icon v-if="ctxTables.includes(name)" class="skills-check"><Select /></el-icon>
                    </li>
                    <el-dropdown-item v-if="!tbFiltered.length" disabled class="ctx-item ctx-empty">
                      {{ tbLoading ? $t('ai.tbLoading') : $t('ai.tbNone') }}
                    </el-dropdown-item>
                  </el-dropdown-menu>
                </template>
              </el-dropdown>

              <!-- 知识库范围：控制这次提问检索哪些资料库。
                   原先是否注入完全由知识库侧的全局开关决定，问 A 系统的表却召回 B 系统的资料也无法控制 -->
              <el-dropdown ref="kbDropRef" data-g="2" trigger="click" placement="top-start" popper-class="ai-model-dropdown"
                           :hide-on-click="false" @command="onKbScopeCommand" @visible-change="onKbVisible">
                <button class="composer-ctx" :class="{ open: kbOpen }" type="button" v-fast-tip="() => kbScopeLabel">
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
              <el-dropdown ref="modelDropRef" data-g="1" trigger="click" placement="top-start" popper-class="ai-model-dropdown"
                           @command="onPickModel" @visible-change="onModelVisible">
                <button class="composer-pick" type="button" v-fast-tip="() => currentModelLabel">
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
                <!-- 发送前看一眼：内容、上下文、技能、脱敏状态一次说清（设置里可改成每次发送都弹） -->
                <button class="voice-btn send-preview-btn" type="button"
                        :title="$t('ai.sendPreview.title')" @click="openSendPreview">
                  <el-icon :size="14"><View /></el-icon>
                </button>
                <!-- 生成中变「停止」键：点一下真掐断（流式/问数/深挖都挂在统一中断入口上）。
                     停止态用实心红底 + 白色方块（停止的通用符号），与发送态的蓝色明显区分 -->
                <el-button type="primary" class="composer-run" :class="{ 'is-stop': generating }"
                           :disabled="!generating && !canRun"
                           :title="generating ? $t('ai.stopGen') : runLabel"
                           @click="generating ? stopGenerating() : runSkill()">
                  <span v-if="generating" class="stop-sq" />
                  <el-icon v-else><Promotion /></el-icon>
                </el-button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- 记事抽屉：记录（AI 对库做过的增删改）+ 记忆（长期事实）。
         两件事放一起是有意的 —— 它们都是「它替你记着的东西」。 -->
    <el-drawer v-model="mateOpen" size="600px" append-to-body class="mate-drawer">
      <template #header>
        <!-- 只留标题：开新对话已经收进左栏「记录」的 + 里，
             抽屉里再放一个同名按钮，两处入口做同一件事 -->
        <div class="mate-head">
          <span>{{ $t('ai.mate') }}</span>
        </div>
      </template>
      <div class="mate-body" v-loading="mateLoading">
        <!-- 拉不下来 ≠ 没有：失败单独说明是哪部分，并给重试，别让用户以为「本来就没东西」 -->
        <div v-if="mateError" class="mate-error">
          <span class="mate-error-txt">{{ mateError }}</span>
          <el-button size="small" text type="primary" @click="refreshMate">{{ $t('common.retry') }}</el-button>
        </div>
        <el-tabs v-model="mateTab">
          <el-tab-pane :label="$t('ai.records')" name="records">
            <!-- 工具条：条数 + 清空（清空只是删记录，不动库里的数据） -->
            <div v-if="changes.length" class="rec-bar">
              <span class="rec-count">{{ $t('ai.recordsCount', { n: changes.length }) }}</span>
              <el-button size="small" text type="danger" class="rec-clear" @click="clearChanges">
                {{ $t('ai.recordsClear') }}
              </el-button>
            </div>
            <div v-if="!changes.length && !unfinishedJobs.length && !mateFailed.includes('changes')"
                 class="mate-empty">{{ $t('ai.recordsEmpty') }}</div>
            <!-- 变更记录：AI 对数据库的每一次增删改都在这里留痕，执行过的还能撤销 -->
            <div v-for="c in changes" :key="c.id" class="rec-card" :class="'is-' + c.risk">
              <div class="rec-head">
                <span class="rec-risk">{{ riskLabel(c.risk) }}</span>
                <span class="rec-title" :title="c.title">{{ c.title }}</span>
                <span class="rec-state" :class="'st-' + c.status">{{ chgStatusLabel(c.status) }}</span>
              </div>
              <pre class="rec-sql">{{ c.sql }}</pre>
              <div class="rec-meta">
                <!-- 定位到表：连接.库.模式.表；SQL 认不出表就退到库、再退到连接名 -->
                <span class="rec-path" :title="changePathOf(c, connNameOf(c))">{{ changePathOf(c, connNameOf(c)) }}</span>
                <span class="rec-time">{{ fmtWhen(c.appliedAt || c.createdAt) }}</span>
              </div>
              <div v-if="c.error" class="rec-err">{{ c.error }}</div>
              <div class="rec-acts">
                <el-button v-if="c.status === 'pending'" size="small" type="primary"
                           :loading="changeBusy === c.id" @click="runChange(c, 'allow')">
                  {{ $t('ai.allow') }}
                </el-button>
                <el-button v-if="c.status === 'pending'" size="small" text @click="runChange(c, 'reject')">
                  {{ $t('ai.chgReject') }}
                </el-button>
                <el-button v-if="c.status === 'applied' && c.undoSql" size="small"
                           :loading="changeBusy === c.id" @click="runChange(c, 'undo')">
                  {{ $t('ai.chgUndo') }}
                </el-button>
                <span v-else-if="c.status === 'applied'" class="rec-note">{{ $t('ai.recordNoUndo') }}</span>
                <!-- 删记录：只把这条从列表里去掉，不会撤销已执行的修改（撤销是上面那个按钮） -->
                <el-button size="small" text type="danger" class="rec-del"
                           :loading="changeBusy === c.id" @click="removeChange(c)">
                  {{ $t('ai.recordDelete') }}
                </el-button>
              </div>
            </div>
            <!-- 后台任务：中断的智能体任务可以续跑（原「待办」页签里唯一有动作的部分，保留在这里） -->
            <div v-if="unfinishedJobs.length" class="rec-jobs">
              <div class="rec-jobs-title">{{ $t('ai.recordJobs') }}</div>
              <div v-for="job in unfinishedJobs" :key="job.id" class="todo-card" :class="'is-' + job.status">
                <div class="todo-head">
                  <span class="todo-status">{{ todoStatusLabel(job.status) }}</span>
                  <span class="todo-title">{{ job.title }}</span>
                </div>
                <div class="todo-result">{{ job.result }}</div>
                <div class="todo-acts">
                  <el-button v-if="job.kind === 'agent'" size="small" text type="primary"
                             :loading="jobBusy === job.id" @click="resumeJob(job)">
                    {{ $t('ai.todoResume') }}
                  </el-button>
                  <el-button size="small" text @click="dismissJob(job)">{{ $t('ai.todoDismiss') }}</el-button>
                </div>
              </div>
            </div>
          </el-tab-pane>
          <el-tab-pane :label="$t('ai.memory')" name="memory">
            <div class="mem-add">
              <el-select v-model="memScope" size="small" class="mem-scope-sel" :title="$t('ai.memScope')"
                         filterable :fit-input-width="false" popper-class="mem-scope-popper"
                         :filter-method="onMemScopeFilter" @change="memScopeTouched = true"
                         @visible-change="(v) => { if (!v) memScopeQuery = '' }">
                <!-- 选中项也带类型 logo（与左侧树同一套图标），一眼认出是哪类库 -->
                <template #prefix>
                  <DbLogo v-if="memScopeType" :type="memScopeType" :size="14" />
                  <el-icon v-else :size="13"><Connection /></el-icon>
                </template>
                <el-option value="global" :label="$t('ai.memScopeAll')">
                  <span class="mem-opt">
                    <el-icon :size="13" class="mem-opt-ic"><Connection /></el-icon>
                    <span class="mem-opt-tx">{{ $t('ai.memScopeAll') }}</span>
                  </span>
                </el-option>
                <!-- 与左侧树同一套分组口径：预置环境 → 自定义目录 → 未分组。
                     组名不是 el-option-group（那个不能点），而是一个禁用项 + 自己的点击拦截：
                     点它只切折叠，不会把选中值改掉。 -->
                <template v-for="g in memScopeRender" :key="g.key">
                  <el-option :value="'hd:' + g.key" :label="g.label" disabled class="mem-grp-hd">
                    <span class="mem-opt mem-grp" :data-grp="g.key">
                      <el-icon :size="11" class="mem-grp-caret" :class="{ 'is-open': !g.collapsed }">
                        <ArrowRight />
                      </el-icon>
                      <span class="mem-opt-tx">{{ g.label }}</span>
                      <span class="mem-grp-n">{{ g.conns.length }}</span>
                    </span>
                  </el-option>
                  <template v-if="!g.collapsed">
                    <el-option v-for="o in g.conns" :key="o.value" :value="o.value" :label="o.label">
                      <span class="mem-opt">
                        <DbLogo :type="o.type" :size="14" />
                        <span class="mem-opt-tx">{{ o.label }}</span>
                      </span>
                    </el-option>
                  </template>
                </template>
              </el-select>
              <el-input v-model="memDraft" size="small" :placeholder="$t('ai.memoryPlaceholder')"
                        @keyup.enter="addMemory" />
              <el-button size="small" type="primary" @click="addMemory">{{ $t('ai.memoryAdd') }}</el-button>
            </div>
            <div v-if="!memories.length && !mateFailed.includes('memory')" class="mate-empty">{{ $t('ai.memoryEmpty') }}</div>
            <div v-for="row in memories" :key="row.id" class="mem-card">
              <!-- 一句话事实 + 范围 + 日期 + 删除压缩成**一行**：显示不下就省略号，悬浮给全文。
                   记忆通常就一句短话，竖着堆两行纯属浪费抽屉宽度。 -->
              <div class="mem-fact" :title="row.fact">{{ row.fact }}</div>
              <div class="mem-meta">
                <!-- 范围只显示**名字**（库里存的是 conn_18e0… 这种内部 id，对用户是纯噪声）。
                     连接已被删掉时明确写「（已删除）」并给「改为全部」的出口 ——
                     否则它会静默失效：注入是按连接 id 匹配的，挂在不存在的 id 上永远不会生效。 -->
                <span class="mem-scope" :class="{ 'is-gone': row.connMissing }"
                      :title="memScopeText(row)">{{ memScopeText(row) }}</span>
                <span v-if="row.createdAt" class="mem-date">{{ String(row.createdAt).slice(0, 10) }}</span>
                <button v-if="row.connMissing" class="mem-fix" type="button"
                        :title="$t('ai.memDetachTip')" @click="detachMemory(row)">{{ $t('ai.memDetach') }}</button>
                <button class="mem-del" type="button" :title="$t('ai.memoryDelete')" @click="removeMemory(row)">
                  <el-icon :size="13"><Delete /></el-icon>
                </button>
              </div>
            </div>
          </el-tab-pane>
        </el-tabs>
      </div>
    </el-drawer>

    <!-- SQL 快捷验证：AI 给的 SQL 先看执行计划 / 试跑一次（不消耗 AI 调用） -->
    <SqlProbeDialog v-model="probeVisible" :sql="probeSql" :conn-id="ctxConnId"
                    :database="ctxEffectiveDatabase" :mode="probeMode" @insert="onProbeInsert" />

    <!-- 「这次到底发给模型什么了」：左栏标题旁的记录图标唤出，只读内存快照 -->
    <AiRequestLog v-model="reqLogOpen" @replay="onReqReplay" />
    <!-- 发送前预览：与事后记录配套，让用户发之前也能看清内容 / 上下文 / 脱敏状态 -->
    <AiSendPreview v-model="sendPreviewOpen" :question="sendPreviewQuestion"
                   :connection-name="ctxConnName" :database="ctxEffectiveDatabase"
                   :tables="ctxTables || []" :skills="currentSkills()"
                   @confirm="confirmSendPreview" />
  </div>
</template>

<script setup>
import { ref, reactive, computed, nextTick, onMounted, onBeforeUnmount, watch, defineAsyncComponent, toRef } from 'vue'
import { saveGenerated } from '../../utils/useExportTask'
import { ElMessage, ElMessageBox } from 'element-plus'
import { ChatDotRound, DocumentCopy, Close, Cpu, Notebook, Download, Grid, Search, Star, Aim, DataAnalysis, Folder, Loading, ArrowDown, ArrowLeft, ArrowRight, Select, Coin, Connection, Promotion, Switch, Plus, Edit, Collection, Microphone, Bell, Tickets, List, Delete, MagicStick, View } from '@element-plus/icons-vue'
import AiMark from '../../common/AiMark.vue'
import AiChangeCard from './AiChangeCard.vue'
import AiToolDetail from './AiToolDetail.vue'
import AiRequestLog from './AiRequestLog.vue'
import AiSendPreview from './AiSendPreview.vue'
import { aiChat, aiChatStream, aiDataDict, aiInsight, aiAskStream, aiWarmup, aiExportDoc, getAiConfig, listConnections, listDatabases, listSchemas, listTables, kbList, kbSearch,
  aiAgentStream, aiJobResumeStream, aiThreadList, aiThreadGet, aiThreadDelete, aiThreadTrim, aiThreadRename, aiMemoryList, aiMemoryAdd, aiMemoryDelete,
  aiMemoryDetach,
  aiChangeList, aiChangeAllow, aiChangeApprove, aiChangeConfirm, aiChangeApply, aiChangeUndo, aiChangeReject,
  aiChangeDelete, aiChangeClear,
  aiJobList, aiJobDismiss, aiJobResume, listPlugins, getSettings, putSetting } from '../../api'
import KbIngestPanel from './KbIngestPanel.vue'
import { renderMarkdown, extractCodeBlocks } from '../../utils/markdown'
import { t, te, locale } from '../../utils/i18n'
import { schemaLevelOf, hideDbLevelOf } from '../../types'
import DbLogo from '../../common/DbLogo.vue'
import { envLabel, PREDEF_ENVS } from '../../utils/envs.js'
import SqlProbeDialog from '../../common/SqlProbeDialog.vue'
// 变更单执行被「AI 写库总开关」拦下时，弹带「去设置」的通知（与其他安全拦截同一套出口）
import { showSafetyNotice, openSafetySettings } from '../../utils/safetyNotice'
// 记录/变更单卡片上的定位路径（连接.库.模式.表，认不出表就退到上一级）
import { changePathOf } from '../../utils/sqlTarget'
// 连接 id → 名字的本地记忆：连接被删掉后（记录里只剩 id）靠它把名字补回来
import { connNameOf as connNameById } from '../../utils/connNames'
// 问数的流式步骤合并（running → done 原地升级）：与独立问数工作区共用同一份实现
import { mergeStep } from '../chatbi/askSteps.js'
import {
  listAskHistory, saveAskHistory, removeAskHistory, ensureAskHistoryStore
} from '../chatbi/askHistory.js'
// 用量脚注：与 SQL 查询里的 AI 弹窗共用同一个组件，保证「token 消耗」只有一种长相
import AiUsageNote from '../../common/AiUsageNote.vue'
// 智能问数回答卡片：懒加载（它连着 echarts，不进 AI 助手的首屏 chunk；问到才拉）
const AskAnswer = defineAsyncComponent(() => import('../chatbi/AskAnswer.vue'))

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
  pending: Object,
  /**
   * 面板是否**真的已经露出来**。
   *
   * <p>右侧窄面板用 `v-show` 常驻挂载（切页签/收起来都不销毁，对话与选中上下文才不会丢），
   * 于是它的 `onMounted` 在首屏就跑了一遍 —— 可它此刻是隐藏的：知识库清单、待办、记忆
   * 这些「只在面板里看得到」的数据白拉三笔（待办 11KB + 记忆 12KB 是大头）。
   *
   * <p>studio 形态（中央工作区页签）不传这个 prop，默认 `true`，行为与以前完全一致。
   */
  active: { type: Boolean, default: true }
})
/** 是否为「中央工作区」形态 */
const isStudio = computed(() => props.mode === 'studio')
/**
 * panel：斜杠命令要求打开某个面板（巡检 / 治理 / 设置），由外层接管
 * run-plan：「指令」页签解析出的可执行操作，交由外层执行（与命令面板同一套逻辑）
 */
const emit = defineEmits(['insert-sql', 'close', 'panel', 'run-plan', 'open-knowledge'])

// 技能页签 / 对话 / 问数记录上下文都放在跨卸载的模块级 store（panelState.js）：
// 中央工作区切走页签再切回来，对话原样还在 —— 只有「新对话」才清空
import { panelState } from './panelState.js'
import {
  ensureAiPanelPrefs, readAiComposer, readAiSession, saveAiComposer, saveAiSession
} from './panelPrefs.js'
import fastTip from '../../utils/fastTip'
// 模板里的 v-fast-tip（底栏 chips 收成图标后的快速悬浮提示，120ms）
const vFastTip = fastTip
/** 形态：studio（中央工作区）/ panel（右侧窄面板）—— 两份状态各存各的，同时挂着也不互相覆盖 */
const _mode = props.mode === 'studio' ? 'studio' : 'panel'
const _ps = panelState[_mode]
const tab = toRef(_ps, 'tab')

/**
 * Skills 列表里的「内置能力」。
 *
 * <p>**自由对话不在列表里**：它就是默认态（不选任何技能时的那段对话），
 * 列进去等于让用户在「技能」里挑一个「不用技能」——零信息量的一个选项。
 *
 * <p>这里留下的只有**对话替代不了**的四个工作台：要读真实表结构并产出可导出报告的
 * 数据字典 / 数据洞察、要画图与明细的智能问数、要上传解析文件的的知识入库。
 * 纯提示词就能做的能力（SQL 审查、优化、诊断、建表规范……）一律做成插件，
 * 按「插件技能」分组列在同一下拉里。
 */
// ⚠️ 必须是 computed：写死成普通数组只会在模块加载时求值一次，之后切语言不会跟着变
const tabs = computed(() => [
  // 智能问数：就是本面板里的一个技能页（模板里 v-if="isAsk" 渲染 ChatBiView）。
  // 刻意**不**单开页签、也不上顶栏菜单 —— 入口只有这里一处。
  { key: 'ask', label: t('ai.tabAsk'), icon: DataAnalysis, desc: t('ai.tabAskDesc') },
  { key: 'dictionary', label: t('ai.tabDictionary'), icon: Notebook, desc: t('ai.tabDictionaryDesc') },
  { key: 'insight', label: t('ai.tabInsight'), icon: Grid, desc: t('ai.tabInsightDesc') },
  { key: 'kbIngest', label: t('ai.tabKbIngest'), icon: Collection, desc: t('ai.tabKbIngestDesc') }
])

/** 当前选中的 Skill；`chat` 是默认态（不在 tabs 里），单独兜一个虚拟项 */
/** Skills 下拉的「内置能力」分组：智能问数不再内置，入口只由已安装的问数插件提供。
 *  tabs 本身仍保留 ask（问数页渲染 / openAskHistory / 斜杠命令都靠它），只是不在此分组露出。 */
const builtinSkills = computed(() => tabs.value.filter(t => t.key !== 'ask'))

const currentSkill = computed(() => tabs.value.find(t => t.key === tab.value)
  || { key: 'chat', label: t('ai.tabChat'), icon: ChatDotRound, desc: t('ai.tabChatDesc') })

// ===== 插件技能（设置 → 插件中心安装的声明式扩展包）=====
// 它们**不是**内置 tab：数量和名字都由装了什么决定。选中后既不弹窗也不切页 ——
// 提示词由后端拼进 system（plugins::augment_system），之后每一轮对话都按它的要求作答。
// 这样「所有技能都用对话完成」，用户不用学第二套表单。
const pluginSkills = ref([])
/** 已启用的「智能问数」插件（type=ask）：装了就在问数里自动生效，不需要选技能。
 *  单列出来只是为了让用户看得见"装了什么"，不参与勾选（否则 prompt 会重复注入）。 */
const askPlugins = ref([])
/** 已选技能（跨卸载保留）：`[{ id, name, description }]` */
const pickedSkills = toRef(_ps, 'pickedSkills')
/** 上次拉清单的时刻：打开下拉会顺带刷新（刚装的插件要能马上看到），5 秒内不重复请求 */
let pluginLoadedAt = 0
let pluginLoadSeq = 0
const loadPluginSkills = async (force = false) => {
  if (!force && Date.now() - pluginLoadedAt < 5000) return
  const seq = ++pluginLoadSeq
  try {
    const data = await listPlugins()
    // 安装/停用会触发强制刷新：慢的旧响应不能覆盖新清单和已选技能
    if (seq !== pluginLoadSeq) return
    pluginLoadedAt = Date.now()
    const enabled = (data?.items || []).filter((p) => p.state === 'enabled')
    // 插件图标统一在数据层兜底：新插件只要 manifest 配了 icon，Skills 下拉 /
    // 斜杠命令 / 已选 chip / 历史消息标签都会显示它；万一没配（老包 / 第三方包），
    // 这里补 🧩，免得某个入口漏出来一个光秃秃的 MagicStick。
    const withIcon = (p) => (p.icon ? p : { ...p, icon: '🧩' })
    pluginSkills.value = enabled.filter((p) => p.type === 'skill').map(withIcon)
    askPlugins.value = enabled.filter((p) => p.type === 'ask').map(withIcon)
    // 插件被卸载/停用后，把已选里的死项摘掉（否则会一直占着 chip、还继续拼 system）
    const alive = new Set(pluginSkills.value.map((p) => p.id))
    pickedSkills.value = pickedSkills.value.filter((s) => alive.has(s.id))
  } catch { /* 插件清单拉不到不影响内置技能 */ }
}
/** 插件技能统一清单：问数插件（点它切到问数页）排前，普通技能（点它叠加生效）排后，
 *  同属「插件技能」一个分组 —— 不再为问数插件单开分组。 */
const pluginItems = computed(() => [
  ...askPlugins.value.map(p => ({
    command: 'ask:' + p.id, id: p.id, name: p.name, icon: p.icon,
    desc: t('ai.pluginAskAutoTip'), active: tab.value === 'ask'
  })),
  ...pluginSkills.value.map(p => ({
    command: 'plugin:' + p.id, id: p.id, name: p.name, icon: p.icon,
    desc: p.description, active: isPicked(p.id)
  }))
])

const pickedIds = computed(() => new Set(pickedSkills.value.map((s) => s.id)))
const isPicked = (id) => pickedIds.value.has(id)
/** 选中 / 取消一个插件技能（可多选，点一下切换） */
const togglePickedSkill = (id) => {
  const i = pickedSkills.value.findIndex((s) => s.id === id)
  if (i >= 0) {
    const gone = pickedSkills.value[i]
    pickedSkills.value.splice(i, 1)
    ElMessage.info(t('ai.skillUnpicked', { name: gone.name }))
    return false
  }
  const item = pluginSkills.value.find((p) => p.id === id)
  if (!item) return false
  pickedSkills.value.push({ id: item.id, name: item.name, icon: item.icon || '', description: item.description || '' })
  // 技能靠对话生效：在问数/字典这些页里选中的，直接切回自由对话，否则选了也看不到效果
  if (tab.value !== 'chat') {
    tab.value = 'chat'
    ElMessage.success(t('ai.skillPickedChat', { name: item.name }))
  } else {
    ElMessage.success(t('ai.skillPicked', { name: item.name }))
  }
  return true
}
const unpickSkill = (id) => { togglePickedSkill(id) }
/** 已选中的内置能力（chat 是默认态，不算「选中了技能」）—— 输入框里只显示它一个 */
const activeSkill = computed(() => (tab.value === 'chat' ? null : currentSkill.value))
/** 摘掉内置技能 = 回到自由对话 */
const backToFreeChat = () => { tab.value = 'chat' }

/**
 * 这一问生效的技能快照：内置技能（切页的那个）算一个 + 已选插件技能若干。
 *
 * <p>发消息时写进消息对象 → 气泡里显示成「[技能] 内容」；同时把名字发给后端落库
 * （meta.skills），这样重开历史记录时标签还在。只存名字不存提示词：插件升级后
 * 提示词会变，把旧提示词快照进库反而误导。
 */
const currentSkills = () => {
  const out = []
  if (activeSkill.value) out.push({ id: activeSkill.value.key, name: activeSkill.value.label })
  for (const s of pickedSkills.value) out.push({ id: s.id, name: s.name, icon: s.icon || '' })
  return out
}

/** 按技能名找图标：历史会话只存了名字，靠插件清单补回来（找不到就不显示） */
const iconOfSkill = (name) => pluginSkills.value.find((p) => p.name === name)?.icon || ''

const skillsOpen = ref(false)
/** 几个下拉互斥：每个都持有 ref，打开一个就收起其它 */
const skillDropRef = ref(null)
const ctxDropRef = ref(null)
const modelDropRef = ref(null)
const closeSkillDrop = () => skillDropRef.value?.handleClose?.()
const closeCtxDrop = () => ctxDropRef.value?.handleClose?.()
const closeModelDrop = () => modelDropRef.value?.handleClose?.()
const tbDropRef = ref(null)
const closeTbDrop = () => tbDropRef.value?.handleClose?.()

/** Skills 下拉展开：刷插件清单（刚在设置里装的要能立刻出现），并收起其它下拉 */
const onSkillVisible = (visible) => {
  skillsOpen.value = visible
  if (!visible) return
  closeCtxDrop()
  closeModelDrop()
  closeTbDrop()
  loadPluginSkills()
}

// ===== 聚焦表（多选）：工具条上独立的表选择器 =====
const tbOpen = ref(false)
const tbItems = ref([])          // 当前上下文的全部表名
const tbLoading = ref(false)
const tbKeyword = ref('')
const tbKey = ref('')            // 连接|库(库.模式)：换上下文自动重拉
const tbFiltered = computed(() => {
  const kw = tbKeyword.value.trim().toLowerCase()
  if (!kw) return tbItems.value
  return tbItems.value.filter(n => n.toLowerCase().includes(kw))
})
const tbLabel = computed(() => ctxTables.value.length === 1 ? ctxTables.value[0]
  : ctxTables.value.length ? t('ai.tbCount', { n: ctxTables.value.length })
    : t('ai.pickTables'))
/** 打开时懒加载表清单：键没变（还是同一个连接+库）就不重复请求 */
const loadTbItems = async () => {
  const key = `${ctxConnId.value || ''}|${ctxEffectiveDatabase.value || ''}`
  if (!ctxEffectiveDatabase.value) { tbItems.value = []; tbKey.value = key; return }
  if (tbKey.value === key) return
  tbKey.value = key
  tbItems.value = []
  tbLoading.value = true
  try {
    const raw = await listTables(ctxConnId.value, ctxEffectiveDatabase.value)
    // 请求期间用户切了连接/库：旧响应必须丢弃，否则勾选的会是旧库的表
    if (tbKey.value !== key) return
    tbItems.value = (raw || [])
      .map(x => (typeof x === 'string' ? x : (x.name || x.table || '')))
      .filter(Boolean)
  } catch { if (tbKey.value === key) tbItems.value = [] }
  finally { if (tbKey.value === key) tbLoading.value = false }
}
/** 表下拉展开/收起：关其它下拉 + 懒加载表清单（搜索词每次清空，重新找） */
const onTbVisible = (visible) => {
  tbOpen.value = visible
  if (!visible) return
  closeSkillDrop()
  closeCtxDrop()
  closeModelDrop()
  kbDropRef.value?.handleClose?.()
  tbKeyword.value = ''
  loadTbItems()
}
/** 勾选/取消一张聚焦表：只动 ctxTables —— 不关下拉、不换连接/库上下文 */
const toggleCtxTable = (name) => {
  const i = ctxTables.value.indexOf(name)
  if (i >= 0) ctxTables.value.splice(i, 1)
  else ctxTables.value.push(name)
}
/** 选择 Skill：切换能力。返回是否真的切了（斜杠命令用它决定要不要提示） */
const pickSkill = (key) => {
  // 插件技能：选中/取消（可多选），不切页 —— 它在对话里叠加生效
  if (typeof key === 'string' && key.startsWith('plugin:')) {
    return togglePickedSkill(key.slice('plugin:'.length))
  }
  // 问数插件：智能问数已不做内置入口，点插件项就是切到问数页
  //（tabs 里仍留 ask 供问数页渲染 / openAskHistory / 斜杠命令使用）
  if (typeof key === 'string' && key.startsWith('ask:')) {
    tab.value = tab.value === 'ask' ? 'chat' : 'ask'
    return true
  }
  if (!tabs.value.some((x) => x.key === key)) return false
  // 再点一次已选中的那个 = 取消，回到自由对话（输入框里那个 chip 也会消失）
  tab.value = tab.value === key ? 'chat' : key
  return true
}

/** 技能运行的错误统一走这里：AI 服务未配置这类「去设置里弄一下」的引导性错误
 *  弹确认框（带「前往设置」直达），其余照常 toast —— 用户不用再自己找设置入口。 */
const aiError = (message) => {
  const text = message == null ? '' : String(message)
  if (/请先在「设置」|Enable and configure the AI service/.test(text)) {
    ElMessageBox.confirm(text, t('sqlq.aiNotConfiguredTitle'), {
      confirmButtonText: t('sqlq.gotoAiSettings'),
      cancelButtonText: t('common.cancel'),
      type: 'warning',
    }).then(() => window.dispatchEvent(new CustomEvent('dc-open-settings', { detail: { tab: 'ai' } }))).catch(() => {})
    return
  }
  ElMessage.error(text || t('ai.runFailed'))
}

/** 对话类 Skill（走消息流，而不是一次性结果卡片） */
const isChatSkill = computed(() =>
  ['chat', 'dictionary', 'insight', 'ask'].includes(tab.value))
const isKbIngest = computed(() => tab.value === 'kbIngest')

/** 统一输入框：所有 Skill 共用，行为由当前 Skill 决定 */
const aiInput = ref('')

/** 各 Skill 的输入框提示语 */
const PLACEHOLDERS = computed(() => ({
  chat: t('ai.phChat'),
  dictionary: t('ai.phDictionary'),
  insight: t('ai.phInsight'),
  ask: t('ai.phAsk')
}))
const composePlaceholder = computed(() => {
  // 选中技能后，提示语换成该技能的一句话说明 —— 用户不必去猜「这个技能是干嘛的」。
  // 插件（对话人格）优先，其次内置能力，都没选才是各页的默认提示。
  const picked = pickedSkills.value[pickedSkills.value.length - 1]
  if (picked?.description) return picked.description
  if (activeSkill.value?.desc) return activeSkill.value.desc
  return PLACEHOLDERS.value[tab.value] || t('ai.inputPlaceholder')
})

/** 当前数据上下文（连接 · 数据库 · 模式），显示在输入框工具条，让「AI 基于哪个库回答」一目了然 */
const contextLabel = computed(() => {
  const conn = (connList.value || []).find(c => String(c.id) === String(ctxConnId.value))
  const name = conn?.name || conn?.type || props.conn?.name || props.conn?.type || ''
  // Oracle / DM 这类「库就是连接固有目标」的类型（见 hideDbLevelOf）：上下文里**没有"库"这一层**，
  // 把服务名（FREE）在这儿再拼一遍只是重复 —— 左侧树、SQL 编辑器工具条都已经不显示它了，
  // 同一个东西不该在三处有两种说法。
  const dbPart = hideDbLevelOf(conn?.type || props.conn?.type) ? '' : ctxDatabase.value
  // 聚焦表在**自己的 pill**上显示（「表」选择器），不再拼进数据源标签里
  return [name, dbPart, ctxSchema.value].filter(Boolean).join(' · ')
})

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
 * 默认「不检索」：检索会把库里的片段拼进提示词 —— 问「这张表怎么设计」这类问题时，
 * 无关资料既增加往返耗时，又容易把回答带偏（还会在回答下方摆一串"引用了哪些资料"）。
 * 需要的人自己点一下「全部资料库」即可，属于**用的时候开**，而不是默认开着。
 *
 * 选择会跟着底栏其他项一起存（panelPrefs 的 ai.composer.<mode>）：用户 2026-11-19 明确
 * 要求「刷新之后底栏已经选择的东西不要变」—— 刷新/重启回到刚才那档，只有新用户是 none。
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
  closeTbDrop()
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
  { name: t('ai.featQuery'), desc: t('ai.featQueryDesc'), icon: AiMark },
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

// 对话历史以**服务端会话**为准：面板打开时恢复最近一次，「新对话」按钮清空。
// 只有这个面板会把 persist 传给后端 —— 一次性接口（测试连接、知识库问答）不落库。
// 三者都放跨卸载的模块级 store（panelState.js）：中央工作区切页签卸载也不丢
const messages = toRef(_ps, 'messages')
/** 当前会话 id（空 = 还没开始，第一句会带着 persist 让后端建） */
const chatThreadId = toRef(_ps, 'chatThreadId')
/** 当前问数对话的本地记录 id：追问并进同一条「记录」，新对话才另起一条 */
const askThreadId = toRef(_ps, 'askThreadId')
/** 深度思考模式：走 /api/ai/agent，让它自己查库、出变更单 */
const agentMode = ref(false)
/** 记事抽屉：记录（变更历史）+ 记忆 */
const mateOpen = ref(false)
const mateTab = ref('records')
const todos = ref([])
/** 变更记录：AI 对数据库的每次增删改都会落一行（含待执行的单子） */
const changes = ref([])
const memories = ref([])
const mateLoading = ref(false)
/** 记事抽屉的加载错误：失败不能显示成「暂无记录 / 暂无记忆」 */
const mateError = ref('')
/** 加载失败的部分：'changes' / 'memory'（空态只在对应部分**真的**拉下来后才显示） */
const mateFailed = ref([])
/** 审批流水里记的名字：桌面端没有登录，名字让用户自己填 */
const approverName = ref(localStorage.getItem('dbmind.ai.approver') || 'user')
/** 正在跑的变更单 id：按钮转圈用，避免连点两次执行 */
const changeBusy = ref('')
/** 正在续跑的待办 id */
const jobBusy = ref('')

const chatLoading = ref(false)
/** 结果区滚动容器（新消息时滚到底） */
const chatBox = ref(null)
/**
 * 等待中的三点气泡：对话一旦自己有了占位（流式消息 / 问数卡片 / 已流出的内容），
 * 就**不该再挂第二个等待态** —— 否则上一段结论下面会多出一个空气泡（只写「… 3s」），
 * 用户看到的就是「一个空框」（真机反馈）。
 */
const showTyping = computed(() => {
  if (!chatLoading.value) return false
  const last = messages.value[messages.value.length - 1]
  if (!last) return true
  const occupied = last.role === 'assistant'
    && (last.streaming || last.askAnswer?.streaming || (last.timeline && last.timeline.length) || last.content)
  return !occupied
})

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
/** 流式输出阶段：chatLoading 已撤（等待态结束）但流还在读，主按钮要保持「停止」态 */
const streaming = ref(false)
/** 非流式请求（问数/深挖/字典/洞察）的进行中控制器：点停止时一并掐断 */
let stopCtrl = null
/** axios 的中断错误：各 catch 里要识别成「用户主动停」而不是当失败弹 ❌ */
const isCanceled = (e) => !!e && (
  e.code === 'ERR_CANCELED' || e.name === 'CanceledError'
  // agentSse 在用户主动停时抛的是本地化文本 Error，不是 axios 取消错误
  || String(e.message || '').includes(t('api.generationStopped'))
)
/** 生成中（等待或流式输出）：主按钮切换为停止键 */
const generating = computed(() => chatLoading.value || streaming.value)
/** 点停止：能掐的都掐 —— 流式对话、问数流、以及挂在 signal 上的普通请求 */
const stopGenerating = () => {
  stopStream()
  if (stopCtrl) {
    const c = stopCtrl
    stopCtrl = null
    try { c.abort() } catch { /* ignore */ }
  }
}

const mdCache = new Map()
/** markdown → HTML（带缓存）。sqlActions=true 时每个 SQL 代码块会带上操作按钮，
 *  与 extractCodeBlocks 的索引一一对应；流式过程中先不给按钮（内容还没稳定）。 */
const renderMd = (text, sqlActions = false) => {
  const src = text == null ? '' : String(text)
  const key = (sqlActions ? '1' : '0') + '\u0000' + src
  const hit = mdCache.get(key)
  if (hit !== undefined) return hit
  let html = renderMarkdown(src, { sqlActions })
  // 引导性错误（AI 服务未配置这类「去设置里弄一下」的消息）→ 消息尾部追加直达链接，
  // 点击由气泡上的委托（onMdSqlAction）转成打开设置。只匹配已知的固定文案，不碰其它内容。
  const goto = /请先在「设置」|启用并配置 AI 服务|Enable and configure the AI service/.test(src) ? 'ai' : ''
  if (goto) html += `<a class="md-goto" data-goto="${goto}" href="javascript:;">${t('ai.gotoSettings')} →</a>`
  if (mdCache.size > 300) mdCache.clear()
  mdCache.set(key, html)
  return html
}

/** 一段文字开头的步骤标题（`**① 看清表结构**`）；没有就返回空 */
const stepHeadingOf = (text) => {
  const first = String(text || '').trim().split('\n')[0] || ''
  const m = first.match(/\*\*([^*]{1,40})\*\*/)
  return m ? m[1].trim() : ''
}

const stepHeadingRe = /^\s*\*\*[^*\n]{1,40}\*\*/

/** 按「步骤标题行」把一段文字切成几步（模型常在一段里连着写 ①②③） */
const splitByStep = (text) => {
  const out = []
  let buf = []
  for (const line of String(text || '').split('\n')) {
    if (stepHeadingRe.test(line) && buf.join('').trim()) {
      out.push(buf.join('\n'))
      buf = []
    }
    buf.push(line)
  }
  if (buf.join('').trim()) out.push(buf.join('\n'))
  return out
}

/**
 * 把时间线切成「步骤块」：一步 = 它的叙述 + 它调用的工具。
 *
 * 为什么要切块：一条时间线平铺时，①的结论、②的标题、中间夹的工具全贴着，
 * 读起来就是一大片（用户：「①②是不是跑一起去了」）。切块后每步一个圆角卡片，
 * 边界一眼可见。变更单与分隔线自己单独一块（不加底色）。
 */
const stepGroups = (m) => {
  const out = []
  let cur = null
  const timeline = (m && m.timeline) || []

  /** 一段文字落位：有步骤标题就开新卡（或接回同名那一步），没有就接在当前卡里 */
  const placeText = (p, pi, text) => {
    const head = stepHeadingOf(text)
    if (!head) {
      if (!cur) {
        cur = { solo: false, head: '', parts: [] }
        out.push(cur)
      }
      cur.parts.push({ p: { ...p, text }, pi })
      return
    }
    if (cur && cur.head === head) {
      // 同一步的后续叙述（工具回来之后又写了一段）：接在本步的卡里，去掉重复的标题行
      const body = text.replace(/^\s*\*\*[^*\n]{1,40}\*\*\s*\n?/, '')
      if (body.trim()) cur.parts.push({ p: { ...p, text: body }, pi })
      return
    }
    cur = { solo: false, head, parts: [{ p: { ...p, text }, pi }] }
    out.push(cur)
  }

  timeline.forEach((p, pi) => {
    if (p.kind === 'change' || p.kind === 'divider') {
      out.push({ solo: true, head: '', parts: [{ p, pi }] })
      cur = null
      return
    }
    if (p.kind === 'text' || p.kind === 'think') {
      // 一段里写了 ①② 两步是常事：按标题行切开，各进各的卡
      for (const section of splitByStep(p.text)) placeText(p, pi, section)
      return
    }
    if (!cur) {
      cur = { solo: false, head: '', parts: [] }
      out.push(cur)
    }
    cur.parts.push({ p, pi })
  })
  return out.filter((g) => g.parts.length)
}

/** 两段 usage 相加（续跑会多几轮调用）；缺一边就取有的那边 */
const addUsage = (a, b) => {
  if (!a) return b || null
  if (!b) return a
  const num = (v) => (Number.isFinite(Number(v)) ? Number(v) : 0)
  return {
    promptTokens: num(a.promptTokens) + num(b.promptTokens),
    completionTokens: num(a.completionTokens) + num(b.completionTokens),
    totalTokens: num(a.totalTokens) + num(b.totalTokens),
  }
}

// ===== 斜杠命令：在输入框里敲 / 直接唤起功能，无需用文字描述需求 =====

/** 各能力页签的一句话说明（作为命令副标题） */
const TAB_DESC = computed(() => ({
  insight: t('ai.hintInsight'),
  dictionary: t('ai.hintDictionary'),
  kbIngest: t('ai.hintKbIngest'),
  ask: t('ai.tabAskDesc')
}))

/**
 * `/` 唤起的列表 = Skills 下拉的**同一份内容**（内置能力 + 插件技能），不多放别的。
 *
 * <p>以前这里还混着一组「面板直达」（敏感数据 / 数据容量 / 质量规则 / 新建连接…）。
 * 那些是「跳去别的页面」，不是技能 —— 混在技能列表里，用户按 / 想选技能却先看到
 * 一堆跳转项。要跳页走命令面板（Ctrl/⌘+K）或左侧导航。
 */
const slashCommands = computed(() => [
  // 顺序与下拉一致：先内置能力，再插件技能
  ...builtinSkills.value.map(sk => ({
    key: 'tab:' + sk.key, label: sk.label, desc: TAB_DESC.value[sk.key] || '', icon: sk.icon,
    group: t('ai.builtinSkills'), action: 'tab', target: sk.key
  })),
  // 插件图标用 manifest 里的 emoji（与 Skills 下拉一致），没配才退回 MagicStick
  ...askPlugins.value.map(p => ({
    key: 'ask:' + p.id, label: p.name, desc: t('ai.pluginAskAutoTip'), icon: p.icon || MagicStick,
    group: t('ai.pluginSkills'), action: 'ask', target: p.id
  })),
  ...pluginSkills.value.map(p => ({
    key: 'plugin:' + p.id, label: p.name, desc: p.description || '', icon: p.icon || MagicStick,
    group: t('ai.pluginSkills'), action: 'plugin', target: p.id
  }))
])

const slashOpen = ref(false)
const slashIndex = ref(0)

/** 仅当输入以 / 开头、且后面还没敲空格时进入命令模式（避免正常句子里带 / 时误触发） */
const onComposeInput = () => {
  slashOpen.value = /^\/\S*$/.test(aiInput.value)
  if (!slashOpen.value) return
  slashIndex.value = 0
  // 命令列表里含插件技能：刚在设置里装了新的，这里要能立刻看到（内部有 5s 节流）
  loadPluginSkills()
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
  // 插件技能：选中/取消（可多选 —— 再按一次 / 继续选，或直接在 Skills 下拉里连点）
  if (cmd.action === 'plugin') {
    slashOpen.value = false
    aiInput.value = ''
    togglePickedSkill(cmd.target)
    return
  }
  slashOpen.value = false
  aiInput.value = ''
  if (cmd.action === 'ask') {
    if (pickSkill('ask:' + cmd.target)) ElMessage.success(t('ai.switchedTo', { name: cmd.label }))
    return
  }
  if (cmd.action === 'tab') {
    // 统一走 pickSkill：技能斜杠命令是自动从内置能力派生的 ——
    // 这里直接赋 tab 会切到没有渲染分支的 key（空白页），必须由 pickSkill 拦下来开页签。
    // 再选一次同一个 = 取消（与下拉的行为一致）
    if (pickSkill(cmd.target)) ElMessage.success(t('ai.switchedTo', { name: cmd.label }))
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
  ctxTables.value = p.table ? [p.table] : []
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

// ===== 上下文（连接 / 数据库 / 模式 / 聚焦表）：工具条上可逐级选择 =====
// 放跨卸载的模块级 store：切页签 / 切技能回来，选好的连接、库和聚焦表都还在 ——
// 之前是局部 ref，卸载即清，问数拿不到聚焦表就走「12 张兜底」（用户：我明明选了表）。
const ctxConnId = toRef(_ps, 'ctxConnId')
const ctxDatabase = toRef(_ps, 'ctxDatabase')
const ctxSchema = toRef(_ps, 'ctxSchema')
/**
 * 聚焦表的来源有两条路：右键「发送到 AI」带进单张表（applyPending），
 * 数据源下拉里勾选（toggleCtxTable，可多选）。最终随请求的 tables 字段发给后端，
 * 与「提问里点到的表」求并集作为候选 —— 消除「问题里写 dify.orders、
 * 工具条却只到 dify」这种看着像选错了库的错觉。
 */
/** **聚焦表（多选）**：数据源下拉里勾选的表，随请求发给后端（ask 的 tables 字段是数组，
 *  后端会把「点名的表」和「提问里点到的表」求并集）。
 *  注意后端从来没有单数的 table 字段 —— 之前发 table:xxx 是被 serde 静默丢掉的，聚焦从未生效过。 */
const ctxTables = toRef(_ps, 'ctxTables')
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
  // Oracle 这类「库就是连接固有目标」的类型，上下文里**没有库这一层**（见 hideDbLevelOf）：
  // 库值直接用连接自己的 database，否则去掉库层后请求会变成"没有库"。
  const base = ctxDatabase.value
    || (hideDbLevelOf(ctxConn.value?.type) ? String(ctxConn.value?.database || '') : '')
  if (!base) return ''
  if (ctxNeedSchema.value && ctxSchema.value) return `${base}.${ctxSchema.value}`
  return base
})
/** 当前选中项（取最深一层）对应的下拉 value，用于高亮 */
const ctxSelectedValue = computed(() => {
  if (ctxSchema.value) return 'sc:' + ctxSchema.value
  if (ctxDatabase.value) return 'db:' + ctxDatabase.value
  if (ctxConnId.value) return 'conn:' + ctxConnId.value
  return ''
})
/** 层级图标：分组 / 连接 / 数据库 / 模式（按前缀 —— 连接挂分组后不再固定在第 0 层）。
 *  连接行不走这里：模板里直接用 DbLogo 渲染数据库类型 logo。 */
const ctxLevelIcon = (item) =>
  item.value.startsWith('grp:') ? Folder
    : item.value.startsWith('conn:') ? Connection
      : item.value.startsWith('db:') ? Coin
        : Grid

/** 连接的类型 code（DbLogo 按它取 logo）；找不到就空串 → DbLogo 自兜底 */
const ctxConnType = (item) => {
  const id = ctxUnwrap(item.value)
  const conn = (connList.value || []).find(c => String(c.id) === String(id))
  return conn?.type || ''
}

// 数据源变化时后台「预热」表结构上下文：
// 冷缓存下首次构建要跑十几次元数据/样例查询（实测可达 18s），提前热好，
// 用户真正提问时首字就只需等模型本身。
let warmTimer = null
const warmCtx = () => {
  if (!ctxConnId.value) return
  clearTimeout(warmTimer)
  warmTimer = setTimeout(() => {
    if (!aiReady.value) return // 未配置 AI：预热请求也不发，别给控制台留 400 痕迹
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
  closeTbDrop()
  if (ctxFlatItems.value.length === 0) await loadCtxConnections()
  await revealCtxSelection()
}

/** 展开下拉中指定 value 的项（已展开则跳过） */
const expandCtxByValue = async (value) => {
  const idx = findCtxIndex(value)
  if (idx < 0 || ctxFlatItems.value[idx].expanded) return
  await expandCtxItem(idx)
}

/** 把当前选中的连接 / 库在下拉里逐级展开，便于看到并确认当前上下文。
 *  连接挂在分组下：先把它的祖先分组逐级展开（grp:父 → grp:子），连接项才在列表里。 */
const revealCtxSelection = async () => {
  if (!ctxConnId.value) return
  const conn = (connList.value || []).find(c => String(c.id) === String(ctxConnId.value))
  const path = String(conn?.group || '').trim()
  const segs = path ? path.split('/') : []
  for (let i = 1; i <= segs.length; i++) await expandCtxByValue('grp:' + segs.slice(0, i).join('/'))
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

// ===== 底栏已选状态的持久化（刷新 / 重启 / 换端口都还在）=====
// 存 dbmind.db 的 app_settings（见 panelPrefs.js）：localStorage 按 origin 隔离，
// 桌面壳换端口就丢，所以不放在那儿。
let prefsHydrated = false

/** 把当前底栏选择写回库（缓存同步生效，落库合并成一次 PUT） */
const persistComposer = () => {
  if (!prefsHydrated) return
  saveAiComposer(_mode, {
    connId: ctxConnId.value,
    database: ctxDatabase.value,
    schema: ctxSchema.value,
    tables: ctxTables.value.slice(),
    modelId: selectedModelId.value,
    agentMode: agentMode.value,
    kbScope: kbScope.value,
    kbPicked: kbPicked.value.slice()
  })
}
// 数组用 join 做浅比较：push/splice 改动也能触发（整数组替换当然也触发）
watch([
  ctxConnId, ctxDatabase, ctxSchema,
  () => ctxTables.value.join('\u0001'),
  selectedModelId, agentMode, kbScope,
  () => kbPicked.value.join('\u0001')
], persistComposer)

/**
 * 刷新/重启后把底栏已选恢复回来。
 *
 * <p>刷新时父组件的 conn/database 还是空的（当前连接还没打开），而上面那个
 * `immediate` 的 watch 会把 ctx 覆盖成空 —— 底栏就成了「什么都没选」。
 * 所以这里在连接清单拉回来之后，用存下来的值再盖回去：
 * 连接还在才还原（删掉的连接不认）；当前已经有连接（用户在左侧树点过）时以当前为准。
 */
const hydrateComposer = () => {
  const saved = readAiComposer(_mode)
  const pid = props.conn?.id ? String(props.conn.id) : ''
  const alive = saved.connId
    ? (connList.value || []).some((c) => String(c.id) === String(saved.connId))
    : false
  // 存的连接已经不在了：整套回落（库和表都挂在它上面，留着只会让提问打到不存在的连接）
  if (saved.connId && !alive) {
    prefsHydrated = true
    persistComposer()
    return
  }
  if (saved.connId && (!pid || pid === String(saved.connId))) {
    ctxConnId.value = saved.connId
    ctxDatabase.value = saved.database
    ctxSchema.value = saved.schema
    // 「发送到 AI」带进来的聚焦表优先：那是用户刚点的，不能被上次的勾选顶掉
    if (!handledPendingSeq) ctxTables.value = saved.tables.slice()
  }
  if (saved.modelId) selectedModelId.value = saved.modelId
  agentMode.value = !!saved.agentMode
  kbScope.value = saved.kbScope
  kbPicked.value = saved.kbPicked.slice()
  prefsHydrated = true
  // 父组件给了别的连接时，把「以当前为准」的结果落回去，避免下次刷新还原到旧连接
  if (pid && saved.connId && pid !== String(saved.connId)) persistComposer()
}

/** 记住「现在打开的是哪一段对话」：刷新后按这个指针接回整段，而不是只挑最近一条 */
watch([tab, chatThreadId, askThreadId], () => {
  if (!prefsHydrated) return
  saveAiSession(_mode, {
    tab: tab.value,
    chatThreadId: chatThreadId.value,
    askThreadId: askThreadId.value
  })
})

/** 去掉层级前缀（conn: / db: / sc:） */
const ctxUnwrap = (v) => String(v || '').replace(/^(conn|db|sc):/, '')

/** 构造上下文下拉项对象 */
const makeCtxItem = (value, label, level, expandable, parentValue) => ({
  value, label, level, expandable, expanded: false, loading: false, parentValue
})

/** 分组的静态子节点（grp:<path> → 子分组/连接项）：分组结构不查后端，展开时直接取 */
const ctxGrpChildren = new Map()

/** 加载顶层连接列表
 *  与左侧树同一套分组口径（见 MainView.buildTree / utils/envs.js）：
 *  - connection.group 存的是**路径**（`公司资源/子组`，`/` 分隔），按段拆开嵌套；
 *  - 预置环境（开发/测试/生产）各算一层目录，未分组（''）垫底；
 *  - 只展示「真有连接」的分组 —— 纯目录（空分组）在数据源选择器里只是噪音。 */
const loadCtxConnections = async () => {
  if (!connList.value.length) await loadConnList()
  ctxGrpChildren.clear()
  const groups = new Map() // 路径 -> 连接[]（保持 connList 原顺序）
  for (const c of connList.value || []) {
    const p = String(c.group || '').trim()
    if (!groups.has(p)) groups.set(p, [])
    groups.get(p).push(c)
  }
  const items = []
  const folderIndex = new Map() // 路径 -> 分组项
  const connItemOf = (c, level, parentValue) => makeCtxItem('conn:' + c.id, c.name || c.id, level, true, parentValue)
  const ensureFolder = (path) => {
    if (folderIndex.has(path)) return folderIndex.get(path)
    const segs = path.split('/')
    const name = segs[segs.length - 1]
    const parentPath = segs.length > 1 ? segs.slice(0, -1).join('/') : null
    const parent = parentPath ? ensureFolder(parentPath) : null
    // 预置环境显示成开发/测试/生产（与树的角标文案一致），其余按尾段名
    const label = PREDEF_ENVS.includes(path) ? envLabel(path) : name
    const item = makeCtxItem('grp:' + path, label, parent ? parent.level + 1 : 0, true, parent ? parent.value : null)
    folderIndex.set(path, item)
    if (parent) ctxGrpChildren.get(parent.value).push(item)
    else items.push(item)
    ctxGrpChildren.set(item.value, [])
    // 直属该层的连接挂在「深层路径段先建好父目录」之后
    for (const c of groups.get(path) || []) ctxGrpChildren.get(item.value).push(connItemOf(c, item.level + 1, item.value))
    return item
  }
  // 顺序：预置环境 → 自定义目录（首次出现顺序）→ 未分组垫底（左侧树同款）
  const custom = [...groups.keys()].filter(p => p && !PREDEF_ENVS.includes(p))
  for (const p of [...PREDEF_ENVS.filter(x => groups.has(x)), ...custom]) ensureFolder(p)
  if (groups.has('')) {
    const item = makeCtxItem('grp:', t('aictx.ungrouped'), 0, true, null)
    items.push(item)
    ctxGrpChildren.set(item.value, (groups.get('') || []).map(c => connItemOf(c, 1, item.value)))
  }
  ctxFlatItems.value = items
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

/** 异步加载某一项的子节点：分组 → 子分组/连接（静态）；连接 → 数据库；数据库 → 模式（仅模式层级类型）。
 *  按 value 前缀分派而不是按层级 —— 连接挂在分组下之后不再固定在第 0 层。 */
const loadCtxChildren = async (item) => {
  const cached = ctxCache.value.get(item.value)
  if (cached) return cached
  let children = []
  try {
    if (item.value.startsWith('grp:')) {
      // 分组结构在 loadCtxConnections 里已建好（不查后端），直接取
      children = ctxGrpChildren.get(item.value) || []
      ctxCache.value.set(item.value, children)
      return children
    }
    if (item.value.startsWith('conn:')) {
      const connId = ctxUnwrap(item.value)
      const conn = (connList.value || []).find(c => String(c.id) === String(connId))
      if (hideDbLevelOf(conn?.type)) {
        // 该类型的「库」就是连接里填的服务名 / PDB（换 PDB 得换连接），永远只有一条 ——
        // 与左侧树、SQL 编辑器同一口径：连接下面**直接列模式**，不画库这一层。
        const raw = await listSchemas(connId, conn?.database || '')
        children = (raw || [])
          .map(s => (typeof s === 'string' ? s : (s.name || s.schema || '')))
          .filter(Boolean)
          .map(s => makeCtxItem('sc:' + s, String(s), item.level + 1, false, item.value))
        ctxCache.value.set(item.value, children)
        return children
      }
      const dbs = await listDatabases(connId)
      const needSchema = connNeedsSchema(connId)
      children = (dbs || []).map(d => makeCtxItem('db:' + d, String(d), item.level + 1, needSchema, item.value))
    } else if (item.value.startsWith('db:')) {
      const connId = ctxUnwrap(item.parentValue)
      const db = ctxUnwrap(item.value)
      const raw = await listSchemas(connId, db)
      children = (raw || [])
        .map(s => (typeof s === 'string' ? s : (s.name || s.schema || '')))
        .filter(Boolean)
        .map(s => makeCtxItem('sc:' + s, String(s), item.level + 1, false, item.value))
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
  // 按**项的类型**（value 前缀）回填，而不是按链上下标 —— Oracle 不画库层，
  // 链是「连接 → 模式」，按下标会把模式名塞进「库」里、模式反而空着。
  let conn = '', db = '', sc = ''
  for (const node of chain) {
    const raw = ctxUnwrap(node.value)
    if (node.value.startsWith('conn:')) conn = raw
    else if (node.value.startsWith('db:')) db = raw
    else if (node.value.startsWith('sc:')) sc = raw
  }
  const changed = conn !== ctxConnId.value || db !== ctxDatabase.value || sc !== ctxSchema.value
  ctxDatabase.value = db
  ctxConnId.value = conn
  ctxSchema.value = sc
  // 用户**主动**换上下文（连接/库/模式真的变了）：聚焦表不再适用，清掉。
  // 上下文没变就不清 —— 重选一下同一个库就把聚焦表抹掉，问数就悄悄退回
  // 「12 张兜底」，用户看到的是「我明明选了表，怎么还去找 12 张」（真机踩过）。
  // （右键「发送到 AI」那条路不走这里 —— 它直接设 ctxTables，见 applyPending）
  if (changed) ctxTables.value = []
}

/** 处理上下文下拉命令：
 * 可展开项既设置当前层级上下文，又展开/折叠下级；
 * 叶子项设置上下文并关闭下拉。 */
const onCtxCommand = async (value) => {
  const idx = findCtxIndex(value)
  if (idx < 0) return
  const item = ctxFlatItems.value[idx]
  // 分组只是目录：点了不该动上下文，更不该清掉「聚焦表」（只有连接/库/模式才算换上下文）
  if (!item.value.startsWith('grp:')) selectCtxByItem(item)
  if (item.expandable) {
    if (item.expanded) { collapseCtxItem(idx); return }
    const hasChildren = await expandCtxItem(idx)
    // 展开类点击**不关下拉**：空库（没有表/模式）就让它空着，用户好接着点别的库 ——
    // 原来这里兜底 closeCtxDrop()，点一个空库整个下拉突然收起，很莫名
    return
  }
  closeCtxDrop()
}

/** AI 服务是否已配置（loadAiModels 从 /api/ai/config 拿 enabled）：
 *  未配置时不发任何自动请求（预热等）—— 400 的响应无论怎么降噪，
 *  浏览器 Network 面板与控制台总有痕迹；不发才是干净的。 */
const aiReady = ref(false)

const loadAiModels = async () => {
  try {
    const cfg = await getAiConfig()
  aiReady.value = cfg.enabled === true
    if (cfg && Array.isArray(cfg.models)) {
      aiModels.value = cfg.models
      // 默认使用 Auto 自动选择模式（后端自动挑选最优可用模型，失败自动切换）
      if (!selectedModelId.value) selectedModelId.value = 'auto'
    }
  } catch (e) { /* ignore */ }
}

// 设置页保存了 AI 配置（模型/Key/启用开关）会广播 dbmind-ai-changed：
// 面板是常驻组件，只在挂载时读过一次配置 —— 不重读的话「刚配好却仍提示未配置、
// 新建的模型也不在下拉里」，用户得关掉面板再打开才行。
const onAiChanged = () => { loadAiModels() }
// 插件中心装/卸/停用后广播：面板里的技能卡片与下拉要立刻跟着变，不用等下次开下拉
const onPluginsChanged = () => loadPluginSkills(true)
onMounted(() => {
  loadAiModels()
  loadPluginSkills(true)
  loadSendPreviewSetting()
  window.addEventListener('dbmind-ai-changed', onAiChanged)
  window.addEventListener('dbmind-plugins-changed', onPluginsChanged)
})

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
  window.removeEventListener('dbmind-ai-changed', onAiChanged)
  window.removeEventListener('dbmind-plugins-changed', onPluginsChanged)
  // 统一走停止入口：除普通 chat 流外，问数/字典/智能体走的是 stopCtrl（axios），
  // 只停 streamCtrl 会在切走 studio 页签后继续烧 token、回调写已卸载组件。
  stopGenerating()
})

/** 高级技能统一执行：表数据洞察（结果都是 Markdown，可直接导出文件）。
 *  健康巡检已挪到「数据治理」——它是纯规则扫描，不依赖 AI 模型。 */
/** 数据洞察：结果直接进聊天流，消息上保留导出入口（表名必填） */
const runInsightInChat = async (table) => {
  if (!table) { ElMessage.warning(t('ai.needTableName')); return }
  if (!ctxConnId.value) { ElMessage.warning(t('ai.needConn')); return }
  if (!ctxEffectiveDatabase.value) { ElMessage.warning(t('ai.needDatabase')); return }
  messages.value.push({ role: 'user', content: table, skills: currentSkills() })
  aiInput.value = ''
  chatLoading.value = true; startWait(); scrollBottom()
  stopCtrl = new AbortController()
  const ctrl = stopCtrl
  try {
    const res = await aiInsight({
      connectionId: ctxConnId.value, database: ctxEffectiveDatabase.value,
      table, question: '', modelId: selectedModelId.value
    }, { signal: ctrl.signal })
    if (res && res.success) {
      const md = (res.content || '') + (res.stats ? t('ai.statsSection', { stats: res.stats }) : '')
      messages.value.push({ role: 'assistant', content: md, exportMd: md, exportTitle: t('ai.tabInsight'), usage: res.usage || null })
    } else {
      messages.value.push({ role: 'assistant', content: '❌ ' + (res?.message || t('ai.analyzeFailed')) })
    }
  } catch (e) {
    if (isCanceled(e)) {
      messages.value.push({ role: 'assistant', content: '> ⏱ ' + t('ai.interrupted', { detail: t('api.generationStopped') }) })
    } else {
      messages.value.push({ role: 'assistant', content: '❌ ' + (e?.message || e?.toString?.() || t('ai.analyzeFailed')) })
    }
  }
  if (stopCtrl === ctrl) stopCtrl = null
  chatLoading.value = false; stopWait(); scrollBottom()
}

/**
 * 智能问数：一句话 → 只读 SQL → 图表 + 带数字的结论。
 *
 * <p>形态与「数据字典 / 数据洞察」完全一致：问题进对话流，回答是同一气泡里的一张结果卡片
 * （见模板里的 `m.askAnswer` 分支）。**不新开页签、不另起工作区** —— 入口就是技能下拉。
 *
 * <p>追问带最近几轮问数（问题 + 结论 + SQL）当上下文，否则模型不知道「它」指的是哪张表；
 * 不带整段对话历史，那是自由对话的东西，塞进问数只会让选表跑偏。
 */
const runAskInChat = async (text) => {
  const q = String(text ?? aiInput.value ?? '').trim()
  if (!q || chatLoading.value) return
  if (!ctxConnId.value) { ElMessage.warning(t('ai.needConn')); return }
  if (!ctxEffectiveDatabase.value) { ElMessage.warning(t('ai.needDatabase')); return }
  const history = []
  for (const m of messages.value.slice(-6)) {
    if (m.role === 'user' && m.content) history.push({ role: 'user', content: m.content })
    else if (m.askAnswer?.sql) history.push({ role: 'assistant', content: `${m.askAnswer.summary || ''}\n${m.askAnswer.sql}` })
  }
  messages.value.push({ role: 'user', content: q, skills: currentSkills() })
  aiInput.value = ''
  chatLoading.value = true; startWait(); scrollBottom()
  // 流式占位：立刻进对话流，让「读表结构 → 写方案 → 执行 → 写结论」一步步点亮；
  // 收到完整回答后整体替换（streaming 标记消失 → 执行过程自动合并成一行）
  const msg = reactive({
    role: 'assistant',
    content: '',
    askAnswer: {
      streaming: true, intent: 'query', question: q,
      summary: '', sql: '', columns: [], columnTypes: [], rows: [], rowCount: 0,
      charts: [], insights: [], followups: [], steps: [],
    },
  })
  messages.value.push(msg)
  // 停止按钮：问数流也挂在统一控制器上，点停止即掐断
  stopCtrl = new AbortController()
  const ctrl = stopCtrl
  try {
    const res = await aiAskStream(
      {
        connectionId: ctxConnId.value,
        database: ctxEffectiveDatabase.value,
        question: q,
        history: history.slice(-4),
        modelId: selectedModelId.value,
        lang: String(locale.value || '').startsWith('en') ? 'en' : 'zh',
        // 选中的技能提示词同样作用于问数（后端 ask.rs 走 augment_system）
        pluginSkills: pickedSkills.value.map(s => s.id),
      },
      (step) => {
        mergeStep(msg.askAnswer.steps, step)
        scrollBottom()
      },
      { signal: ctrl.signal },
    )
    msg.askAnswer = res
    // 落本地历史：问数不进服务端会话，左栏「记录」靠它才能回看。
    // 以对话为单位 —— 同一段对话里的追问并入同一条记录，只有「新对话」才另起一条
    const rec = saveAskHistory(q, res, askThreadId.value)
    if (rec?.id) askThreadId.value = rec.id
    askHist.value = listAskHistory()
  } catch (e) {
    // 卡在流里的半截过程不留在界面上：换成一条明确的错误消息
    msg.askAnswer = null
    msg.content = isCanceled(e)
      ? '> ⏱ ' + t('ai.interrupted', { detail: t('api.generationStopped') })
      : '❌ ' + (e?.message || t('ai.runFailed'))
  } finally {
    if (stopCtrl === ctrl) stopCtrl = null
    chatLoading.value = false; stopWait(); scrollBottom()
  }
}

/** 数据字典：同上，进聊天流（表名可留空 = 整库） */
/** 数据字典的聊天流预览：只展示前 5 张表的小节，其余以一句提示带过（全量在导出文件里） */
const trimDictPreview = (md, maxTables = 5) => {
  const src = md == null ? '' : String(md)
  const parts = src.split(/\n(?=#{2,3}\s*表\s)/)
  const sections = parts.slice(1)
  if (sections.length <= maxTables) return src
  const rest = sections.length - maxTables
  const kept = parts[0] + '\n' + sections.slice(0, maxTables).join('\n')
  return kept + '\n\n> 📄 其余 ' + rest + ' 张表未在此展示 —— 点上方「导出」可下载完整数据字典。'
}
const runDictInChat = async (text) => {
  if (!ctxConnId.value) { ElMessage.warning(t('ai.needConn')); return }
  // 与巡检/洞察同一条规则：没选库会回落到连接默认库（SQL Server 是 master），
  // 生成的字典会全是系统表 —— 明确让用户先选库
  if (!ctxEffectiveDatabase.value) { ElMessage.warning(t('ai.needDatabase')); return }
  messages.value.push({ role: 'user', content: text || t('ai.tabDictionary'), skills: currentSkills() })
  aiInput.value = ''
  chatLoading.value = true; startWait(); scrollBottom()
  stopCtrl = new AbortController()
  const ctrl = stopCtrl
  try {
    const tables = text.split(/[,，\s]+/).map(s => s.trim()).filter(Boolean)
    const res = await aiDataDict({
      connectionId: ctxConnId.value,
      database: ctxEffectiveDatabase.value,
      tables,
      modelId: selectedModelId.value
    }, { signal: ctrl.signal })
    if (res && res.success) {
      const md = res.content || ''
      messages.value.push({ role: 'assistant', content: trimDictPreview(md), exportMd: md, exportTitle: t('ai.tabDictionary'), usage: res.usage || null })
    } else {
      messages.value.push({ role: 'assistant', content: '❌ ' + (res?.message || t('ai.genFailed')) })
    }
  } catch (e) {
    if (isCanceled(e)) {
      messages.value.push({ role: 'assistant', content: '> ⏱ ' + t('ai.interrupted', { detail: t('api.generationStopped') }) })
    } else {
      messages.value.push({ role: 'assistant', content: '❌ ' + (e?.message || e?.toString?.() || t('ai.genFailed')) })
    }
  }
  if (stopCtrl === ctrl) stopCtrl = null
  chatLoading.value = false; stopWait(); scrollBottom()
}

/**
 * 范围约束**不再拼进提问**，改走结构化字段 `table`。
 *
 * <p>以前是把「（只基于表 x 回答，不要扩散到整库或其它表…）」这段直接拼在 prompt 后面，
 * 造成两个问题：
 * <ul>
 *   <li>它会随消息一起落库，重开这条记录就在对话里明晃晃打出来一行用户没敲过的字；</li>
 *   <li>它只带**一张**表（右键聚焦的那张），用户一次问两张表时等于被它限制住 ——
 *       模型被明令「不要扩散」，于是只回答一张。</li>
 * </ul>
 * 现在聚焦表只作为 `table` 字段传给后端，由后端把它和「提问里点到的表」求并集去展开；
 * 模型看到的上下文里有什么表就答什么表，不需要一句元指令来限制。
 */
/* token 数的格式化统一在 common/AiUsageNote.vue 里（这里不再各写一份） */

/** 老版本拼进提问的范围约束：打开历史会话时把这段剥掉，别让用户看到自己没写过的话 */
const SCOPE_HINT_RE = /\s*[（(][^（()）]*?(?:基于表|based only on the table)[^（()）]*?[)）]\s*$/i

const stripScopeHint = (text) => {
  let out = String(text || '')
  // 一条消息尾部最多拼过一次；循环几次纯粹是兜底（万一历史里叠了两层）
  for (let i = 0; i < 3; i += 1) {
    const next = out.replace(SCOPE_HINT_RE, '')
    if (next === out) break
    out = next
  }
  return out
}

const sendChat = async () => {
  const text = aiInput.value.trim()
  if (chatLoading.value) return
  // 数据字典 / 数据洞察：结果直接进聊天流（与对话同一形态），消息上保留导出入口。
  // 数据字典允许空输入（= 整库），所以分派必须放在空文本检查**之前**；
  // 数据洞察的表名必填校验在 runInsightInChat 内部（为空时提示，不发请求）。
  if (tab.value === 'dictionary') return runDictInChat(text)
  if (tab.value === 'insight') {
    if (!text) { ElMessage.warning(t('ai.needTableName')); return }
    return runInsightInChat(text)
  }
  if (!text) return
  // 智能问数：同一套对话流里出「结论 + 图表 + 明细」，只是后端接口不同
  if (tab.value === 'ask') return runAskInChat(text)
  // 深挖模式：交给智能体（会自己查库、出变更单），与普通对话是两条实现
  if (agentMode.value) return runAgentChat(text)
  // 多轮上下文：本次提问之前的对话轮次。
  // 只带最近 8 条（约 4 轮）——历史越长，请求体越大、模型首字越慢，收益却很低
  const history = messages.value
    .filter(m => (m.role === 'user' || m.role === 'assistant') && m.content)
    .slice(-8)
    .map(m => ({ role: m.role, content: m.content }))
  messages.value.push({ role: 'user', content: text, skills: currentSkills() })
  aiInput.value = ''
  chatLoading.value = true
  startWait()
  scrollBottom()
  // 与回答并行跑一次检索拿来源：不阻塞、失败也不影响对话，只是把「用了什么资料」摆出来
  const srcPromise = recallSources(text)
  const payload = {
    prompt: text,
    // 聚焦表走结构化字段（不再拼成 prompt 尾巴）：后端会把它和「提问里点到的表」求并集，
    // 用户一次问两张表时两张都会被展开，而不是被一句「不要扩散」按回一张。
    // 字段名必须是 tables（数组）—— 之前发的 table 单数字段后端根本没有，一直被静默忽略
    tables: ctxTables.value.slice(),
    connectionId: ctxConnId.value,
    database: ctxEffectiveDatabase.value,
    modelId: selectedModelId.value,
    history,
    // 知识库范围：null=全部库，[]=本次不注入，[...]=只在这些库里找。
    // 必须随请求发给后端，只在界面上控制等于没控制。
    kbIds: kbIdsForQuery(),
    // 选中的插件技能：后端把它们的提示词拼进 system（这段对话持续生效，不是一次任务）
    pluginSkills: pickedSkills.value.map(s => s.id),
    // 这一问生效的技能名（含内置技能）：后端只写进消息 meta，供气泡回显与历史恢复
    skillLabels: currentSkills().map(s => s.name),
    // 落库与续聊：只有本面板带这两个字段（一次性接口不带，否则每点一次按钮
    // 就多一条空会话）。已有会话就续写，没有就让后端建一个并回传 id。
    persist: !chatThreadId.value,
    threadId: chatThreadId.value || undefined
  }
  try {
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
      streaming.value = true
        await aiChatStream(payload, (d) => {
          acc += d
          if (flushTimer !== null) return
          flushTimer = setTimeout(flush, 60)
        }, {
          signal: streamCtrl.signal,
          onUsage: (u) => { msg.usage = u },
          // 后端在建好会话后回传 id：顺带刷一次左栏，新的这段立刻出现在「记录」里
          onThread: (id) => { chatThreadId.value = id; if (isStudio.value) loadThreads() },
          onNotice: (text) => { msg.notice = text }
        })
        finalFlush()
        if (!acc) throw new Error('empty-stream')
        msg.streaming = false
        streamCtrl = null
        streaming.value = false
        scrollBottom()
      } catch (e) {
        streamCtrl = null
        streaming.value = false
        finalFlush()
        if (acc) {
          // 已收到部分内容：保留并标注中断，避免界面一直停在「生成中」
          msg.streaming = false
          msg.content = acc + '\n\n> ⏱ ' + t('ai.interrupted', { detail: (e?.message || t('ai.responseInterrupted')) })
          scrollBottom()
        } else if (String(e?.message || '').includes(t('api.generationStopped'))) {
          // 用户主动点停止且还没收到内容：就地标注，**不要**退回非流式重试
          // （那等于「停了又发一次」，白花一次调用）
          messages.value.pop()
          messages.value.push({ role: 'assistant', content: '> ⏱ ' + t('ai.interrupted', { detail: t('api.generationStopped') }) })
        } else {
          messages.value.pop()
          chatLoading.value = true
          // 回退请求也要兜住：这里在 catch 块内，再抛出去就是 uncaught（控制台刷屏的真凶之一）
          try {
            const res = await aiChat(payload)
            if (res && res.success) {
              if (res.threadId) { chatThreadId.value = res.threadId; if (isStudio.value) loadThreads() }
              const m = reactive({ role: 'assistant', content: res.content, sources: [], usage: res.usage || null })
              messages.value.push(m)
              srcPromise.then(s => { m.sources = s })
            } else {
              messages.value.push({ role: 'assistant', content: '❌ ' + (res?.message || t('ai.requestFailed')) })
            }
          } catch (e2) {
            messages.value.push({ role: 'assistant', content: '❌ ' + (e2?.message || t('ai.requestFailed')) })
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

// ---------------------------------------------------------------- AI 同事
// 会话 / 待办 / 记忆 / 变更单。四块共享一个前提：**执行权在人手里** ——
// 后台只出单，界面上每一次「执行」都是一次明确点击（并记进审批流水）。

const safeParse = (text) => { try { return JSON.parse(text) } catch { return null } }

/**
 * 记忆挂在哪条连接上 —— 只回名字。
 *
 * 库里记的是 `conn_18e0261b38fae5c0008d` 这种内部 id，直接摆给用户没有意义
 * （既认不出是哪条库，也点不动）；连接已被删掉时返回空串，这一格就不渲染。
 */
const connNameOf = (row) => {
  const stored = String(row?.connectionName || '')
  const id = String(row?.connectionId || row?.connection_id || '')
  if (!id) return stored
  const hit = (connList.value || []).find(c => String(c.id) === id)
  // 三层：现存连接的名字 → 记录里存的快照（连接已删也认得出）→ 本地见过的名字（id 换过）
  return hit?.name || hit?.type || stored || connNameById(id)
}

/** 记事抽屉：记录（变更历史）+ 记忆（长期事实），后台任务只用来提示「有没跑完的排查」 */
const refreshMate = async () => {
  mateLoading.value = true
  mateError.value = ''
  mateFailed.value = []
  try {
    // allSettled：三边互不拖累，但要能分辨「真的空」和「没拉下来」
    const [jobsRes, memRes, chgRes] = await Promise.allSettled([
      aiJobList(),
      aiMemoryList({ limit: 200 }),
      aiChangeList({})
    ])
    if (jobsRes.status === 'fulfilled') {
      todos.value = jobsRes.value?.jobs || []
    } else {
      mateFailed.value.push('jobs')
      console.warn('[ai] 后台任务加载失败：', jobsRes.reason?.message || jobsRes.reason)
    }
    if (memRes.status === 'fulfilled') {
      memories.value = memRes.value?.memories || []
    } else {
      mateFailed.value.push('memory')
      console.warn('[ai] 记忆加载失败：', memRes.reason?.message || memRes.reason)
    }
    if (chgRes.status === 'fulfilled') {
      changes.value = chgRes.value?.changes || []
    } else {
      mateFailed.value.push('changes')
      console.warn('[ai] 变更记录加载失败：', chgRes.reason?.message || chgRes.reason)
    }
    if (mateFailed.value.length) {
      const labelOf = (k) => t(k === 'changes' ? 'ai.records' : k === 'jobs' ? 'ai.recordJobs' : 'ai.memory')
      const parts = mateFailed.value.map(labelOf)
      // 顿号只适合中文；英文用逗号分隔，别混进中文标点
      const sep = String(locale.value || '').startsWith('en') ? ', ' : '、'
      mateError.value = t('ai.mateLoadFailed', { parts: parts.join(sep) })
    }
  } finally {
    mateLoading.value = false
  }
}

const openMate = async (tabKey) => {
  mateTab.value = tabKey || 'records'
  mateOpen.value = true
  // 「记忆 → 作用范围」下拉要列出**全部连接**：连接清单没拿到就补一次（面板常驻时也要保证）
  if (!connList.value.length) await loadConnList()
  await refreshMate()
}

/** alert = 值班发现问题；waiting = 有变更单等人确认；done = 没事（不打扰） */
const todoStatusLabel = (status) => (te('ai.jobStatus.' + status) ? t('ai.jobStatus.' + status) : status)
const riskLabel = (risk) => (te('ai.risk.' + risk) ? t('ai.risk.' + risk) : risk)
/** 变更单状态（pending / applied / undone / rejected / failed）—— 记录页里要中文化 */
const chgStatusLabel = (status) => (te('ai.chgStatus.' + status) ? t('ai.chgStatus.' + status) : status)
const alertCount = computed(() => todos.value.filter(job => job.status === 'alert').length)
/** 记录页签底部只列「还没结束」的后台任务：跑完的没有动作可做，不占地方 */
const unfinishedJobs = computed(() => todos.value.filter(job => ['alert', 'failed', 'running'].includes(job.status)))
/** 记录里的时间：只到分钟（`2026-11-19T09:12:33Z` → `2026-11-19 09:12`） */
const fmtWhen = (value) => {
  if (!value) return ''
  const text = String(value).replace('T', ' ')
  return text.length >= 16 ? text.slice(0, 16) : text
}

/**
 * 继续跑一张中断的待办。
 *
 * 只对智能体任务有效：普通问答没有「步骤」，续跑没有意义。
 * 跑完就地刷新这张卡片（结论 + 步骤数），并把新产出的变更单提出来给人点。
 */
const resumeJob = async (job) => {
  if (jobBusy.value) return
  jobBusy.value = job.id
  try {
    const res = await aiJobResume(job.id)
    if (!res || !res.success) throw new Error(res?.message || t('ai.requestFailed'))
    job.status = 'done'
    job.result = res.content || job.result
    job.steps = res.steps || job.steps
    ElMessage.success(t('ai.jobResumed', { n: res.rounds || 0 }))
    await refreshMate()
    if ((res.changes || []).length) {
      messages.value.push(reactive({
        role: 'assistant',
        content: res.content || '',
        steps: res.steps || [],
        // 与首跑一致：路径要用这张待办自己的连接与库（续跑不挂在当前上下文上）
        changes: (res.changes || []).map((c) => ({
          ...c,
          connectionId: job.connectionId,
          database: job.database
        })),
        usage: null
      }))
      scrollBottom()
    }
  } catch (e) {
    ElMessage.error(e?.message || t('ai.requestFailed'))
  } finally {
    jobBusy.value = ''
  }
}

const dismissJob = async (job) => {
  try {
    const res = await aiJobDismiss(job.id)
    if (!res?.success) throw new Error(res?.message || t('ai.requestFailed'))
    job.status = 'dismissed'
    ElMessage.success(t('ai.chgDone'))
  } catch (e) {
    ElMessage.error(e?.message || t('ai.requestFailed'))
  }
}

const memDraft = ref('')
/**
 * 记忆的「作用范围」：全部连接 / 任意一条连接（列表里直接选，不要求先在工具条选上下文 ——
 * 之前只列「当前连接」，没选上下文时下拉里就只剩一项，等于选不到连接）。
 *
 * 表级刻意不做：每轮对话都已经把表结构（列/类型/注释）注入上下文，记忆再挂表只会在
 * 表改名/删除之后变成一条错误的口径，而且没有任何机制能让它自动失效。
 */
const memScope = ref('global')
/** 用户手动选过就不再跟着上下文跑（否则每次切连接都把人家选的范围改掉） */
const memScopeTouched = ref(false)
/**
 * 下拉里的连接按**分组**摆（与左侧树同一套口径：预置环境 → 自定义目录 → 未分组，
 * 组名后面带条数）。一条平铺的长列表找起来太费劲 —— 54 条连接时纯靠滚动。
 */
const memScopeGroups = computed(() => {
  const groups = new Map() // 路径 -> 连接[]
  for (const c of connList.value || []) {
    const p = String(c.group || '').trim()
    if (!groups.has(p)) groups.set(p, [])
    groups.get(p).push(c)
  }
  const custom = [...groups.keys()].filter(p => p && !PREDEF_ENVS.includes(p))
  const order = [...PREDEF_ENVS.filter(p => groups.has(p)), ...custom]
  if (groups.has('')) order.push('')
  return order.map(p => ({
    key: p || '__none',
    // 预置环境显示成开发/测试/生产（与树、上下文下拉一致），其余按分组路径原名
    label: p === '' ? t('aictx.ungrouped') : (PREDEF_ENVS.includes(p) ? envLabel(p) : p),
    conns: (groups.get(p) || []).map(c => ({
      value: 'conn:' + c.id,
      label: c.name || c.type || String(c.id),
      type: c.type || ''
    }))
  }))
})
/** 扁平值清单：只用来判断「当前选中项还在不在选项里」 */
const memScopeOptions = computed(() => [
  { value: 'global', label: t('ai.memScopeAll') },
  ...memScopeGroups.value.flatMap(g => g.conns)
])
/**
 * 分组折叠：点组名展开/合并（默认全部展开）。只记在本次面板生命周期里 ——
 * 这是「找起来方便」的操作，不值得写进 localStorage 变成又一个要解释的偏好。
 */
const memScopeCollapsed = reactive(new Set())
const toggleMemScopeGroup = (key) => {
  if (memScopeCollapsed.has(key)) memScopeCollapsed.delete(key)
  else memScopeCollapsed.add(key)
}
/**
 * 下拉里的搜索词（`filter-method` 自己接管过滤）：
 * 这样「搜索」与「折叠」不打架 —— 有关键字时一律按名字过滤、忽略折叠状态，
 * 否则用户搜一条被折叠起来的连接会得到「没有数据」。
 */
const memScopeQuery = ref('')
const onMemScopeFilter = (q) => {
  memScopeQuery.value = String(q || '').trim().toLowerCase()
}
/** 实际渲染用的分组（含折叠态与搜索结果） */
const memScopeRender = computed(() => {
  const q = memScopeQuery.value
  return memScopeGroups.value
    .map(g => {
      const conns = q ? g.conns.filter(c => String(c.label).toLowerCase().includes(q)) : g.conns
      return { ...g, conns, collapsed: !q && memScopeCollapsed.has(g.key) }
    })
    .filter(g => g.conns.length > 0 || !q)
})
/**
 * 组名是「禁用的 el-option」：Element 自己的点击处理会把禁用项拦掉，
 * 所以在 document 捕获阶段先截住，只做折叠切换，绝不改动选中值。
 */
const onMemScopeDocClick = (ev) => {
  const el = ev.target?.closest?.('.mem-grp')
  if (!el) return
  ev.preventDefault()
  ev.stopPropagation()
  toggleMemScopeGroup(String(el.dataset.grp || ''))
}
onMounted(() => document.addEventListener('click', onMemScopeDocClick, true))
onBeforeUnmount(() => document.removeEventListener('click', onMemScopeDocClick, true))
/** 选中项的数据库类型（下拉触发器上的 logo）；「全部连接」时为空 */
const memScopeType = computed(() => {
  const hit = memScopeOptions.value.find(o => o.value === memScope.value)
  return hit?.type || ''
})
// 没手选过就默认跟着当前上下文连接；手选过则只在选项失效时回落到「全部连接」
watch([ctxConnId, () => memScopeOptions.value.map(o => o.value).join(',')], () => {
  if (memScopeTouched.value) {
    if (!memScopeOptions.value.some(o => o.value === memScope.value)) memScope.value = 'global'
    return
  }
  memScope.value = ctxConnId.value ? 'conn:' + ctxConnId.value : 'global'
}, { immediate: true })

/** 卡片上的范围文字：全部连接 / 连接名 · 库 / 连接名（已删除） */
const memScopeText = (row) => {
  if (row.scope === 'global' || !row.connectionId) return t('ai.memScopeAll')
  const name = String(row.connectionName || '') || connNameById(row.connectionId)
  const db = String(row.database || '')
  const label = name ? (db ? `${name} · ${db}` : name) : (db || String(row.connectionId))
  return row.connMissing ? t('ai.memGoneName', { name: label }) : label
}

const addMemory = async () => {
  const fact = String(memDraft.value || '').trim()
  if (!fact) return
  const scope = memScope.value === 'global' ? 'global' : 'connection'
  const payload = { fact, scope, source: 'user' }
  if (scope === 'connection') {
    const connId = String(memScope.value).replace(/^conn:/, '')
    payload.connectionId = connId
    // 选中的正是当前上下文的那条连接时，顺手带上当前库（「本库金额单位是分」这类口径）
    if (ctxConnId.value && connId === String(ctxConnId.value) && ctxEffectiveDatabase.value) {
      payload.database = ctxEffectiveDatabase.value
    }
  }
  try {
    const res = await aiMemoryAdd(payload)
    if (!res?.success) throw new Error(res?.message || t('ai.requestFailed'))
    memDraft.value = ''
    await refreshMate()
  } catch (e) {
    ElMessage.error(e?.message || t('ai.requestFailed'))
  }
}

/** 连接已删的老记忆：改成「全部连接」（否则它永远不会被注入） */
const detachMemory = async (row) => {
  try {
    const res = await aiMemoryDetach(row.id)
    if (!res?.success) throw new Error(res?.message || t('ai.requestFailed'))
    ElMessage.success(t('ai.memDetached'))
    await refreshMate()
  } catch (e) {
    ElMessage.error(e?.message || t('ai.requestFailed'))
  }
}

const removeMemory = async (row) => {
  try {
    await aiMemoryDelete(row.id)
    memories.value = memories.value.filter(item => item.id !== row.id)
  } catch (e) {
    ElMessage.error(e?.message || t('ai.requestFailed'))
  }
}

/** 这张变更单挂在哪个气泡上（后续续跑要接着写进同一个气泡） */
const msgOfChange = (change) => messages.value.find((m) => (m.changes || []).some((c) => c.id === change.id))

/**
 * 变更单动作。执行完就地改卡片状态（不重开面板），并把操作人记进流水 ——
 * 「谁批的」比「有人批了」有用得多。
 */
const runChange = async (change, act) => {
  if (changeBusy.value) return
  changeBusy.value = change.id
  const who = approverName.value || 'user'
  try {
    localStorage.setItem('dbmind.ai.approver', who)
    // 「允许」= 一步执行（批准 + 执行都在后端一个请求里完成）。
    // 不在前端连点 approve/apply：中途失败会留下「已批准但其实没执行」的半截状态。
    if (act === 'allow') {
      const res = await aiChangeAllow(change.id, who)
      if (!res || !res.success) throw new Error(res?.message || t('ai.requestFailed'))
      change.status = res.status || 'applied'
      ElMessage.success(t('ai.chgApplied', { n: res.affectedRows ?? -1 }))
      refreshMate()
      // 批完（这一批都定了）就让智能体接着干活：后续的思考与结论继续吐在同一个气泡里
      continueJob(msgOfChange(change))
      return
    }
    const call = {
      approve: aiChangeApprove,
      confirm: aiChangeConfirm,
      apply: aiChangeApply,
      undo: aiChangeUndo,
      reject: aiChangeReject
    }[act]
    const res = await call(change.id, who)
    if (!res || !res.success) throw new Error(res?.message || t('ai.requestFailed'))
    change.status = res.status || change.status
    if (act === 'apply') ElMessage.success(t('ai.chgApplied', { n: res.affectedRows ?? -1 }))
    else ElMessage.success(t('ai.chgDone'))
    refreshMate()
    // 拒绝/执行完也算「这一批处理完了」：接着跑（继续只读验证或换方案）
    if (act === 'reject' || act === 'apply') continueJob(msgOfChange(change))
  } catch (e) {
    const msg = e?.message || t('ai.requestFailed')
    // 总开关没开（默认关，安全红线）：给一条带「去设置」按钮的通知，
    // 光弹一句红字用户不知道去哪放开，会以为「允许」坏了
    if (/aiWriteEnabled/.test(msg)) {
      showSafetyNotice(msg, {
        title: t('ai.writeOffTitle'),
        hints: [t('ai.writeOffHint')],
        onAction: openSafetySettings,
        actionText: t('tdet.prodBlockedGo')
      })
      return
    }
    // 其余情况后端会说明「为什么现在不能执行」（比如高危还差一次确认），原样展示
    ElMessage.error(msg)
  } finally {
    changeBusy.value = ''
  }
}

/**
 * 跳过：这次不执行，但**不**拒绝 —— 单子仍是 pending，留在「记录」里，
 * 想清楚了下一次还能点允许。只在当前气泡里收起，免得一直占着视线。
 */
const skipChange = (change) => {
  change.skipped = true
  // 全部「先不执行」也算定了：智能体可以接着做剩下的（不落实这张单子）
  const msg = msgOfChange(change)
  if (msg) continueJob(msg)
}

/**
 * 删一条记录。**只删记录**，不撤销已执行的修改 —— 确认框里必须写清楚，
 * 否则用户会把它当成「撤销」来点。
 */
const removeChange = async (change) => {
  if (changeBusy.value) return
  try {
    await ElMessageBox.confirm(
      t('ai.recordDeleteConfirm', { title: change.title }),
      t('ai.recordDelete'),
      { type: 'warning', confirmButtonText: t('common.confirm'), cancelButtonText: t('common.cancel') }
    )
  } catch {
    return // 用户取消
  }
  changeBusy.value = change.id
  try {
    const res = await aiChangeDelete(change.id)
    if (!res || !res.success) throw new Error(res?.message || t('ai.requestFailed'))
    changes.value = changes.value.filter((row) => row.id !== change.id)
    ElMessage.success(t('ai.recordDeleted'))
  } catch (e) {
    ElMessage.error(e?.message || t('ai.requestFailed'))
  } finally {
    changeBusy.value = ''
  }
}

/** 清空记录（同样只删记录，不撤销任何已执行的修改） */
const clearChanges = async () => {
  if (!changes.value.length) return
  try {
    await ElMessageBox.confirm(
      t('ai.recordsClearConfirm', { n: changes.value.length }),
      t('ai.recordsClear'),
      { type: 'warning', confirmButtonText: t('common.confirm'), cancelButtonText: t('common.cancel') }
    )
  } catch {
    return
  }
  mateLoading.value = true
  try {
    const res = await aiChangeClear()
    if (!res || !res.success) throw new Error(res?.message || t('ai.requestFailed'))
    changes.value = []
    ElMessage.success(t('ai.recordsCleared', { n: res.deleted ?? 0 }))
  } catch (e) {
    ElMessage.error(e?.message || t('ai.requestFailed'))
  } finally {
    mateLoading.value = false
  }
}

/**
 * 深挖模式：走 `/api/ai/agent`。
 *
 * 和普通对话的区别是「它自己决定查什么」，所以必须把工具轨迹与产出的变更单
 * 一起摆出来 —— 只给结论不给依据，用户没法判断该不该点执行。
 */
/**
 * 深度思考的帧 → 一条**只往下追加**的时间线（首轮与「审批后续跑」共用）。
 *
 * <p>关键约束（用户反馈「流式输出怎么跑上面去了」）：任何一段文字都只在它首次出现的位置
 * 往下长，**绝不回填、绝不挪位**。
 * 所以 delta 一律追加到时间线最后那个「正在写」的条目上（kind 先按 text 渲染）；
 * 如果这一轮说完话就去调工具，**同一个元素**原地变成灰色思考块（位置不变），
 * 工具行接在它下面，下一轮再往下写。结束时不移动任何东西。
 */
const agentEventSink = (msg, ctxConn, ctxDb) => {
  let lastScroll = 0
  const tick = () => {
    const now = Date.now()
    if (now - lastScroll > 60) { lastScroll = now; scrollBottom() }
  }
  /** 末尾那段「正在写」的文字（没有就新起一段） */
  const livePart = () => {
    const last = msg.timeline[msg.timeline.length - 1]
    if (last && (last.kind === 'text' || last.kind === 'think')) return last
    const part = { kind: 'text', text: '' }
    msg.timeline.push(part)
    return part
  }
  return (ev) => {
    if (!ev) return
    if (ev.t === 'delta') {
      livePart().text += ev.d || ''
      tick()
      return
    }
    // 这一轮说完话就去调工具：刚才那段是「思考」，原地变灰（不动位置）
    if (ev.t === 'round_end') {
      if (ev.calls > 0) {
        const last = msg.timeline[msg.timeline.length - 1]
        if (last && (last.kind === 'text' || last.kind === 'think')) {
          if (last.text.trim()) last.kind = 'think'
          else msg.timeline.pop() // 一个字都没说：不留空块
        }
      }
      tick()
      return
    }
    if (ev.t === 'step_start') {
      msg.timeline.push({ kind: 'tool', running: true, tool: ev.tool, args: ev.args })
      tick()
      return
    }
    if (ev.t === 'step') {
      // 认领同一次调用的「进行中」条目；没认领到（断帧）就补一条
      const live = [...msg.timeline].reverse().find((s) => s.kind === 'tool' && s.running)
      if (live) Object.assign(live, ev.step, { kind: 'tool', running: false })
      else msg.timeline.push({ kind: 'tool', ...ev.step })
      tick()
      return
    }
    if (ev.t === 'change' && ev.change) {
      // 变更单只带 id/sql/risk，卡片上的「连接.库.模式.表」要知道是哪条连接、哪个库
      const card = { ...ev.change, connectionId: ctxConn, database: ctxDb }
      // 同一句 SQL 被重复提出时后端会去重、返回**已有**的单号：这里必须更新原卡，
      // 不能再 push 一张同 id 的（v-for 重复 key，界面上会少画一张卡 ——
      // 用户看到的就是「提示生成了变更单，但卡片没有」）。
      const old = (msg.changes || []).find((c) => c.id === card.id)
      if (old) {
        Object.assign(old, card)
        // 之前「跳过」过的单子又被提出来了：把动作放回来，否则是一张点不动的卡
        old.skipped = false
      } else {
        msg.changes.push(card)
        // **卡片也要按时间顺序落在时间线里**（不能固定堆在气泡底部）：
        // 否则续跑时新吐的文字会跑到「上一轮那张卡片」的上面，看着像顺序错乱。
        msg.timeline.push({ kind: 'change', change: card })
      }
    }
  }
}

/** 气泡里有没有可显示的东西（正文 / 时间线 / 问数结果）—— 决定要不要露「正在生成」的三点 */
const hasBody = (m) => !!(m && (m.content || (m.timeline && m.timeline.length) || m.askAnswer))

/** 还没落进时间线的变更单（正常都应该落位了；流里丢帧 / 老消息才需要兜底渲染） */
const orphanChanges = (m) => {
  const parts = (m && m.timeline) || []
  const placed = new Set(parts.filter((p) => p.kind === 'change').map((p) => p.change.id))
  return ((m && m.changes) || []).filter((c) => !placed.has(c.id))
}

/**
 * 把服务端那份变更单并进消息。**卡片对象必须复用**：
 * 时间线里存的就是同一张卡的引用（点了「允许」要立刻变状态），整表换成新对象会让
 * 时间线上那张变成点不动的僵尸，而 msg.changes 里那份一直停在 pending（续跑不触发）。
 * 流里没吐出来的新单子补进时间线，插在最后一段文字之前（保持「过程 → 单子 → 结论」）。
 */
const mergeChanges = (msg, list, ctxConn, ctxDb) => {
  const rawById = new Map((list || []).map((r) => [r.id, r]))
  const next = (msg.changes || []).map((old) => {
    const raw = rawById.get(old.id)
    return raw ? Object.assign(old, raw) : old
  })
  const seen = new Set(next.map((c) => c.id))
  for (const raw of list || []) {
    if (seen.has(raw.id)) continue
    seen.add(raw.id)
    next.push({ ...raw, connectionId: ctxConn, database: ctxDb })
  }
  msg.changes = next
  const placed = new Set((msg.timeline || []).filter((p) => p.kind === 'change').map((p) => p.change.id))
  const fresh = next.filter((c) => !placed.has(c.id))
  if (!fresh.length) return next
  const tl = msg.timeline || (msg.timeline = [])
  let idx = tl.length
  for (let i = tl.length - 1; i >= 0; i--) {
    if (tl[i].kind === 'text') { idx = i; break }
  }
  tl.splice(idx, 0, ...fresh.map((c) => ({ kind: 'change', change: c })))
  return next
}

const runAgentChat = async (text) => {
  // 多轮上下文：与普通对话同一份口径（最近 8 条，成对出现）
  // 少了这一份，「继续 / 那就按这个改」智能体会答「上次的上下文我这边没有」，
  // 然后重头再体检一遍 —— 既费 token 又不接话。
  const history = messages.value
    .filter(m => (m.role === 'user' || m.role === 'assistant') && m.content)
    .slice(-8)
    .map(m => ({ role: m.role, content: m.content }))
  messages.value.push({ role: 'user', content: text, skills: currentSkills() })
  aiInput.value = ''
  chatLoading.value = true
  startWait()
  scrollBottom()
  // 停止按钮：流式请求同样挂在 signal 上 —— 中断后界面立刻松开（服务端那轮会自然结束并丢弃）
  stopCtrl = new AbortController()
  const ctrl = stopCtrl
  // 这个气泡是**边跑边长**的：思考（灰）→ 工具调用 → 变更单 → 结论（正文）
  const msg = reactive({
    role: 'assistant',
    content: '',
    // 时间线：text（回答/正在写的字）· think（这一轮的思考，灰）· tool（工具调用）
    timeline: [],
    changes: [],
    usage: null,
    jobId: '',
    streaming: true,
    waiting: false
  })
  messages.value.push(msg)
  const ctxConn = ctxConnId.value
  const ctxDb = ctxEffectiveDatabase.value
  const onEv = agentEventSink(msg, ctxConn, ctxDb)
  try {
    const res = await aiAgentStream({
      prompt: text,
      tables: ctxTables.value.slice(),
      connectionId: ctxConnId.value,
      database: ctxEffectiveDatabase.value,
      modelId: selectedModelId.value,
      threadId: chatThreadId.value || undefined,
      history,
      // 按步骤做（看表 → 看结构 → 取证 → 给建议 → 出单）每步都要查几条，
      // 6 轮会卡在「还没下结论」上；后端上限 12，用尽时会强制再要一次结论
      maxRounds: 10,
      // 知识库范围：与普通对话同义（null = 全部、[] = 本次不注入、[...] = 指定库）。
      // 底栏选了什么就要真生效 —— 以前深度思考不发这个字段，选了「全部资料库」它也不查
      kbIds: kbIdsForQuery(),
      // 选中的插件技能同样带过去（后端把提示词拼进 system）：选着技能切深度思考不该失效
      pluginSkills: pickedSkills.value.map((s) => s.id),
      // 落库：与普通对话同一口径 —— 智能体以前不进会话，刷新后这段对话直接消失
      persist: !chatThreadId.value
    }, onEv, { signal: ctrl.signal })
    if (res && res.threadId) { chatThreadId.value = res.threadId; if (isStudio.value) loadThreads() }
    // 收尾：末尾那段「正在写的字」就是回答正文；工具条目以服务端那份为准（防丢帧）
    settleTimeline(msg, res)
    msg.jobId = (res && res.jobId) || ''
    // 整次回答的 token 用量（后端按轮相加后随 done 帧回来），气泡底部脚注要用
    if (res && res.usage) msg.usage = addUsage(msg.usage, res.usage)
    if (res && Array.isArray(res.changes) && res.changes.length) mergeChanges(msg, res.changes, ctxConn, ctxDb)
    // 有单子就是「等审批」：界面提示「批准后我接着往下做」，点完会自动续跑
    msg.waiting = msg.changes.length > 0
    if (msg.waiting) refreshMate()
  } catch (e) {
    // 中断/失败都把已流出的部分留着（思考与结论都算跑过的东西），末尾补一行说明
    msg.timeline.push({ kind: 'think', text: (isCanceled(e) ? '⏱ ' : '❌ ') + (isCanceled(e) ? t('api.generationStopped') : (e?.message || t('ai.requestFailed'))) })
  } finally {
    msg.streaming = false
    msg.timeline = msg.timeline.map((s) => (s.kind === 'tool' ? { ...s, running: false } : s))
    if (stopCtrl === ctrl) stopCtrl = null
    chatLoading.value = false
    stopWait()
    scrollBottom()
  }
}

/**
 * 审批后继续：这一批变更单都处理完了，就让这张 job 接着往下跑。
 *
 * <p>服务端按已落库的步骤**重放**（不会再执行任何工具），新的思考、工具与结论
 * 继续吐在同一个气泡里 —— 用户看到的是「批完之后它接着干活」，而不是整段结果一次砸下来。
 */
const continueJob = async (msg) => {
  if (!msg || !msg.jobId || msg.continuing) return
  // 还有没定的单子：等人把这一批都处理完再接着跑（否则点一张就续一次，后续步骤白跑）
  const pendingLeft = (msg.changes || []).some((c) => c.status === 'pending' && !c.skipped)
  if (pendingLeft) return
  msg.continuing = true
  msg.waiting = false
  msg.streaming = true
  chatLoading.value = true
  startWait()
  stopCtrl = new AbortController()
  const ctrl = stopCtrl
  const c0 = (msg.changes || [])[0] || {}
  const ctxConn = c0.connectionId || ctxConnId.value
  const ctxDb = c0.database || ctxEffectiveDatabase.value
  const onEv = agentEventSink(msg, ctxConn, ctxDb)
  // 分隔线：续跑的字要另起一段往下写（不接在上一段结论的末尾）
  msg.timeline.push({ kind: 'divider' })
  try {
    // 聚焦表按**当前底栏**传：用户中途换了表，续跑就该按新表来（不是 job 里那张旧表）
    const res = await aiJobResumeStream(msg.jobId, {
      maxRounds: 10,
      tables: ctxTables.value.slice()
    }, onEv, { signal: ctrl.signal })
    settleTimeline(msg, res, true)
    // 续跑也花了 token：累加到底部脚注，别让数字只反映第一段
    if (res && res.usage) msg.usage = addUsage(msg.usage, res.usage)
    if (res && Array.isArray(res.changes) && res.changes.length) mergeChanges(msg, res.changes, ctxConn, ctxDb)
    if (res && res.jobId) msg.jobId = res.jobId
    msg.waiting = (msg.changes || []).some((c) => c.status === 'pending' && !c.skipped)
    if (msg.changes.length) refreshMate()
  } catch (e) {
    msg.timeline.push({ kind: 'think', text: (isCanceled(e) ? '⏱ ' : '❌ ') + (isCanceled(e) ? t('api.generationStopped') : (e?.message || t('ai.requestFailed'))) })
  } finally {
    msg.continuing = false
    msg.streaming = false
    msg.timeline = msg.timeline.map((s) => (s.kind === 'tool' ? { ...s, running: false } : s))
    if (stopCtrl === ctrl) stopCtrl = null
    chatLoading.value = false
    stopWait()
    scrollBottom()
  }
}

/**
 * 收尾：把时间线定格。
 *  - 末尾那段 `text` 就是结论（写进 m.content —— 历史、复制、导出都靠它）；
 *  - 工具条目以服务端返回的 steps 为准（流里万一断帧也不丢）。
 * `append` = 续跑：结论接在前一段后面，而不是把它顶掉。
 */
const settleTimeline = (msg, res, append = false) => {
  const timeline = msg.timeline
  const lastText = [...timeline].reverse().find((p) => p.kind === 'text')
  if (lastText) {
    msg.content = append ? [msg.content, lastText.text].filter(Boolean).join('\n\n') : lastText.text
  } else if (res && res.content) {
    timeline.push({ kind: 'text', text: res.content })
    msg.content = append ? [msg.content, res.content].filter(Boolean).join('\n\n') : res.content
  }
  // 服务端的步骤比流里全：缺的补在结论之前（不打断「先过程、后结论」的阅读顺序）
  const have = timeline.filter((p) => p.kind === 'tool').length
  const steps = (res && res.steps) || []
  if (steps.length > have) {
    const extra = steps.slice(have).map((s) => ({ kind: 'tool', ...s }))
    const idx = lastText ? timeline.indexOf(lastText) : timeline.length
    timeline.splice(idx >= 0 ? idx : timeline.length, 0, ...extra)
  }
}

// ---------------------------------------------------------------- 记录（会话列表）
/**
 * 左栏展开 / 收起。**默认收起**（用户反馈：一进来先看见一列历史，对话区被挤窄）——
 * 收起态只留一条竖窄条（展开键 + 新建 + 竖排「记录」），想翻历史再点开。
 * 选择记在 localStorage；键名带 v2 是因为旧键里存的是「默认展开」时代的选择，
 * 沿用会让老用户升级后仍然是展开的。
 */
const HIST_OPEN_KEY = 'dbmind_ai_hist_open.v2'
const histOpen = ref(localStorage.getItem(HIST_OPEN_KEY) === '1')
/** 「请求记录」对话框：看最近几次真正发给模型的 system + 消息（后端内存快照） */
const reqLogOpen = ref(false)
const toggleHist = () => {
  histOpen.value = !histOpen.value
  try { localStorage.setItem(HIST_OPEN_KEY, histOpen.value ? '1' : '0') } catch { /* 隐私模式写不了，忽略 */ }
}

// 列表数据只服务中央工作区的左栏；侧栏窄面板拿不到这个位置，也就不发这次请求。
const threads = ref([])
const histLoading = ref(false)
/** 列表里只显示「月-日」，完整时间在 title 上；空值不渲染。
 *  问数记录的时间是本地毫秒数、会话记录是 ISO 字符串，两种都要认。 */
const dayOf = (value) => {
  if (typeof value === 'number' && Number.isFinite(value)) {
    const d = new Date(value)
    if (Number.isNaN(d.getTime())) return ''
    const p = (n) => String(n).padStart(2, '0')
    return `${p(d.getMonth() + 1)}-${p(d.getDate())}`
  }
  const s = String(value || '')
  return s.length >= 10 ? s.slice(5, 10) : ''
}

/** 本地问数历史：问数走 /api/ai/ask 不落服务端会话，靠这份本地记录让左栏也能回看 */
const askHist = ref(listAskHistory() || [])
/**
 * 点一条问数记录：把这段对话**整段**铺回对话流（每一轮问答，不只最后一轮），
 * 并把技能切到「智能问数」；之后继续追问仍并回这条记录。
 */
const openAskHistory = (h) => {
  if (!h || !h.answer) return
  tab.value = 'ask'
  chatThreadId.value = ''
  askThreadId.value = h.id
  // 新结构按 turns 回放；老版本条目（一条记录 = 一轮）走兜底
  const turns = Array.isArray(h.turns) && h.turns.length
    ? h.turns
    : [{ q: h.question, answer: h.answer }]
  messages.value = turns.flatMap((t) => [
    { role: 'user', content: t.q },
    { role: 'assistant', content: '', askAnswer: t.answer },
  ])
  // 换记录时退出编辑态（否则编辑框会挂在新记录的同一条下标上）
  editingIndex.value = -1
  scrollBottom()
}
const deleteAskHistory = (h) => {
  if (!h?.id) return
  removeAskHistory(h.id)
  if (askThreadId.value === h.id) askThreadId.value = ''
  askHist.value = listAskHistory()
}

/**
 * 左栏的统一列表：问数（本地）与对话（服务端）各取「标题 + 时间」摊平，
 * 按时间新的在前 —— 用户不需要知道背后是两种存储，点开就能回到那一段。
 */
const histItems = computed(() => {
  const asks = (askHist.value || []).map((h) => ({
    kind: 'ask',
    id: h.id,
    title: h.question,
    day: dayOf(h.ts),
    ts: Number(h.ts) || 0,
    raw: h,
  }))
  const ths = (threads.value || []).map((th) => ({
    kind: 'thread',
    id: th.id,
    title: th.title || t('ai.historyUntitled'),
    day: dayOf(th.updatedAt || th.createdAt),
    ts: Date.parse(th.updatedAt || th.createdAt || '') || 0,
    raw: th,
  }))
  return [...asks, ...ths].sort((a, b) => b.ts - a.ts)
})

const loadThreads = async () => {
  histLoading.value = true
  try {
    const res = await aiThreadList()
    threads.value = Array.isArray(res?.threads) ? res.threads : []
  } catch { /* 列表拉不到不影响正在聊的这一段 */ } finally {
    histLoading.value = false
  }
}

/** 把某条会话的消息装进对话区：打开历史与「恢复上次」共用这一份 */
const applyThread = async (id) => {
  const detail = await aiThreadGet(id)
  const rows = detail?.messages || []
  chatThreadId.value = id
  // 会话内容被对话线程整个换掉了：问数记录的「当前对话」也随之作废，
  // 之后的追问该另起一条记录，而不是并进一条八竿子打不着的旧记录
  askThreadId.value = ''
  // 变更单的最新状态以「记录」里的那份为准（消息 meta 里存的是生成时的快照）：
  // 用户批准/拒绝后刷新，卡片不该还写着「待确认」
  const chgById = new Map((changes.value || []).map(c => [c.id, c]))
  messages.value = rows.map(row => {
    // 落库时把技能名写进 meta.skills、智能体的工具轨迹与变更单写进 meta.steps/changes；
    // 老消息没有 meta（null）就没有这些
    const meta = row.meta && typeof row.meta === 'object' ? row.meta : null
    const skills = Array.isArray(meta?.skills)
      ? meta.skills.map((name, i) => ({ id: `s${i}`, name: String(name) }))
      : []
    const chgCards = Array.isArray(meta?.changes)
      ? meta.changes.map((c) => {
        const live = chgById.get(c?.id)
        return live ? { ...c, status: live.status, error: live.error || c.error, appliedAt: live.appliedAt } : c
      })
      : []
    // 老会话里存着「（只基于表 … 回答…）」这段元指令：不剥掉的话，重开记录就看见
    // 一行用户从没敲过的话（还会被当成历史再次发给模型，强化「不要扩散」）
    const bodyText = stripScopeHint(row.content)
    // 深度思考的还原：**每一步的叙述 + 这一步的工具**，按轮次放回去（meta.texts 是每轮说过的
    // 那段话）。没有 texts 的老消息只能退回「工具全堆在前面」—— 用户反馈「刷新后位置变了」
    // 就是老行为：工具全在最上面，看图和结论对不上。
    const steps = Array.isArray(meta?.steps) ? meta.steps : []
    const thoughtByRound = new Map()
    for (const one of (Array.isArray(meta?.texts) ? meta.texts : [])) {
      const text = typeof one?.text === 'string' ? one.text.trim() : ''
      if (text) thoughtByRound.set(Number(one.round) || 0, one.text)
    }
    let parts = []
    if (steps.length) {
      const rounds = [...new Set(steps.map((s) => Number(s.round) || 0))].sort((a, b) => a - b)
      for (const round of rounds) {
        const said = thoughtByRound.get(round)
        if (said) parts.push({ kind: 'think', text: said })
        for (const s of steps) {
          if ((Number(s.round) || 0) === round) parts.push({ kind: 'tool', ...s })
        }
      }
      if (bodyText) parts.push({ kind: 'text', text: bodyText })
    }
    // 变更单也落进时间线：卡片就是按位置渲染的，只留在 changes 里会画不出来
    const timeline = chgCards.length
      ? [...parts, ...chgCards.map((c) => ({ kind: 'change', change: c }))]
      : parts
    return {
      role: row.role,
      // 服务端消息 id：编辑这条重新作答时按它精确剪掉后面的问答
      msgId: typeof row.id === 'number' ? row.id : undefined,
      content: bodyText,
      usage: row.usage ? safeParse(row.usage) : null,
      skills,
      timeline,
      changes: chgCards,
      // 重开记录后还要能「批准后继续」：jobId 存进 meta，等审批的单子也照旧提示
      jobId: (meta && typeof meta.jobId === 'string') ? meta.jobId : '',
      waiting: chgCards.some((c) => c.status === 'pending')
    }
  })
  // 换记录时退出编辑态（编辑框是按下标挂的，换了内容就会挂错）
  editingIndex.value = -1
  scrollBottom()
}

/**
 * 刷新后接回「刚才打开的那一段对话」，而不是只挑最近一条。
 *
 * <p>两类记录分开存：问数（`/api/ai/ask`）在本地 localStorage 里按 id 存整段问答，
 * 自由对话 / 深度思考在服务端 ai_threads 里 —— 指针（ai.session）记住是哪一个，
 * 刷新后按类型各自还原。只按「最近一条会话」恢复的话，问数对话刷新后对话区是空的
 * （记录还在左栏，用户看到的就是「对话丢了」）。
 *
 * <p>没有指针 / 指针指向的记录已被删掉时，退回「最近一条会话」——
 * 不恢复的话「关掉再打开」永远从零开始，记忆也显得没生效。
 */
const restoreSession = async () => {
  await ensureAiPanelPrefs()
  // 左栏「记录」清单必须先拉，且**不能被下面的对话守卫挡住**：
  // threads 是组件局部 ref，切走页签卸载再挂载时归零；而 messages 在 panelState 里
  // 跨卸载保留，守卫会命中直接 return —— 结果对话还在、左侧记录却空了，
  // 只有整页刷新（清空 panelState）才加载。列表加载与"是否恢复对话"是两件事。
  const listLoad = isStudio.value ? loadThreads() : null
  // 页签切走再切回：panelState 里对话还留着，别覆盖（这是跨卸载，不是刷新）
  if (messages.value.length) {
    if (listLoad) await listLoad
    return
  }
  const s = readAiSession(_mode)
  if (s.askThreadId) {
    const h = (askHist.value || []).find((x) => x.id === s.askThreadId)
    if (h) {
      openAskHistory(h)
      if (listLoad) await listLoad
      return
    }
  }
  if (s.chatThreadId) {
    if (['chat', 'insight', 'dictionary'].includes(s.tab)) tab.value = s.tab
    try {
      await applyThread(s.chatThreadId)
      if (listLoad) await listLoad
      return
    } catch { /* 记录被删了：走下面的兜底 */ }
  }
  if (listLoad) await listLoad
  const latest = threads.value[0]
  if (!latest) return
  try { await applyThread(latest.id) } catch { /* 恢复失败就当新对话，不打扰用户 */ }
}

/**
 * 面板挂载时的恢复顺序：先读库里的偏好（要等一次请求）→ 再拉连接清单
 * （恢复的连接要校验还在）→ 套用底栏选择 → 接回上次那段对话。
 */
const bootRestore = async () => {
  await ensureAiPanelPrefs()
  // 问数记录：换端口后 localStorage 是空的（origin 变了），从库里那份镜像灌回来 ——
  // 必须在 restoreSession 之前，否则指针指向的那条记录还「不存在」
  await ensureAskHistoryStore()
  askHist.value = listAskHistory()
  await loadConnList()
  hydrateComposer()
  await restoreSession()
}

/** 点左栏某条记录：续聊这一段，并把技能切回「自由对话」（会话内容是对话技能的） */
const openThread = async (th) => {
  if (!th?.id || th.id === chatThreadId.value) return
  tab.value = 'chat'
  try { await applyThread(th.id) } catch { ElMessage.error(t('ai.requestFailed')) }
}

/** 重命名一条记录：后端 POST /threads 是 upsert，带 id 就是改标题 */
const renameThread = async (th) => {
  if (!th?.id) return
  let title = ''
  try {
    const res = await ElMessageBox.prompt(t('ai.historyRenameHint'), t('ai.historyRename'), {
      inputValue: th.title || '',
      inputPlaceholder: t('ai.historyUntitled'),
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
      inputValidator: (v) => (String(v ?? '').trim() ? true : t('ai.historyRenameEmpty'))
    })
    title = String(res?.value ?? '').trim()
  } catch { return /* 取消 */ }
  if (!title) return
  try {
    await aiThreadRename(th.id, title)
    th.title = title
    await loadThreads()
  } catch { ElMessage.error(t('ai.requestFailed')) }
}

/** 删除一条记录：删的正是当前这段时，对话区一起清空 */
const deleteThread = async (th) => {
  if (!th?.id) return
  const title = th.title || t('ai.historyUntitled')
  try {
    await ElMessageBox.confirm(t('ai.historyDeleteConfirm', { title }), t('ai.historyDelete'), {
      type: 'warning',
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel')
    })
  } catch { return }
  try {
    await aiThreadDelete(th.id)
    if (chatThreadId.value === th.id) { chatThreadId.value = ''; messages.value = [] }
    await loadThreads()
  } catch { ElMessage.error(t('ai.requestFailed')) }
}

const newThread = () => {
  chatThreadId.value = ''
  askThreadId.value = ''
  messages.value = []
  ElMessage.success(t('ai.newThreadDone'))
}

/** 左栏「+」：开新对话。旧的那段已经落库，记录里还在 */
const onHistoryNew = async () => {
  newThread()
  await loadThreads()
}

const scrollBottom = () => nextTick(() => {
  if (chatBox.value) chatBox.value.scrollTop = chatBox.value.scrollHeight
})

const copySql = async (text) => {
  try { await navigator.clipboard.writeText(text); ElMessage.success(t('ai.copied')) } catch (e) { ElMessage.error(t('sqlq.copyFailed')) }
}

// ---------------------------------------------------------------- 用户消息：编辑 / 复制
/** 正在编辑的提问下标（-1 = 没有在编辑） */
const editingIndex = ref(-1)
const editingText = ref('')

const startEdit = (i) => {
  const m = messages.value[i]
  if (!m || m.role !== 'user' || generating.value) return
  editingIndex.value = i
  editingText.value = m.content || ''
  nextTick(() => {
    const el = document.querySelector(`.ai-panel .msg-edit-input textarea`)
    if (el) { el.focus(); el.setSelectionRange(el.value.length, el.value.length) }
  })
}
const cancelEdit = () => {
  editingIndex.value = -1
  editingText.value = ''
}

/**
 * 编辑后重新作答：**这条提问之后的问答全部作废**，用新内容重新发一遍。
 *
 * <p>不新开会话：仍然接着同一条记录（服务端会话是追加式的，重答就是新的一轮），
 * 这样「追问的上文」还在，用户也不会突然多出一堆记录。
 */
const submitEdit = async () => {
  const text = editingText.value.trim()
  const idx = editingIndex.value
  if (!text || idx < 0 || generating.value) return
  // 服务端会话也剪掉：把这条（含）之后的问答删掉，否则刷新/重开时旧答案又整段回来，等于没改。
  // 优先用它自己的消息 id（精确）；老消息没有 id 才退回「前 idx 条」。
  if (chatThreadId.value && tab.value !== 'ask') {
    const before = messages.value[idx] && messages.value[idx].msgId
    try {
      await aiThreadTrim(chatThreadId.value, before ? { beforeId: before } : { keep: idx })
    } catch { /* 剪失败不挡重发 */ }
  }
  // 问数记录存在本地：改的是这条记录里的某一问 ⇒ 整条记录作废，重发时会新建一条，
  // 否则点开旧记录，被改掉的那几轮又回来了
  if (tab.value === 'ask' && askThreadId.value) {
    removeAskHistory(askThreadId.value)
    askThreadId.value = ''
    askHist.value = listAskHistory()
  }
  // splice(idx) 把这条（它自己也要重发）及之后的消息全部去掉
  messages.value.splice(idx)
  cancelEdit()
  aiInput.value = text
  await sendChat()
}

const copyMsg = async (m) => {
  try {
    await navigator.clipboard.writeText(m.content || '')
    ElMessage.success(t('ai.copied'))
  } catch (e) {
    ElMessage.error(t('sqlq.copyFailed'))
  }
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
  emit('insert-sql', text)
}

/**
 * markdown 里每个 SQL 代码块的操作按钮走事件委托：
 * v-html 出来的内容无法绑定 Vue 事件，只能靠 data-sql-act / data-sql-idx 分发。
 * 索引与 renderMarkdown(src, { sqlActions: true }) 生成的按钮严格对应，
 * 所以这里用同一份原文重新抽取 SQL。
 */
const onMdSqlAction = (e, md) => {
  // 「前往设置」直达链接（引导性错误消息尾部）：广播给 MainView 打开设置并定位页签
  const go = e.target instanceof Element ? e.target.closest('[data-goto]') : null
  if (go) {
    e.preventDefault()
    e.stopPropagation()
    window.dispatchEvent(new CustomEvent('dc-open-settings', { detail: { tab: go.dataset.goto } }))
    return
  }
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
  // 只插入、不执行：这段 SQL 可能是改数据的语句，跑不跑由人在编辑器里确认
  else emit('insert-sql', text)
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
  // PDF 后端尚未实现（doc.rs 明确返回 not_implemented）。菜单已置灰，这里再挡一层，
  // 避免旧缓存/命令行直接触发时弹出难懂的后端错误文案。
  if (format === 'pdf') { ElMessage.warning(t('ai.pdfHint')); return }
  exporting.value = format
  try {
    const title = nameHint || currentSkill.value.label
    const base = [title, ctxDatabase.value || ctxConnName.value].filter(Boolean).join('_')
    const filename = base + '.' + format
    // 先选目录再让后端转换（PDF 等慢转换也不会让用户等完才发现没选位置）
    const saved = await saveGenerated({
      filename,
      generate: async () => {
        try {
          return await aiExportDoc({ title, markdown, format, fileName: base })
        } catch (e) {
          // 后端报错时 response.data 也是 Blob，saveGenerated 的通用 errMsg 读不出内容；
          // 在这里先把后端 message 解出来再抛，用户才看得到真正的失败原因。
          throw new Error(await blobErrMsg(e) || t('qa.exportFailed'))
        }
      }
    })
    // 取消 / 保存失败都不该说「已导出」；失败提示由 saveGenerated 统一给过。
    // 注意以前 return 写在 try 里而重置写在 try 外，取消一次按钮就一直转圈
    if (saved.canceled || saved.ok === false) return
    ElMessage.success(t('ai.exportedFmt', { fmt: format.toUpperCase() }))
  } catch (e) {
    ElMessage.error(await blobErrMsg(e) || t('qa.exportFailed'))
  } finally {
    exporting.value = ''
  }
}

// ===== 统一执行入口与状态（按当前 Skill 分派） =====

/** 任一 Skill 正在执行 → 主按钮 loading */
const busy = computed(() => chatLoading.value)

/** 是否已有结果（有对话消息即有） */
const hasResult = computed(() => messages.value.length > 0)
/** 空态默认面板：无结果且不在执行中时展示。
 *  知识入库自带完整的引导界面，再叠一层欢迎页只会把它的入口挤到下面去。 */
const showWelcome = computed(() => !hasResult.value && !busy.value && !isKbIngest.value)

/** 主按钮文案随 Skill 变化 */
const RUN_LABEL = computed(() => ({
  chat: t('ai.runChat'), dictionary: t('ai.runDictionary'), insight: t('ai.runInsight'), ask: t('ai.runAsk')
}))
const runLabel = computed(() => RUN_LABEL.value[tab.value] || t('ai.runChat'))

/** 是否可执行：数据字典允许留空（整库） */
const canRun = computed(() => {
  if (busy.value) return false
  if (tab.value === 'dictionary') return !!ctxConnId.value
  return !!aiInput.value.trim()
})

/**
 * 发送前内容预览：让用户真正看见「这次要给模型什么」。开关存 app_settings（不是
 * localStorage —— 桌面壳端口会变，按 origin 隔离的偏好会莫名丢失）。
 */
const sendPreviewOpen = ref(false)
const sendPreviewQuestion = ref('')
const sendPreviewRequired = ref(false)
let pendingSend = null
const loadSendPreviewSetting = async () => {
  try {
    const all = await getSettings()
    const v = all?.['ai.sendPreview']
    sendPreviewRequired.value = v === true || v === 'true'
  } catch (e) { /* 老内核没这个键，默认不拦 */ }
}
const openSendPreview = () => {
  sendPreviewQuestion.value = aiInput.value.trim()
  pendingSend = null
  sendPreviewOpen.value = true
}
const confirmSendPreview = () => {
  const fn = pendingSend
  pendingSend = null
  sendPreviewOpen.value = false
  if (fn) fn()
}
/** 从「这次发给模型什么了」回放：把原问题填回输入框，用户改一词就能再问一次 */
const onReqReplay = (text) => {
  aiInput.value = String(text || '').trim()
  reqLogOpen.value = false
  nextTick(() => scrollBottom())
}
/** 发送前预览的公共闸门：没开开关就直接跑；开了就先给用户看一眼。 */
const withSendPreview = (text, run) => {
  if (!sendPreviewRequired.value) { run(); return }
  sendPreviewQuestion.value = text
  pendingSend = run
  sendPreviewOpen.value = true
}

/** 统一执行：按当前 Skill 分派到对应实现 */
const runSkill = async () => {
  if (!canRun.value) return
  const text = aiInput.value.trim()
  withSendPreview(text, () => {
    switch (tab.value) {
      case 'chat':
      case 'insight':
      case 'dictionary':
      case 'ask': sendChat(); break
    }
  })
}

/**
 * 只有「面板真的可见」时才值得拉的几笔：知识库清单、待办、记忆。
 *
 * <p>面板隐藏时（右侧窄面板用 v-show 常驻）这些数据一个都看不到，却在首屏占了三笔请求。
 * 打开面板时补上：`openMate` 本来就会再拉一次待办/记忆，知识库清单也是下拉展开才用。
 */
let inboxLoaded = false
const loadInboxData = () => {
  if (inboxLoaded) return
  inboxLoaded = true
  loadKbMeta()
  refreshMate()
}

onMounted(() => {
  scrollBottom()
  warmCtx()
  // 插件清单要先拉到：历史消息只存技能名，气泡上的 emoji 图标靠清单按名反查
  loadPluginSkills()
  // 接上次会话 + 底栏已选**必须首屏就跑**：用户可能先点「发送到 AI」，那一刻对话得已经恢复在，
  // 否则这条新消息会插进空对话里，历史看起来"被清空了"。
  bootRestore()
  if (props.active) loadInboxData()
})
watch(() => props.active, (v) => { if (v) loadInboxData() })

// ===== 底栏上下文 chips 小屏压缩：永远单行，放不下按级收成纯图标（v-fast-tip 快速提示） =====
// 收级次序（data-g 越小越先收）：1 模型 → 2 知识库 → 3 聚焦表 → 4 数据源；
// 深度思考/记事/自由对话是模式入口，不收。与 SQL 工具条同一套交互口径。
const composerBarEl = ref(null)
const composerBoxEl = ref(null)
const COMPOSER_MAX_G = 7
// 与 SQL 工具条同一套口径：观察外层稳定容器、rAF 合并、二分求最小收起级数。
// Server 2012 R2 的 WebView2 合成链路很慢，逐级 class 切换 + 每轮 scrollWidth 强制重排
// 会把拖窗口的每一帧拖成卡顿；级数不变时也不再碰 DOM，避免 RO 自激。
let composerFitRaf = 0
const applyComposerLv = (lv) => {
  const bar = composerBarEl.value
  if (!bar) return
  bar.querySelectorAll('[data-g]').forEach((el) => {
    const on = Number(el.dataset.g) <= lv
    if (el.classList.contains('icon-only') !== on) el.classList.toggle('icon-only', on)
  })
}
const composerOverflow = () => {
  const bar = composerBarEl.value
  if (!bar) return false
  return bar.scrollWidth > bar.clientWidth + 1
}
const fitComposerBar = () => {
  const bar = composerBarEl.value
  if (!bar || !bar.offsetParent) return
  const fits = (lv) => { applyComposerLv(lv); return !composerOverflow() }
  applyComposerLv(0)
  if (fits(0)) return
  // 二分找「最小的能放下的级别」。fits(lv)=true 表示收起 <=lv 后不再溢出，
  // 所以 mid 本身是可行解 → hi=mid（不是 mid-1）；false 表示 mid 还不够 → lo=mid+1
  // （不是 lo=mid）。原来边界各偏一格，收敛到「还溢出」的级别，最右的发送组
  // 被挤出滚动区且滚动条隐藏 —— 表现为小屏/mac 看不到发送按钮。
  let lo = 0
  let hi = COMPOSER_MAX_G
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2)
    if (fits(mid)) hi = mid
    else lo = mid + 1
  }
  applyComposerLv(lo)
}
const scheduleComposerFit = () => {
  if (composerFitRaf) return
  composerFitRaf = requestAnimationFrame(() => { composerFitRaf = 0; fitComposerBar() })
}
let composerRo = null
onMounted(() => {
  composerRo = new ResizeObserver(scheduleComposerFit)
  const target = composerBoxEl.value || composerBarEl.value
  if (target) composerRo.observe(target)
  nextTick(fitComposerBar)
  // 字体就绪后 chip 文字宽度会变（首帧回退字体窄），重新量一次
  if (document.fonts?.ready) document.fonts.ready.then(scheduleComposerFit).catch(() => {})
})
onBeforeUnmount(() => {
  if (composerRo) { composerRo.disconnect(); composerRo = null }
  if (composerFitRaf) cancelAnimationFrame(composerFitRaf)
})
// 文案宽度变化（语言、选中的库/模型名）也要重新量
watch([currentModelLabel, kbScopeLabel, () => ctxTables.value.length, () => locale.value], () => nextTick(scheduleComposerFit))
</script>

<style scoped>
/* 技能结果消息（数据字典 / 数据洞察）的导出按钮：气泡内右对齐的小按钮 */
.msg-export { display: flex; justify-content: flex-end; margin-top: 6px; padding-top: 6px; border-top: 1px dashed var(--dc-border); }
.msg-export-btn {
  display: inline-flex; align-items: center; gap: 4px; cursor: pointer;
  font: inherit; font-size: 12px; color: var(--dc-text-dim);
  background: transparent; border: 1px solid var(--dc-border); border-radius: 6px; padding: 3px 10px;
  transition: color .15s ease, border-color .15s ease;
}
.msg-export-btn:hover { color: var(--dc-primary); border-color: var(--dc-primary); }
.msg-export-btn:disabled { opacity: .5; cursor: default; }

/* 引导性错误消息尾部的「前往设置」直达链接（data-goto，点击走气泡上的委托） */
.ai-markdown :deep(.md-goto) {
  display: inline-block; margin-left: 8px; padding: 1px 8px; border-radius: 6px;
  font-size: 12px; color: var(--dc-primary); background: transparent;
  text-decoration: none; transition: background .15s ease;
}
.ai-markdown :deep(.md-goto:hover) { background: var(--dc-bg-soft); }
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
/* 中央工作区：左栏「记录」+ 右栏对话，横向两列。
   侧栏（420px 窄面板）仍是单列 —— 再切一栏就没地方聊了。 */
.ai-panel.is-studio .ai-main { flex-direction: row; }
.ai-panel.is-studio .ai-body { flex: 1 1 auto; min-width: 0; }

/* 左栏「记录」：一条会话一行，悬浮出删除；顶部 + 新建 */
.ai-hist {
  flex: none; width: 216px; min-height: 0;
  display: flex; flex-direction: column;
  border-right: 1px solid var(--dc-border);
  background: var(--dc-bg-soft);
  overflow: hidden;
  transition: width .18s ease;
}
.hist-top { display: flex; align-items: center; gap: 8px; padding: 12px 8px 8px 12px; }
.hist-title {
  flex: 1; min-width: 0; font-size: 12px; font-weight: 600;
  color: var(--dc-text-dim); letter-spacing: .4px;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.hist-new,
.hist-toggle {
  flex: none; width: 24px; height: 24px; display: inline-flex;
  align-items: center; justify-content: center;
  border: 1px solid var(--dc-border); border-radius: 6px;
  background: transparent; color: var(--dc-text-mid); cursor: pointer;
  transition: color .15s ease, border-color .15s ease, background .15s ease;
}
.hist-new:hover,
.hist-toggle:hover { color: var(--dc-primary); border-color: var(--dc-primary); background: var(--dc-primary-wash); }
/* 收起：只留一条窄轨（竖排标题 + 两个图标），对话区吃掉剩下的宽度 */
.ai-hist.is-collapsed { width: 44px; }
.hist-top.is-col { flex-direction: column; align-items: center; gap: 8px; padding: 12px 0 8px; }
.hist-vlabel {
  writing-mode: vertical-rl; text-orientation: mixed;
  font-size: 11px; letter-spacing: 2px; color: var(--dc-text-weak);
  margin-top: 4px; user-select: none;
}
/* position:relative —— v-loading 的遮罩是绝对定位的，父级没定位会盖住整个面板 */
.hist-list { position: relative; flex: 1; min-height: 0; overflow-y: auto; padding: 0 8px 10px; }
.hist-item {
  position: relative; display: flex; align-items: center; gap: 6px;
  width: 100%; margin-bottom: 2px; padding: 7px 8px;
  border: none; border-radius: 7px; background: transparent;
  color: var(--dc-text-mid); font-size: 12.5px; font-family: inherit;
  text-align: left; cursor: pointer;
  transition: background .15s ease, color .15s ease;
}
.hist-item:hover { background: var(--dc-bg-card); color: var(--dc-text); }
.hist-item.active { background: var(--dc-primary-wash); color: var(--dc-primary); font-weight: 500; }
.hist-item-tx { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.hist-item-day { flex: none; font-size: 11px; color: var(--dc-text-weak); font-variant-numeric: tabular-nums; }
/* 悬浮时日期让位给删除：同一行位置有限，平时看时间、要删时才出按钮 */
.hist-item:hover .hist-item-day { display: none; }
/* 重命名 / 删除：平时都不显示，悬浮这一行才一起出现 ——
   列表默认只该有「标题 + 日期」，两个图标常驻会把标题挤成半截。 */
.hist-ren,
.hist-del {
  flex: none; display: none; align-items: center; justify-content: center;
  width: 20px; height: 20px; border-radius: 5px;
}
.hist-ren { color: var(--dc-text-weak); }
.hist-del { color: var(--dc-danger); }
.hist-item:hover .hist-ren,
.hist-item:hover .hist-del { display: inline-flex; }
.hist-ren:hover { color: var(--dc-primary); background: var(--dc-primary-wash); }
.hist-del:hover { background: rgba(220, 60, 60, .14); }
.hist-empty { padding: 22px 8px; text-align: center; font-size: 11.5px; line-height: 1.7; color: var(--dc-text-weak); }

/* Skills 按钮：文案恒定「Skills」，不随当前页变化（当前在哪由输入框里的 chip 表达） */
.skill-btn {
  display: inline-flex; align-items: center; gap: 5px; flex-shrink: 0;
  height: 28px; padding: 0 10px; border: 1px solid var(--dc-border); border-radius: 8px; cursor: pointer;
  background: transparent; color: var(--dc-text); font-size: 13px; font-family: inherit;
  transition: background .15s ease, color .15s ease, border-color .15s ease;
}
.skill-btn:hover { border-color: var(--dc-primary); color: var(--dc-primary); }
.skill-btn.open { border-color: var(--dc-primary); color: var(--dc-primary); background: var(--dc-primary-wash); }
.skill-btn-ic { color: var(--dc-primary); display: inline-flex; align-items: center; font-size: 14px; }
.skill-btn-tx { font-weight: 400; }
.skill-btn-caret { font-size: 13px; color: var(--dc-text-weak); transition: transform .18s ease; }
.skill-btn.open .skill-btn-caret { transform: rotate(180deg); }

/* 选中的技能 chip：**只有选了技能才渲染这一行**（默认自由对话时输入框上方是空的）。
   内置能力占一个，插件技能可叠加多个；点 × 摘掉（内置那个 × 就是回自由对话） */
.picked-skills { display: flex; flex-wrap: wrap; gap: 6px; padding: 9px 12px 0; }
.picked-skill {
  display: inline-flex; align-items: center; gap: 5px; max-width: 100%;
  padding: 2px 6px 2px 2px; border-radius: 999px;
  background: var(--dc-primary-wash); border: 1px solid var(--dc-primary-glow);
  color: var(--dc-primary); font-size: 11.5px; line-height: 1.7;
}
.picked-skill-tx { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
/* 技能图标统一装进小圆底：内置能力（el-icon）与插件 emoji 都走这一层 ——
   裸字形在 chip 里忽大忽小、还贴边，圆底既统一尺寸也与消息气泡里的标签同款 */
.picked-skill-ic {
  display: inline-flex; align-items: center; justify-content: center; flex: none;
  width: 16px; height: 16px; border-radius: 50%;
  background: color-mix(in srgb, var(--dc-primary) 16%, transparent);
  color: var(--dc-primary); font-size: 11px; line-height: 1;
}
.picked-skill-ic > svg { width: 10px; height: 10px; }
.picked-skill-x { border-radius: 50%; cursor: pointer; transition: background .15s ease, color .15s ease; }
.picked-skill-x:hover { background: var(--dc-primary); color: #fff; }

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
/* ===== 底栏 chips 小屏压缩（fitComposerBar 按级标 .icon-only）=====
   收成图标：文字隐藏（Element 自动包装的外层 span 是内部节点，scoped 要 :deep 穿透）、
   对称 padding 让图标居中；悬浮 v-fast-tip 快速提示。条目本身不可收缩，溢出才触发。 */
.composer-bar > * { flex-shrink: 0; }
.composer-bar .icon-only .composer-ctx-tx,
.composer-bar .icon-only .composer-pick-tx,
.composer-bar .icon-only .skill-btn-tx { display: none; }
/* 收窄 padding：.icon-only 可能在外层 el-dropdown 根 div 上（四个下拉），
   也可能直接加在按钮自身（深度思考 / 记事，它们的 data-g 就在 button 上），两种都要盖住 */
.composer-bar .icon-only .composer-ctx,
.composer-bar .icon-only .composer-pick,
.composer-bar .icon-only .skill-btn,
.composer-bar .composer-ctx.icon-only,
.composer-bar .composer-pick.icon-only,
.composer-bar .skill-btn.icon-only { padding: 0 8px; max-width: none; }
.composer-bar .icon-only .composer-ctx :deep(span),
.composer-bar .icon-only .composer-pick :deep(span) { display: none; }
.composer-bar .icon-only .composer-ctx :deep(.el-icon:first-child),
.composer-bar .icon-only .composer-pick :deep(.el-icon:first-child) { margin: 0; }



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
.welcome-title { font-size: 14px; font-weight: 700; color: var(--dc-text); }
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
/* 用量脚注的样式在 common/AiUsageNote.vue（与 SQL 查询的 AI 弹窗共用一套） */
/* 气泡 + 下方操作条：外层这一列负责对齐（用户靠右、助手靠左），
   气泡本身不再吃 100% 宽度 —— 长回答最多 820px，短句跟着内容走 */
.msg-main {
  display: flex; flex-direction: column; gap: 3px;
  min-width: 0; flex: 0 1 auto;
  max-width: min(820px, calc(100% - 40px));
}
.msg.assistant .msg-main { align-items: flex-start; }
.msg.user .msg-main { align-items: flex-end; }
.msg-bubble {
  position: relative; min-width: 0;
  max-width: 100%;
  /* 单行消息的高度一路收：11px 内边距 + 1.8 行高 = 47px → 8px/1.7 = 40px（还是显高）
     → 6px 内边距 + 1.55 行高 ≈ 34px，刚好跟 28px 的头像对得上，也不至于挤到文字。
     四个角统一 12px 圆角（原来贴着头像那一角是 4px 直角）。 */
  padding: 6px 12px; border-radius: 12px;
  font-size: 14px; line-height: 1.55;
  transition: box-shadow .18s ease, border-color .18s ease;
}
/* 助手的回答给足呼吸感：行距放宽、上下留白多一点（贴边的长文看着很闷） */
.msg.assistant .msg-bubble { padding: 10px 14px; line-height: 1.7; }
/* 用户气泡：主色与面板底按比例混合出的柔和蓝（不透明，保证箭头同色无缝），
   比纯主色实底更内敛；不支持 color-mix 时回退为半透明主色 */
.msg.user { --user-bubble-bg: var(--dc-primary-soft); }
@supports (background: color-mix(in srgb, #fff, #000)) {
  .msg.user { --user-bubble-bg: color-mix(in srgb, var(--dc-primary) 26%, var(--dc-bg-card)); }
}
.msg.user .msg-bubble { background: var(--user-bubble-bg); border: none; color: var(--dc-text); }
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
  box-shadow: var(--ai-bubble-shadow, none);
}
/* ---- 微信风格小箭头 ---- */
.msg.assistant .msg-bubble::before {
  content: ''; position: absolute; top: 9px; left: -8px;
  width: 0; height: 0; border-style: solid;
  border-width: 7px 8px 7px 0;
  border-color: transparent var(--dc-border) transparent transparent;
}
.msg.assistant .msg-bubble::after {
  content: ''; position: absolute; top: 10px; left: -6px;
  width: 0; height: 0; border-style: solid;
  border-width: 6px 7px 6px 0;
  border-color: transparent var(--ai-bubble-bg) transparent transparent;
}
.msg.user .msg-bubble::before {
  content: ''; position: absolute; top: 10px; right: -6px;
  width: 0; height: 0; border-style: solid;
  border-width: 6px 0 6px 7px;
  border-color: transparent transparent transparent var(--user-bubble-bg);
}

.msg-text { white-space: pre-wrap; word-break: break-word; }
/* 用户气泡里的技能标签：做成「小徽章」—— 不透明的药丸底 + 主色细边 + 图标圆底。
   原来是在主色气泡上叠一层半透明白，深浅两种主题下都发灰、跟气泡糊成一块，
   用户看不出「这是个技能标记」；改成反衬卡片底后，一眼是贴在气泡上的标签。 */
.msg-skill-tag {
  display: inline-flex; align-items: center; gap: 5px;
  margin-right: 7px; padding: 1px 9px 1px 2px; border-radius: 999px;
  background: color-mix(in srgb, var(--dc-bg-card) 80%, transparent);
  border: 1px solid color-mix(in srgb, var(--dc-primary) 40%, transparent);
  color: var(--dc-text);
  font-size: 11.5px; font-weight: 500; letter-spacing: .2px;
  line-height: 1.65; vertical-align: 1px; white-space: nowrap;
  box-shadow: 0 1px 2px rgba(15, 23, 42, .14);
  backdrop-filter: blur(3px);
}
/* 不支持 color-mix 的旧内核：退回不透明卡片底 + 现成的主色描边变量 */
@supports not (background: color-mix(in srgb, #fff, #000)) {
  .msg-skill-tag { background: var(--dc-bg-card); border-color: var(--dc-primary-glow); }
}
/* 图标圆底：插件 emoji 与内置图标都收进 15px 圆里，跟 chip 的图标同一视觉 */
.msg-skill-tag .msg-skill-ic {
  display: inline-flex; align-items: center; justify-content: center; flex: none;
  width: 15px; height: 15px; border-radius: 50%;
  background: color-mix(in srgb, var(--dc-primary) 16%, transparent);
  color: var(--dc-primary); font-size: 10px; line-height: 1;
}
.msg-skill-tag .msg-skill-ic > svg { width: 9px; height: 9px; }
.msg-skill-tx { line-height: 1.6; }
.msg-body { white-space: pre-wrap; }
/* 用户消息：气泡**下方**一排小图标（编辑 / 复制）。
   常驻显示会把气泡撑高又抢视线，悬停出来最安静；键盘焦点也算「悬停」（focus-within）。 */
.msg-tools {
  display: flex; gap: 2px; margin-top: 1px;
  opacity: 0; transition: opacity .15s ease;
}
.msg:hover .msg-tools, .msg-tools:focus-within { opacity: 1; }
.msg-tool {
  display: inline-flex; align-items: center; justify-content: center;
  width: 22px; height: 22px; padding: 0; border: none; border-radius: 6px;
  background: transparent; color: var(--dc-text-dim); cursor: pointer;
  transition: background .15s ease, color .15s ease;
}
.msg-tool:hover:not(:disabled) { background: var(--ai-inset-bg, var(--dc-bg-soft)); color: var(--dc-text); }
.msg-tool:disabled { opacity: .45; cursor: not-allowed; }
.msg-tool .el-icon { font-size: 13px; }
/* 编辑态：输入框 + 取消 / 重新发送。
   原来框只有一句提问那么大，改长问题要滚 —— 给一个下限宽度、更松的行高 */
.msg-edit { display: flex; flex-direction: column; gap: 8px; width: 100%; min-width: min(460px, 64vw); }
.msg-edit-input :deep(.el-textarea__inner) {
  font-size: 13.5px; line-height: 1.7; padding: 8px 10px;
}
.msg-edit-acts { display: flex; justify-content: flex-end; gap: 6px; }
/* 消息内代码块：底色比气泡深一档（--ai-inset-bg），靠底色而不是边框拉开层次，
   去掉描边后块内更干净、也不与气泡边框「线条打架」。
   注意 v-html 渲染出来的 <pre> 没有 scoped 属性，必须用 :deep() 才能命中 */
.msg.assistant .ai-markdown :deep(pre) {
  background: var(--ai-inset-bg);
  border-color: transparent;
  border-radius: 8px;
  line-height: 1.65;
}
/* 等待气泡：**要看得见**（用户拍图「气泡看不到了」）。
   走过两版弯路，两头都要避开：
   - 14px padding 的空白框 → 像另一条完整回答，用户说「空框很难看」；
   - 透明无底无框 → 只剩头像加三个点飘着，用户说「气泡看不到了」。
   现在：沿用普通回答的卡片底 + 圆角 + 小箭头（跟上下气泡连成一条视觉线），
   内边距收到 7px 让整块只有一行高，内容固定是「三点 + 正在思考…」——不是空框。 */
.typing {
  display: flex; align-items: center; gap: 5px;
  white-space: nowrap;
}
.msg.assistant.is-typing .msg-bubble,
.msg.assistant .msg-bubble.typing {
  padding: 7px 12px; min-height: 0; line-height: 1.5;
}
.typing .wait-tx { font-size: 12.5px; color: var(--dc-text-dim); }
.typing .dot { width: 6px; height: 6px; border-radius: 8px; background: var(--dc-text-dim); animation: dc-blink 1.2s infinite; }
.typing .dot:nth-child(2) { animation-delay: .2s; }
.typing .dot:nth-child(3) { animation-delay: .4s; }
.typing .wait { margin-left: 2px; font-size: 12px; color: var(--dc-text-dim); }
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
/* 三个点现在跟在文字后面：children 是 [.txt, .dot, .dot, .dot]，
   所以延迟挂在第 3、4 个孩子上（第 2 个是第一个点，不延迟） */
.streaming-hint .dot:nth-child(3) { animation-delay: .2s; }
.streaming-hint .dot:nth-child(4) { animation-delay: .4s; }
.streaming-hint .txt { margin-right: 1px; font-size: 12px; color: var(--dc-text-dim); }
/* 停止：贴右侧、胶囊形、**淡红**（这是一个"中断"动作，红色比灰色更说明问题）。
   日常是低饱和红（10% 底 + 45% 边），悬停才变实心红 + 白字 —— 不刺眼但不会找不到 */
.sh-stop {
  margin-left: auto;
  display: inline-flex; align-items: center; gap: 5px;
  height: 22px; padding: 0 10px;
  border: 1px solid rgba(214, 69, 69, .45); border-radius: 999px;
  background: rgba(214, 69, 69, .10); color: #d64545;
  font-size: 11.5px; line-height: 1; cursor: pointer;
  transition: color .15s ease, border-color .15s ease, background .15s ease;
}
.sh-stop:hover { color: #fff; border-color: #d64545; background: #d64545; }
/* 停止符号用实心方块（与输入框那个停止键同一语言），不额外引图标 */
.sh-stop-sq { width: 6px; height: 6px; border-radius: 1.5px; background: currentColor; }
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
/* 问数回答：卡片内容有自己的排版，气泡内不再额外加边距 */
.msg-ask { margin: -2px 0; }
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
/* 插件技能的 emoji 图标：和 Element 线性图标同一格，别让 emoji 自身的行高把这一行撑歪 */
.slash-ic.slash-emoji {
  display: inline-flex; align-items: center; justify-content: center;
  width: 14px; height: 14px; font-size: 12px; line-height: 1;
}
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
  /* 极窄（如 AI 列只剩 ~300px）时全部收成图标也放不下：允许横向滚动兜底，不换行不丢入口 */
  overflow-x: auto; scrollbar-width: none;
}
.composer-bar::-webkit-scrollbar { display: none; }
.composer-param { flex-shrink: 0; width: 150px; }
.composer-param :deep(.el-input__wrapper) {
  background: var(--dc-bg-deep); border-radius: 8px;
  box-shadow: 0 0 0 1px var(--dc-border) inset; min-height: 28px;
}
.composer-param :deep(.el-input__inner) { font-size: 13px; color: var(--dc-text); }
/* 固定发送图标按钮：与工具条其它控件同高（28px），圆形、图标居中 */
.composer-run { margin-left: auto; flex-shrink: 0; border-radius: 8px; width: 28px; height: 28px; padding: 0; }
.composer-run :deep(.el-icon) { font-size: 15px; }
/* 生成中的停止态：实心正红 + 白色方块（不是蓝色，也不依赖主题变量有没有定义）。
   !important：element-plus 的 .el-button--primary 用 CSS 变量铺底色，主题里变量会被改写，
   不加保险会被它按加载顺序盖回去（真机出现过计算值变透明） */
.composer-run.is-stop {
  background: #d64545 !important; border-color: #d64545 !important;
}
.composer-run.is-stop:hover { background: #e05a5a !important; border-color: #e05a5a !important; }
.composer-run.is-stop:active { background: #c23a3a !important; border-color: #c23a3a !important; }
/* 点击后按钮处于 :focus：element-plus 的焦点光环（蓝色 box-shadow/outline）会套在
   停止键外面 —— 看起来「大一圈还发蓝」。两个按钮本来都是 28x28，光环才是元凶 */
.composer-run:focus-visible { outline: none; }
.composer-run:focus { box-shadow: none !important; }
.composer-run.is-stop:focus, .composer-run.is-stop:focus-visible { box-shadow: none !important; outline: none; }
.composer-run .stop-sq {
  display: inline-block; width: 10px; height: 10px;
  border-radius: 2px; background: #fff;
}
/* 右侧操作组：语音输入 + 发送整体靠右。
   position:sticky + right:0 —— 极窄面板下左侧 chips 即使全收成图标仍溢出时，
   overflow-x:auto 会把发送组滚出可视区（滚动条又是隐藏的），用户就看不到发送键了。
   sticky 让它永远贴在滚动容器右缘；背景必须是 composer-box 的底色，
   否则横向滚动时下面的 chip 会从它底下透出来。 */
.composer-right {
  margin-left: auto; display: inline-flex; align-items: center; gap: 8px; flex-shrink: 0;
  position: sticky; right: 0; z-index: 2;
  background: var(--dc-bg-input);
  padding-left: 8px;
  border-radius: 8px;
}
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

/* ===== 深度思考：一条只往下长的时间线（思考灰、工具 chip、结论正常） ===== */
.ai-timeline { display: flex; flex-direction: column; gap: 10px; }
.ai-timeline > .tl-text { margin: 0; }
/* 步骤块：一步一个圆角卡（本步的叙述 + 本步的工具），块与块之间留空 ——
   平铺时 ①的结论、②的标题、中间的工具全贴在一起，读起来像「两步跑一起去了」。 */
.tl-step {
  display: flex; flex-direction: column; gap: 6px;
  padding: 9px 11px;
  border-radius: 12px;
  background: var(--ai-inset-bg, var(--dc-bg-soft));
  border: 1px solid var(--dc-border-soft, var(--dc-border));
}
/* 变更单 / 分隔线自己一块，不加底色（卡片本来就有自己的框） */
.tl-step.is-solo { padding: 0; background: transparent; border: none; }
/* 思考文字：块底已经给了层次，这里只把字压暗，不再画左侧竖条（用户要求去掉） */
.tl-think {
  align-self: stretch;
  font-size: 12.5px; line-height: 1.7; color: var(--dc-text-mid);
  word-break: break-word;
  max-height: 320px; overflow: auto;
}
/* v-html 进来的段落（scoped 样式要用 :deep 才够得着） */
.tl-think :deep(> :first-child) { margin-top: 0; }
.tl-think :deep(> :last-child) { margin-bottom: 0; }
.tl-think :deep(p) { margin: .35em 0; }
.tl-think :deep(h1), .tl-think :deep(h2), .tl-think :deep(h3), .tl-think :deep(h4) {
  font-size: 13px; margin: .6em 0 .25em;
}
/* 灰底里嵌代码块 / 表格：底色再压一点，避免和思考块的底糊在一起 */
.tl-think :deep(pre) { margin: .4em 0; }
.tl-think :deep(table) { font-size: 12px; margin: .5em 0; }
.tl-think :deep(li) { margin: .15em 0; }
/* 工具调用：紧凑的胶囊，参数允许折成两行（原来单行省略，SQL 被截得看不出查了什么）。
   用 grid 而不是 flex-wrap：耗时要始终钉在右上角，不能被长参数挤到第二行去。 */
.tl-tool-wrap {
  align-self: flex-start; max-width: 100%; min-width: 0;
  border-radius: 9px;
  /* 工具行在步骤卡里：透明底 + 细描边，和卡的底拉开一层，又不会多出一块深色 */
  background: transparent;
  border: 1px solid var(--dc-border-soft, var(--dc-border));
}
.tl-tool-wrap.is-open {
  border-color: var(--dc-border);
  background: var(--dc-bg-card);
}
.tl-tool {
  /* 抽出后只剩 4 格：箭头 / 图标 / 工具名 / 耗时（原来的参数摘要已进展开体） */
  display: grid; grid-template-columns: auto auto auto minmax(0, 1fr);
  align-items: center; column-gap: 6px; row-gap: 2px;
  width: 100%; max-width: 100%;
  padding: 5px 10px; border-radius: 10px;
  border: none; background: transparent; text-align: left;
  font-family: inherit;
  font-size: 12px; color: var(--dc-text-dim); line-height: 1.5;
  cursor: pointer;
}
.tl-tool:hover { background: var(--dc-primary-wash); }
.tl-tool-caret { color: var(--dc-text-weak); font-size: 11px; transition: transform .15s ease; }
.tl-tool-wrap.is-open .tl-tool { color: var(--dc-text-mid); }
.tl-tool-wrap.is-open .tl-tool-caret { color: var(--dc-primary); }
.tl-tool .el-icon { color: var(--dc-purple); font-size: 13px; }
.tl-tool-name { color: var(--dc-link); font-weight: 600; white-space: nowrap; }
/* 收起态只有「工具名 + 耗时」：表名 / SQL 摘要一律进展开体（用户：后面只留时间就好） */
.tl-tool-ms { justify-self: end; font-size: 11px; color: var(--dc-text-dim); white-space: nowrap; }
/* 正在跑：描边提亮 + 转圈，让「它在动」看得见 */
.tl-tool-wrap.is-run { border-color: var(--dc-primary); background: var(--dc-primary-wash); }
.tl-tool-wrap.is-bad .tl-tool-name { color: var(--dc-danger, #d64545); }
/* 展开体：与胶囊连成一张卡（上边一条细线分隔），内容样式在 AiToolDetail.vue */
.tl-tool-detail {
  border-top: 1px solid var(--dc-border);
  min-width: 0;
}
/* 续跑分隔：只有一条细线（用户明确说别显示「接着做」三个字） */
.tl-sep {
  height: 1px; margin: 5px 0; align-self: stretch;
  background: var(--dc-border); opacity: .75;
}
.is-spin { animation: aiSpin 1s linear infinite; }
@keyframes aiSpin { to { transform: rotate(360deg); } }
/* 等审批：出单后停在这里，点「允许 / 拒绝」才继续。
   做成一个虚线小胶囊（而不是一行裸字 + 转圈），一眼看出「这是等我操作」而不是报错 */
.msg-await {
  display: inline-flex; align-items: center; gap: 6px;
  margin-top: 10px; padding: 3px 10px; border-radius: 999px;
  background: rgba(209, 154, 0, .12);
  border: 1px dashed rgba(209, 154, 0, .45);
  font-size: 12px; color: var(--dc-warn, #b8860b);
}
.msg-await .el-icon { animation: aiSpin 2s linear infinite; }

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

/* ---------------- AI 同事：工具条入口 / 变更单卡片 / 待办与记忆抽屉 ---------------- */

/* 「深挖 / 同事」两个入口：与「上下文」按钮同一副骨架，开启态用主色描边 */
.mate-btn { gap: 4px; }
.mate-btn.open {
  border-color: var(--dc-primary);
  color: var(--dc-primary);
  background: var(--dc-primary-wash);
}
.mate-badge {
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: 8px;
  background: var(--dc-danger);
  color: #fff;
  font-size: 11px;
  line-height: 16px;
  text-align: center;
}

/* 变更单卡片的样式在 AiChangeCard.vue（对话时间线里就地渲染、记录抽屉复用同一张卡） */
.msg-changes { margin-top: 8px; display: flex; flex-direction: column; gap: 8px; }
/* 记录页里的定位路径 chip：与聚焦表 tag 同一副长相（描边小圆角），一眼看出是哪张表 */
.rec-path {
  display: inline-flex; align-items: center; max-width: 100%;
  margin-top: 0; padding: 0 6px; height: 19px; border-radius: 5px;
  border: 1px solid var(--dc-border); background: var(--dc-bg-soft);
  font-size: 11px; line-height: 19px; color: var(--dc-text-mid);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}

/* 服务端提示条（预算降级等）：比正文弱一档，但必须看得见 */
.msg-notice {
  margin-top: 8px;
  padding: 6px 8px;
  border-radius: 4px;
  background: rgba(209, 154, 0, .12);
  color: #a87900;
  font-size: 12px;
  line-height: 1.6;
}

/* 抽屉内部（记事：待办 + 记忆）
   两栏统一一套卡片观感：8px 圆角、hover 提亮、右侧动作只在悬浮时变清晰 ——
   原来只有 1px 描边且卡片间距 8px，一堆待办堆起来边界发糊。 */
/* 记事抽屉：默认 600px（460 太窄 —— 「连接.库.模式.表」和一行 SQL 全在折行），
   窄窗口下按视口收，别整屏铺满 */
.mate-drawer { max-width: 92vw; }
.mate-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.mate-body { height: 100%; }
.mate-empty { padding: 24px 4px; text-align: center; font-size: 12px; color: var(--dc-text-weak); }
/* 加载失败提示：与空态明显不同（左侧红条 + 可点重试），避免「失败」被读成「没有」 */
.mate-error {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
  padding: 6px 10px;
  border: 1px solid var(--dc-danger-soft, #f3c9c9);
  border-left: 3px solid var(--dc-danger, #d64545);
  border-radius: 6px;
  background: var(--dc-danger-wash, rgba(214, 69, 69, .06));
  font-size: 12px;
  color: var(--dc-danger, #d64545);
}
.mate-error-txt { flex: 1; min-width: 0; }
/* 记录页签：AI 对库的每次增删改一条，等宽 SQL 折行显示，动作在卡片底部 */
.rec-card {
  margin-bottom: 10px;
  padding: 10px 12px;
  border: 1px solid var(--dc-border-soft);
  border-left: 3px solid var(--dc-primary);
  border-radius: 8px;
  background: var(--dc-bg-card);
  transition: background .15s ease, border-color .15s ease;
}
.rec-card:hover { background: var(--dc-bg-soft); }
.rec-card.is-medium { border-left-color: #d19a00; }
.rec-card.is-high { border-left-color: var(--dc-danger); }
.rec-head { display: flex; align-items: center; gap: 8px; }
.rec-risk {
  flex: none; padding: 0 6px; border-radius: 4px;
  font-size: 11px; line-height: 18px;
  background: var(--dc-primary-soft); color: var(--dc-primary);
}
.rec-card.is-medium .rec-risk { background: rgba(209, 154, 0, .14); color: #a87900; }
.rec-card.is-high .rec-risk { background: rgba(220, 60, 60, .14); color: var(--dc-danger); }
.rec-title { flex: 1; min-width: 0; font-size: 12.5px; color: var(--dc-text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.rec-state { flex: none; font-size: 11px; color: var(--dc-text-weak); }
.rec-state.st-applied { color: var(--dc-success, #67c23a); }
.rec-state.st-failed { color: var(--dc-danger); }
.rec-state.st-undone { color: var(--dc-text-dim); }
.rec-sql {
  margin: 6px 0 0; padding: 6px 8px; border-radius: 4px;
  background: var(--dc-bg-deep); color: var(--dc-text-mid);
  font-size: 12px; white-space: pre-wrap; word-break: break-all;
  max-height: 132px; overflow: auto;
}
.rec-meta { display: flex; align-items: center; gap: 8px; margin-top: 6px; font-size: 11px; color: var(--dc-text-dim); }
.rec-time { margin-left: auto; font-variant-numeric: tabular-nums; }
.rec-err { margin-top: 6px; font-size: 11px; line-height: 1.6; color: var(--dc-danger); word-break: break-all; }
.rec-acts { display: flex; align-items: center; gap: 8px; margin-top: 8px; }
.rec-note { font-size: 11px; color: var(--dc-text-dim); }
/* 记录工具条：条数左、清空右；删除按钮在每条卡片右侧，别和「允许」挨在一起误点 */
.rec-bar { display: flex; align-items: center; gap: 8px; margin: 0 0 8px; }
.rec-count { font-size: 12px; color: var(--dc-text-weak); }
.rec-clear, .rec-del { margin-left: auto; }
.rec-jobs { margin-top: 14px; }
.rec-jobs-title { margin-bottom: 8px; font-size: 12px; font-weight: 600; color: var(--dc-text-mid); }
.todo-card {
  margin-bottom: 10px;
  padding: 10px 12px;
  border: 1px solid var(--dc-border-soft);
  border-left: 3px solid var(--dc-border);
  border-radius: 8px;
  background: var(--dc-bg-card);
  transition: background .15s ease, border-color .15s ease;
}
.todo-card:hover { background: var(--dc-bg-soft); }
.todo-card.is-alert { border-left-color: var(--dc-danger); }
.todo-card.is-waiting { border-left-color: #d19a00; }
.todo-card.is-done { border-left-color: var(--dc-success, #67c23a); }
.todo-card.is-dismissed { opacity: .55; }
.todo-head { display: flex; align-items: flex-start; gap: 8px; }
/* 状态做成胶囊；颜色跟左边那条色条同一套语义 */
.todo-status {
  flex: none; margin-top: 1px; font-size: 11px; line-height: 18px;
  padding: 0 7px; border-radius: 999px;
  background: var(--dc-bg-soft); color: var(--dc-text-mid);
}
.todo-card.is-alert .todo-status { background: rgba(220, 60, 60, .14); color: var(--dc-danger); }
.todo-card.is-waiting .todo-status { background: rgba(209, 154, 0, .16); color: #d19a00; }
/* 标题最多两行：待办标题常常是一整句问题，单行省略后看不出问的是什么 */
.todo-title {
  flex: 1; min-width: 0; font-size: 12.5px; line-height: 1.6; color: var(--dc-text);
  display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;
}
.todo-result {
  margin-top: 6px;
  max-height: 132px;
  overflow: auto;
  font-size: 12px;
  line-height: 1.65;
  color: var(--dc-text-mid);
  white-space: pre-wrap;
  word-break: break-word;
}
.todo-acts { margin-top: 6px; display: flex; justify-content: flex-end; gap: 4px; }
.todo-acts :deep(.el-button) { margin-left: 0; }
.mem-add { display: flex; gap: 8px; margin-bottom: 12px; }
.mem-add :deep(.el-input) { flex: 1; min-width: 0; }
/* 作用范围：最多显示「连接名 · 库名」，太长就靠省略号（全文在 title 上） */
.mem-scope-sel { flex: none; width: 164px; }
.mem-scope-sel :deep(.el-select__selected-item) { overflow: hidden; text-overflow: ellipsis; }
/* 下拉项：类型 logo + 名字，名字过长走省略号（悬浮有原生 title 兜底） */
.mem-opt { display: flex; align-items: center; gap: 7px; min-width: 0; }
.mem-opt-ic { color: var(--dc-text-mid); }
.mem-opt-tx { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
/* 分组头：视觉上是分组标题，行为上是「展开/合并」开关（禁用项 + 捕获阶段点击）。
   height/line-height 必须一起改：Element 的选项是固定 34px 行高，只改 height 会留下
   一段空白（内层仍按 34px 撑开），看起来就像每个组头下面挖了个坑。 */
.mem-scope-popper .el-select-dropdown__item.mem-grp-hd,
.mem-scope-popper .el-select-dropdown__item.mem-grp-hd.is-disabled {
  height: 28px; min-height: 28px; line-height: 20px; padding: 4px 12px; cursor: pointer;
  color: var(--dc-text-dim); background: transparent; opacity: 1;
}
.mem-scope-popper .el-select-dropdown__item.mem-grp-hd:hover { background: var(--dc-bg-soft); }
.mem-grp { width: 100%; height: 20px; line-height: 20px; }
.mem-grp-caret { flex: none; color: var(--dc-text-weak); transition: transform .15s ease; }
.mem-grp-caret.is-open { transform: rotate(90deg); }
.mem-grp-n { margin-left: auto; font-size: 11px; color: var(--dc-text-weak); }
/* 一行卡：事实（可省略）+ 连接 + 日期 + 删除，全部垂直居中对齐 */
.mem-card {
  display: flex; align-items: center; gap: 10px;
  margin-bottom: 8px; padding: 6px 8px 6px 14px;
  border: 1px solid var(--dc-border-soft); border-radius: 10px; background: var(--dc-bg-card);
  transition: background .15s ease, border-color .15s ease;
}
.mem-card:hover { border-color: var(--dc-border); background: var(--dc-bg-soft); }
/* 事实正文：占满剩余宽度，超长走省略号（全文在 title 悬浮里） */
.mem-fact {
  flex: 1 1 auto; min-width: 0;
  font-size: 13px; color: var(--dc-text); line-height: 1.6;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
/* 元信息：不再另起一行（原来还带一条分割线，一行卡里全去掉） */
.mem-meta {
  flex: none; display: flex; align-items: center; gap: 10px;
  font-size: 11px; color: var(--dc-text-weak);
}
.mem-date { flex: none; font-variant-numeric: tabular-nums; letter-spacing: .2px; }
/* 删除：红色垃圾桶。平时半透明，悬浮卡片才全亮 —— 记忆是「长期保留」的东西，
   不该有个随时抢注意力的按钮；但危险语义必须一眼看出来，所以用红不用灰 */
.mem-del {
  flex: none; display: inline-flex; align-items: center; justify-content: center;
  width: 24px; height: 24px; margin-left: 2px;
  border: none; border-radius: 6px; background: transparent;
  color: var(--dc-danger); cursor: pointer; opacity: .55;
  transition: opacity .15s ease, background .15s ease;
}
.mem-card:hover .mem-del { opacity: 1; }
.mem-del:hover { background: rgba(220, 60, 60, .14); }
/* 作用范围：一行卡里它不该抢正文的宽度，给个上限再省略（全文在 title 上） */
.mem-scope {
  flex: none; max-width: 170px; min-width: 0;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
/* 连接已被删：红字说清楚，别让用户以为这条还在生效 */
.mem-scope.is-gone { color: var(--dc-danger); }
.mem-fix {
  flex: none; padding: 0 2px; border: none; background: transparent;
  font-size: 11px; color: var(--dc-primary); cursor: pointer;
}
.mem-fix:hover { text-decoration: underline; }
</style>
