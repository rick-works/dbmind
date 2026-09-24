<template>
  <el-dialog
    v-model="visible"
    width="980px"
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
              <!-- 这一行是"开关 + 说明"并排（el-form-item__content 是 flex），
                   文案紧贴着开关；加 tip-inline 把它往右挪开一点 -->
              <div class="form-tip tip-inline">{{ $t('settings.ai.auditTip') }}</div>
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
            <el-form-item :label="$t('settings.editor.tabSize')">
              <el-select v-model="editorForm.tabSize" style="width:120px">
                <el-option :value="2" :label="$t('settings.editor.spaces', { n: 2 })" />
                <el-option :value="4" :label="$t('settings.editor.spaces', { n: 4 })" />
              </el-select>
            </el-form-item>
            <el-form-item :label="$t('settings.editor.lineNumbers')">
              <el-checkbox v-model="editorForm.lineNumbers" />
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
                      <el-switch v-model="editorForm.sqlDenseOperators" />
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
                      <el-switch v-model="editorForm.sqlUseTabs" />
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
                      <el-switch v-model="editorForm.sqlNewlineBeforeSemicolon" />
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
            <el-form-item :label="$t('settings.query.confirmDanger')">
              <el-checkbox v-model="queryForm.confirmDanger">
                {{ $t('settings.query.confirmDangerLabel') }}
              </el-checkbox>
            </el-form-item>
          </el-form>

          <div class="actions">
            <el-button type="primary" @click="saveQuery">
              <el-icon style="margin-right:4px"><Check /></el-icon>{{ $t('settings.query.save') }}
            </el-button>
            <el-button text @click="resetQuery">{{ $t('settings.editor.reset') }}</el-button>
          </div>
        </div>

        <!-- 4. 驱动下载 -->
        <div v-show="activeTab === 'driver'" class="settings-panel">
          <div class="panel-title">{{ $t('settings.driver.title') }}</div>
          <div class="panel-desc">{{ $t('settings.driver.desc') }}</div>

          <el-form label-width="110px" label-position="left" class="ai-form">
            <el-form-item :label="$t('settings.driver.mirror')">
            <el-select v-model="driverForm.mirror" style="width:200px">
              <!-- 镜像商名是**专有名词**：Maven Central 哪个语言都这么写；
                   阿里云 / 华为云 / 腾讯云 在英文界面下用它们自己的英文名 -->
              <el-option value="maven" label="Maven Central" />
              <el-option value="aliyun" :label="$t('settings.driver.mirrorAliyun')" />
              <el-option value="huawei" :label="$t('settings.driver.mirrorHuawei')" />
              <el-option value="tencent" :label="$t('settings.driver.mirrorTencent')" />
            </el-select>
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
                <el-tag v-if="t.builtin" size="small" type="info">{{ $t('settings.driver.tagBuiltin') }}</el-tag>
                <el-tag v-else-if="t.ready" size="small" type="success">{{ $t('settings.driver.tagReady') }}</el-tag>
                <el-tag v-else size="small" type="danger">{{ $t('settings.driver.tagMissing') }}</el-tag>
              </div>
              <!-- t.tip 来自后端（getDriverTypes），前端无从翻译 —— 那是"后端文案"那一档，
                   要翻得后端按 Accept-Language 返回，见 i18n.js 头注释里的说明 -->
              <div class="drv-sub" :title="t.tip">{{ t.tip }}</div>
            </div>
            <div class="drv-acts">
              <el-button v-if="!t.builtin" size="small" :loading="driverBusy === t.code"
                         @click="downloadDriver(t)">
                <el-icon style="margin-right:4px"><Download /></el-icon>{{ $t('settings.driver.download') }}
              </el-button>
              <el-button v-if="!t.builtin" size="small" :loading="driverBusy === t.code + ':up'"
                         @click="pickDriverFile(t)">
                <el-icon style="margin-right:4px"><Upload /></el-icon>{{ $t('settings.driver.upload') }}
              </el-button>
              <el-button v-if="t.dir" size="small" text @click="openDriverDir(t)">{{ $t('common.openDir') }}</el-button>
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
            <!-- 两个目录都只读展示：能看到文件在哪、又没有误操作面。
                 后端能力（含迁移清单与安全策略）都保留着，要放开时把 PATH_SETTINGS_EDITABLE 改成 true 即可 -->
            <el-form-item :label="$t('settings.paths.dataDir')">
              <el-input v-if="PATH_SETTINGS_EDITABLE" v-model="pathForm.dataDir">
                <template #append>
                  <el-button @click="resetDataDir">{{ $t('settings.editor.reset') }}</el-button>
                </template>
              </el-input>
              <div v-else class="path-readonly" :title="pathInfo.dataDir">{{ pathInfo.dataDir }}</div>
            </el-form-item>
            <el-form-item :label="$t('settings.paths.driverDir')">
              <el-input v-if="PATH_SETTINGS_EDITABLE" v-model="pathForm.driverDir">
                <template #append>
                  <el-button @click="resetDriverDir">{{ $t('settings.editor.reset') }}</el-button>
                </template>
              </el-input>
              <div v-else class="path-readonly" :title="pathInfo.driverDir">{{ pathInfo.driverDir }}</div>
            </el-form-item>
          </el-form>



          <div v-if="PATH_SETTINGS_EDITABLE" class="actions">
            <el-button type="primary" :loading="savingPath" @click="savePaths">
              <el-icon style="margin-right:4px"><Check /></el-icon>{{ $t('settings.paths.save') }}
            </el-button>
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
            <span class="about-logo"><img :src="logoMdUrl" alt="DBMind" /></span>
            <div class="about-meta">
              <div class="about-name">{{ $t('settings.about.name') }}</div>
              <div class="about-ver">{{ $t('settings.about.version', { version: APP_VERSION }) }}</div>
            </div>
          </div>

          <div class="about-rows">
            <div class="about-row">
              <span class="about-k">{{ $t('settings.paths.dataDir') }}</span>
              <code class="about-v">{{ pathInfo.dataDir }}</code>
            </div>
            <div class="about-row">
              <span class="about-k">{{ $t('settings.paths.driverDir') }}</span>
              <code class="about-v">{{ pathInfo.driverDir }}</code>
            </div>
          </div>

          <div class="about-dbs">
            <div class="about-dbs-title">{{ $t('settings.about.supportedDbs') }}</div>
            <div class="about-dbs-list">
              <!-- 数据库名（MySQL / Oracle / 达梦 DM…）是**产品名**，不翻译：英文界面里也这么写 -->
              <span v-for="d in aboutDbs" :key="d" class="about-db">{{ d }}</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  </el-dialog>

</template>

<script setup>
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  MagicStick, Connection, Brush,
  FolderOpened, EditPen, DataLine, Download, Bell, Operation, Monitor, Plus, Delete,
  CircleCheck, Timer, Moon, Pointer, Refresh, Search, Upload, Flag, InfoFilled
} from '@element-plus/icons-vue'
// `LOCALES` 直接当语言选项用：它里面每个选项的 label 都写着自己的语言，不需要再包一层
import { LOCALES as localeOptions, locale, setLocale, t } from '../../utils/i18n'
import { formatSql, keywordCandidates, sqlKeywordPattern } from '../../utils/sqlFormat'
import { getAiConfig, saveAiConfig, aiChat, getPathSettings, savePathSettings, browseBackupDirs, getDriverMirror, saveDriverMirror, getDriverTypes, getDriverStatus, installDriver, uploadDriver, openLocalDir, getAiUsage } from '../../api'
import { editorDefaults, queryDefaults, notifyDefaults, migrateEditor } from '../../utils/settings'
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

const props = defineProps({ modelValue: Boolean })
const emit = defineEmits(['update:modelValue'])

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const activeTab = ref('ai')

// 页签只存 i18nKey，不存中文原文 —— 文案统一由字典给（见 utils/i18n.js）。
// 存一份中文再「顺便翻译」等于同一句话有两个真相，改文案时必然漏掉一处。
const tabs = [
  { key: 'general', i18nKey: 'settings.tab.general', icon: Flag },
  { key: 'ai', i18nKey: 'settings.tab.ai', icon: MagicStick },
  { key: 'theme', i18nKey: 'settings.tab.theme', icon: Moon },
  { key: 'editor', i18nKey: 'settings.tab.editor', icon: EditPen },
  { key: 'format', i18nKey: 'settings.tab.format', icon: Brush },
  { key: 'query', i18nKey: 'settings.tab.query', icon: DataLine },
  { key: 'driver', i18nKey: 'settings.tab.driver', icon: Download },
  { key: 'paths', i18nKey: 'settings.tab.paths', icon: FolderOpened },
  { key: 'notify', i18nKey: 'settings.tab.notify', icon: Bell },
  { key: 'shortcut', i18nKey: 'settings.tab.shortcut', icon: Pointer },
  { key: 'about', i18nKey: 'settings.tab.about', icon: Monitor }
]

// 「关于」页展示的数据库清单（与 src/types 注册表保持一致）
const aboutDbs = [
  'MySQL', 'MariaDB', 'PostgreSQL', 'SQLite', 'H2', 'Apache Doris', 'ClickHouse',
  'MongoDB', 'Redis', 'Elasticsearch', 'SQL Server', 'Oracle', 'DB2', '达梦 DM', 'Kingbase', 'Derby'
]

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
 * 2. 账本（`~/.dbmind/ai-usage.json`）只记「每天调用了几次」，
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
  const newModel = { id: genId(), name: '', baseUrl: '', apiKey: '', model: '', embedding: false }
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
/**
 * 是否开放「数据目录 / 驱动目录」的修改入口。
 *
 * 换数据目录会牵动连接库、AI 配置、提示词、备份/还原等多处数据；换驱动目录则要搬驱动 jar，
 * 且 SQL Server 集成认证的原生库路径（桌面壳注入的 {@code -Djava.library.path}）要重启才更新。
 * 后端已经把保护做齐了（只复制不删除源、目标已有文件不覆盖、全部成功才切换、失败保持原目录），
 * 但当前版本先不开放这两个入口更稳妥：设置页只**只读展示**路径，用户知道文件在哪、又没有误操作面。
 *
 * 以后要放开，把这里改成 true 即可（后端接口、迁移清单与安全策略都已就绪）。
 */
const PATH_SETTINGS_EDITABLE = false
const pathInfo = ref({ os: '', osLabel: '', homeDir: '', defaultDataDir: '', defaultDriverDir: '', dataDir: '', driverDir: '', exportDir: '' })
const pathForm = ref({ dataDir: '', driverDir: '' })
const savingPath = ref(false)
const osTagText = computed(() => ({ windows: 'Windows', mac: 'macOS', linux: 'Linux', other: t('settings.paths.osOther') }[pathInfo.value.os] || t('settings.paths.osUnknown')))

const loadPaths = async () => {
  try {
    const info = await getPathSettings()
    pathInfo.value = info
    pathForm.value = { dataDir: info.dataDir, driverDir: info.driverDir }
    // 空 = 用默认目录；有值 = 用户指定过
  } catch (e) { ElMessage.error(t('settings.paths.msgLoadFailed', { detail: (e?.message || e?.toString?.() || t('common.unknownError')) })) }
}

const resetDataDir = () => { pathForm.value.dataDir = pathInfo.value.defaultDataDir }
const resetDriverDir = () => { pathForm.value.driverDir = pathInfo.value.defaultDriverDir }


/**
 * 保存路径设置。
 *
 * 后端保证：数据目录迁移「全部成功才切换」，任何一项失败都会保持原目录不变。
 * 所以这里必须把情况分开说清楚 —— 失败时不能只弹"已保存"，否则用户会以为搬好了。
 */
const savePaths = async () => {
  savingPath.value = true
  try {
    // 数据目录入口已隐藏：提交时始终带当前实际值，避免空值被后端当成「恢复默认」而触发迁移
    const info = await savePathSettings({
      dataDir: pathInfo.value.dataDir || pathForm.value.dataDir,
      driverDir: pathForm.value.driverDir
    })
    pathInfo.value = info
    pathForm.value = { dataDir: info.dataDir, driverDir: info.driverDir }
    const m = info.migration || {}

    if (m.failed && m.failed.length) {
      savingPath.value = false
      // 明确说清「哪个目录没切换」——后端保证失败的那个目录保持原值，用户不至于以为已经生效
      const lines = []
      if (m.dataDirSwitched === false) lines.push(t('settings.paths.notSwitched', { name: t('settings.paths.dataDir'), dir: info.dataDir }))
      if (m.driverDirSwitched === false) lines.push(t('settings.paths.notSwitched', { name: t('settings.paths.driverDir'), dir: info.driverDir }))
      await ElMessageBox.alert(
        (lines.length ? lines.join('\n') + '\n\n' : '')
        + t('settings.paths.migFailedHead') + '\n- ' + m.failed.join('\n- ')
        + t('settings.paths.migFailedTail'),
        t('settings.paths.saveIncomplete'),
        { confirmButtonText: t('common.gotIt'), customStyle: { whiteSpace: 'pre-line' } }
      ).catch(() => {})
      return
    }

    const bits = []
    // 分隔符取自字典：中文的「、」「；」放到英文里会很扎眼，反之亦然
    if (m.moved && m.moved.length) bits.push(t('settings.paths.migCopied', { n: m.moved.length }))
    if (m.kept && m.kept.length) bits.push(t('settings.paths.migKept', { n: m.kept.length }))
    if (m.notMigrated && m.notMigrated.length) {
      bits.push(t('settings.paths.migStayed', { names: m.notMigrated.join(t('common.listSep')) }))
    }
    const detail = bits.join(t('common.detailSep'))
    ElMessage.success(detail
      ? t('settings.paths.msgSavedWith', { detail })
      : t('settings.paths.msgSaved'))
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
const saveEditor = () => {
  localStorage.setItem('dbmind_editor', JSON.stringify(editorForm.value))
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
const loadQuery = () => {
  try {
    const raw = JSON.parse(localStorage.getItem('dbmind_query') || '{}')
    queryForm.value = { ...queryDefaults, ...raw }
  } catch { queryForm.value = { ...queryDefaults } }
}
const saveQuery = () => {
  localStorage.setItem('dbmind_query', JSON.stringify(queryForm.value))
  ElMessage.success(t('settings.query.msgSaved'))
}
const resetQuery = () => { queryForm.value = { ...queryDefaults }; saveQuery() }

// 驱动下载镜像源：后端持久化于 settings.json，保存后立即生效
const driverForm = ref({ mirror: 'maven' })
const loadDriver = async () => {
  try {
    const d = await getDriverMirror()
    driverForm.value.mirror = (d && d.mirror) || 'maven'
  } catch (e) { ElMessage.error(t('settings.driver.msgLoadFailed', { detail: (e?.message || e?.toString?.() || t('common.unknownError')) })) }
}
const saveDriver = async () => {
  try {
    const d = await saveDriverMirror(driverForm.value.mirror)
    driverForm.value.mirror = (d && d.mirror) || 'maven'
    ElMessage.success(t('settings.driver.msgSaved'))
  } catch (e) { ElMessage.error(t('settings.driver.msgSaveFailed', { detail: (e?.message || e?.toString?.() || t('common.unknownError')) })) }
}

// ===== 支持的数据源 × 驱动状态（手动下载 / 上传）=====
const driverTypes = ref([])
const driverStatus = ref({})
const driverLoading = ref(false)
const driverBusy = ref('')        // 正在下载/上传的类型 code（上传时是 `<code>:up`）
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
  try {
    const d = await installDriver(t.code)
    ElMessage.success(d?.message || t('settings.driver.msgDownloaded', { label: t.label }))
    await loadDriverList()
  } catch (e) {
    ElMessage.error(e?.message || t('settings.driver.msgDownloadFailed'))
  } finally {
    driverBusy.value = ''
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
  localStorage.setItem('dbmind_notify', JSON.stringify(notifyForm.value))
  ElMessage.success(t('settings.notify.msgSaved'))
}

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
    load()
    loadPaths()
    loadEditor()
    loadQuery()
    loadDriver()
    loadDriverList()
    loadNotify()
    loadKnowledge()
    shortcutMap.value = loadShortcuts()
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

.settings-body { display: flex; height: 560px; }
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

.ai-form { max-width: 560px; }
/* 数据目录只读展示：虚线淡底，样式上就表明「这里点不动」，避免用户反复尝试 */
.path-readonly {
  width: 100%; height: 32px; line-height: 30px;
  padding: 0 11px; border-radius: 6px;
  background: var(--dc-bg-soft); border: 1px dashed var(--dc-border);
  color: var(--dc-text-dim); font-size: 13px;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  cursor: default; user-select: text;
}
.form-tip { font-size: 12px; color: var(--dc-text-weak); margin-top: 4px; }
.unit { font-size: 13px; color: var(--dc-text-mid); margin-left: 8px; }
.presets { margin-top: 8px; padding-top: 16px; border-top: 1px solid var(--dc-border); }
.preset-label { font-size: 12px; color: var(--dc-text-dim); margin-bottom: 8px; }
.preset-btns { display: flex; gap: 8px; flex-wrap: wrap; }
.actions { margin-top: 20px; display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }

/* ===== 驱动下载：数据源 × 驱动状态列表 ===== */
.drv-search { width: 180px; }
.drv-sum { margin-left: auto; font-size: 12px; color: var(--dc-text-dim); }
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
.fmt-glob-cols { display: flex; align-items: flex-start; gap: 12px 48px; margin-top: 2px; }
.fmt-glob-col { flex: 1 1 0; min-width: 0; display: flex; flex-direction: column; gap: 24px; }
.fmt-glob-sec { min-width: 0; }
.fmt-glob-sec-title {
  display: flex; align-items: center; gap: 6px;
  font-size: 14px; font-weight: 500; letter-spacing: .3px; color: var(--dc-text-dim);
  margin: 2px 0 12px;
}
.fmt-glob-sec-title::before { content: ''; width: 3px; height: 13px; background: var(--dc-primary); border-radius: 2px; }
.fmt-glob-it { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
.fmt-glob-it:last-child { margin-bottom: 0; }
.fmt-glob-lb {
  font-size: 14px; color: var(--dc-text-mid); flex: 0 1 auto; overflow: hidden;
  text-overflow: ellipsis; white-space: nowrap;
}
.fmt-glob-sel { width: 150px; flex: 0 0 auto; font-size: 14px; }
.fmt-glob-num { width: 140px; flex: 0 0 auto; font-size: 14px; }
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
  max-width: 620px; border: 1px solid var(--dc-border); border-radius: 10px;
  background: var(--dc-bg-soft); overflow: hidden;
}
.about-row { display: flex; align-items: center; gap: 12px; padding: 10px 14px; border-bottom: 1px solid var(--dc-border); }
.about-row:last-child { border-bottom: none; }
.about-k { width: 64px; flex-shrink: 0; font-size: 13px; color: var(--dc-text-dim); }
.about-v {
  font-family: 'SF Mono', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  font-size: 13px; color: var(--dc-link); overflow-wrap: anywhere; user-select: all;
}
.about-dbs { margin-top: 20px; }
.about-dbs-title { font-size: 13px; color: var(--dc-text-dim); margin-bottom: 10px; }
.about-dbs-list { display: flex; flex-wrap: wrap; gap: 8px; max-width: 640px; }
.about-db {
  font-size: 13px; padding: 3px 10px; border-radius: 999px;
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border); color: var(--dc-text-mid);
}
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
.settings-dialog .el-dialog__body { padding: 0; }
.settings-dialog .el-dialog__headerbtn { top: 4px; right: 8px; }
.fmt-preview-note { color: var(--dc-text-dim); }
.fmt-sql-kw { color: var(--dc-sql-kw); font-weight: 600; }
.fmt-kw-hl { background: var(--dc-sql-hl-wash); color: var(--dc-sql-hl); font-weight: 700; text-decoration: underline; text-decoration-color: var(--dc-sql-hl); }
</style>