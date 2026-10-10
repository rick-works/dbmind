<template>
  <el-dialog
    v-model="visible"
    width="min(1200px, 96vw)"
    :close-on-click-modal="true"
    align-center
    class="settings-dialog"
    destroy-on-close
  >
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Operation /></el-icon></span>
        <span>{{ $t('settings.title') }}</span>
      </div>
    </template>
    <div class="settings-body">
      <!-- 左侧 Tab 导航 -->
      <div class="settings-tabs">
        <div
          v-for="t in tabs"
          :key="t.key"
          class="settings-tab"
          :class="{ active: activeTab === t.key }"
          @click="activeTab = t.key"
        >
          <component :is="t.icon" class="tab-icon" />
          <!-- 页签文案走字典：写死中文的话切英文时这一列整条不会变 -->
          <span class="tab-label">{{ $t(t.i18nKey) }}</span>
        </div>
      </div>

      <!-- 右侧内容区 -->
      <div class="settings-content">
        <!-- 0. 通用（界面语言） -->
        <div v-show="activeTab === 'general'" class="settings-panel">
          <div class="panel-title">{{ $t('settings.language.title') }}</div>
          <div class="panel-desc">{{ $t('settings.language.desc') }}</div>

          <!-- 语言名**故意不翻译**：每个选项用它自己的语言写自己
               （简体中文 / English），否则切到英文后中文选项也跟着变，
               不懂当前语言的人反而找不到自己要选哪个。 -->
          <div class="theme-options">
            <div
              v-for="opt in localeOptions"
              :key="opt.code"
              class="theme-opt"
              :class="{ active: locale === opt.code }"
              @click="setLocale(opt.code)"
            >
              <div class="theme-opt-info">
                <div class="theme-opt-name">{{ opt.label }}</div>
                <div class="theme-opt-desc">{{ opt.enLabel }}</div>
              </div>
              <el-icon v-if="locale === opt.code" class="theme-opt-check"><CircleCheck /></el-icon>
            </div>
          </div>

          <div class="theme-hint">
            <el-icon :size="13"><InfoFilled /></el-icon>{{ $t('settings.language.partialHint') }}
          </div>

          <!-- 顶栏菜单：选择顶栏显示哪些功能入口、调整顺序（持久化到 dbmind.db） -->
          <div class="panel-title topmenu-title">{{ $t('settings.topMenu.title') }}</div>
          <div class="panel-desc">{{ $t('settings.topMenu.desc') }}</div>
          <div class="topmenu-editor">
            <div class="topmenu-row" v-for="(mi, i) in topMenuAll" :key="mi.id">
              <el-checkbox :model-value="!topMenuCfg.hidden.includes(mi.id)" @change="toggleTopMenu(mi.id)" />
              <span class="topmenu-label">
                <el-icon class="topmenu-ic"><component :is="mi.icon" /></el-icon>{{ mi.label() }}
              </span>
              <span class="topmenu-ops">
                <el-button size="small" text :disabled="i === 0" @click="moveTopMenu(i, -1)"><el-icon><ArrowUp /></el-icon></el-button>
                <el-button size="small" text :disabled="i === topMenuAll.length - 1" @click="moveTopMenu(i, 1)"><el-icon><ArrowDown /></el-icon></el-button>
              </span>
            </div>
          </div>
        </div>

        <!-- 0.5 安全与会话（后端 app_settings：改安全开关与会话上限都会即时生效） -->
        <div v-show="activeTab === 'safety'" class="settings-panel">
          <div class="panel-title">{{ $t('settings.safety.title') }}</div>
          <div class="panel-desc">{{ $t('settings.safety.desc') }}</div>

          <el-form label-width="130px" label-position="left" class="ai-form">
            <el-form-item :label="$t('settings.safety.protectProduction')">
              <!-- 生产保护 = 全局禁止一切写操作，比「只读连接」（连接级，树上挂徽标）更狠一档 -->
              <el-switch v-model="safetyForm.protectProduction" />
              <div class="form-tip">{{ $t('settings.safety.protectProductionTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.safety.aiWrite')">
              <el-switch v-model="safetyForm.aiWriteEnabled" />
              <div class="form-tip">{{ $t('settings.safety.aiWriteTip') }}</div>
            </el-form-item>
            <!-- 安全确认（DELETE/DROP/TRUNCATE 前二次确认）：从「查询」页签迁来 —— 它是安全属性。
                 状态仍在 queryForm（localStorage dbmind_query），改动即时持久化 -->
            <el-form-item :label="$t('settings.query.confirmDanger')">
              <el-checkbox v-model="queryForm.confirmDanger">
                {{ $t('settings.query.confirmDangerLabel') }}
              </el-checkbox>
            </el-form-item>
            <el-form-item :label="$t('settings.safety.maxSessions')">
              <!-- 数字输入不进词典：任何语言下都是同一个数字 -->
              <el-input-number v-model="safetyForm.maxSessions" :min="0" :max="128" style="width:160px" />
              <div class="form-tip">{{ $t('settings.safety.maxSessionsTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.safety.idleRecycle')">
              <el-input-number v-model="safetyForm.idleMinutes" :min="0" :max="1440" style="width:160px" />
              <div class="form-tip">{{ $t('settings.safety.idleRecycleTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.safety.tunnelIdle')">
              <el-input-number v-model="safetyForm.tunnelIdleMinutes" :min="0" :max="1440" style="width:160px" />
              <div class="form-tip">{{ $t('settings.safety.tunnelIdleTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.safety.blockDangerous')">
              <el-switch v-model="safetyForm.blockDangerous" />
              <div class="form-tip">{{ $t('settings.safety.blockDangerousTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.safety.maxWriteRows')">
              <el-input-number v-model="safetyForm.maxWriteRows" :min="0" :max="10000000" :step="100" style="width:160px" />
              <div class="form-tip">{{ $t('settings.safety.maxWriteRowsTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.safety.legacyTls')">
              <el-switch v-model="safetyForm.allowLegacyTls" />
              <div class="form-tip">{{ $t('settings.safety.legacyTlsTip') }}</div>
            </el-form-item>
          </el-form>

          <div class="actions">
            <el-button type="primary" :icon="Check" @click="saveSafety">{{ $t('settings.safety.save') }}</el-button>
          </div>
        </div>

        <!-- 1. MCP 服务 -->
        <div v-show="activeTab === 'mcp'" class="settings-panel">
          <div class="panel-title">{{ $t('settings.mcp.title') }}</div>
          <div class="panel-desc">{{ $t('settings.mcp.desc') }}</div>

          <el-form label-width="150px" label-position="left" class="ai-form">
            <el-form-item :label="$t('settings.mcp.defaultConnection')">
              <el-select v-model="mcpForm.defaultConnection" clearable style="width:240px">
                <el-option v-for="c in connOptions" :key="c.id" :value="c.name" :label="c.name" />
              </el-select>
              <div class="form-tip">{{ $t('settings.mcp.defaultConnectionTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.mcp.maxRows')">
              <el-input-number v-model="mcpForm.maxRows" :min="1" :max="100000" :step="500" style="width:160px" />
              <div class="form-tip">{{ $t('settings.mcp.maxRowsTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.mcp.timeoutSecs')">
              <el-input-number v-model="mcpForm.timeoutSecs" :min="1" :max="600" style="width:160px" />
              <div class="form-tip">{{ $t('settings.mcp.timeoutSecsTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.mcp.toolsStructure')">
              <el-switch v-model="mcpForm.toolsStructure" />
              <div class="form-tip">{{ $t('settings.mcp.toolsStructureTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.mcp.toolsQuery')">
              <el-switch v-model="mcpForm.toolsQuery" />
              <div class="form-tip">{{ $t('settings.mcp.toolsQueryTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.mcp.toolsHistory')">
              <el-switch v-model="mcpForm.toolsHistory" />
              <div class="form-tip">{{ $t('settings.mcp.toolsHistoryTip') }}</div>
            </el-form-item>
          </el-form>

          <div class="actions">
            <el-button type="primary" :icon="Check" @click="saveMcp">{{ $t('settings.mcp.save') }}</el-button>
          </div>
        </div>

        <!-- 1. AI 服务 -->
        <div v-show="activeTab === 'ai'" class="settings-panel">
          <div class="panel-title">{{ $t('settings.ai.title') }}</div>

          <el-form label-width="110px" label-position="left" class="ai-form">
            <el-form-item :label="$t('settings.ai.enabled')">
              <el-switch v-model="form.enabled" />
            </el-form-item>
            <el-form-item :label="$t('settings.ai.privacy')">
              <el-select v-model="form.privacyMode" style="width:200px">
                <el-option value="allow" :label="$t('settings.ai.privacyAllow')" />
                <el-option value="localOnly" :label="$t('settings.ai.privacyLocal')" />
              </el-select>
              <div class="form-tip">{{ $t('settings.ai.privacyTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.ai.audit')">
              <el-switch v-model="form.auditEnabled" />
              <!-- 说明统一换行到第二行（与隐私模式等各行对齐）；tip-inline 的旧偏移已废弃 -->
              <div class="form-tip">{{ $t('settings.ai.auditTip') }}</div>
            </el-form-item>
          </el-form>

          <div class="model-tabs-wrap" style="margin-top:8px">
            <el-tabs v-model="activeModelId" type="card" closable @tab-remove="removeModelById">
              <el-tab-pane
                v-for="(m, idx) in form.models"
                :key="m.id"
                :label="m.name || $t('settings.ai.modelFallbackName', { n: idx + 1 })"
                :name="m.id"
                closable
              >
                <el-form label-width="110px" label-position="left" class="ai-form">
                  <el-form-item :label="$t('settings.ai.name')">
                    <el-input v-model="m.name" :placeholder="$t('settings.ai.namePlaceholder')" />
                  </el-form-item>
                  <el-form-item :label="$t('settings.ai.baseUrl')">
                    <el-input v-model="m.baseUrl" placeholder="https://api.deepseek.com/v1" />
                    <div class="form-tip">{{ $t('settings.ai.baseUrlTip') }}</div>
                  </el-form-item>
                  <el-form-item label="API Key">
                    <el-input v-model="m.apiKey" type="password" show-password autocomplete="new-password"
                      :placeholder="m.hasKey ? $t('settings.ai.apiKeyPlaceholderSet') : $t('settings.ai.apiKeyPlaceholderNew')" />
                  </el-form-item>
                  <el-form-item :label="$t('settings.ai.model')">
                    <el-input v-model="m.model" placeholder="deepseek-chat" />
                  </el-form-item>
                  <el-form-item :label="$t('settings.ai.embedding')">
                    <el-switch v-model="m.embedding" />
                    <div class="form-tip">
                      {{ $t('settings.ai.embeddingTip') }}
                    </div>
                  </el-form-item>
                  <!-- 后端一直消费 maxTokens（进请求体），但表单里没有输入框 —— 用户改不了。
                       留空/0 = 不限制，跟随服务端默认 -->
                  <el-form-item :label="$t('settings.ai.maxTokens')">
                    <el-input-number v-model="m.maxTokens" :min="0" :max="200000" :step="1024"
                      controls-position="right" style="width:180px" />
                    <div class="form-tip">{{ $t('settings.ai.maxTokensTip') }}</div>
                  </el-form-item>
                </el-form>
                <div class="presets">
                  <div class="preset-label">{{ $t('settings.ai.presetLabel') }}</div>
                  <div class="preset-btns">
                    <el-button size="small" v-for="p in presets" :key="p.name" :icon="Connection" @click="applyPreset(idx, p)">{{ p.name }}</el-button>
                  </div>
                </div>
              </el-tab-pane>
            </el-tabs>
          </div>

          <div class="actions" style="margin-top:12px">
            <el-button :icon="Plus" @click="addModel">{{ $t('settings.ai.addModel') }}</el-button>
            <el-button :loading="!!testingId" @click="testActiveModel">
              <el-icon style="margin-right:4px"><Connection /></el-icon>{{ $t('settings.ai.test') }}
            </el-button>
            <el-button type="primary" :loading="saving" @click="save">
              <el-icon style="margin-right:4px"><Check /></el-icon>{{ $t('settings.ai.saveConfig') }}
            </el-button>
          </div>

          <!-- 用量统计：它属于「模型服务用了多少」，放在这里比放在知识库里合适 -->
          <div class="kb-card" style="margin-top:18px">
            <div class="kb-card-head">
              <span class="kb-card-title">{{ $t('settings.ai.usageTitle') }}</span>
              <span class="kb-count" style="margin-left:auto">{{ $t('settings.ai.usageRange', { n: usageRange }) }}</span>
            </div>
            <div class="kb-usage">
              <div class="kb-stat">
                <span class="kb-stat-n">{{ usageStats.calls }}</span>
                <span class="kb-stat-l">{{ $t('settings.ai.usageCalls') }}</span>
              </div>
              <div class="kb-stat">
                <span class="kb-stat-n">{{ usageStats.activeDays }}</span>
                <span class="kb-stat-l">{{ $t('settings.ai.usageDays') }}</span>
              </div>
              <div class="kb-stat">
                <span class="kb-stat-n">{{ usageStats.perDay }}</span>
                <span class="kb-stat-l">{{ $t('settings.ai.usagePerDay') }}</span>
              </div>
            </div>

            <!-- 按模型汇总（token）：模型商回 usage 才有 token 数，不回就只有次数 ——
                 如实写"未统计 token"，**不补 0**（否则看起来像"这个模型不花钱"）。 -->
            <div class="kb-models">
              <div class="kb-models-head">
                <span class="kb-models-title">{{ $t('settings.ai.byModel') }}</span>
                <span class="kb-models-note">{{ $t('settings.ai.byModelNote') }}</span>
              </div>
              <div v-if="usageModels.length" class="kb-models-list">
                <div v-for="m in usageModels" :key="m.model" class="kb-model">
                  <span class="kb-model-name" :title="m.model">{{ m.model }}</span>
                  <span class="kb-model-calls">{{ $t('settings.ai.callsCount', { n: m.calls }) }}</span>
                  <span class="kb-model-tokens"
                        :title="$t('settings.ai.tokenTitle', {
                          input: fmtTokens(m.promptTokens),
                          output: fmtTokens(m.completionTokens),
                          total: fmtTokens(m.totalTokens)
                        })">
                    {{ m.totalTokens ? $t('settings.ai.tokens', { n: fmtTokens(m.totalTokens) }) : $t('settings.ai.noTokens') }}
                  </span>
                </div>
              </div>
              <div v-else class="kb-model-empty">{{ $t('settings.ai.noModelRecords') }}</div>
            </div>
          </div>

        </div>

        <!-- 知识库已独立为顶部导航的「知识库」工作台（左栏库列表 + 文档 / 召回测试 / 分段设置），
             设置里不再重复放一份入口与实现 -->

        <!-- 主题 -->
        <div v-show="activeTab === 'theme'" class="settings-panel">
          <div class="panel-title">{{ $t('settings.theme.title') }}</div>
          <div class="panel-desc">{{ $t('settings.theme.desc') }}</div>

          <div class="theme-options">
            <div
              v-for="opt in themeOptions"
              :key="opt.mode"
              class="theme-opt"
              :class="{ active: themeForm.mode === opt.mode }"
              @click="setThemeMode(opt.mode)"
            >
              <div class="theme-prev" :class="'prev-' + opt.mode">
                <span class="pv-sidebar"></span>
                <span class="pv-main">
                  <span class="pv-line"></span>
                  <span class="pv-line short"></span>
                  <span class="pv-line"></span>
                  <span class="pv-ctl"></span>
                </span>
              </div>
              <div class="theme-opt-info">
                <div class="theme-opt-name">{{ $t(opt.i18nKey) }}</div>
                <div class="theme-opt-desc">{{ $t(opt.descKey) }}</div>
              </div>
              <el-icon v-if="themeForm.mode === opt.mode" class="theme-opt-check"><CircleCheck /></el-icon>
            </div>
          </div>

          <div v-if="themeForm.mode === 'system'" class="theme-hint">
            <el-icon :size="13"><Timer /></el-icon>{{ $t('settings.theme.followHint', { mode: systemModeText }) }}
          </div>
        </div>

        <!-- 编辑器 -->
        <div v-show="activeTab === 'editor'" class="settings-panel">
          <div class="panel-title">{{ $t('settings.editor.title') }}</div>
          <div class="panel-desc">{{ $t('settings.editor.desc') }}</div>

          <el-form label-width="110px" label-position="left" class="ai-form">
            <el-form-item :label="$t('settings.editor.fontSize')">
              <el-input-number v-model="editorForm.fontSize" :min="10" :max="20" :step="1" controls-position="right" style="width:120px" />
              <span class="unit">px</span>
            </el-form-item>
            <el-form-item :label="$t('settings.editor.gridFontSize')">
              <el-input-number v-model="editorForm.gridFontSize" :min="10" :max="20" :step="1" controls-position="right" style="width:120px" />
              <span class="unit">px</span>
            </el-form-item>
            <el-form-item :label="$t('settings.editor.gridRowHeight')">
              <el-input-number v-model="editorForm.gridRowHeight" :min="20" :max="48" :step="2" controls-position="right" style="width:120px" />
              <span class="unit">px</span>
              <div class="form-tip">{{ $t('settings.editor.gridRowHeightTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.editor.tabSize')">
              <el-select v-model="editorForm.tabSize" style="width:120px">
                <el-option :value="2" :label="$t('settings.editor.spaces', { n: 2 })" />
                <el-option :value="4" :label="$t('settings.editor.spaces', { n: 4 })" />
              </el-select>
            </el-form-item>
            <el-form-item :label="$t('settings.editor.lineNumbers')">
              <el-checkbox v-model="editorForm.lineNumbers" />
            </el-form-item>
            <!-- 自动换行 / 脚本自动保存：之前只有默认值、没有任何开关 —— 用户改不了
                 （autoSave 的逻辑却一直活着，等于一个谁也关不掉/开不上的暗开关）。
                 小地图开关已随功能下线移除 -->
            <el-form-item :label="$t('settings.editor.wordWrap')">
              <el-checkbox v-model="editorForm.wordWrap" />
            </el-form-item>
            <el-form-item :label="$t('settings.editor.autoCloseBrackets')">
              <el-checkbox v-model="editorForm.autoCloseBrackets" />
              <div class="form-tip">{{ $t('settings.editor.autoCloseBracketsTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.editor.quickSuggest')">
              <el-checkbox v-model="editorForm.quickSuggest" />
              <div class="form-tip">{{ $t('settings.editor.quickSuggestTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.editor.autoSave')">
              <el-checkbox v-model="editorForm.autoSave" />
              <div class="form-tip">{{ $t('settings.editor.autoSaveTip') }}</div>
            </el-form-item>

          </el-form>

          <div class="actions">
            <el-button type="primary" @click="saveEditor">
              <el-icon style="margin-right:4px"><Check /></el-icon>{{ $t('settings.editor.save') }}
            </el-button>
            <el-button text @click="resetEditor">{{ $t('settings.editor.reset') }}</el-button>
          </div>
        </div>

        <!-- 3. SQL 格式化 -->
        <div v-show="activeTab === 'format'" class="settings-panel">
          <div class="panel-title">{{ $t('settings.format.title') }}</div>
          <div class="fmt-grid">
            <div class="fmt-card fmt-card-wide">
              <div class="fmt-card-title">
                <el-icon :size="14"><component :is="Brush" /></el-icon>{{ $t('settings.format.globalTitle') }}
                <span class="fmt-card-sub">{{ $t('settings.format.globalSub') }}</span>
              </div>

              <div class="fmt-glob-cols">
                <div class="fmt-glob-col">
                  <div class="fmt-glob-sec">
                    <div class="fmt-glob-sec-title">{{ $t('settings.format.caseSection') }}</div>
                    <div class="fmt-glob-it">
                      <span class="fmt-glob-lb" :title="$t('settings.format.keywordTip')">{{ $t('settings.format.keyword') }}</span>
                      <el-select v-model="editorForm.sqlKeywordCase" class="fmt-glob-sel">
                        <el-option value="upper" :label="$t('settings.format.caseUpper')" />
                        <el-option value="lower" :label="$t('settings.format.caseLower')" />
                        <el-option value="preserve" :label="$t('settings.format.casePreserve')" />
                      </el-select>
                    </div>
                    <div class="fmt-glob-it">
                      <span class="fmt-glob-lb" :title="$t('settings.format.identifierTip')">{{ $t('settings.format.identifier') }}</span>
                      <el-select v-model="editorForm.sqlIdentifierCase" class="fmt-glob-sel">
                        <el-option value="upper" :label="$t('settings.format.caseUpper')" />
                        <el-option value="lower" :label="$t('settings.format.caseLower')" />
                        <el-option value="preserve" :label="$t('settings.format.casePreserve')" />
                      </el-select>
                    </div>
                    <div class="fmt-glob-it">
                      <span class="fmt-glob-lb" :title="$t('settings.format.dataTypeTip')">{{ $t('settings.format.dataType') }}</span>
                      <el-select v-model="editorForm.sqlDataTypeCase" class="fmt-glob-sel">
                        <el-option value="upper" :label="$t('settings.format.caseUpper')" />
                        <el-option value="lower" :label="$t('settings.format.caseLower')" />
                        <el-option value="preserve" :label="$t('settings.format.casePreserve')" />
                      </el-select>
                    </div>
                    <div class="fmt-glob-it">
                      <span class="fmt-glob-lb" :title="$t('settings.format.functionTip')">{{ $t('settings.format.function') }}</span>
                      <el-select v-model="editorForm.sqlFunctionCase" class="fmt-glob-sel">
                        <el-option value="upper" :label="$t('settings.format.caseUpper')" />
                        <el-option value="lower" :label="$t('settings.format.caseLower')" />
                        <el-option value="preserve" :label="$t('settings.format.casePreserve')" />
                      </el-select>
                    </div>
                  </div>

                  <div class="fmt-glob-sec">
                    <div class="fmt-glob-sec-title">{{ $t('settings.format.operatorSection') }}</div>
                    <div class="fmt-glob-it">
                      <span class="fmt-glob-lb" :title="$t('settings.format.logicalNewlineTip')">{{ $t('settings.format.logicalNewline') }}</span>
                      <el-select v-model="editorForm.sqlLogicalOperatorNewline" class="fmt-glob-sel">
                        <el-option value="before" :label="$t('settings.format.lyClause')" />
                        <el-option value="after" :label="$t('settings.format.lyAfterBreak')" />
                      </el-select>
                    </div>
                    <div class="fmt-glob-it">
                      <span class="fmt-glob-lb" :title="$t('settings.format.denseTip')">{{ $t('settings.format.dense') }}</span>
                      <span class="fmt-glob-swslot"><el-switch v-model="editorForm.sqlDenseOperators" /></span>
                    </div>
                  </div>
                </div>

                <div class="fmt-glob-col">
                  <div class="fmt-glob-sec">
                    <div class="fmt-glob-sec-title">{{ $t('settings.format.indentSection') }}</div>
                    <div class="fmt-glob-it">
                      <span class="fmt-glob-lb" :title="$t('settings.format.indentWidthTip')">{{ $t('settings.format.indentWidth') }}</span>
                      <el-input-number v-model="editorForm.sqlTabWidth" :min="1" :max="8" :disabled="editorForm.sqlUseTabs" class="fmt-glob-num" />
                    </div>
                    <div class="fmt-glob-it">
                      <span class="fmt-glob-lb">{{ $t('settings.format.useTabs') }}</span>
                      <span class="fmt-glob-swslot"><el-switch v-model="editorForm.sqlUseTabs" /></span>
                    </div>
                    <div class="fmt-glob-it">
                      <span class="fmt-glob-lb" :title="$t('settings.format.exprWidthTip')">{{ $t('settings.format.exprWidth') }}</span>
                      <el-input-number v-model="editorForm.sqlExpressionWidth" :min="20" :max="200" :step="10" class="fmt-glob-num" />
                    </div>
                  </div>

                  <div class="fmt-glob-sec">
                    <div class="fmt-glob-sec-title">{{ $t('settings.format.spacingSection') }}</div>
                    <div class="fmt-glob-it">
                      <span class="fmt-glob-lb" :title="$t('settings.format.linesBetweenTip')">{{ $t('settings.format.linesBetween') }}</span>
                      <el-select v-model="editorForm.sqlLinesBetweenQueries" class="fmt-glob-sel">
                        <el-option :value="0" :label="$t('settings.format.linesNone')" />
                        <el-option :value="1" :label="$t('settings.format.linesN', { n: 1 })" />
                        <el-option :value="2" :label="$t('settings.format.linesN', { n: 2 })" />
                        <el-option :value="3" :label="$t('settings.format.linesN', { n: 3 })" />
                      </el-select>
                    </div>
                    <div class="fmt-glob-it">
                      <span class="fmt-glob-lb" :title="$t('settings.format.semicolonTip')">{{ $t('settings.format.semicolon') }}</span>
                      <span class="fmt-glob-swslot"><el-switch v-model="editorForm.sqlNewlineBeforeSemicolon" /></span>
                    </div>
                  </div>
                </div>
              </div>
            </div>

            <div class="fmt-card fmt-card-wide">
              <div class="fmt-card-title">
                <el-icon :size="14"><component :is="Key" /></el-icon>{{ $t('settings.format.rulesTitle') }}
                <span class="fmt-card-sub">{{ $t('settings.format.rulesSub') }}</span>
              </div>

              <div class="fmt-tbl">
                <div class="fmt-tr fmt-th">
                  <span class="fmt-tc-idx">{{ $t('settings.format.colIndex') }}</span>
                  <span class="fmt-tc-kw">{{ $t('settings.format.colKeyword') }}</span>
                  <span class="fmt-tc-cs">{{ $t('settings.format.colCase') }}</span>
                  <span class="fmt-tc-ly">{{ $t('settings.format.colLayout') }}</span>
                  <span class="fmt-tc-op"></span>
                </div>
                <div v-if="!editorForm.sqlKeywordRules.length" class="fmt-tbl-empty">{{ $t('settings.format.emptyRules') }}</div>
                <div v-for="(rule, i) in editorForm.sqlKeywordRules" :key="i" class="fmt-tr">
                  <span class="fmt-tc-idx">{{ i + 1 }}</span>
                  <div class="fmt-tc-kw">
                    <el-autocomplete
                      v-model="rule.kw"
                      :fetch-suggestions="queryKwSuggestions"
                      :placeholder="$t('settings.format.kwPlaceholder')"
                    />
                  </div>
                  <el-select v-model="rule.cs" class="fmt-tc-cs" :title="$t('settings.format.csKeepTip')">
                    <el-option value="upper" :label="$t('settings.format.caseUpper')" />
                    <el-option value="lower" :label="$t('settings.format.caseLower')" />
                    <el-option value="keep" :label="$t('settings.format.casePreserve')" />
                  </el-select>
                  <el-select v-model="rule.ly" class="fmt-tc-ly">
                    <el-option value="none" :label="$t('settings.format.lyNone')" />
                    <el-option value="clause" :label="$t('settings.format.lyClause')" />
                    <el-option value="join" :label="$t('settings.format.lyJoin')" />
                    <el-option value="afterBreak" :label="$t('settings.format.lyAfterBreak')" />
                    <el-option value="afterBreakIndent" :label="$t('settings.format.lyAfterBreakIndent')" />
                    <el-option value="setop" :label="$t('settings.format.lySetop')" />
                  </el-select>
                  <el-button class="fmt-tc-op" text :icon="Delete" :title="$t('settings.format.deleteRule')" @click="removeKeywordRule(i)" />
                </div>
              </div>

              <div class="fmt-rule-add">
                <el-button type="primary" plain :icon="Plus" @click="addKeywordRule">{{ $t('settings.format.addRule') }}</el-button>
              </div>
            </div>
          </div>

          <div class="fmt-preview">
            <div class="fmt-preview-head">
              <span class="fmt-preview-title">{{ $t('settings.format.previewTitle') }}</span>
              <span class="fmt-preview-tip">{{ $t('settings.format.previewTip') }}</span>
            </div>
            <pre class="fmt-preview-code" v-html="previewSql"></pre>
          </div>

          <div class="actions">
            <el-button type="primary" :icon="Check" @click="saveEditor">{{ $t('settings.format.save') }}</el-button>
            <el-button text @click="resetEditor">{{ $t('settings.editor.reset') }}</el-button>
          </div>
        </div>

        <!-- 3. 查询 -->
        <div v-show="activeTab === 'query'" class="settings-panel">
          <div class="panel-title">{{ $t('settings.query.title') }}</div>
          <div class="panel-desc">{{ $t('settings.query.desc') }}</div>

          <el-form label-width="110px" label-position="left" class="ai-form">
            <el-form-item :label="$t('settings.query.pageSize')">
              <!-- 页大小是**纯数字**：不放进字典。放进字典就要为每种语言各维护 9 条，
                   而它们在任何语言下都是同一个数字 —— 徒增错漏面。 -->
              <el-select v-model="queryForm.pageSize" style="width:160px">
                <el-option v-for="n in [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000]" :key="n"
                           :value="n" :label="$t('settings.query.perPage', { n })" />
              </el-select>
              <div class="form-tip">{{ $t('settings.query.pageSizeTip') }}</div>
            </el-form-item>
            <!-- 「安全确认」已迁到「安全与会话」页签：它是安全属性，不该混在查询参数里 -->
            <!-- 下面几项存**后端**（app_settings）：超时/上限由执行入口读，缓存 TTL 由内核读 -->
            <el-form-item :label="$t('settings.query.timeoutSecs')">
              <el-input-number v-model="queryForm.timeoutSecs" :min="1" :max="600" style="width:160px" />
              <div class="form-tip">{{ $t('settings.query.timeoutSecsTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.query.maxRows')">
              <el-input-number v-model="queryForm.maxRows" :min="100" :max="100000" :step="500" style="width:160px" />
              <div class="form-tip">{{ $t('settings.query.maxRowsTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.query.cacheTtl')">
              <el-input-number v-model="queryForm.cacheTtlSecs" :min="30" :max="86400" style="width:160px" />
              <div class="form-tip">{{ $t('settings.query.cacheTtlTip') }}</div>
            </el-form-item>
            <!-- 审计日志不再自动清理，也没有保留上限设置：要清就去「日志」页点「清空日志」 -->
            <!-- NULL 显示样式：本地项（localStorage dbmind_query），watch 即时落盘并热生效 -->
            <el-form-item :label="$t('settings.query.nullStyle')">
              <el-select v-model="queryForm.nullStyle" style="width:160px">
                <el-option value="null" :label="$t('settings.query.nullAsNull')" />
                <el-option value="paren" :label="$t('settings.query.nullAsParen')" />
                <el-option value="blank" :label="$t('settings.query.nullAsBlank')" />
              </el-select>
              <div class="form-tip">{{ $t('settings.query.nullStyleTip') }}</div>
            </el-form-item>
          </el-form>

          <div class="actions">
            <el-button type="primary" @click="saveQuery">
              <el-icon style="margin-right:4px"><Check /></el-icon>{{ $t('settings.query.save') }}
            </el-button>
            <el-button text @click="resetQuery">{{ $t('settings.editor.reset') }}</el-button>
            <!-- 维护动作：历史与结构缓存都在无限增长/有有效期，总要有个手动出口 -->
            <el-button text :icon="Delete" @click="onClearHistory">{{ $t('settings.query.clearHistory') }}</el-button>
            <el-button text :icon="Refresh" @click="onRefreshCache">{{ $t('settings.query.refreshCache') }}</el-button>
          </div>
        </div>

        <!-- 4. 驱动管理（顶栏也有直达入口） -->
        <div v-show="activeTab === 'driver'" class="settings-panel">
          <div class="panel-title">{{ $t('settings.driver.title') }}</div>
          <div class="panel-desc">{{ $t('settings.driver.desc') }}</div>

          <el-form label-width="110px" label-position="left" class="ai-form">
            <el-form-item :label="$t('settings.driver.mirror')">
            <el-select v-model="driverForm.mirror" class="driver-mirror-select" :style="{ width: mirrorSelectWidth + 'px' }">
              <!-- 镜像商名是**专有名词**：Maven Central 哪个语言都这么写；
                   阿里云 / 华为云 / 腾讯云 在英文界面下用它们自己的英文名 -->
              <!-- 「自动」是默认：不给用户添选择题 —— 后台按网络实时挑最快的源，
                   用户设过镜像才用他设的（见后端 driver_base_candidates） -->
              <el-option value="auto" :label="$t('settings.driver.mirrorAuto')" />
              <el-option value="maven" label="Maven Central" />
              <el-option value="aliyun" :label="$t('settings.driver.mirrorAliyun')" />
              <el-option value="huawei" :label="$t('settings.driver.mirrorHuawei')" />
              <el-option value="tencent" :label="$t('settings.driver.mirrorTencent')" />
              <el-option value="custom" :label="$t('settings.driver.mirrorCustom')" />
            </el-select>
            </el-form-item>
            <el-form-item v-if="driverForm.mirror === 'custom'" :label="$t('settings.driver.customUrl')">
              <el-input v-model="driverForm.customUrl" :placeholder="$t('settings.driver.customUrlPh')"
                        style="width:420px" clearable />
              <div class="form-tip">{{ $t('settings.driver.customUrlTip') }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.driver.actualSource')">
              <div class="mirror-link">
                <a v-if="mirrorBase" :href="mirrorBase" target="_blank" rel="noopener">{{ mirrorBase }}</a>
                <!-- 自动模式没有"某一个"仓库根可显示，就说明它会怎么选 —— 空占位等于什么都没说 -->
                <span v-else>{{ $t(driverForm.mirror === 'auto' ? 'settings.driver.actualSourceAuto' : 'settings.driver.actualSourceNone') }}</span>
              </div>
            </el-form-item>
            </el-form>

            <div class="actions">
            <el-button type="primary" @click="saveDriver">
            <el-icon style="margin-right:4px"><Check /></el-icon>{{ $t('settings.driver.saveMirror') }}
            </el-button>
            <el-button @click="loadDriverList" :loading="driverLoading">
            <el-icon style="margin-right:4px"><Refresh /></el-icon>{{ $t('common.refresh') }}
            </el-button>
            <el-input v-model="driverKeyword" clearable :placeholder="$t('settings.driver.search')"
                    :prefix-icon="Search" class="drv-search" />
            <span class="drv-sum">{{ $t('settings.driver.readySummary', { ready: driverReadyCount, total: driverRows.length }) }}</span>
            </div>

            <!-- 支持的数据源 × 驱动状态：每行都能单独「下载」或「上传」。
             上传这条路是给内网 / 离线机器准备的 —— 拉不到 Maven 时唯一的出路。 -->
            <div v-for="g in driverGroups" :key="g.name" class="drv-group">
            <div class="drv-group-title">{{ g.name }}<span class="drv-group-count">{{ g.items.length }}</span></div>
            <div v-for="t in g.items" :key="t.code" class="drv-row">
            <DbLogo :type="t.code" :size="24" />
            <div class="drv-main">
              <div class="drv-name">
                <span class="drv-label">{{ t.label }}</span>
                <!-- 非关系型（NoSQL）的驱动打在宿主 jar 里，随应用分发 —— 与 SQLite 同样视为内置，
                     不给下载/上传按钮（给了也是无效操作） -->
                <el-tag v-if="t.builtin || t.category === 'NOSQL'" size="small" type="info">{{ $t('settings.driver.tagBuiltin') }}</el-tag>
                <el-tag v-else-if="t.ready" size="small" type="success">{{ $t('settings.driver.tagReady') }}</el-tag>
                <el-tag v-else size="small" type="danger">{{ $t('settings.driver.tagMissing') }}</el-tag>
              </div>
              <!-- t.tip 来自后端（getDriverTypes），前端无从翻译 —— 那是"后端文案"那一档，
                   要翻得后端按 Accept-Language 返回，见 i18n.js 头注释里的说明 -->
              <div class="drv-sub" :title="t.tip">{{ t.category === 'NOSQL' ? $t('settings.driver.tipNoSql') : t.tip }}</div>
              <!-- 下载中：进度条直接挂在行里（后端边下边报数，几十兆也能看出在动） -->
              <div v-if="driverDl && driverDl.code === t.code" class="drv-dl">
                <el-progress :percentage="Math.max(0, Math.min(100, driverDl.percent || 0))" :stroke-width="4" :show-text="false" />
                <span class="drv-dl-txt">
                  <template v-if="driverDl.total">{{ $t('settings.driver.dling', { percent: driverDl.percent || 0, done: fmtBytes(driverDl.received), total: fmtBytes(driverDl.total) }) }}</template>
                  <template v-else>{{ $t('settings.driver.dlingUnknown', { percent: driverDl.percent || 0 }) }}</template>
                </span>
              </div>
            </div>
            <div class="drv-acts">
              <template v-if="!t.builtin && t.category !== 'NOSQL'">
                <el-button size="small" :loading="driverBusy === t.code"
                           @click="downloadDriver(t)">
                  <el-icon style="margin-right:4px"><Download /></el-icon>{{ $t('settings.driver.download') }}
                </el-button>
                <el-button size="small" :loading="driverBusy === t.code + ':up'"
                           @click="pickDriverFile(t)">
                  <el-icon style="margin-right:4px"><Upload /></el-icon>{{ $t('settings.driver.upload') }}
                </el-button>
              </template>
              <el-button v-if="t.dir && t.category !== 'NOSQL'" size="small" text @click="openDriverDir(t)">{{ $t('common.openDir') }}</el-button>
            </div>
            </div>
            </div>
            <el-empty v-if="!driverRows.length" :description="$t('settings.driver.empty')" :image-size="80" />

            <!-- 隐藏的文件选择器：可多选 —— 主驱动与额外依赖（如 ClickHouse 的 slf4j）一次传完 -->
            <input ref="driverFileRef" type="file" accept=".jar" multiple
               style="display:none" @change="onDriverFilePicked" />

            </div>

        <!-- 5. 存储路径 -->
        <div v-show="activeTab === 'paths'" class="settings-panel">
          <div class="panel-title">{{ $t('settings.paths.title') }}</div>
          <div class="panel-desc">{{ $t('settings.paths.desc') }}</div>

          <div class="os-banner">
            <span class="os-tag">{{ osTagText }}</span>
            <span class="os-detail">{{ pathInfo.osLabel }}</span>
          </div>

          <el-form label-width="110px" label-position="left" class="ai-form">
            <el-form-item :label="$t('settings.paths.dataDir')">
              <el-input v-model="pathForm.dataDir" :placeholder="pathInfo.defaultDataDir">
                <template #append>
                  <el-button @click="resetDataDir">{{ $t('settings.editor.reset') }}</el-button>
                </template>
              </el-input>
              <div class="form-tip">{{ $t('settings.paths.dataDirTip') }}</div>
            </el-form-item>
          </el-form>

          <div class="actions">
            <el-button type="primary" :loading="savingPath" @click="savePaths">
              <el-icon style="margin-right:4px"><Check /></el-icon>{{ $t('settings.paths.save') }}
            </el-button>
          </div>
        </div>

        <!-- 5.5 缓存 -->
        <div v-show="activeTab === 'cache'" class="settings-panel">
          <div class="panel-title">{{ $t('settings.cache.title') }}</div>
          <div class="panel-desc">{{ $t('settings.cache.desc') }}</div>

          <div class="cache-list" v-loading="cacheLoading">
            <label v-for="item in cacheItems" :key="item.key" class="cache-item">
              <el-checkbox v-model="item.checked" />
              <span class="cache-name">{{ $t('settings.cache.items.' + item.key) }}</span>
              <span class="cache-size">{{ cacheSizeText(item) }}</span>
            </label>
            <div v-if="!cacheItems.length && !cacheLoading" class="cache-empty">{{ $t('settings.cache.empty') }}</div>
          </div>

          <div class="actions">
            <el-button type="danger" plain :loading="cacheClearing" :disabled="!checkedCacheKeys.length" @click="clearCheckedCache">
              <el-icon style="margin-right:4px"><Delete /></el-icon>{{ $t('settings.cache.clear') }}{{ checkedCacheKeys.length ? ` (${checkedCacheKeys.length})` : '' }}
            </el-button>
          </div>
        </div>

        <!-- 日志：全类型审计（查询 / 增删改 / DDL / 事务 / AI 调用），可筛可清 -->
        <div v-show="activeTab === 'logs'" class="settings-panel log-panel">
          <!-- 运行时日志级别：程序自身的诊断输出（tracing），与下面的审计日志是两回事 -->
          <div class="panel-title log-title">{{ $t('settings.logs.runtimeTitle') }}</div>
          <div class="panel-desc">{{ $t('settings.logs.runtimeDesc') }}</div>
          <div class="log-level-options">
            <div
              v-for="lv in logLevels"
              :key="lv.value"
              class="theme-opt log-opt"
              :class="{ active: logLevel === lv.value }"
              @click="pickLogLevel(lv.value)"
            >
              <div class="theme-opt-info">
                <div class="theme-opt-name">{{ lv.label }}</div>
                <div class="theme-opt-desc">{{ lv.desc }}</div>
              </div>
              <el-icon v-if="logLevel === lv.value" class="theme-opt-check"><CircleCheck /></el-icon>
            </div>
          </div>

          <div class="panel-title log-title">{{ $t('settings.logs.auditTitle') }}</div>
          <div class="panel-desc">{{ $t('settings.logs.desc') }}</div>

          <div class="log-filter-row">
            <span class="log-level-label">{{ $t('settings.logs.categoryLabel') }}</span>
            <el-select v-model="auditLevel" size="small" style="width: 168px" @change="onAuditLevelChange">
              <el-option value="all" :label="$t('settings.logs.level.all')" />
              <el-option value="query" :label="$t('settings.logs.level.query')" />
              <el-option value="write" :label="$t('settings.logs.level.write')" />
              <el-option value="ddl" :label="$t('settings.logs.level.ddl')" />
              <el-option value="tx" :label="$t('settings.logs.level.tx')" />
              <el-option value="exec" :label="$t('settings.logs.level.exec')" />
              <el-option value="ai" :label="$t('settings.logs.level.ai')" />
              <el-option value="error" :label="$t('settings.logs.level.error')" />
              <el-option value="off" :label="$t('settings.logs.level.off')" />
            </el-select>
            <el-input v-model="logQuery" size="small" style="width: 220px" clearable
                      :placeholder="$t('settings.logs.searchPh')" @keyup.enter="logPage = 1; loadLogs()" @clear="logPage = 1; loadLogs()" />
            <el-button size="small" :icon="Search" @click="loadLogs">{{ $t('settings.logs.refresh') }}</el-button>
            <el-button size="small" type="danger" plain style="margin-left: auto" @click="clearAllLogs">
              <el-icon style="margin-right:4px"><Delete /></el-icon>{{ $t('settings.logs.clear') }}
            </el-button>
          </div>

          <div class="log-list" v-loading="logLoading">
            <div v-for="(item, i) in logItems" :key="i" class="log-item">
              <span class="log-time">{{ fmtLogTime(item.createdAt) }}</span>
              <span class="log-tag" :class="'k-' + item.kind">{{ $t('settings.logs.kind.' + item.kind) }}</span>
              <span class="log-sql" :title="item.sql">{{ item.sql }}</span>
              <span class="log-conn" :title="item.connection">{{ item.connection }}</span>
              <span class="log-dur" v-if="item.durationMs">{{ item.durationMs }} ms</span>
              <span class="log-status" :class="item.status">{{ item.status }}</span>
              <!-- 复制这条 SQL：hover 行时在右侧浮现 -->
              <el-button
                class="log-copy"
                size="small"
                text
                :icon="DocumentCopy"
                :title="$t('settings.logs.copySql')"
                @click.stop="copyLogSql(item.sql)"
              />
            </div>
            <div v-if="!logItems.length && !logLoading" class="cache-empty">{{ $t('settings.logs.empty') }}</div>
          </div>

          <div class="log-pager">
            <span class="log-count">{{ $t('settings.logs.count', { n: logTotal }) }}</span>
            <el-pagination
              layout="prev, pager, next"
              size="small"
              background
              :page-size="logRows"
              :total="logTotal"
              :current-page="logPage"
              @current-change="(p) => { logPage = p; loadLogs() }"
            />
          </div>
        </div>

        <!-- 6. 通知 -->
        <div v-show="activeTab === 'notify'" class="settings-panel">
          <div class="panel-title">{{ $t('settings.notify.title') }}</div>
          <div class="panel-desc">{{ $t('settings.notify.desc') }}</div>

          <el-form label-width="110px" label-position="left" class="ai-form">
            <el-form-item :label="$t('settings.notify.group')">
              <div class="checkbox-group">
                <el-checkbox v-model="notifyForm.success">{{ $t('settings.notify.onSuccess') }}</el-checkbox>
                <el-checkbox v-model="notifyForm.autoClose">{{ $t('settings.notify.autoClose') }}</el-checkbox>
              </div>
            </el-form-item>
          </el-form>

          <div class="actions">
            <el-button type="primary" @click="saveNotify">
              <el-icon style="margin-right:4px"><Check /></el-icon>{{ $t('settings.notify.save') }}
            </el-button>
          </div>
        </div>

        <!-- 7. 快捷键 -->
        <div v-show="activeTab === 'shortcut'" class="settings-panel">
          <div class="panel-title">{{ $t('shortcut.title') }}</div>
          <div class="panel-desc">{{ $t('shortcut.desc') }}</div>

          <div class="shortcut-box">
            <div class="scb-head">
              <span class="scb-title">{{ $t('shortcut.bindings') }}</span>
              <span class="flex-spacer"></span>
              <el-button size="small" text type="primary" :disabled="!hasCustomShortcuts" @click="resetAllShortcuts">
                {{ $t('settings.editor.reset') }}
              </el-button>
            </div>
            <div v-for="g in shortcutGroups" :key="g.key" class="scb-group">
              <!-- 组名与条目文案都在字典里，键名由 id 派生（见 utils/shortcuts.js 头注释） -->
              <div class="scb-group-title">{{ $t('shortcut.group.' + g.key) }}</div>
              <div
                v-for="item in shortcutDefsOfGroup(g.key)"
                :key="item.id"
                class="scb-row"
                :class="{ recording: recordingId === item.id, changed: isCustomShortcut(item.id) }"
                @click="beginRecordShortcut(item.id)"
              >
                <div class="scb-label">
                  {{ $t('shortcut.' + item.id + '.label') }}
                  <span class="scb-desc">{{ $t('shortcut.' + item.id + '.desc') }}</span>
                </div>
                <div class="scb-side">
                  <template v-if="recordingId === item.id">
                    <span class="scb-rec-hint">{{ $t('shortcut.recording') }}</span>
                  </template>
                  <template v-else-if="keysOf(item.id)">
                    <!-- 循环变量原来叫 t 会**遮蔽**全局 t()（模板里也有），改叫 tk；模板一律用 $t -->
                    <kbd v-for="(tk, ti) in tokensOf(item.id)" :key="ti">{{ tk }}</kbd>
                    <span class="scb-clear" :title="$t('shortcut.unbind')" @click.stop="clearShortcut(item.id)">{{ $t('shortcut.clear') }}</span>
                  </template>
                  <span v-else class="scb-none">{{ $t('shortcut.unbound') }}</span>
                </div>
              </div>
            </div>
          </div>
        </div>

        <!-- 9. 关于 -->
        <div v-show="activeTab === 'about'" class="settings-panel">
          <div class="about-hero">
            <span class="about-logo"><img :src="logoMdUrl" alt="DBmind" /></span>
            <div class="about-meta">
              <div class="about-name">{{ $t('settings.about.name') }}</div>
              <!-- 版本号按语义化三段展示：版本 v大版本.小版本.修订，hover 每段看含义 -->
              <div class="about-ver">
                <span class="ver-prefix">{{ $t('settings.about.verLabel') }}</span>
                <span class="ver-seg" :title="$t('settings.about.verMajorTip')">{{ verParts[0] }}</span>
                <span class="ver-dot">.</span>
                <span class="ver-seg" :title="$t('settings.about.verMinorTip')">{{ verParts[1] }}</span>
                <span class="ver-dot">.</span>
                <span class="ver-seg" :title="$t('settings.about.verPatchTip')">{{ verParts[2] }}</span>
              </div>
            </div>
          </div>

          <!-- 目录说明：与「版本说明」同规格的板块标题 -->
          <div class="about-block">
            <div class="about-dbs-title">{{ $t('settings.about.dirsTitle') }}</div>
            <div class="about-rows">
              <div class="about-row">
                <span class="about-k">{{ $t('settings.paths.dataDir') }}</span>
                <code class="about-v">{{ pathInfo.dataDir }}</code>
              </div>
              <div class="about-row">
                <span class="about-k">{{ $t('settings.paths.driverDir') }}</span>
                <code class="about-v">{{ pathInfo.driverDir }}</code>
              </div>
              <div class="about-row">
                <span class="about-k">{{ $t('settings.paths.logDir') }}</span>
                <code class="about-v">{{ pathInfo.logDir || (pathInfo.dataDir + '/logs') }}</code>
              </div>
            </div>
          </div>


          <!-- 版本说明：标题外置（与其他板块同规格）；版本号做卡片式下拉头，点击展开说明 -->
          <div class="about-block">
            <div class="about-dbs-title">{{ $t('settings.about.releaseTitle') }}</div>
            <div class="about-ver-rule">{{ $t('settings.about.verRule') }}</div>
            <div class="about-fold" :class="{ open: releaseOpen }">
              <button type="button" class="about-fold-head" @click="releaseOpen = !releaseOpen">
                <span>{{ $t('settings.about.releaseVersion') }}</span>
                <el-icon class="about-fold-arrow"><ArrowDown /></el-icon>
              </button>
              <el-collapse-transition>
                <div class="about-fold-body" v-show="releaseOpen">
                  <!-- 版本倒序平铺：1.0.1 在上，1.0.0 在下；每条带自己的类型标签与说明 -->
                  <div class="about-vitem" v-for="rel in releaseList" :key="rel.version">
                    <div class="about-rel-ver">
                      <span class="about-rel-no">{{ rel.version }}</span>
                      <span class="about-rel-tags">
                        <span class="about-rel-tag" :class="'tag-' + rel.level">{{ $t('settings.about.verTag.' + rel.level) }}</span>
                        <span class="about-rel-tag tag-latest" v-if="rel.latest">{{ $t('settings.about.releaseLatest') }}</span>
                      </span>
                      <span class="about-rel-date" v-if="rel.date">{{ rel.date }}</span>
                    </div>
                    <div class="about-release-line" v-for="(line, li) in rel.notes" :key="li">{{ line }}</div>
                  </div>
                </div>
              </el-collapse-transition>
            </div>
          </div>

          <!-- 开源说明：免费使用，禁止售卖 -->
          <div class="about-block">
            <div class="about-dbs-title">{{ $t('settings.about.licenseTitle') }}</div>
            <div class="about-block-body">
              <div class="about-license">{{ $t('settings.about.licenseBody') }}</div>
              <a class="about-link" href="https://github.com/rick-works/dbmind" target="_blank" rel="noopener">github.com/rick-works/dbmind</a>
            </div>
          </div>
        </div>
      </div>
    </div>
  </el-dialog>

</template>

<script setup>
import { ref, computed, watch, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  MagicStick, Connection, Brush, Box,
  FolderOpened, EditPen, DataLine, Download, Bell, Operation, Monitor, Plus, Delete,
  CircleCheck, Timer, Moon, Pointer, Refresh, Search, Upload, Flag, InfoFilled, Lock, Check, Key,
  ArrowUp, ArrowDown, Document, DocumentCopy
} from '@element-plus/icons-vue'
import { topMenuAll, topMenuCfg, toggleTopMenu, moveTopMenu } from '../../utils/topMenu'
// `LOCALES` 直接当语言选项用：它里面每个选项的 label 都写着自己的语言，不需要再包一层
import { LOCALES as localeOptions, locale, setLocale, t } from '../../utils/i18n'
import { formatSql, keywordCandidates, sqlKeywordPattern } from '../../utils/sqlFormat'
import { getAiConfig, saveAiConfig, aiChat, getPathSettings, savePathSettings, browseBackupDirs, getDriverMirror, saveDriverMirror, getDriverTypes, getDriverStatus, getDriverProgress, installDriver, uploadDriver, openLocalDir, getAiUsage, getSettings, putSetting, clearSchemaCache, clearHistory, getLegacyTls, saveLegacyTls, getCacheItems, clearCaches, listConnections, getLogs, clearLogs } from '../../api'
import { editorDefaults, queryDefaults, notifyDefaults, migrateEditor, reloadEditorSettings, reloadQuerySettings, persistUI } from '../../utils/settings'
// 「刷新结构缓存」要连**前端那份** localStorage 缓存一起清（内核缓存清了它还在也会显示旧清单）
import { clearSchemaCache as clearLocalSchemaCache } from '../../utils/schemaCache'
import DbLogo from '../../common/DbLogo.vue'
import {
  loadShortcuts, saveShortcuts, SHORTCUT_DEFS, SHORTCUT_GROUPS,
  keysTokens, eventToKeys, setShortcutSuppressed
} from '../../utils/shortcuts'
import { APP_VERSION } from '../../version'
import logoMdUrl from '../../assets/logo-md.png'
import {
  getThemeSettings, saveThemeSettings, applyTheme, getResolvedTheme, onResolvedThemeChange
} from '../../utils/theme'

const props = defineProps({ modelValue: Boolean, initialTab: { type: String, default: '' } })
const emit = defineEmits(['update:modelValue'])

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

// 默认落在「通用」：点齿轮开设置时先看的是最常用的那一页（显式指定页签的入口不受影响，见下方 initialTab）
const activeTab = ref('general')

// 页签只存 i18nKey，不存中文原文 —— 文案统一由字典给（见 utils/i18n.js）。
// 存一份中文再「顺便翻译」等于同一句话有两个真相，改文案时必然漏掉一处。
// 排序逻辑：**使用频率从高到低** —— 外观与日常操作（通用/主题/编辑器/格式化/查询）
// 在前，AI 与安全居中，系统级（驱动/存储/缓存）靠后，通知/快捷键/关于收尾
const tabs = [
  { key: 'general', i18nKey: 'settings.tab.general', icon: Flag },
  { key: 'theme', i18nKey: 'settings.tab.theme', icon: Moon },
  { key: 'editor', i18nKey: 'settings.tab.editor', icon: EditPen },
  { key: 'format', i18nKey: 'settings.tab.format', icon: Brush },
  { key: 'shortcut', i18nKey: 'settings.tab.shortcut', icon: Pointer },
  { key: 'query', i18nKey: 'settings.tab.query', icon: DataLine },
  { key: 'ai', i18nKey: 'settings.tab.ai', icon: MagicStick },
  { key: 'safety', i18nKey: 'settings.tab.safety', icon: Lock },
  { key: 'mcp', i18nKey: 'settings.tab.mcp', icon: Connection },
  // 图标与顶栏「驱动管理」入口用同一个（utils/topMenu.js 里的 Box）——
  // 同一个功能在两处出现不同图标，用户要对两次才能确认是同一个地方
  { key: 'driver', i18nKey: 'settings.tab.driver', icon: Box },
  { key: 'paths', i18nKey: 'settings.tab.paths', icon: FolderOpened },
  { key: 'cache', i18nKey: 'settings.tab.cache', icon: Delete },
  { key: 'logs', i18nKey: 'settings.tab.logs', icon: Document },
  { key: 'notify', i18nKey: 'settings.tab.notify', icon: Bell },
  { key: 'about', i18nKey: 'settings.tab.about', icon: Monitor }
]


// 「关于」页的版本说明：随每次发版更新（i18n 键 settings.about.releaseNote1..N）
const releaseOpen = ref(false) // 版本说明默认收起，点标题栏展开（卡片式下拉）

// 版本号三段（大版本.小版本.修订），缺失段补 0：APP_VERSION 形如 0.1.1
const verParts = computed(() => {
  const p = String(APP_VERSION || '0.0.0').split('.')
  return [p[0] || '0', p[1] || '0', p[2] || '0']
})

// 版本说明：按版本倒序（第一个为最新，标「最新」）。版本号从 1.0.0 起 —— 1.0.0 是首个
// 正式版，之后修 bug 只加修订号（1.0.1）、新增功能加小版本（1.1.0）、不兼容变更才加大版本（2.0.0）。
// 文案在 locales 里按 `settings.about.relN.version` / `relN.noteM` / `relN.level` 组织。
const releaseList = computed(() => {
  const out = []
  for (let i = 1; i <= 10; i++) {
    const version = t(`settings.about.rel${i}.version`)
    if (!version || version === `settings.about.rel${i}.version`) break
    const notes = []
    for (let j = 1; j <= 12; j++) {
      const line = t(`settings.about.rel${i}.note${j}`)
      if (!line || line === `settings.about.rel${i}.note${j}`) break
      notes.push(line)
    }
    out.push({
      version,
      notes,
      level: t(`settings.about.rel${i}.level`),
      date: t(`settings.about.rel${i}.date`),
      latest: i === 1
    })
  }
  return out
})

// ---------- AI 用量（展示在「AI 服务」页签）----------
const usage = ref({})

/** AI 用量：知识库已独立成顶部导航的「知识库」工作台，这里只取「AI 服务」页签要展示的用量 */
const loadKnowledge = async () => {
  try { usage.value = (await getAiUsage(30)) || {} } catch (e) { /* ignore */ }
}

/**
 * 卡片标题里的「近 N 天」。
 *
 * `usage.days` 是**逐日明细数组**，不是天数 —— 早前直接渲染它，
 * Vue 会把数组序列化成 JSON，标题就变成「近 [{"calls":0,"date":"…"}] 天」（实测）。
 * 取明细条数才等于窗口天数；接口没给明细时退回请求时的 30 天。
 */
const usageRange = computed(() => {
  const list = Array.isArray(usage.value.days) ? usage.value.days.length : 0
  return list || 30
})

/**
 * 用量卡片里的三个数 —— **只用账本真实有的数据**。
 *
 * 两个坑都是实测踩到的：
 * 1. `usage.total` **本身就是总数**（不是 `{calls,…}` 对象），早前读 `total.calls`
 *    永远是 `undefined` ⇒ 界面显示 0，而账本里明明有 12 次调用；
 * 2. 账本（主库 dbmind.db 的 ai_usage_days / ai_usage_models 两张表）只记「每天调用了几次」，
 *    「失败次数 / 平均耗时」根本没记 —— 那两张卡只能显示假的 0。
 *    要它们就得先在 `record_usage` 里带上 ok 与耗时（未做，故先换成能算出来的真指标）。
 */
const usageStats = computed(() => {
  const days = Array.isArray(usage.value.days) ? usage.value.days : []
  const total = typeof usage.value.total === 'number'
    ? usage.value.total
    : days.reduce((sum, day) => sum + (day.calls || 0), 0)
  const window = usage.value.window || days.length || 30
  const activeDays = days.filter((day) => (day.calls || 0) > 0).length
  return { calls: total, activeDays, perDay: (total / window).toFixed(1) }
})

/** 按模型的用量明细（后端已按总 token 降序排好）。老账本没有 models 字段 → 空数组，不报错。 */
const usageModels = computed(() => (Array.isArray(usage.value.models) ? usage.value.models : []))

/** token 数的千分位；缺字段显示 —，**不显示 0**（0 与"没统计到"是两回事）。 */
const fmtTokens = (n) => (n === 0 || n) ? Number(n).toLocaleString() : '—'

// ---------- 主题 ----------
const themeForm = ref(getThemeSettings())
const themeResolved = ref(getResolvedTheme())
let offThemeChange = null
onMounted(() => {
  offThemeChange = onResolvedThemeChange((r) => { themeResolved.value = r })
})
onBeforeUnmount(() => {
  if (offThemeChange) offThemeChange()
})
// 与 tabs 同理：这里只存键，文案交给字典（切换语言时模板读的是字典，会自动更新）
const themeOptions = [
  { mode: 'system', i18nKey: 'settings.theme.system', descKey: 'settings.theme.systemDesc' },
  { mode: 'light', i18nKey: 'settings.theme.light', descKey: 'settings.theme.lightDesc' },
  { mode: 'dark', i18nKey: 'settings.theme.dark', descKey: 'settings.theme.darkDesc' }
]
// 计算属性里用 t() 而不是 $t：`$t` 只在模板上下文里有
const systemModeText = computed(() =>
  t(themeResolved.value === 'dark' ? 'settings.theme.systemDark' : 'settings.theme.systemLight'))
const setThemeMode = (mode) => {
  themeForm.value.mode = mode
  applyTheme(mode)
  saveThemeSettings({ mode })
}

// ---------- AI 配置 ----------
const form = ref({ enabled: false, models: [], privacyMode: 'allow', auditEnabled: false })
const saving = ref(false)
const testingId = ref('')
const activeModelId = ref('')

// 用 computed：模块级数组只在加载时求值一次，切换语言后预设名不会跟着变
const presets = computed(() => [
  { name: 'DeepSeek', baseUrl: 'https://api.deepseek.com/v1', model: 'deepseek-chat' },
  { name: t('settings.ai.presetQwen'), baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1', model: 'qwen-plus' },
  { name: 'OpenAI', baseUrl: 'https://api.openai.com/v1', model: 'gpt-4o-mini' },
  { name: 'Kimi', baseUrl: 'https://api.moonshot.cn/v1', model: 'moonshot-v1-8k' },
  { name: t('settings.ai.presetOllamaLocal'), baseUrl: 'http://localhost:11434/v1', model: 'qwen2.5' }
])

const genId = () => Math.random().toString(36).slice(2, 10)

const addModel = () => {
  const newModel = { id: genId(), name: '', baseUrl: '', apiKey: '', model: '', maxTokens: 0, embedding: false }
  form.value.models.push(newModel)
  activeModelId.value = newModel.id
}

const removeModel = (idx) => {
  const removed = form.value.models.splice(idx, 1)
  if (activeModelId.value && removed[0]?.id === activeModelId.value) {
    activeModelId.value = form.value.models[0]?.id || ''
  }
}

const removeModelById = (id) => {
  const idx = form.value.models.findIndex(m => m.id === id)
  if (idx !== -1) removeModel(idx)
}

const applyPreset = (idx, p) => {
  const m = form.value.models[idx]
  if (m) {
    m.baseUrl = p.baseUrl
    m.model = p.model
    if (!m.name) m.name = p.name
  }
}

/**
 * 提交给后端的载荷。
 *
 * <p>后端的模型列表按 Map&lt;String,String&gt; 接收，勾选框的布尔值要转成字符串再发，
 * 否则后端取值时会因为类型不是 String 直接抛错。
 */
const payload = () => ({
  ...form.value,
  models: (form.value.models || []).map(m => ({ ...m, embedding: m.embedding ? 'true' : 'false' }))
})

const save = async () => {
  const names = form.value.models.map(m => m.name?.trim()).filter(Boolean)
  const dup = names.find((n, i) => names.indexOf(n) !== i)
  if (dup) {
    ElMessage.error(t('settings.ai.msgDupName', { name: dup }))
    return
  }
  saving.value = true
  try {
    await saveAiConfig(payload())
    ElMessage.success(t('settings.ai.msgSaved'))
  } catch (e) {
    console.error('保存配置异常:', e)
    const detail = e && e.message ? e.message : String(e)
    ElMessage.error(t('settings.ai.msgSaveFailed', { detail }))
  } finally {
    saving.value = false
  }
}

const testActiveModel = async () => {
  const modelId = activeModelId.value
  if (!modelId) { ElMessage.warning(t('settings.ai.msgNoModel')); return }
  testingId.value = modelId
  try {
    await saveAiConfig(payload())
    const res = await aiChat({ system: t('settings.ai.testPrompt'), prompt: 'ping', modelId })
    if (res && res.success) {
      // 模型会把它收到的测试提示原样回一句，所以这条提示本身也要跟着语言走 ——
      // 否则英文界面下会弹出一句中文（模型只是照抄，不是界面漏翻）
      ElMessage.success(res.content
        ? t('settings.ai.msgTestOkWith', { detail: String(res.content).slice(0, 60) })
        : t('settings.ai.msgTestOk'))
    } else {
      ElMessage.error(t('settings.ai.msgTestError', { detail: (res && res.message) ? res.message : JSON.stringify(res) }))
    }
  } catch (e) {
    console.error('AI 测试连接异常:', e)
    let detail = ''
    if (e && e.message) detail = e.message
    else if (typeof e === 'string') detail = e
    else detail = String(e)
    ElMessage.error(t('settings.ai.msgTestFailed', { detail }))
  } finally {
    testingId.value = ''
  }
}

const load = async () => {
  try {
    const cfg = await getAiConfig()
    if (!cfg || typeof cfg !== 'object') {
      ElMessage.error(t('settings.ai.msgLoadBadShape'))
      return
    }
    const models = Array.isArray(cfg.models) ? cfg.models : []
    if (models.length === 0 && cfg.baseUrl) {
      // 兼容旧版单模型配置
      models.push({ id: genId(), name: t('settings.ai.defaultModelName'), baseUrl: cfg.baseUrl || '', apiKey: cfg.apiKey || '', model: cfg.model || '' })
    }
    form.value = {
      enabled: cfg.enabled,
      models,
      privacyMode: cfg.privacyMode || 'allow',
      auditEnabled: !!cfg.auditEnabled
    }
    activeModelId.value = models[0]?.id || ''
  } catch (e) { ElMessage.error(t('settings.ai.msgLoadFailed', { detail: (e?.message || e?.toString?.() || t('common.unknownError')) })) }
}

// ---------- 存储路径 ----------
/** 数据目录迁移已接入（后端：快照 + 复制 + 指针文件，热切换立即生效）；驱动跟着数据目录走，不单独设置 */
const pathInfo = ref({ os: '', osLabel: '', homeDir: '', defaultDataDir: '', defaultDriverDir: '', dataDir: '', driverDir: '', exportDir: '' })
const pathForm = ref({ dataDir: '' })
const savingPath = ref(false)
const osTagText = computed(() => ({ windows: 'Windows', mac: 'macOS', linux: 'Linux', other: t('settings.paths.osOther') }[pathInfo.value.os] || t('settings.paths.osUnknown')))

const loadPaths = async () => {
  try {
    const info = await getPathSettings()
    pathInfo.value = info
    pathForm.value = { dataDir: info.dataDir }
    // 空 = 用默认目录；有值 = 用户指定过
  } catch (e) { ElMessage.error(t('settings.paths.msgLoadFailed', { detail: (e?.message || e?.toString?.() || t('common.unknownError')) })) }
}

const resetDataDir = () => { pathForm.value.dataDir = pathInfo.value.defaultDataDir }

/**
 * 迁移数据目录。
 *
 * 后端语义（sys.rs 的 prepare_and_open）：元数据库做一致性快照、驱动整目录复制到目标、
 * 写下「指针文件」，然后**热切换引擎** —— 立即生效，不用重启（旧引擎的宿主 JVM 由内核回收）。
 * 切换完成后整页刷新一次：连接树等 SPA 状态要重新从新引擎拉取。
 */
const savePaths = async () => {
  const target = (pathForm.value.dataDir || '').trim()
  if (!target || target === pathInfo.value.dataDir) return
  try {
    await ElMessageBox.confirm(
      t('settings.paths.migrateConfirm', { dir: target }),
      t('settings.paths.save'),
      { type: 'warning', confirmButtonText: t('common.confirm'), cancelButtonText: t('common.cancel'), closeOnClickModal: false }
    )
  } catch { return }
  savingPath.value = true
  try {
    const info = await savePathSettings({ dataDir: target })
    ElMessage.success(t('settings.paths.migrated', { dir: info.dataDir }))
    // 引擎已热切换，但页面上的连接树 / 标签页等状态还来自旧引擎 —— 整页刷新拉新
    setTimeout(() => { window.location.reload() }, 1200)
  } catch (e) { ElMessage.error(e?.message || e?.toString?.() || t('common.unknownError')) }
  savingPath.value = false
}

// ---------- localStorage 配置（默认值统一来自 utils/settings.js） ----------
const editorForm = ref({ ...editorDefaults, sqlKeywordRules: [] })
const loadEditor = () => {
  try {
    const raw = JSON.parse(localStorage.getItem('dbmind_editor') || '{}')
    // 一次性迁移：旧默认值 autoSave=true 已改为 false，清理遗留的旧缓存
    if (raw.autoSave === true && !raw._asMigrated) {
      delete raw.autoSave
      raw._asMigrated = true
      localStorage.setItem('dbmind_editor', JSON.stringify(raw))
    }
    // 一次性迁移：历史「关键字串+排版」/ v2 规则 → v3 结构（{kw, cs, ly}），并清理已下线全局开关
    migrateEditor(raw)
    const rules = Array.isArray(raw.sqlKeywordRules)
      ? raw.sqlKeywordRules.map((r) => ({
        kw: (r && r.kw) || '',
        cs: (r && r.cs) === 'keep' ? 'keep' : (r && r.cs) === 'lower' ? 'lower' : 'upper',
        ly: ['none', 'clause', 'join', 'afterBreak', 'afterBreakIndent', 'setop'].includes(r && r.ly) ? r.ly : 'none'
      }))
      : []
    editorForm.value = { ...editorDefaults, ...raw, sqlKeywordRules: rules }
  } catch { editorForm.value = { ...editorDefaults, sqlKeywordRules: [] } }
}
// **改完即生效**：用户最常踩的坑就是切了开关没点保存，回去看编辑器"没反应"。
// 深度监听静默落盘 + 热更新（落盘与刷新只在这一处做），「保存」按钮仅作显式确认。
watch(editorForm, () => {
  persistUI('dbmind_editor', JSON.stringify(editorForm.value))
  reloadEditorSettings()
}, { deep: true })
const saveEditor = () => {
  ElMessage.success(t('settings.editor.msgSaved'))
}
const resetEditor = () => { editorForm.value = { ...editorDefaults, sqlKeywordRules: [] }; saveEditor() }

// ---------- 关键字排版规则（SQL 格式化） ----------
// 每条规则 = { kw: 关键字/短语, cs: upper=大写 | lower=小写 | keep=保持原样, ly: 排版方式 }
const kwLyValues = ['none', 'clause', 'join', 'afterBreak', 'afterBreakIndent', 'setop']
const kwLyLabel = (ly) => ({
  none: t('settings.format.lyNone'),
  clause: t('settings.format.lyClause'),
  join: t('settings.format.lyJoin'),
  afterBreak: t('settings.format.lyAfterBreak'),
  afterBreakIndent: t('settings.format.lyAfterBreakIndent'),
  setop: t('settings.format.lySetop')
})[ly] || t('settings.format.lyNone')
const kwCsLabel = (cs) => (cs === 'keep' ? t('settings.format.casePreserve') : (cs === 'lower' ? t('settings.format.caseLower') : t('settings.format.caseUpper')))
const addKeywordRule = () => { editorForm.value.sqlKeywordRules.push({ kw: '', cs: 'upper', ly: 'none' }) }
const removeKeywordRule = (index) => { editorForm.value.sqlKeywordRules.splice(index, 1) }

// 关键字候选：标准 SQL 关键字（含常用短语）的去重列表，输入时实时过滤；
// 其它方言专属关键字（如 T-SQL 的 TOP）也可直接手输，规则在对应连接的编辑器中仍会生效。
const kwCandidates = computed(() => keywordCandidates('sql'))
const queryKwSuggestions = (query, cb) => {
  const q = String(query || '').trim().toUpperCase()
  const hit = q ? kwCandidates.value.filter((w) => w.includes(q)) : kwCandidates.value
  cb(hit.slice(0, 60).map((w) => ({ value: w })))
}

// SQL 格式化动态预览（实时响应配置变化）
// 效果预览：统一渲染同一段示例 SQL。若配置了关键字排版规则，规则直接套用在这段示例上，
// 效果体现在排版里，且被规则管理的关键字会做醒目高亮；关键字不在示例中时仅在备注里提示。
const fmtDemoSql = "select u.id,u.name,count(o.id) as cnt,coalesce(u.age,0) as age from users u join orders o on o.user_id=u.id where u.age>18 and o.status='paid' group by u.id,u.name,u.age order by cnt desc limit 10;"
// 预览用：转义 HTML，并对关键字做语法高亮（内置关键字 + 自定义规则关键字，自定义的更醒目）
const escHtml = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
const escapeReg = (s) => String(s).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
const previewSql = computed(() => {
  const f = editorForm.value
  const rules = (Array.isArray(f.sqlKeywordRules) ? f.sqlKeywordRules : []).filter((r) => r && String(r.kw || '').trim())
  const kwList = rules.map((r) => String(r.kw).trim()).filter(Boolean)
  // 关键字高亮正则：一次构造，同时覆盖内置关键字 + 自定义关键字（预览固定按标准 SQL）
  const kwRe = sqlKeywordPattern('sql', kwList)
  const hl = (text) => text.replace(kwRe, (m) => {
    const up = m.toUpperCase()
    const isCustom = kwList.some((k) => k.split(/\s+/).some((p) => p.toUpperCase() === up))
    return `<span class="${isCustom ? 'fmt-kw-hl' : 'fmt-sql-kw'}">${m}</span>`
  })
  // 始终渲染同一段示例 SQL：无规则走默认排版；有规则则整段套用规则后再格式化
  let out = fmtDemoSql
  try { out = formatSql(fmtDemoSql, f) } catch (e) { /* 格式化失败则保留原文 */ }
  const rendered = hl(escHtml(out))
  if (!rules.length) return rendered
  // 顶部备注列出生效规则；关键字不在示例 SQL 中出现时给出提示，避免误以为规则未生效
  const outUp = out.toUpperCase()
  const notes = rules.map((r) => {
    const k = String(r.kw).trim()
    const cs = r.cs === 'lower' ? 'lower' : (r.cs === 'keep' ? 'keep' : 'upper')
    const ly = kwLyValues.includes(r.ly) ? r.ly : 'none'
    const visible = k.split(/\s+/).every((w) => new RegExp(`\\b${escapeReg(w.toUpperCase())}\\b`).test(outUp))
    return `<span class="fmt-preview-note">${t('settings.format.previewNote', { kw: escHtml(k), cs: kwCsLabel(cs), ly: kwLyLabel(ly) })}${visible ? '' : t('settings.format.previewAbsent')}</span>`
  })
  return notes.join('\n') + '\n' + rendered
})

const queryForm = ref({ ...queryDefaults })
const loadQuery = async () => {
  try {
    const raw = JSON.parse(localStorage.getItem('dbmind_query') || '{}')
    queryForm.value = { ...queryDefaults, ...raw }
  } catch { queryForm.value = { ...queryDefaults } }
  // 超时/上限/历史保留与缓存 TTL 存**后端**：以服务端为准盖掉占位默认值（拿不到就维持默认）
  try {
    const s = await getSettings()
    const num = (key, fallback) => {
      const n = Number(s?.[key])
      return Number.isFinite(n) && n >= 0 ? n : fallback
    }
    queryForm.value.timeoutSecs = num('query.timeoutSecs', queryForm.value.timeoutSecs)
    queryForm.value.cacheTtlSecs = num('schema.ttlSecs', queryForm.value.cacheTtlSecs)
    queryForm.value.maxRows = num('query.maxRows', queryForm.value.maxRows)

    logLevel.value = (s?.['log.level'] || 'info').trim()
  } catch { /* 后端拿不到就维持本地默认（离线/降级也不该挡住设置页） */ }
}
// 「安全确认」迁到安全页签后，勾选也必须**即时持久化**（不能要求用户去查询页签点保存）
watch(queryForm, () => {
persistUI('dbmind_query', JSON.stringify({ pageSize: queryForm.value.pageSize, confirmDanger: queryForm.value.confirmDanger, nullStyle: queryForm.value.nullStyle }))
reloadQuerySettings()
}, { deep: true })
const saveQuery = async () => {
  // 本地两项由上面的 watch 即时落盘，这里只负责后端几项（改完即时生效）
  // 后端几项：改完即时生效（执行入口每次读、保留策略改完立即清一遍）
  try {
    await putSetting('query.timeoutSecs', Math.round(queryForm.value.timeoutSecs))
    await putSetting('schema.ttlSecs', Math.round(queryForm.value.cacheTtlSecs))
    await putSetting('query.maxRows', Math.round(queryForm.value.maxRows))

  } catch (e) {
    ElMessage.error(t('settings.query.msgSaveFailed', { detail: (e?.message || t('common.unknownError')) }))
    return
  }
  ElMessage.success(t('settings.query.msgSaved'))
}
const resetQuery = async () => {
  queryForm.value = { ...queryDefaults }
  await saveQuery()
}

/** 清空查询历史（后端整表清空 —— 没有"只删一条"的接口，确认框里把话说死） */
const onClearHistory = async () => {
  try {
    await ElMessageBox.confirm(t('settings.query.clearHistoryConfirm'), t('settings.query.clearHistory'), {
      type: 'warning', confirmButtonText: t('common.delete'), cancelButtonText: t('common.cancel'), closeOnClickModal: false
    })
  } catch { return }
  try {
    await clearHistory()
    ElMessage.success(t('settings.query.historyCleared'))
  } catch (e) { ElMessage.error(t('settings.query.msgSaveFailed', { detail: (e?.message || t('common.unknownError')) })) }
}

/** 刷新结构缓存：前端那份 localStorage 缓存与内核缓存要**一起**清，少一半都会看到旧清单 */
const onRefreshCache = async () => {
  try {
    await clearSchemaCache()
    clearLocalSchemaCache()
    ElMessage.success(t('settings.query.cacheRefreshed'))
  } catch (e) { ElMessage.error(t('settings.query.msgSaveFailed', { detail: (e?.message || t('common.unknownError')) })) }
}

// ===== 安全与会话（全部存后端 app_settings；安全开关与会话上限改动即时生效） =====
// protectProduction 默认 true：与后端种子（storage.rs 的 seed_settings）保持一致，
// 否则首次加载、或后端暂时读不到时，开关会显示成"关"，与内核实际策略不符。
const safetyForm = ref({ protectProduction: true, aiWriteEnabled: false, maxSessions: 32, idleMinutes: 0, tunnelIdleMinutes: 30, allowLegacyTls: false, blockDangerous: false, maxWriteRows: 0 })

// ---------- MCP 服务 ----------
// 配置存后端 app_settings：MCP 壳**每次工具调用都现读**，保存后下一次调用即生效，不用重启客户端
const mcpForm = ref({ defaultConnection: '', maxRows: 2000, timeoutSecs: 30, toolsStructure: true, toolsQuery: true, toolsHistory: true })
const connOptions = ref([])
const loadMcp = async () => {
  try {
    const [s, conns] = await Promise.all([getSettings(), listConnections().catch(() => [])])
    connOptions.value = Array.isArray(conns) ? conns : []
    const str = (key, fallback) => (typeof s?.[key] === 'string' && s[key] !== '' ? s[key] : fallback)
    mcpForm.value.defaultConnection = str('mcp.defaultConnection', '')
    mcpForm.value.maxRows = Number(s?.['mcp.maxRows']) > 0 ? Number(s['mcp.maxRows']) : 2000
    mcpForm.value.timeoutSecs = Number(s?.['mcp.timeoutSecs']) > 0 ? Number(s['mcp.timeoutSecs']) : 30
    const flag = (key, fallback) => (s?.[key] === 'true' || s?.[key] === '1' ? true : (s?.[key] === 'false' || s?.[key] === '0' ? false : fallback))
    mcpForm.value.toolsStructure = flag('mcp.toolsStructure', true)
    mcpForm.value.toolsQuery = flag('mcp.toolsQuery', true)
    mcpForm.value.toolsHistory = flag('mcp.toolsHistory', true)
  } catch (e) {
    ElMessage.error(t('settings.query.msgSaveFailed', { detail: (e?.message || t('common.unknownError')) }))
  }
}
const saveMcp = async () => {
  try {
    await putSetting('mcp.defaultConnection', mcpForm.value.defaultConnection || '')
    await putSetting('mcp.maxRows', Math.round(mcpForm.value.maxRows))
    await putSetting('mcp.timeoutSecs', Math.round(mcpForm.value.timeoutSecs))
    await putSetting('mcp.toolsStructure', mcpForm.value.toolsStructure)
    await putSetting('mcp.toolsQuery', mcpForm.value.toolsQuery)
    await putSetting('mcp.toolsHistory', mcpForm.value.toolsHistory)
  } catch (e) {
    ElMessage.error(t('settings.query.msgSaveFailed', { detail: (e?.message || t('common.unknownError')) }))
    return
  }
  ElMessage.success(t('settings.mcp.msgSaved'))
}
const loadSafety = async () => {
  try {
    const [s, tls] = await Promise.all([getSettings(), getLegacyTls().catch(() => null)])
    // ⚠️ `num` 必须定义在**第一次使用之前**：原先它写在下方，`maxWriteRows` 那行先调用了它
    // → TDZ 抛 ReferenceError → 整个 try 被 catch 吞掉 → 表单永远停在默认值
    //（表象是"设置页显示的开关/数字与实际生效的对不上，一保存还把用户的设置覆盖成默认"）。
    const num = (key, fallback) => {
      const n = Number(s?.[key])
      return Number.isFinite(n) && n >= 0 ? n : fallback
    }
    const flag = (key, fallback) => (s?.[key] === 'true' || s?.[key] === '1') ? true : (s?.[key] === 'false' || s?.[key] === '0' ? false : fallback)
    // 兜底取 true：与后端种子一致（生产保护默认开启）
    safetyForm.value.protectProduction = flag('safety.protectProduction', true)
    safetyForm.value.aiWriteEnabled = flag('safety.aiWriteEnabled', false)
    safetyForm.value.blockDangerous = flag('safety.blockDangerousStatements', false)
    safetyForm.value.maxWriteRows = num('safety.maxWriteRows', 0)
    safetyForm.value.maxSessions = num('session.maxPerHost', 32)
    // 内核按**秒**读；界面用分钟更好读（0 = 不按空闲回收）
    safetyForm.value.idleMinutes = Math.round(num('session.idleTimeoutSecs', 0) / 60)
    // SSH 隧道空闲回收同理（默认 30 分钟）
    safetyForm.value.tunnelIdleMinutes = Math.round(num('tunnel.idleTimeoutSecs', 1800) / 60)
    safetyForm.value.allowLegacyTls = !!(tls && tls.allowLegacyTls)
  } catch { /* 同上：降级到默认值 */ }
}
const saveSafety = async () => {
  try {
    await putSetting('safety.protectProduction', safetyForm.value.protectProduction)
    await putSetting('safety.aiWriteEnabled', safetyForm.value.aiWriteEnabled)
    await putSetting('safety.blockDangerousStatements', safetyForm.value.blockDangerous)
    await putSetting('safety.maxWriteRows', Math.round(safetyForm.value.maxWriteRows))
    await putSetting('session.maxPerHost', safetyForm.value.maxSessions)
    await putSetting('session.idleTimeoutSecs', Math.round(safetyForm.value.idleMinutes) * 60)
    await putSetting('tunnel.idleTimeoutSecs', Math.round(safetyForm.value.tunnelIdleMinutes) * 60)
    // 幂等接口，直接存，不为它维护「上次值」的状态
    await saveLegacyTls(safetyForm.value.allowLegacyTls)
    ElMessage.success(t('settings.safety.msgSaved'))
  } catch (e) { ElMessage.error(t('settings.safety.msgSaveFailed', { detail: (e?.message || t('common.unknownError')) })) }
}

// 日志级别：选完即存即生效（web 壳热切换 tracing 过滤器），不用单独的保存按钮
const logLevel = ref('info')
const saveLogLevel = async (level) => {
  try {
    await putSetting('log.level', level)
    ElMessage.success(t('settings.language.logLevelSaved', { level }))
  } catch (e) { ElMessage.error(t('settings.safety.msgSaveFailed', { detail: (e?.message || t('common.unknownError')) })) }
}
const pickLogLevel = (level) => {
  if (logLevel.value === level) return
  logLevel.value = level
  saveLogLevel(level)
}
// 卡片选项（与界面语言同款视觉）：label 是 tracing 的级别名（专有名词不翻译）
const logLevels = [
  { value: 'error', label: 'error', desc: t('settings.language.logDescError') },
  { value: 'warn', label: 'warn', desc: t('settings.language.logDescWarn') },
  { value: 'info', label: 'info', desc: t('settings.language.logDescInfo') },
  { value: 'debug', label: 'debug', desc: t('settings.language.logDescDebug') },
  { value: 'trace', label: 'trace', desc: t('settings.language.logDescTrace') }
]

// 驱动下载镜像源：后端持久化于主库 dbmind.db 的 app_settings（driver.mirror），保存后立即生效。
// 值可以是关键字（maven/aliyun/huawei/tencent）或自定义内网仓库根（http(s):// 开头的 URL）——
// 关键字 → URL 的映射前后端各有一份（后端是 agent.rs 的 driver_mirror_base），改动要两边同步。
const MIRROR_BASES = {
  // 自动：值本身不代表某个仓库根（后端按网络实时挑），所以这里给空串 ——
  // 关键是**它必须在表里**，否则下面 `MIRROR_BASES[raw] !== undefined` 判定不过，
  // 'auto' 会被当成"旧数据里的自定义 URL"或干脆退化成 Maven Central。
  auto: '',
  // Maven Central 也得给出真实仓库根：以前这里是空串，于是选中它时「实际下载源」什么都不显示，
  // 只剩下方那句通用提示 —— 用户没法确认到底会从哪儿下。（与后端 MAVEN_CENTRAL 常量一致）
  maven: 'https://repo.maven.apache.org/maven2',
  aliyun: 'https://maven.aliyun.com/repository/public/',
  huawei: 'https://repo.huaweicloud.com/repository/maven/',
  tencent: 'https://mirrors.cloud.tencent.com/nexus/repository/maven-public/'
}
const driverForm = ref({ mirror: 'auto', customUrl: '' })
// 实际生效的仓库根：关键字查表；自定义用填写的 URL（空 = Maven Central）
const mirrorBase = computed(() => {
  if (driverForm.value.mirror === 'custom') {
    return (driverForm.value.customUrl || '').trim()
  }
  return MIRROR_BASES[driverForm.value.mirror] || ''
})
// 下拉宽度跟着**当前选中项**的文字走：选「自动」时那串说明要完整显示，选「华为云」时就该收窄 ——
// 写死一个宽度，不是这次被截断，就是那次空出一大截。用 canvas 量选中项本身的文字 + 箭头余量，
// 字体从下拉自身取；量不出来退回一个"够宽"的值，宁宽不截。
const MIRROR_LABELS = () => ({
  auto: t('settings.driver.mirrorAuto'),
  maven: 'Maven Central',
  aliyun: t('settings.driver.mirrorAliyun'),
  huawei: t('settings.driver.mirrorHuawei'),
  tencent: t('settings.driver.mirrorTencent'),
  custom: t('settings.driver.mirrorCustom')
})
const mirrorSelectWidth = ref(240)
const measureMirrorWidth = () => {
  const text = MIRROR_LABELS()[driverForm.value.mirror] || ''
  let font = '14px sans-serif'
  try {
    const el = document.querySelector('.driver-mirror-select input, .driver-mirror-select .el-select__selected-item')
    if (el) {
      const cs = getComputedStyle(el)
      font = [cs.fontStyle, cs.fontWeight, cs.fontSize, cs.fontFamily].filter(Boolean).join(' ')
    }
  } catch { /* 取不到字体就用默认 */ }
  try {
    const ctx = document.createElement('canvas').getContext('2d')
    ctx.font = font
    // 不要再设下限：下限就是变相的固定宽度（上一版给了 180px，于是选「阿里云」也一直是那么宽）。
    // 宽度只由**当前这条文字**决定，+56 是右侧箭头与左右内边距。
    mirrorSelectWidth.value = Math.ceil(ctx.measureText(String(text)).width) + 56
  } catch {
    mirrorSelectWidth.value = 320
  }
}
// 选中项一变就重新量（下拉里换选项，框宽跟着变）
watch(() => driverForm.value.mirror, () => nextTick(measureMirrorWidth))
onMounted(() => nextTick(measureMirrorWidth))
const saveDriver = async () => {
  // 自定义模式存填写的 URL（必须是 http(s) 开头的仓库根）；关键字模式存关键字
  const value = driverForm.value.mirror === 'custom'
    ? (driverForm.value.customUrl || '').trim()
    : driverForm.value.mirror
  try {
    const d = await saveDriverMirror(value)
    const raw = (d && d.mirror) || 'auto'
    if (MIRROR_BASES[raw] !== undefined) {
      driverForm.value.mirror = raw || 'auto'
      driverForm.value.customUrl = ''
    } else if (raw) {
      driverForm.value.mirror = 'custom'
      driverForm.value.customUrl = raw
    } else {
      driverForm.value.mirror = 'maven'
    }
    ElMessage.success(t('settings.driver.msgSaved'))
  } catch (e) { ElMessage.error(t('settings.driver.msgSaveFailed', { detail: (e?.message || e?.toString?.() || t('common.unknownError')) })) }
}

// ===== 支持的数据源 × 驱动状态（手动下载 / 上传）=====
const driverTypes = ref([])
const driverStatus = ref({})
const driverLoading = ref(false)
const driverBusy = ref('')        // 正在下载/上传的类型 code（上传时是 `<code>:up`）
// 下载进度：后端下载时实时上报 { status, percent, received, total, index, files }
// 以前点「下载」只有一个转圈按钮，几十兆的包在慢镜像下像卡死
const driverDl = ref(null)
let driverDlTimer = null
const stopDriverDlPoll = () => { if (driverDlTimer) { clearInterval(driverDlTimer); driverDlTimer = null } }
const startDriverDlPoll = (code) => {
  stopDriverDlPoll()
  if (!code) return
  const tick = async () => {
    try {
      const p = await getDriverProgress(code)
      driverDl.value = p && p.status && p.status !== 'idle' ? { ...p, code } : null
      if (p && ['done', 'failed'].includes(p.status)) { driverDl.value = { ...p, code }; stopDriverDlPoll() }
    } catch { /* 后端无此接口时忽略 */ }
  }
  tick()
  driverDlTimer = setInterval(tick, 400)
}
const fmtBytes = (v) => {
  const n = Number(v || 0)
  if (!n) return '0 B'
  if (n < 1024) return n + ' B'
  if (n < 1024 * 1024) return (n / 1024).toFixed(0) + ' KB'
  return (n / 1024 / 1024).toFixed(1) + ' MB'
}
const driverKeyword = ref('')
const driverFileRef = ref(null)
const driverUploadCode = ref('')  // 选完文件后要落到哪个类型

/**
 * 类型清单 + 就绪状态合并成列表行。
 * 「还缺什么」在这里一次算好（模板里不再拼字符串），并且**优先说缺件**而不是那句笼统的
 * reason —— 用户在这个页面上要的正是一句话：「把 xxx.jar 放进来就齐了」。
 */
const driverRows = computed(() => {
  const kw = driverKeyword.value.trim().toLowerCase()
  return driverTypes.value
    .map((dt) => {
      const s = driverStatus.value[dt.code] || {}
      const builtin = s.builtin ?? dt.builtin ?? false
      const ready = s.ready === true
      const jars = s.jars || []
      const missing = s.missing || []
      let tip
      if (builtin) tip = t('settings.driver.tipBuiltin')
      else if (ready) tip = jars.length ? t('settings.driver.tipReadyWith', { jars: jars.join(t('common.listSep')) }) : t('settings.driver.tipReady')
      else if (missing.length) tip = t('settings.driver.tipMissing', { jars: missing.join(t('common.listSep')) })
      else tip = s.reason || t('settings.driver.tipNotDownloaded')
      return {
        code: dt.code,
        label: dt.label || s.label || dt.code,
        category: dt.category || '',
        builtin,
        ready,
        dir: s.driverDir || '',
        tip
      }
    })
    .filter((r) => !kw || r.label.toLowerCase().includes(kw) || r.code.toLowerCase().includes(kw))
})
const driverReadyCount = computed(() => driverRows.value.filter((r) => r.ready).length)
const driverGroups = computed(() => {
  const bucket = (name, cat) => ({ name, items: driverRows.value.filter((r) => r.category === cat) })
  return [
    bucket(t('settings.driver.catRelational'), 'RELATIONAL'),
    bucket(t('settings.driver.catFile'), 'RELATIONAL_FILE'),
    bucket(t('settings.driver.catNoSql'), 'NOSQL')
  ].filter((g) => g.items.length)
})

const loadDriverList = async () => {
  driverLoading.value = true
  try {
    const [types, status] = await Promise.all([getDriverTypes(), getDriverStatus()])
    driverTypes.value = Array.isArray(types) ? types : []
    driverStatus.value = status || {}
  } catch (e) {
    ElMessage.error(t('settings.driver.msgListFailed', { detail: (e?.message || e?.toString?.() || t('common.unknownError')) }))
  } finally {
    driverLoading.value = false
  }
}

/** 下载：走 /install（会清掉失败缓存重新拉）。后端把失败原因写得很具体，原样显示即可 */
const downloadDriver = async (t) => {
  driverBusy.value = t.code
  startDriverDlPoll(t.code)
  try {
    const d = await installDriver(t.code)
    ElMessage.success(d?.message || t('settings.driver.msgDownloaded', { label: t.label }))
    await loadDriverList()
  } catch (e) {
    ElMessage.error(e?.message || t('settings.driver.msgDownloadFailed'))
  } finally {
    driverBusy.value = ''
    // 留一拍把 done/failed 显示完再收进度条
    setTimeout(() => { stopDriverDlPoll(); driverDl.value = null }, 1500)
  }
}

const pickDriverFile = (t) => {
  driverUploadCode.value = t.code
  if (driverFileRef.value) driverFileRef.value.click()
}

const onDriverFilePicked = async (ev) => {
  const files = Array.from(ev.target.files || [])
  ev.target.value = ''   // 清空，允许再次选同一个文件
  const code = driverUploadCode.value
  driverUploadCode.value = ''
  if (!code || !files.length) return
  driverBusy.value = code + ':up'
  try {
    const d = await uploadDriver(code, files)
    // 文件名与类型声明不一致时后端会如实回 ready:false + 还缺哪些 —— 用 warning 说清，别假装成功
    if (d && d.ready === false) ElMessage.warning(d.message || t('settings.driver.msgUploadIncomplete'))
    else ElMessage.success(d?.message || t('settings.driver.msgUploaded'))
    await loadDriverList()
  } catch (e) {
    ElMessage.error(e?.message || t('settings.driver.msgUploadFailed'))
  } finally {
    driverBusy.value = ''
  }
}

const openDriverDir = async (t) => {
  try { await openLocalDir(t.dir) } catch (e) { ElMessage.error(t('settings.driver.msgOpenDirFailed', { detail: (e?.message || e) })) }
}

const notifyForm = ref({ ...notifyDefaults })
const loadNotify = () => {
  try {
    const raw = JSON.parse(localStorage.getItem('dbmind_notify') || '{}')
    notifyForm.value = { ...notifyDefaults, ...raw }
  } catch { notifyForm.value = { ...notifyDefaults } }
}
const saveNotify = () => {
  persistUI('dbmind_notify', JSON.stringify(notifyForm.value))
  ElMessage.success(t('settings.notify.msgSaved'))
}
// 同编辑器：改完即存（main.js 的弹层包装每次弹出实时读），不依赖保存按钮
watch(notifyForm, () => {
  persistUI('dbmind_notify', JSON.stringify(notifyForm.value))
}, { deep: true })

// ===== 缓存（后端列体量，勾选清理） =====
// 知识库文档/向量也在清单里 —— 它们是「占空间的运行产物」，但清掉要重灌，
// 所以确认框的措辞按「清理」而不是「刷新」。
const cacheItems = ref([])
const cacheLoading = ref(false)
const cacheClearing = ref(false)
const checkedCacheKeys = computed(() => cacheItems.value.filter(i => i.checked).map(i => i.key))
const loadCache = async () => {
  cacheLoading.value = true
  try {
    const d = await getCacheItems()
    cacheItems.value = (d.items || []).map(i => ({ ...i, checked: false }))
  } catch { /* 清单拿不到就给空态，不弹错误打断设置页 */ }
  cacheLoading.value = false
}
const cacheSizeText = (item) => {
  const parts = []
  if (item.rows > 0) parts.push(t('settings.cache.rows', { n: item.rows }))
  if (item.files > 0) parts.push(t('settings.cache.files', { n: item.files }))
  if (item.bytes >= 1024 * 1024) parts.push(t('settings.cache.mb', { n: (item.bytes / 1024 / 1024).toFixed(1) }))
  else if (item.bytes >= 1024) parts.push(t('settings.cache.kb', { n: Math.round(item.bytes / 1024) }))
  return parts.join(' · ') || t('settings.cache.itemEmpty')
}
const clearCheckedCache = async () => {
  const keys = checkedCacheKeys.value
  if (!keys.length) return
  try {
    await ElMessageBox.confirm(t('settings.cache.confirm', { n: keys.length }), t('settings.cache.clear'), {
      type: 'warning', confirmButtonText: t('common.confirm'), cancelButtonText: t('common.cancel'), closeOnClickModal: false
    })
  } catch { return }
  cacheClearing.value = true
  try {
    await clearCaches(keys)
    ElMessage.success(t('settings.cache.cleared'))
    await loadCache()
  } catch (e) { ElMessage.error(t('settings.cache.msgFailed', { detail: (e?.message || t('common.unknownError')) })) }
  cacheClearing.value = false
}

// ---------- 日志：全类型审计（查询 / 增删改 / DDL / 事务 / AI 调用） ----------
// 审计类别只做「展示筛选」，记忆在本地即可 —— 不动 audit.level（写入侧范围设置），
// 避免用户为了看历史而把后续记录也一并关掉
const AUDIT_FILTER_KEY = 'dbmind.auditFilter'
const auditLevel = ref(localStorage.getItem(AUDIT_FILTER_KEY) || 'all')
const logQuery = ref('')
const logItems = ref([])
const logLoading = ref(false)
const logPage = ref(1)
// 每页 50 条；列表固定高度、内部滚动（表格里允许滚动条）
const logRows = ref(50)
// 总条数由后端按同一筛选条件 count 出来（不再是前端估算）
const logTotal = ref(0)

// 切换审计类别：只改本地记忆 + 重新拉取（写入侧的记录范围设置不动）
const onAuditLevelChange = (v) => {
  localStorage.setItem(AUDIT_FILTER_KEY, v)
  logPage.value = 1
  loadLogs()
}
const loadLogs = async () => {
  logLoading.value = true
  try {
    // 每页固定 50 条；后端同时返回同条件下的真实总条数（total）
    const rows = logRows.value
    const r = await getLogs({
      limit: rows,
      offset: (logPage.value - 1) * rows,
      q: logQuery.value || undefined,
      // 审计类别：all / query 查询 / write 数据 / ddl 表 / tx 事务 / exec 其他 / ai AI / error 失败 / off
      level: auditLevel.value || 'all'
    })
    logItems.value = r.items || []
    if (typeof r.total === 'number') logTotal.value = r.total
  } catch { ElMessage.error(t('settings.logs.msgFailed')) }
  logLoading.value = false
}
// 复制一条日志的 SQL：clipboard API 在非 HTTPS 环境下不可用，回落 execCommand
const copyLogSql = async (sql) => {
  const text = sql || ''
  if (!text) return
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text)
    } else {
      const ta = document.createElement('textarea')
      ta.value = text
      ta.style.position = 'fixed'
      ta.style.opacity = '0'
      document.body.appendChild(ta)
      ta.select()
      document.execCommand('copy')
      document.body.removeChild(ta)
    }
    ElMessage.success(t('settings.logs.copied'))
  } catch { ElMessage.error(t('settings.logs.msgFailed')) }
}
// 清空全部审计日志（执行历史 + AI 调用，一次全清）。日志不做自动清理，
// 也不做部分保留 —— 要清就一次清干净。
const clearAllLogs = async () => {
  try {
    await ElMessageBox.confirm(t('settings.logs.confirm'), t('settings.logs.clear'), {
      type: 'warning', confirmButtonText: t('common.confirm'), cancelButtonText: t('common.cancel'), closeOnClickModal: false
    })
  } catch { return }
  try {
    await clearLogs()
    ElMessage.success(t('settings.logs.cleared'))
    logPage.value = 1
    loadLogs()
  } catch { ElMessage.error(t('settings.logs.msgFailed')) }
}
// 时间显示：后端给的 ISO 串可能带「Z+08:00」混合时区（非法格式，Date 解析不可靠）——
// 直接正则取 YYYY-MM-DDTHH:mm:ss 重排为 MM-DD HH:mm:ss，解析不了就原样返回
const fmtLogTime = (raw) => {
  const m = /(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})/.exec(String(raw || ''))
  return m ? `${m[1]}-${m[2]}-${m[3]} ${m[4]}:${m[5]}:${m[6]}` : String(raw || '')
}
// 切到哪个页签就加载哪个页签的数据。
// 「驱动管理」原来只靠打开设置时那一次 loadDriverList()：那次是在面板还隐藏（v-show 未显示）时跑的，
// 结果列表常常是空的，得手点一下「刷新」才出来（真机反馈）。挂在这里，切过去就一定是新的。
// nextTick：等面板真的显示出来再去加载，避免又踩同样的时机问题。
watch(activeTab, (tab) => {
  if (tab === 'logs') { loadLogs() }
  if (tab === 'driver') { nextTick(loadDriverList) }
})

// ---------- 快捷键自定义 ----------
const shortcutGroups = SHORTCUT_GROUPS
const shortcutMap = ref(loadShortcuts())
const recordingId = ref('')
let recordingCleanup = null

const shortcutDefsOfGroup = (g) => SHORTCUT_DEFS.filter(d => d.group === g)
const keysOf = (id) => shortcutMap.value[id] || ''
const tokensOf = (id) => keysTokens(shortcutMap.value[id])
const isCustomShortcut = (id) => {
  const d = SHORTCUT_DEFS.find(x => x.id === id)
  return !!d && (shortcutMap.value[id] || '') !== d.default
}
const hasCustomShortcuts = computed(() => SHORTCUT_DEFS.some(d => (shortcutMap.value[d.id] || '') !== d.default))

const stopRecording = () => {
  if (recordingCleanup) { recordingCleanup(); recordingCleanup = null }
  setShortcutSuppressed(false)
  recordingId.value = ''
}

// 点击行开始录制：监听 window keydown（capture），期间抑制所有视图的快捷键分发
const beginRecordShortcut = (id) => {
  stopRecording()
  recordingId.value = id
  setShortcutSuppressed(true)
  const onKey = (e) => {
    e.preventDefault()
    e.stopPropagation()
    if (e.stopImmediatePropagation) e.stopImmediatePropagation()
    if (e.isComposing) return
    if (e.key === 'Escape') { stopRecording(); return }
    const keys = eventToKeys(e)
    if (!keys) return // 纯修饰键（Ctrl/Shift/Alt 单独按下）忽略
    shortcutMap.value = { ...shortcutMap.value, [id]: keys }
    saveShortcuts(shortcutMap.value)
    // 名称取自字典（键名由 id 派生）；万一 id 对不上就退回 id 本身，别显示空白
    const name = SHORTCUT_DEFS.some(d => d.id === id) ? t(`shortcut.${id}.label`) : id
    ElMessage.success(t('shortcut.msgChanged', { name, keys }))
    stopRecording()
  }
  window.addEventListener('keydown', onKey, true)
  recordingCleanup = () => window.removeEventListener('keydown', onKey, true)
}

const clearShortcut = (id) => {
  stopRecording()
  shortcutMap.value = { ...shortcutMap.value, [id]: '' }
  saveShortcuts(shortcutMap.value)
}

const resetAllShortcuts = () => {
  stopRecording()
  const d = {}
  SHORTCUT_DEFS.forEach(x => { d[x.id] = x.default })
  shortcutMap.value = d
  saveShortcuts(d)
  ElMessage.success(t('shortcut.msgReset'))
}

onBeforeUnmount(stopRecording)

// 弹窗打开时加载数据
watch(visible, (v) => {
  if (v) {
    // 调用方可以点名页签（首页 MCP 卡「前往设置」直达 MCP 页签）
    if (props.initialTab) activeTab.value = props.initialTab
    load()
    loadPaths()
    loadEditor()
    loadQuery()
    loadSafety()
    loadMcp()
    loadCache()
    loadDriver()
    loadDriverList()
    loadNotify()
    loadKnowledge()
    shortcutMap.value = loadShortcuts()
    // 日志页签：弹窗重开也刷新（activeTab 没变时 watch 不会触发，清空后新执行的日志就看不到）
    if (activeTab.value === 'logs') { loadLogs() }
  } else {
    stopRecording()
  }
})
</script>

<style scoped>
.dlg-title { display: flex; align-items: center; gap: 8px; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}

/* 高度 = 视口的 80%（640px 起步、900px 封顶）：矮屏不出屏，大屏多露内容 ——
   以前定死 560px，内容多时（比如 MCP 页签）右侧滚动条一长条，看着憋屈 */
/* 内容区高度跟随视口（不再用 clamp 的 640px 下限：矮视口下会把弹窗撑出屏幕）。
   左侧页签栏自己内部滚动，右侧面板内容超高时也各自内部滚 */
.settings-body { display: flex; height: calc(100vh - 190px); }
.settings-tabs {
  width: 150px; flex-shrink: 0; border-right: 1px solid var(--dc-border);
  padding: 14px 8px; overflow-y: auto;
  background: var(--dc-bg-soft);
}
.settings-tab {
  display: flex; align-items: center; gap: 10px;
  padding: 9px 10px 9px 14px; border-radius: 6px;
  cursor: pointer; font-size: 14px; color: var(--dc-text-dim);
  transition: all .15s; margin-bottom: 4px;
  position: relative;
}
.settings-tab:hover { background: var(--dc-primary-wash); color: var(--dc-text); }
.settings-tab.active {
  background: var(--dc-primary-wash); color: var(--dc-primary); font-weight: 600;
}
.settings-tab.active::before {
  content: ''; position: absolute; left: 0; top: 50%;
  transform: translateY(-50%);
  width: 3px; height: 18px;
  background: var(--dc-primary); border-radius: 0 3px 3px 0;
}
.tab-icon { width: 18px; height: 18px; flex-shrink: 0; }
.settings-content { flex: 1; overflow-y: auto; padding: 14px 24px 24px; }
.settings-panel { animation: dc-fade-in .2s ease; }
.panel-title { font-weight: 700; font-size: 15px; margin-bottom: 6px; color: var(--dc-text-strong); }
.panel-desc { font-size: 13px; color: var(--dc-text-dim); margin-bottom: 18px; line-height: 1.7; }
.panel-desc code { background: var(--dc-bg-soft); padding: 1px 6px; border-radius: 4px; color: var(--dc-link); }

.ai-form { max-width: 800px; }
/* 数据目录只读展示：虚线淡底，样式上就表明「这里点不动」，避免用户反复尝试 */
.path-readonly {
  width: 100%; height: 32px; line-height: 30px;
  padding: 0 11px; border-radius: 6px;
  background: var(--dc-bg-soft); border: 1px dashed var(--dc-border);
  color: var(--dc-text-dim); font-size: 13px;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  cursor: default; user-select: text;
}
/* 表单提示**统一换行到第二行**：tip 与开关/输入框挤在同一行时会忽左忽右，
   全部占满一行才齐整（el-form-item 的内容区默认不换行，这里放开） */
.ai-form :deep(.el-form-item__content) { flex-wrap: wrap; }
.form-tip { width: 100%; font-size: 12px; color: var(--dc-text-weak); margin-top: 4px; line-height: 1.6; }
/* 顶栏菜单配置（通用页签）：与上一个区块（日志级别）隔开一档 */
.topmenu-title { margin-top: 26px; }
/* 顶栏菜单配置（通用页签） */
.topmenu-editor {
  border: 1px solid var(--dc-border, #dcdfe6); border-radius: 8px;
  padding: 6px; max-width: 460px; background: var(--dc-bg-soft, transparent);
}
.topmenu-row {
  display: flex; align-items: center; gap: 8px;
  padding: 4px 6px; border-radius: 6px;
}
.topmenu-row:hover { background: var(--dc-bg-hover, rgba(125, 125, 125, .08)); }
.topmenu-label {
  flex: 1; font-size: 13px; color: var(--dc-text); display: inline-flex; align-items: center;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.topmenu-ic { margin-right: 4px; color: var(--dc-text-dim); }
.topmenu-ops { display: inline-flex; }
.topmenu-ops .el-button + .el-button { margin-left: 2px; }
/* 缓存清单：一行一项，名称靠左、体量靠右 */
.cache-list { max-width: 560px; border: 1px solid var(--dc-border); border-radius: 8px; padding: 4px 0; }
.cache-item { display: flex; align-items: center; gap: 10px; padding: 9px 14px; cursor: pointer; }
.cache-item + .cache-item { border-top: 1px solid var(--dc-border); }
.cache-item:hover { background: var(--dc-hover, rgba(0,0,0,.03)); }
.cache-name { flex: 1; font-size: 13px; }
.cache-size { font-size: 12px; color: var(--dc-text-dim); font-variant-numeric: tabular-nums; }
.cache-empty { padding: 18px 14px; text-align: center; font-size: 12px; color: var(--dc-text-dim); }
/* 日志：级别选择行 + 筛选行 + 日志列表 */
.log-level-row { display: flex; align-items: center; gap: 12px; margin: 14px 0 6px; }
.log-level-label { font-size: 13px; color: var(--dc-text-mid); flex-shrink: 0; }
/* 紧凑行样式已随模板一并移除（运行时/审计级别恢复标题+说明+卡片的完整布局） */
.log-filter-row { display: flex; align-items: center; gap: 8px; margin: 10px 0; }
/* 复制按钮：常驻显示（行内最右列），hover 才提亮 */
.log-item .log-copy { flex-shrink: 0; color: var(--dc-text-dim); transition: color .12s; }
.log-item .log-copy:hover { color: var(--dc-primary); }
.log-count { font-size: 12px; color: var(--dc-text-dim); }
/* 日志面板：高度 = 内容区高度（settings-body 跟随视口），面板本身在极端矮视口下
   内部滚动（滚动条落在面板内、不在弹窗右缘），列表吃满剩余空间并在内部滚动，
   分页条紧随列表底部可见 */
.log-panel {
  height: 100%;
  display: flex; flex-direction: column;
  overflow-y: auto;
}
/* 日志页签：内容容器不留底部内边距 —— 面板底边与左侧页签栏底边落在同一条线上，
   列表顺势多占这 24px。:has 按 .log-panel 存在与否区分，其它页签保持原样 */
.settings-content:has(.log-panel) { padding-bottom: 0; }
.log-panel .log-list {
  flex: 1 1 auto;
  min-height: 120px;
  overflow-y: auto;
  border: 1px solid var(--dc-border);
  border-radius: 8px; background: var(--dc-bg-soft);
}
.log-panel .log-pager { display: flex; align-items: center; justify-content: flex-end; gap: 14px; margin: 10px 0 0; }
.log-panel .log-count { font-size: 12.5px; color: var(--dc-text-dim); font-variant-numeric: tabular-nums; }
.log-panel .el-pagination { justify-content: flex-end; }
.log-item {
  display: flex; align-items: center; gap: 10px;
  padding: 7px 12px; border-bottom: 1px solid var(--dc-border); font-size: 12.5px;
}
.log-item:last-child { border-bottom: none; }
.log-item:hover { background: var(--dc-bg-hover, rgba(148,163,184,.08)); }
.log-time { color: var(--dc-text-dim); flex-shrink: 0; font-variant-numeric: tabular-nums; }
.log-tag {
  flex-shrink: 0; padding: 1px 8px; border-radius: 999px; font-size: 11px; font-weight: 600;
  border: 1px solid var(--dc-border); color: var(--dc-text-mid);
}
.log-tag.k-query { color: #4f8cff; border-color: rgba(79,140,255,.4); }
.log-tag.k-write { color: #f59e0b; border-color: rgba(245,158,11,.45); }
.log-tag.k-ddl { color: #a78bfa; border-color: rgba(167,139,250,.45); }
.log-tag.k-tx { color: #2dd4bf; border-color: rgba(45,212,191,.45); }
.log-tag.k-ai { color: #f472b6; border-color: rgba(244,114,182,.45); }
.log-tag.k-exec { color: var(--dc-text-dim); }
.log-sql {
  flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  font-family: "SF Mono", ui-monospace, Consolas, monospace; color: var(--dc-text);
}
.log-conn { flex-shrink: 0; max-width: 130px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--dc-text-mid); }
.log-dur { flex-shrink: 0; color: var(--dc-text-dim); font-variant-numeric: tabular-nums; }
.log-status { flex-shrink: 0; font-size: 11px; font-weight: 600; }
.log-status.ok { color: var(--el-color-success); }
.log-status.canceled { color: var(--el-color-warning); }
.log-status.error { color: var(--el-color-danger); }
/* 日志级别：标题与「界面语言设置」同级，卡片整行铺开自动换行 */
/* 段标题的上边距只用于分隔页签内的两段内容；首个标题不留空隙，与其它设置页一致 */
.log-title { margin-top: 22px; }
.log-title:first-child { margin-top: 0; }
.log-level-options { display: flex; gap: 10px; flex-wrap: wrap; width: 100%; max-width: 560px; }
.log-opt { flex: 1 1 96px; min-width: 96px; padding: 9px 12px; }
.log-opt .theme-opt-name { font-family: var(--dc-mono, monospace); font-size: 13px; }
.log-tip { max-width: 560px; margin-top: 10px; }
.unit { font-size: 13px; color: var(--dc-text-mid); margin-left: 8px; }
.presets { margin-top: 8px; padding-top: 16px; border-top: 1px solid var(--dc-border); }
.preset-label { font-size: 12px; color: var(--dc-text-dim); margin-bottom: 8px; }
.preset-btns { display: flex; gap: 8px; flex-wrap: wrap; }
.actions { margin-top: 20px; display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }

/* ===== 驱动下载：数据源 × 驱动状态列表 ===== */
.drv-search { width: 180px; }
.drv-sum { margin-left: auto; font-size: 12px; color: var(--dc-text-dim); }
/* 实际下载源：展示真正的仓库根，链接样式弱化（可点开验证通不通） */
.mirror-link { font-size: 13px; word-break: break-all; }
.mirror-link a { color: var(--dc-primary, #409eff); text-decoration: none; }
.mirror-link a:hover { text-decoration: underline; }
.drv-group { margin-top: 16px; }
.drv-group-title {
  display: flex; align-items: center; gap: 6px;
  font-size: 12px; color: var(--dc-text-dim); margin-bottom: 6px;
}
.drv-group-count { color: var(--dc-text-dim); }
/* 行底色**不铺** var(--dc-bg-card)：设置弹窗本身是暗底，铺上去每一行都变成一块亮卡片
   （暗色主题下尤其刺眼）。改成透明 + 一条细边框：靠边框分组、靠 hover 提示可点。 */
.drv-row {
  display: flex; align-items: center; gap: 10px;
  padding: 7px 10px; margin-bottom: 6px;
  border: 1px solid var(--dc-border); border-radius: 8px; background: transparent;
  transition: border-color .15s ease, background .15s ease;
}
.drv-row:hover { background: rgba(127, 127, 127, .08); border-color: var(--dc-primary, #409eff); }
.drv-main { flex: 1; min-width: 0; }
.drv-name { display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--dc-text-strong); }
/* 下载中的行内进度条：与标题左对齐，不挤压右侧按钮 */
.drv-dl { display: flex; align-items: center; gap: 8px; margin-top: 6px; }
.drv-dl .el-progress { flex: 1; }
.drv-dl-txt { font-size: 11.5px; color: var(--dc-text-dim); white-space: nowrap; font-variant-numeric: tabular-nums; }
.drv-sub {
  font-size: 12px; color: var(--dc-text-dim); margin-top: 2px;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.drv-acts { display: flex; gap: 6px; flex-shrink: 0; }
/* 状态标签：不用 Element 的默认配色 —— 默认（含 effect="plain"）在暗底上会是白底/浅底，
   一行行看过去就是一片亮点。这里统一成**半透明底 + 语义色文字**，暗色主题下也不刺眼。
   选择器比 Element 自己的 .el-tag--xxx 多一层，稳妥压过它。 */
.drv-row :deep(.el-tag) {
  background: rgba(127, 127, 127, .14); border-color: transparent;
  color: var(--dc-text-mid);
}
.drv-row :deep(.el-tag.el-tag--success) {
  background: rgba(103, 194, 58, .16); color: var(--dc-success, #67c23a);
}
.drv-row :deep(.el-tag.el-tag--danger) {
  background: rgba(245, 108, 108, .16); color: var(--dc-danger, #f56c6c);
}
.drv-row :deep(.el-tag.el-tag--info) {
  background: rgba(144, 147, 153, .16); color: var(--dc-text-dim);
}

/* AI 模型 Tab 暗色主题 */
.model-tabs-wrap .el-tabs__header { margin-bottom: 12px; }
.model-tabs-wrap .el-tabs--card>.el-tabs__header { border-bottom: 1px solid var(--dc-border); }
.model-tabs-wrap .el-tabs--card>.el-tabs__header .el-tabs__nav { border: 1px solid var(--dc-border); border-radius: 6px; }
.model-tabs-wrap .el-tabs--card>.el-tabs__header .el-tabs__item { border-left: 1px solid var(--dc-border); background: var(--dc-bg-soft); color: var(--dc-text-dim); transition: none; }
.model-tabs-wrap .el-tabs--card>.el-tabs__header .el-tabs__item:first-child { border-left: none; }
.model-tabs-wrap .el-tabs--card>.el-tabs__header .el-tabs__item.is-active { background: var(--dc-bg-raised); color: var(--dc-text); border-bottom-color: var(--dc-border); }
.model-tabs-wrap .el-tabs--card>.el-tabs__header .el-tabs__item:hover { color: var(--dc-text); }
.model-tabs-wrap .el-tabs__item .is-icon-close { color: var(--dc-text-dim); }
.model-tabs-wrap .el-tabs__item .is-icon-close:hover { color: var(--dc-danger); background: transparent; }


.checkbox-group { display: flex; flex-wrap: wrap; gap: 8px 20px; }
.checkbox-group .el-checkbox { margin-right: 0; }
.settings-divider { margin: 22px 0 10px; border-top-color: var(--dc-border); }
.settings-divider .el-divider__text { font-size: 13px; font-weight: 600; color: var(--dc-text-dim); background: transparent; padding: 0; left: 0; }

.os-banner { display: flex; align-items: center; gap: 10px; margin-bottom: 18px; padding: 10px 14px; background: var(--dc-bg-soft); border: 1px solid var(--dc-border); border-radius: 8px; }
.os-tag { font-size: 12px; font-weight: 700; padding: 2px 10px; border-radius: 10px; background: var(--dc-primary-wash); color: var(--dc-primary); letter-spacing: .5px; flex-shrink: 0; }
.os-detail { font-size: 13px; color: var(--dc-text-dim); word-break: break-all; }

.driver-info { margin-top: 8px; padding: 10px 14px; background: var(--dc-bg-soft); border-radius: 8px; }
.di-row { font-size: 13px; color: var(--dc-text-dim); display: flex; align-items: center; gap: 6px; line-height: 1.8; }
.di-row .el-icon { color: var(--dc-primary); }

.shortcut-box { margin-top: 4px; padding: 12px 14px; background: var(--dc-bg-soft); border: 1px solid var(--dc-border); border-radius: 8px; }
.scb-head { display: flex; align-items: center; gap: 8px; margin-bottom: 2px; }
.scb-head .flex-spacer { flex: 1; }
.scb-title { font-weight: 700; font-size: 14px; color: var(--dc-text); }
.scb-hint { font-size: 12px; color: var(--dc-text-dim); }
.scb-group { margin-top: 14px; }
.scb-group-title { font-size: 12px; font-weight: 600; color: var(--dc-text-dim); letter-spacing: .5px; margin-bottom: 4px; }
.scb-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 6px 10px; border-radius: 6px; cursor: pointer; font-size: 13px; transition: background .15s; }
.scb-row:hover { background: var(--dc-primary-wash); }
.scb-row.recording { background: var(--dc-primary-wash); box-shadow: inset 0 0 0 1px var(--dc-primary); }
.scb-row.changed .scb-label { color: var(--dc-link); }
.scb-label { display: flex; flex-direction: column; gap: 1px; color: var(--dc-text); }
.scb-desc { font-size: 12px; color: var(--dc-text-weak); }
.scb-side { display: flex; align-items: center; gap: 6px; min-height: 24px; flex-shrink: 0; }
.scb-side kbd { font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; font-size: 12px; padding: 2px 6px; border-radius: 4px; background: var(--dc-bg-input); border: 1px solid var(--dc-border-strong); color: var(--dc-text); box-shadow: 0 1px 0 var(--dc-bg-deep); }
.scb-none { color: var(--dc-text-weak); font-size: 13px; }
.scb-rec-hint { color: var(--dc-link); font-size: 13px; }
.scb-clear { color: var(--dc-danger); font-size: 12px; cursor: pointer; margin-left: 4px; user-select: none; }
.scb-clear:hover { text-decoration: underline; }

/* SQL 格式化面板 */
.fmt-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; margin-bottom: 18px; }
.fmt-card {
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border); border-radius: 10px;
  padding: 14px 16px 10px;
}
.fmt-card-wide { grid-column: 1 / -1; }
.fmt-card-title {
  display: flex; align-items: center; gap: 6px;
  font-size: 14px; font-weight: 600; color: var(--dc-text); margin-bottom: 12px;
}
.fmt-card-title .el-icon { color: var(--dc-primary); }
.fmt-card-sub { font-size: 13px; font-weight: 400; color: var(--dc-text-dim); margin-left: 2px; }
.fmt-glob-cols { display: flex; align-items: stretch; gap: 12px 40px; margin-top: 2px; }
.fmt-glob-col { flex: 1 1 0; min-width: 0; display: flex; flex-direction: column; gap: 18px; }
/* 两列各自等高（flex 行的默认拉伸），**第二个小节贴底**：左列第一节 4 行、右列 3 行，
   内容排下去第二节的起始位置必然错开 —— 把末节推到底部后，两列末节行数相同（都是 2 行），
   「运算符」与「语句间隔」的标题就齐平了 */
.fmt-glob-col .fmt-glob-sec { min-width: 0; }
.fmt-glob-col .fmt-glob-sec + .fmt-glob-sec { margin-top: auto; }
.fmt-glob-sec { min-width: 0; }
.fmt-glob-sec-title {
  display: flex; align-items: center; gap: 6px;
  font-size: 13px; font-weight: 600; letter-spacing: .3px; color: var(--dc-text-dim);
  margin: 0 0 10px;
}
.fmt-glob-sec-title::before { content: ''; width: 3px; height: 13px; background: var(--dc-primary); border-radius: 2px; }
/* 每行 = 左标签 + 右侧 160px 固定槽（下拉/数字/开关都占同一个槽）：
   之前用 space-between，开关被撑到列最右缘、和下拉框（150px）数字框（140px）三种右缘互不对齐，
   开关行中间一大段空白，看着松散。统一槽位后左缘右缘都在一条线上。 */
.fmt-glob-it { display: flex; align-items: center; gap: 12px; margin-bottom: 14px; min-height: 32px; }
.fmt-glob-it:last-child { margin-bottom: 0; }
.fmt-glob-lb {
  font-size: 14px; color: var(--dc-text-mid); flex: 1 1 auto; overflow: hidden;
  text-overflow: ellipsis; white-space: nowrap;
}
.fmt-glob-sel { width: 160px; flex: 0 0 auto; font-size: 14px; }
.fmt-glob-num { width: 160px; flex: 0 0 auto; font-size: 14px; }
.fmt-glob-swslot { width: 160px; flex: 0 0 auto; display: flex; align-items: center; }
/* 归一化 Element 控件内部字号，与行标签保持一致 */
.fmt-glob-sel :deep(.el-select__wrapper),
.fmt-glob-sel :deep(.el-select__selected-item),
.fmt-glob-num :deep(.el-input__inner) { font-size: 14px; }
/* 整个格式化区统一基准字号，控件/按钮归一 */
.fmt-grid, .fmt-preview { font-size: 14px; }
.fmt-grid :deep(.el-button), .fmt-rule-add :deep(.el-button) { font-size: 14px; }
.fmt-tbl { display: flex; flex-direction: column; }
.fmt-tr { display: flex; align-items: center; gap: 10px; margin-bottom: 8px; }
.fmt-th {
  font-size: 14px; font-weight: 600; color: var(--dc-text-dim);
  border-bottom: 1px dashed var(--dc-border); padding-bottom: 6px; margin-bottom: 10px;
}
.fmt-tc-idx { flex: 0 0 36px; text-align: center; font-size: 14px; color: var(--dc-text-dim); }
.fmt-tc-kw { flex: 1 1 auto; min-width: 0; }
.fmt-tc-cs { flex: 0 0 118px; }
.fmt-tc-ly { flex: 0 0 226px; }
.fmt-tc-op { flex: 0 0 36px; }
.fmt-tc-kw :deep(.el-autocomplete) { width: 100%; }
.fmt-tc-kw :deep(.el-input) { width: 100%; }
.fmt-tc-kw :deep(.el-input__wrapper) { background: var(--dc-bg-code); box-shadow: 0 0 0 1px var(--dc-border) inset; }
/* 表格内控件字号与行标签统一 */
.fmt-tc-kw :deep(.el-input__inner),
.fmt-tc-cs :deep(.el-select__wrapper),
.fmt-tc-cs :deep(.el-select__selected-item),
.fmt-tc-ly :deep(.el-select__wrapper),
.fmt-tc-ly :deep(.el-select__selected-item) { font-size: 14px; }
.fmt-tbl-empty {
  font-size: 13px; color: var(--dc-text-dim); background: var(--dc-bg-code);
  border: 1px dashed var(--dc-border); border-radius: 8px; padding: 12px 14px;
  margin-bottom: 4px;
}
.fmt-rule-add { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; margin: 10px 0; }
.fmt-preview {
  background: var(--dc-bg-code); border: 1px solid var(--dc-border); border-radius: 10px;
  padding: 12px 14px;
}
.fmt-preview-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px; }
.fmt-preview-title { font-size: 14px; font-weight: 600; color: var(--dc-text); display: flex; align-items: center; gap: 6px; }
.fmt-preview-tip { font-size: 13px; color: var(--dc-text-dim); }
.fmt-preview-code {
  margin: 0; font-family: 'SF Mono', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  font-size: 13px; line-height: 1.7; color: var(--dc-code-text); white-space: pre-wrap; word-break: break-word;
}

/* AI 知识库 */
/* ---- AI 服务页签：用量统计卡片（知识库已移到顶部导航的独立工作台）---- */
.kb-card {
  border: 1px solid var(--dc-border); border-radius: var(--dc-radius);
  background: var(--dc-bg-soft); padding: 12px 14px; margin-bottom: 12px;
}
.kb-card-head { display: flex; align-items: center; gap: 8px; margin-bottom: 10px; }
.kb-card-title { font-size: 14px; font-weight: 600; color: var(--dc-text-strong); }
.kb-count { font-size: 12px; color: var(--dc-text-weak); font-variant-numeric: tabular-nums; }
.kb-usage { display: flex; gap: 10px; }
.kb-stat {
  flex: 1; display: flex; flex-direction: column; align-items: center; gap: 2px;
  padding: 12px 8px; border-radius: 10px; border: 1px solid var(--dc-border); background: var(--dc-bg-soft);
}
.kb-stat-n { font-size: 18px; font-weight: 700; color: var(--dc-text-strong); font-variant-numeric: tabular-nums; }
.kb-stat-l { font-size: 12px; color: var(--dc-text-dim); }
/* 按模型汇总：一行一个模型；名字可省略，次数 / token 用等宽数字便于竖向比较 */
.kb-models { margin-top: 10px; border-top: 1px solid var(--dc-border); padding-top: 10px; }
.kb-models-head { display: flex; align-items: baseline; gap: 8px; margin-bottom: 6px; }
.kb-models-title { font-size: 13px; font-weight: 600; color: var(--dc-text-strong); }
.kb-models-note { font-size: 11px; color: var(--dc-text-weak); }
.kb-models-list { display: flex; flex-direction: column; gap: 4px; }
.kb-model {
  display: flex; align-items: center; gap: 10px; font-size: 12px;
  padding: 4px 8px; border-radius: 6px; background: var(--dc-bg-soft);
}
.kb-model-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--dc-text); }
.kb-model-calls { color: var(--dc-text-dim); font-variant-numeric: tabular-nums; }
.kb-model-tokens { color: var(--dc-text-strong); font-variant-numeric: tabular-nums; }
.kb-model-empty { font-size: 12px; color: var(--dc-text-weak); }
/* 「开关 + 说明」并排的行（如「调用审计」）：说明文字右移一点，别贴着开关。
   选择器带上 .ai-form 是为了**不受书写顺序影响**地压过 .form-tip 自己的边距 */
.ai-form .form-tip.tip-inline { margin-left: 14px; }

/* 关于 */
.about-hero { display: flex; align-items: center; gap: 12px; margin-bottom: 20px; }
/* 「关于」页的品牌图：这里原本是"蓝色渐变方块 + Element Plus 的 Monitor 矢量图标"
   —— 那个图标不是 logo。现在直接放真正的品牌图：图自带底板，所以去掉渐变与投影，
   尺寸沿用 46px、圆角 13px 与方块时代一致，视觉位置不变。 */
.about-logo {
  width: 46px; height: 46px; border-radius: 13px; display: inline-flex;
  align-items: center; justify-content: center; flex-shrink: 0;
}
.about-logo img {
  width: 46px; height: 46px; border-radius: 13px;
  object-fit: contain; display: block;
}
.about-name { font-size: 17px; font-weight: 700; color: var(--dc-text-strong); letter-spacing: .3px; line-height: 1.2; }
.about-ver {
  font-size: 13px; color: var(--dc-text-dim); margin-top: 2px;
  font-variant-numeric: tabular-nums;
}
.about-rows {
  border: 1px solid var(--dc-border); border-radius: 10px;
  background: var(--dc-bg-soft); overflow: hidden;
}
.about-row { display: flex; align-items: center; gap: 12px; padding: 10px 14px; border-bottom: 1px solid var(--dc-border); }
.about-row:last-child { border-bottom: none; }
.about-k { width: 64px; flex-shrink: 0; font-size: 13px; color: var(--dc-text-dim); }
.about-v {
  font-family: 'SF Mono', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  font-size: 13px; color: var(--dc-link); overflow-wrap: anywhere; user-select: all;
}

/* 关于页板块标题：与其他设置页签的 .panel-title 同字号同字重（15px/700），无分隔横线 */
.about-dbs-title {
  font-size: 15px; font-weight: 700; color: var(--dc-text-strong, var(--dc-text));
  margin-bottom: 6px;
}
/* 版本说明（折叠）与开源说明块 */
.about-block { margin-top: 20px; }
/* 版本号卡片下拉：整卡圆角底色，头行显示版本号 + 右侧箭头，展开显示说明列表 */
.about-fold {
  border-radius: 10px; background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border); overflow: hidden;
}
.about-fold-head {
  width: 100%; height: 40px; padding: 0 14px;
  display: flex; align-items: center; justify-content: space-between;
  background: transparent; border: none; cursor: pointer;
  font-size: 13px; font-weight: 600; color: var(--dc-text); text-align: left;
  transition: background .15s;
}
.about-fold-head:hover { background: var(--dc-bg-hover, rgba(148,163,184,.12)); }
.about-fold-arrow { color: var(--dc-text-dim); font-size: 14px; transition: transform .2s; }
.about-fold.open .about-fold-arrow { transform: rotate(180deg); }
.about-fold-body { padding: 2px 14px 12px; }
/* 版本号三段式：大版本.小版本.修订，悬停每段看含义 */
.ver-prefix { margin-right: 2px; }
.ver-seg { font-weight: 700; color: var(--dc-text-strong, var(--dc-text)); cursor: help; border-bottom: 1px dotted var(--dc-text-dim); }
.ver-dot { color: var(--dc-text-dim); font-weight: 700; }
/* 版本说明：版本号 + 类型标签 + 说明行 + 左侧时间线竖线 */
.about-vitem { position: relative; padding-left: 16px; }
/* 每个版本项上方一条实线：把各版本横向切齐，扫读更清楚 */
.about-vitem + .about-vitem { margin-top: 14px; padding-top: 14px; }
.about-vitem::after {
  content: ""; position: absolute; left: 16px; right: 0; top: 0; height: 1px;
  background: var(--dc-border);
}
.about-vitem:first-child { padding-top: 14px; }
/* 时间线竖线：从本版本圆点中心向下贯穿到下一版本圆点（top 落在圆点中心，
   所以最新版圆点上方不会露出一小截线头） */
.about-vitem::before {
  content: ""; position: absolute; left: 5px; top: 23px; bottom: -23px;
  width: 1px; background: var(--dc-border);
}
.about-vitem:last-child::before { bottom: 18px; }
/* 节点圆点：与版本号同一水平线，落在左侧缩进区骑在竖线上
   （left 用负值从版本行拉回 vitem 的缩进带，-16px = .about-vitem 的 padding-left） */
.about-rel-ver { position: relative; }
.about-rel-ver::before {
  content: ""; position: absolute; left: -16px; top: 50%;
  width: 11px; height: 11px; margin-top: -5.5px; border-radius: 50%;
  background: var(--dc-primary, #4f8cff); box-shadow: 0 0 0 3px var(--dc-bg-soft);
}
/* 首个正式版用中性灰节点，与后续修订的蓝色区分 */
.about-vitem:first-of-type .about-rel-ver::before { background: var(--dc-text-dim); }
/* 首个正式版（1.0.0）用中性灰，与后续修订的蓝色区分 */
.about-vitem:first-of-type .about-vdot { background: var(--dc-text-dim); }
/* 版本行用四列网格：圆点 | 版本号 | 标签 | 发布日期。
   固定列宽保证「版本号 / 标签 / 日期」在各版本之间垂直对齐（v1.0.1 与 v1.0.0 同一列） */
.about-rel-ver {
  display: grid; grid-template-columns: 58px 128px 1fr;
  align-items: center; column-gap: 8px;
  font-size: 13px; font-weight: 700; color: var(--dc-text); margin-bottom: 8px;
  font-family: var(--dc-mono, ui-monospace, Consolas, monospace);
}
.about-rel-no { white-space: nowrap; }
.about-rel-tags { display: flex; align-items: center; gap: 6px; }
/* 发布日期：右对齐灰色小字 */
.about-rel-date { justify-self: end; font-size: 12px; font-weight: 400; color: var(--dc-text-dim); }
.about-rel-tag {
  font-family: var(--dc-text, inherit); font-size: 11px; font-weight: 600;
  color: var(--dc-primary, #4f8cff); background: var(--dc-primary-wash, rgba(79,140,255,.12));
  border-radius: 999px; padding: 1px 8px;
}
/* 版本类型标签配色：修订=绿、小版本=蓝、大版本=橙、首个版本=灰 */
.about-rel-tag.tag-patch { color: #16a34a; background: rgba(34, 197, 94, .14); }
.about-rel-tag.tag-minor { color: #2563eb; background: rgba(59, 130, 246, .14); }
.about-rel-tag.tag-major { color: #d97706; background: rgba(245, 158, 11, .16); }
.about-rel-tag.tag-first { color: var(--dc-text-dim); background: rgba(148, 163, 184, .16); }
.about-rel-tag.tag-latest { color: var(--dc-primary, #4f8cff); background: var(--dc-primary-wash, rgba(79,140,255,.12)); }
/* 规则说明按其他页签 .panel-desc 的字号（13px） */
.about-ver-rule { margin: 0 0 18px; font-size: 13px; color: var(--dc-text-dim); line-height: 1.7; }
.about-block-body { font-size: 13px; color: var(--dc-text-mid); line-height: 1.8; }
/* 说明行：浅底条目，比裸文字更整齐；左边缘与版本号列对齐（缩进 = 圆点+版本号列宽） */
.about-release-line {
  position: relative; padding: 6px 10px 6px 22px; margin: 0 0 4px 16px;
  background: var(--dc-bg-soft); border-radius: 6px;
  font-size: 13px; color: var(--dc-text-mid, var(--dc-text-dim)); line-height: 1.65;
}
.about-release-line::before { content: '·'; position: absolute; left: 10px; color: var(--dc-primary, #4f8cff); font-weight: 700; }
.about-license { white-space: pre-line; margin-bottom: 8px; }
.about-link { color: var(--dc-primary); text-decoration: none; font-size: 13px; }
.about-link:hover { text-decoration: underline; }
</style>

<style>
/* ---------- 主题选择卡片（仅出现在设置弹窗内） ---------- */
.theme-options { display: flex; gap: 16px; margin-top: 6px; flex-wrap: wrap; }
.theme-opt {
  position: relative; width: 206px; cursor: pointer;
  border: 1.5px solid var(--dc-border); border-radius: 12px;
  background: var(--dc-bg-soft); padding: 10px 10px 12px;
  transition: border-color .18s ease, transform .18s ease, box-shadow .18s ease;
}
.theme-opt:hover { border-color: var(--dc-border-strong); transform: translateY(-1px); }
.theme-opt.active { border-color: var(--dc-primary); box-shadow: 0 0 0 3px var(--dc-primary-wash); }
.theme-prev {
  position: relative; display: flex; height: 84px; margin-bottom: 9px;
  border-radius: 8px; overflow: hidden; border: 1px solid var(--dc-border-soft);
}
.pv-sidebar { width: 34%; height: 100%; }
.pv-main { position: relative; flex: 1; height: 100%; padding: 11px 9px; display: flex; flex-direction: column; gap: 7px; }
.pv-line { display: block; height: 7px; width: 100%; border-radius: 4px; }
.pv-line.short { width: 58%; }
.pv-ctl {
  position: absolute; right: 8px; bottom: 8px; width: 52px; height: 22px;
  border-radius: 11px; border: 1px solid transparent;
}
.pv-ctl::after {
  content: ''; position: absolute; top: 4px; width: 14px; height: 14px; border-radius: 50%;
  background: var(--dc-primary); box-shadow: 0 1px 3px rgba(0, 0, 0, .2);
}
/* 跟随系统：半边浅色半边深色 */
.prev-system .pv-sidebar { background: #1f222b; }
.prev-system .pv-main { background: #eef1f6; }
.prev-system .pv-line { background: #c6cdd8; }
.prev-system .pv-ctl { background: #dfe4ee; border-color: #cfd6e2; }
.prev-system .pv-ctl::after { left: 4px; }
/* 浅色 */
.prev-light .pv-sidebar { background: #e2e7f0; }
.prev-light .pv-main { background: #ffffff; }
.prev-light .pv-line { background: #e6ebf3; }
.prev-light .pv-ctl { background: #f1f4f9; border-color: #cfd6e2; }
.prev-light .pv-ctl::after { left: 4px; }
/* 深色 */
.prev-dark .pv-sidebar { background: #1f222b; }
.prev-dark .pv-main { background: #12151b; }
.prev-dark .pv-line { background: #2c3140; }
.prev-dark .pv-ctl { background: #242833; border-color: #3a3f4f; }
.prev-dark .pv-ctl::after { left: 4px; }
.theme-opt-info { padding-right: 4px; }
.theme-opt-name { color: var(--dc-text-strong); font-size: 14px; font-weight: 600; }
.theme-opt-desc { color: var(--dc-text-dim); font-size: 12px; line-height: 1.5; margin-top: 2px; }
.theme-opt-check {
  position: absolute; top: 8px; right: 8px; font-size: 18px;
  color: var(--dc-primary); background: var(--dc-bg-soft); border-radius: 50%;
}
.theme-hint {
  display: flex; align-items: center; gap: 6px; margin-top: 14px;
  color: var(--dc-text-dim); font-size: 13px;
}

/* 覆盖 el-dialog 在暗色主题下的样式 */
.settings-dialog .el-dialog__header { margin-right: 0; padding: 16px 20px; border-bottom: 1px solid var(--dc-border); }
/* 内容区不再写死高度、不再自己滚动：弹窗高度完全由内容决定（各页签不被统一拉高），
   也没有弹窗级滚动条；日志页签的列表高度在 scoped 样式里按视口反推，超长只在列表内滚 */
.settings-dialog .el-dialog__body { padding: 0; }
/* 内容放不下时允许滚动，但滚动条不显示（浏览器缩放/小窗下也不出难看的竖条） */
.settings-dialog .el-dialog__body { scrollbar-width: none; -ms-overflow-style: none; }
.settings-dialog .el-dialog__body::-webkit-scrollbar { width: 0; height: 0; display: none; }
.settings-dialog .el-dialog__headerbtn { top: 4px; right: 8px; }
.fmt-preview-note { color: var(--dc-text-dim); }
.fmt-sql-kw { color: var(--dc-sql-kw); font-weight: 600; }
.fmt-kw-hl { background: var(--dc-sql-hl-wash); color: var(--dc-sql-hl); font-weight: 700; text-decoration: underline; text-decoration-color: var(--dc-sql-hl); }
</style>