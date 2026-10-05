<template>
  <div class="sql-query-view" :class="{ 'with-result': resultVisible }" ref="rootRef"
       :style="resultVisible ? { '--editor-ratio': String(editorRatio) } : null">
    <div class="editor-area">
      <div class="editor-head">
        <!-- 连接选择器 -->
        <div class="selector-group">
          <el-select v-model="selectedConnId" size="small" class="conn-select"
                     @change="onConnChange">
            <template #prefix>
              <DbLogo v-if="selectedConn" :type="selectedConn.type" :size="16" class="sel-logo" />
              <el-icon v-else class="sel-icon"><Connection /></el-icon>
            </template>
            <el-option-group v-for="g in connGroups" :key="g.folder" :label="g.folder">
              <el-option v-for="c in g.items" :key="c.id" :label="c.name" :value="c.id">
                <div class="opt-row">
                  <DbLogo :type="c.type" :size="16" />
                  <span class="opt-name">{{ c.name }}</span>
                </div>
              </el-option>
            </el-option-group>
          </el-select>

          <!-- Catalog 选择器（仅 catalog 方言，如 Doris）：与库拆成两个下拉，和左侧树同一结构 -->
          <el-select v-if="dbCatalogs.length" v-model="selectedCatalog" size="small"
                     class="db-catalog-select" @change="onCatalogChange">
            <template #prefix>
              <el-icon class="sel-icon"><Files /></el-icon>
            </template>
            <el-option v-for="c in dbCatalogs" :key="c" :label="c" :value="c" />
          </el-select>

          <!-- 数据库选择器（catalog 方言下显示的是当前 catalog 内的裸库名） -->
          <el-select v-model="selectedDbShort" size="small" class="db-select"
                     :loading="loadingDbs" @change="onDbShortChange">
            <template #prefix>
              <el-icon class="sel-icon"><Coin /></el-icon>
            </template>
            <el-option v-for="db in dbOptions" :key="db" :label="db" :value="db" />
          </el-select>

          <!-- Schema 选择器（仅 SQL Server / PostgreSQL） -->
          <el-select v-if="showSchemaSelect" v-model="selectedSchema" size="small"
                     class="schema-select" :loading="loadingSchemas" @change="onSchemaChange">
            <template #prefix>
              <el-icon class="sel-icon"><Folder /></el-icon>
            </template>
            <el-option v-for="s in schemas" :key="s" :label="s" :value="s" />
          </el-select>
        </div>

        <span class="flex-spacer"></span>
        <div class="head-actions">
          <el-button v-if="running" size="small" type="danger" @click="stopSql">
            <el-icon style="margin-right:4px"><VideoPause /></el-icon>{{ $t('sqlq.stop') }} </el-button>
          <el-button v-else size="small" type="primary" @click="() => runSql()"
                     :title="hasEditorSelection ? $t('sqlq.runSelTitle', { n: selChars }) : $t('shortcut.query.runAll.label')">
            <el-icon style="margin-right:4px"><CaretRight /></el-icon>{{ hasEditorSelection ? $t('sqlq.runSel') : $t('evf.everyRun') }}
          </el-button>
          <el-button v-if="!isNoSql" size="small" :icon="Brush" @click="formatSql"
                     :title="hasEditorSelection ? $t('sqlq.fmtSelTitle') : $t('sqlq.fmtAllTitle')">{{ hasEditorSelection ? $t('sqlq.fmtSel') : $t('sce.format') }}</el-button>
          <!-- 事务模式：begin 关掉编辑器会话的 autocommit，之后的写语句都挂在事务里；
               提交/回滚收尾。全 agent 数据源通用（宿主走 JDBC 标准接口，无方言语法） -->
          <template v-if="!isNoSql">
            <el-button size="small" :type="txMode ? 'warning' : 'default'" :loading="txBusy"
                       @click="toggleTxMode" :title="$t('sqlq.txToggleTip')">{{ $t('sqlq.txMode') }}</el-button>
            <el-button v-if="txMode" size="small" type="success" :loading="txBusy"
                       @click="txCommit" :title="$t('sqlq.txCommitTip')">{{ $t('sqlq.txCommit') }}</el-button>
            <el-button v-if="txMode" size="small" type="danger" plain :loading="txBusy"
                       @click="txRollback">{{ $t('sqlq.txRollback') }}</el-button>
            <span v-if="txMode" class="tx-dot" :class="{ dirty: txDirty }" :title="txDirty ? $t('sqlq.txDirty') : $t('sqlq.txClean')"></span>
          </template>
          <!-- CSV 导入向导：把 CSV/TSV 文件分批 INSERT 进目标表（全 SQL 数据源） -->
          <el-button v-if="!isNoSql" size="small" :icon="Upload" @click="csvVisible = true"
                     :title="$t('csv.title')">{{ $t('csv.import') }}</el-button>
          <!-- SQL 片段库：命名保存常用 SQL，点击插入；「+」把选区/全文存为片段 -->
          <el-dropdown v-if="!isNoSql" trigger="click" placement="bottom-end" popper-class="hist-dropdown" :hide-on-click="false">
            <el-button size="small" :icon="Collection" :title="$t('sqlq.snippets')">{{ $t('sqlq.snippets') }}</el-button>
            <template #dropdown>
              <div class="hist-head">
                <span>{{ $t('sqlq.snippets') }}</span>
                <el-button size="small" text type="primary" @click.stop="saveSnippet">{{ $t('sqlq.snipSave') }}</el-button>
              </div>
              <div class="hist-list" v-if="snippets.length">
                <div class="hist-item" v-for="(s, i) in snippets" :key="s.id" @click="applySnippet(s)">
                  <div class="hist-meta">
                    <span class="hist-db">{{ s.name }}</span>
                    <span class="hist-time">{{ new Date(s.ts).toLocaleString() }}</span>
                  </div>
                  <div class="hist-sql" :title="s.sql">{{ s.sql }}</div>
                  <div class="hist-ops" @click.stop>
                    <el-button size="small" text type="danger" @click="removeSnippet(i)">{{ $t('common.delete') }}</el-button>
                  </div>
                </div>
              </div>
              <div class="hist-empty" v-else>{{ $t('sqlq.snipEmptyList') }}</div>
            </template>
          </el-dropdown>
          <!-- SQL 执行历史：本地保存最近执行的 SQL，一键回填复用 -->
          <el-dropdown trigger="click" placement="bottom-end" popper-class="hist-dropdown" :hide-on-click="false">
            <el-button size="small" :icon="Clock" :title="$t('sqlq.history')">历史</el-button>
            <template #dropdown>
              <div class="hist-head">
                <span>{{ $t('sqlq.history') }}</span>
                <el-button size="small" text type="danger" @click.stop="clearHistory" :disabled="!historyList.length">{{ $t('common.clear') }}</el-button>
              </div>
              <div class="hist-list" v-if="historyList.length">
                <div class="hist-item" v-for="(h, i) in historyList" :key="h.id" @click="applyHistory(h)">
                  <div class="hist-meta">
                    <span class="hist-db">{{ h.db || $t('sqlq.defaultDb') }}</span>
                    <span class="hist-time">{{ formatHistTime(h.ts) }}</span>
                    <span class="hist-cost" v-if="h.cost">{{ h.cost }}ms</span>
                  </div>
                  <div class="hist-sql" :title="h.sql">{{ h.sql }}</div>
                  <div class="hist-ops" @click.stop>
                    <el-button size="small" text type="danger" @click="removeHistory(i)">{{ $t('common.delete') }}</el-button>
                  </div>
                </div>
              </div>
              <div class="hist-empty" v-else>{{ $t('sqlq.historyEmpty') }}</div>
            </template>
          </el-dropdown>
          <!-- AI 模型：紧靠 AI 功能按钮（与 AI 面板「选择模型」同一套下拉，popper-class 复用全局样式） -->
          <el-dropdown v-if="aiModels.length > 0" trigger="click" placement="bottom-end"
                       popper-class="ai-model-dropdown" @command="onPickAiModel">
            <button class="model-btn" type="button" :title="'AI 模型：' + currentAiModelLabel">
              <el-icon class="model-btn-ic"><Cpu /></el-icon>
              <span class="model-btn-tx">{{ currentAiModelLabel }}</span>
              <el-icon class="model-btn-caret"><ArrowDown /></el-icon>
            </button>
            <template #dropdown>
              <div class="ai-dd-head">{{ $t('ai.pickModel') }}</div>
              <el-dropdown-menu>
                <el-dropdown-item command="auto" :class="{ active: selectedAiModelId === 'auto' }">
                  <span class="skills-name">{{ $t('ai.modelAuto') }}</span>
                  <el-icon v-if="selectedAiModelId === 'auto'" class="skills-check"><Select /></el-icon>
                </el-dropdown-item>
                <el-dropdown-item v-for="m in aiModels" :key="m.id" :command="m.id"
                                  :class="{ active: selectedAiModelId === m.id }">
                  <span class="skills-name">{{ m.name || m.model }}</span>
                  <el-icon v-if="selectedAiModelId === m.id" class="skills-check"><Select /></el-icon>
                </el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>
          <!-- AI 助手：解释 / 优化 / 诊断 / 改写 整合成一个入口，避免工具栏按钮堆积 -->
          <el-dropdown trigger="click" placement="bottom-end" popper-class="ai-model-dropdown"
                       @command="onAiAction">
            <button class="ai-act-btn" type="button" :title="$t('nav.ai')">
              <el-icon class="ai-act-btn-ic"><MagicStick /></el-icon>
              <span>{{ $t('nav.ai') }}</span>
              <el-icon class="ai-act-btn-caret"><ArrowDown /></el-icon>
            </button>
            <template #dropdown>
              <div class="ai-dd-head">{{ $t('nav.ai') }}</div>
              <el-dropdown-menu>
                <el-dropdown-item command="explain">
                  <el-icon class="skills-ic"><MagicStick /></el-icon>
                  <span class="skills-name">{{ $t('sqlq.aiExplain') }}</span>
                </el-dropdown-item>
                <el-dropdown-item command="optimize">
                  <el-icon class="skills-ic"><TrendCharts /></el-icon>
                  <span class="skills-name">{{ $t('sqlq.aiOptimize') }}</span>
                </el-dropdown-item>
                <el-dropdown-item command="diagnose">
                  <el-icon class="skills-ic"><DataAnalysis /></el-icon>
                  <span class="skills-name">{{ $t('sqlq.aiDiagnose') }}</span>
                </el-dropdown-item>
                <el-dropdown-item command="rewrite">
                  <el-icon class="skills-ic"><EditPen /></el-icon>
                  <span class="skills-name">{{ $t('sqlq.aiRewrite') }}</span>
                </el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>
          <el-button size="small" type="success" :icon="Document" @click="onSaveClick">{{ $t('common.save') }}</el-button>
        </div>
      </div>
      <div class="editor-body" @contextmenu.prevent="onEditorContextMenu">
        <!-- 语句面包屑：多段脚本时列出每条语句，点击选中并定位；悬停 ▶ 只执行那一条 -->
        <div class="stmt-bar" v-if="stmtBreadcrumbs.length">
          <span class="stmt-bar-label">{{ $t('sqlq.stmtCount', { n: stmtBreadcrumbs.length }) }}</span>
          <button v-for="s in stmtBreadcrumbs.slice(0, 40)" :key="s.i" type="button" class="stmt-chip"
                  :title="s.firstLine" @click="gotoStatement(s)">
            语句 {{ s.i }}
            <span class="stmt-run" :title="$t('sqlq.runThisStmt')" @click.stop="runStatement(s)"><el-icon :size="10"><CaretRight /></el-icon></span>
          </button>
          <span class="stmt-chip" v-if="stmtBreadcrumbs.length > 40">…</span>
        </div>
        <VueMonacoEditor
          v-if="monacoReady"
          ref="editorRef"
          v-model:value="sql"
          :theme="editorTheme"
          language="sql"
          :options="editorOptions"
          class="vue-monaco-editor"
          @mount="onEditorMount"
        />
        <div v-else class="editor-loading">{{ $t('sce.loading') }}</div>
      </div>
      <!-- 底部状态栏：光标位置 / 选中信息 / 总行数 / 格式化方言 -->
      <div class="editor-status">
        <span class="st-item">行 {{ cursorLine }}，列 {{ cursorColumn }}</span>
        <span v-if="hasEditorSelection" class="st-item st-hl">选中 {{ selChars }} 字符 / {{ selLines }} 行</span>
        <span class="st-item st-dim">{{ $t('sqlq.editorLineCount', { n: sqlLineCount }) }}</span>
        <span class="flex-spacer"></span>
        <span v-if="!isNoSql" class="st-item st-dim" :title="$t('sqlq.dialectTitle')">{{ String(fmtDialect || 'sql').toUpperCase() }}</span>
      </div>
    </div>

    <!-- 分隔条：拖动调整高度；双击在「结果最大化 → 编辑器最大化 → 还原」之间循环 -->
    <div v-if="resultVisible" class="resizer" @mousedown="onResizerStart"
         @dblclick="onResizerDblClick"
         :title="$t('sqlq.resizerTitle')"></div>

    <div class="result-area" v-if="resultVisible">
      <div class="result-head">
        <span class="result-title">{{ showResultTabs ? ('结果集 ' + resultItems.length) : $t('sqlq.result') }}</span>
        <span class="flex-spacer"></span>
        <el-dropdown trigger="click" :hide-on-click="false" popper-class="col-vis-dropdown">
          <el-button size="small" text :icon="Operation"
                     :title="$t('sqlq.visibleColsTitle', { shown: resultVisibleCols.length, total: (result.columns || []).length })" />
          <template #dropdown>
            <div class="col-vis" @mousedown.stop>
              <div class="col-vis-head">
                <span>{{ $t('sqlq.visibleCols') }}</span>
                <el-button size="small" text type="primary" @click="showAllResultCols">{{ $t('common.selectAll') }}</el-button>
              </div>
              <el-checkbox v-for="c in result.columns" :key="c" :model-value="!hiddenResultCols.has(c)"
                           @change="toggleResultColVisible(c)" class="col-vis-item">{{ c }}</el-checkbox>
            </div>
          </template>
        </el-dropdown>
        <el-dropdown @command="onExport">
          <el-button size="small" text :icon="Download" :title="$t('qa.exportBtn')" />
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item command="current-csv">{{ $t('qa.exportCurCsv') }}</el-dropdown-item>
              <el-dropdown-item command="current-excel">{{ $t('qa.exportCurExcel') }}</el-dropdown-item>
              <el-dropdown-item divided command="all-csv">{{ $t('qa.exportAllCsv') }}</el-dropdown-item>
              <el-dropdown-item command="all-excel">{{ $t('qa.exportAllExcel') }}</el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
        <el-button size="small" text :icon="DataAnalysis" :title="$t('sqlq.pivotTitle')"
                   @click="openPivot" :disabled="!result?.rows?.length" />
        <el-button size="small" text :icon="Close" @click="resultVisible = false" :title="$t('sqlq.closeResult')" />
      </div>
      <div class="result-tabs" v-if="showResultTabs">
        <button v-for="(item, i) in resultItems" :key="'tab' + i" type="button"
                class="result-tab" :class="{ active: i === activeResultIdx }"
                @click="selectResultTab(i)" :title="item.res.message || item.res.failedSql || ''">
          <span class="result-tab-dot" :class="item.res && !item.res.success ? 'err' : 'ok'"></span>
          {{ item.label }}
          <!-- 耗时并入 tab：哪个结果慢一眼可见，不再单开一块条形图 -->
          <span class="result-tab-ms" v-if="item.res && item.res.executeTime"
                :title="$t('sqlq.msServerTip')">{{ item.res.executeTime }}ms</span>
        </button>
      </div>
      <div class="result-grid">
        <!-- 加载遮罩：查询/翻页时可取消（与数据表一致） -->
        <div v-if="running" class="grid-loading-overlay">
          <div class="grid-loading-box">
            <el-icon class="is-loading" :size="26"><Loading /></el-icon>
            <span class="grid-loading-text">{{ $t('sqlq.querying') }}</span>
            <el-button size="small" @click="stopSql">
              <el-icon style="margin-right:4px"><VideoPause /></el-icon>{{ $t('tree.multiCancel') }} </el-button>
          </div>
        </div>
        <!-- 数据表格（模板里 ref 自动解包：editorSettings 已是设置对象，.value 反而是 undefined —— 真机崩过） -->
        <div v-if="result?.success && result?.rows?.length" class="data-table-wrap" ref="resultTableWrapRef" tabindex="0"
        :style="{ '--grid-fs': (editorSettings.gridFontSize || 14) + 'px' }"
             @scroll.passive="onResultTableScroll"
             @mousemove="onResultTableMove" @mousedown="onResultTableDown" @mouseleave="onResultTableLeave"
             @contextmenu.prevent="onResultGridContextMenu">
          <table class="data-table" :class="{ 'col-resizing': resultColResizing }"
                 :style="{ width: resultTableWidth + 'px' }">
            <colgroup>
              <col class="row-sel-col" style="width: 40px" />
              <col v-for="c in resultVisibleCols" :key="'c' + c.idx"
                   :style="{ width: (resultColWidths[c.idx] || resultDefaultColWidth(c.name)) + 'px' }" />
            </colgroup>
            <thead>
              <tr :class="{ 'selected': headerSelected, 'row-sel-top': headerSelected, 'row-sel-bottom': selEdges.headerBottom }">
                <!-- 左上角（原全选复选框位置）= 标题行的行头：单击选中标题行，按住往下拖可连选数据行 -->
                <th class="row-sel-th" :class="{ 'row-num-on': headerSelected }"
                    :title="$t('sqlq.headerRowTitle')"
                    @mousedown.prevent="onResultHeaderRowDown($event)"><span class="row-num-tx">#</span></th>
                <th v-for="c in resultVisibleCols" :key="'h' + c.idx" :data-gkey="'0:' + c.idx"
                    :title="c.name + $t('sqlq.colTitleSuffix')"
                    :class="{ 'col-selected': selectedCols.has(c.name), 'col-sel-l': selEdges.colLeft.has(c.name), 'col-sel-r': selEdges.colRight.has(c.name),
                              'sort-asc': resultSortColumn === c.name && resultSortDir === 'ASC',
                              'sort-desc': resultSortColumn === c.name && resultSortDir === 'DESC' }"
                    @mousedown="onResultColDragStart(c.idx, $event)"
                    @click="onResultHeaderClickOrSelect(c.name, $event)"
                    @contextmenu.prevent.stop="onResultHeaderContextMenu($event, c.name)"
                    @dblclick="onResultHeaderDblClick($event, c.idx)">
                  <span class="th-text">
                    <span class="th-line1">
                      <span class="th-type-ic" :class="resultTypeClass(c.idx)" :title="resultTypeOf(c.idx) || resultKindOf(c.idx)">
                        <el-icon><component :is="resultTypeIcon(c.idx)" /></el-icon>
                      </span>
                      <span class="th-label">{{ c.name }}</span>
                    </span>
                    <!-- 第二行：字段注释（单表 SELECT 时后端按方言取，JOIN/聚合不猜） -->
                    <span v-if="resComment(c.name)" class="th-comment" :title="resComment(c.name)">{{ resComment(c.name) }}</span>
                  </span>
                  <span class="th-sort" :class="{ 'is-sorted': resultSortColumn === c.name }"
                        :title="resultSortColumn === c.name ? (resultSortDir === 'ASC' ? $t('sqlq.sortAscTitle') : $t('sqlq.sortDescTitle')) : $t('sqlq.sortNoneTitle')"
                        @mousedown.stop @click.stop="onResultHeaderClick(c.name)">
                    <el-icon v-if="resultSortColumn !== c.name"><Sort /></el-icon>
                    <el-icon v-else-if="resultSortDir === 'ASC'"><SortUp /></el-icon>
                    <el-icon v-else><SortDown /></el-icon>
                  </span>
                </th>
              </tr>
            </thead>
            <tbody>
              <tr v-if="vtGapTop > 0" class="vt-gap">
                <td :colspan="resultVisibleCols.length + 1" :style="{ height: vtGapTop + 'px' }"></td>
              </tr>
              <tr v-for="(row, idx) in vtVisibleRows" :key="'r' + (vtStart + idx)" :data-rid="vtStart + idx"
                  :class="{
                    'selected': resultSelectedSet.has(vtStart + idx),
                    'row-sel-top': selEdges.rowTop.has(vtStart + idx),
                    'row-sel-bottom': selEdges.rowBottom.has(vtStart + idx),
                    'col-sel-bottom': (vtStart + idx) === lastResultRowIdx
                  }">
                <!-- 行号列（Excel 行头）：单击选中该行、Ctrl 切换、Shift 连选、按住拖动连选多行；双击/右键查看整行详情 -->
                <td class="row-sel-td" :class="{ 'row-num-on': resultSelectedSet.has(vtStart + idx) }"
                    :title="'第 ' + (vtStart + idx + 1) + ' 行（按住拖动可连选多行；双击查看整行详情）'"
                    @mousedown.prevent="onResultRowNumDown(vtStart + idx, $event)"
                    @dblclick.stop="openRowDetail(vtStart + idx)"
                    @contextmenu.prevent.stop="onResultRowContextMenu($event, vtStart + idx)">
                  <span class="row-num-tx">{{ vtStart + idx + 1 }}</span>
                </td>
                <td v-for="c in resultVisibleCols" :key="'d' + (vtStart + idx) + '_' + c.idx"
                    :class="[cellAlignClass(row[c.name], resultTypeOf(c.idx)), { 'null-cell': row[c.name] == null, 'col-selected': selectedCols.has(c.name), 'col-sel-l': selEdges.colLeft.has(c.name), 'col-sel-r': selEdges.colRight.has(c.name), 'active-cell': resultActiveCell && resultActiveCell.rowIdx === (vtStart + idx) && resultActiveCell.col === c.name && noResultBulkSelection }]"
                    :title="row[c.name] == null ? nullDisplay() : String(row[c.name])" :data-gkey="(vtStart + idx + 1) + ':' + c.idx"
                    @click="onResultCellClick(vtStart + idx, c.name)"
                    @dblclick.stop="onCellQuickEdit(vtStart + idx, c.name, row)"
                    @contextmenu.prevent.stop="onResultContextMenu($event, vtStart + idx, c.name)">
                  <!-- v-memo：内容没变就跳过该格的 vnode 创建与 diff；memo key 用原始值，值变必重渲染
                       （NULL 样式也在 key 里：设置页改样式能直接重渲染，不必重跑查询）。
                       展示走 formatDbValue（ISO 时间戳的 T 换空格），原始值不动（复制/编辑仍拿原文） -->
                  <span v-memo="[row[c.name], querySettingsLive.nullStyle]">{{ row[c.name] == null ? nullDisplay() : formatDbValue(row[c.name]) }}</span>
                </td>
              </tr>
              <tr v-if="vtGapBottom > 0" class="vt-gap">
                <td :colspan="resultVisibleCols.length + 1" :style="{ height: vtGapBottom + 'px' }"></td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-else-if="result && !result.success" class="error-block">
          <div class="error-head">
            <div class="error-head-left">
              <el-icon class="error-icon"><CircleCloseFilled /></el-icon>
              <span class="error-title">{{ isNoSql ? $t('sqlq.cmdFailed') : $t('sqlq.sqlFailed') }}</span>
              <span class="error-time" v-if="Math.max(elapsedTime || 0, result.executeTime || 0)">耗时 {{ Math.max(elapsedTime || 0, result.executeTime || 0) }}ms</span>
            </div>
            <el-button v-if="!isNoSql" size="small" type="primary" :icon="MagicStick" :loading="aiFixLoading" @click="askAiFix">{{ $t('sqlq.aiFix') }}</el-button>
          </div>
          <div class="error-message">{{ result.message || $t('common.unknownError') }}</div>
        </div>
        <el-empty v-else :description="(result && result.affectedRows >= 0) ? $t('sqlq.affectedOk', { n: result.affectedRows }) : $t('sqlq.noResult')" />
        <!-- 单元格编辑缓冲：双击改过的值先攒在这里，确认后一次提交
             （事务模式开着就落在事务里；快照守卫保证翻页/重跑后的过期缓冲不会误提交） -->
        <div v-if="pendingEdits.length" class="edit-bar">
          <el-icon :size="14" color="var(--el-color-warning)"><EditPen /></el-icon>
          <span class="edit-bar-text">{{ $t('sqlq.editsN', { n: pendingEdits.length }) }}</span>
          <el-button size="small" type="primary" :loading="editsBusy" @click="commitEdits">{{ $t('sqlq.commitEdits') }}</el-button>
          <el-button size="small" :disabled="editsBusy" @click="discardEdits">{{ $t('sqlq.discardEdits') }}</el-button>
        </div>
        <div v-if="result?.success && result?.rows?.length" class="result-footer">
          <span class="result-time" :title="$t('sqlq.msE2eTip')">
            <!-- 耗时取「前端实测」与「服务端」的较大值：服务端 executeTime 只计执行段，
                 不含网络往返/连接获取，真实耗时要远大于它（真机反馈 244ms 之谜） -->
            {{ running ? formatElapsed(elapsedTime) : formatElapsed(Math.max(elapsedTime || 0, result.executeTime || 0)) }}
          </span>
          <!-- 选中区汇总（底栏状态区）：框选单元格、选中整行或整列时给出
               格子数 / 求和 / 均值 / 最小 / 最大（只统计数值列），排查数据时不用自己算 -->
          <span v-if="resultSelectionSummary" class="result-summary" :title="$t('sqlq.summaryTitle')">
            <span class="rs-item">选中 <b>{{ resultSelectionSummary.cells }}</b> 格</span>
            <template v-if="resultSelectionSummary.nums">
              <span class="rs-item">{{ $t('sqlq.sum') }} <b>{{ fmtNum(resultSelectionSummary.sum) }}</b></span>
              <span class="rs-item">{{ $t('sqlq.avg') }} <b>{{ fmtNum(resultSelectionSummary.avg) }}</b></span>
              <span class="rs-item">{{ $t('sqlq.min') }} <b>{{ fmtNum(resultSelectionSummary.min) }}</b></span>
              <span class="rs-item">{{ $t('sqlq.max') }} <b>{{ fmtNum(resultSelectionSummary.max) }}</b></span>
            </template>
          </span>
          <!-- 总数未知（后端没统计，totalCount 为 -1）时不显示总数：
               那个数字取的是本页行数，等于把「这一页取回多少行」说成「总共多少行」。
               异步计数在跑时给一条「总数统计中…」的状态（COUNT 移出了主链路，
               数据先回显、总数后到，这里补上过渡期的口径）；算不出（超时）就安静消失。
               总数是自己渲染的（不用分页器自带的 total 段）：可点击重新统计、
               悬停有「点我重新统计」提示 —— 表数据被别人改过时手动刷一下。 -->
          <span v-if="countPending && displayTotal === null" class="count-pending">
            <el-icon class="is-loading"><Loading /></el-icon> {{ $t('sqlq.counting') }}
          </span>
          <span v-else-if="displayTotal !== null" class="total-refresh"
                :title="$t('sqlq.recountTip')" @click="recountTotal">
            {{ $t('sqlq.totalN', { n: Number(displayTotal).toLocaleString() }) }}
          </span>
          <el-pagination
            v-model:current-page="currentPage"
            :page-size="pageSize"
            :total="pageTotal"
            :page-sizes="queryPageSizes"
            layout="sizes, prev, pager, next, jumper"
            size="small"
            background
            @current-change="onPageChange"
            @size-change="onPageSizeChange"
          />
        </div>
      </div>
    </div>

    <!-- top 与「执行计划 / 试跑」弹窗（SqlProbeDialog）对齐，保证上下级弹窗位置一致 -->
    <el-dialog v-model="aiDialogVisible" width="640px" top="8vh" class="sqlq-dialog">
      <template #header>
        <div class="dlg-title">
          <span class="dlg-title-ic"><el-icon :size="16"><MagicStick /></el-icon></span>
          <span>{{ aiDialogTitle }}</span>
        </div>
      </template>
      <div class="ai-dialog-body">
        <div v-if="aiLoading" class="ai-loading">
          <el-icon class="is-loading" :size="28"><Loading /></el-icon>
          <p>{{ $t('sqlq.aiThinking') }}</p>
        </div>
        <template v-else-if="aiResult">
          <div v-if="aiErrorReason" class="ai-error-reason">
            <span class="ai-reason-label">{{ $t('sqlq.errorReason') }}</span>{{ aiErrorReason }}
          </div>
          <!-- 结论一律按 Markdown 渲染；含 SQL 时代码块右上角自带「插入编辑器 / 复制」图标按钮 -->
          <div class="ai-markdown" v-html="renderMarkdown(aiResultMd, { sqlActions: true })"
               @click="onAiMdAction($event)"></div>
        </template>
                <div v-if="aiUsage" class="ai-usage">本次回答消耗：{{ Number(aiUsage.totalTokens || 0).toLocaleString() }} tokens<template v-if="aiUsage.promptTokens != null">（{{ Number(aiUsage.promptTokens).toLocaleString() }}）</template></div>

      </div>
    </el-dialog>

    <!-- 保存脚本弹窗 -->
    <el-dialog v-model="saveDialogVisible" width="400px" class="sqlq-dialog">
      <template #header>
        <div class="dlg-title">
          <span class="dlg-title-ic"><el-icon :size="16"><Document /></el-icon></span>
          <span>{{ $t('shortcut.query.save.label') }}</span>
        </div>
      </template>
      <el-input v-model="saveName" clearable @keyup.enter="doSaveScript" />
      <template #footer>
        <el-button @click="cancelSaveDialog">{{ $t('tree.multiCancel') }}</el-button>
        <el-button type="primary" @click="doSaveScript">{{ $t('common.save') }}</el-button>
      </template>
    </el-dialog>
  </div>

  <!-- 结果表格右键上下文菜单（支持二级子菜单） -->
  <teleport to="body">
    <div v-if="resCtx.visible" class="grid-ctx-menu" :style="{ left: resCtx.x + 'px', top: resCtx.y + 'px' }"
         @contextmenu.prevent @mousedown.stop>
      <div v-for="(item, i) in resCtx.items" :key="i"
           :class="['ctx-item', { 'ctx-disabled': item.disabled, 'ctx-sep': item.sep, 'ctx-active': item.sub && resCtxSub && resCtxSub.parentIndex === i }]"
           @click="(item.sep || item.sub || item.disabled) ? null : onResultCtxItem(item)"
           @mouseenter="onResCtxItemHover(item, i, $event)">
        <span class="ctx-label">{{ item.label }}</span>
        <span v-if="item.shortcut" class="ctx-shortcut">{{ item.shortcut }}</span>
        <span v-if="item.sub" class="ctx-arrow">›</span>
      </div>
    </div>
    <div v-if="resCtx.visible && resCtxSub && resCtxSub.items && resCtxSub.items.length" class="grid-ctx-menu grid-ctx-sub"
         :style="{ left: resCtxSub.x + 'px', top: resCtxSub.y + 'px' }"
         @contextmenu.prevent @mousedown.stop>
      <div v-for="(item, i) in resCtxSub.items" :key="i"
           :class="['ctx-item', { 'ctx-disabled': item.disabled, 'ctx-sep': item.sep }]"
           @click="(item.sep || item.disabled) ? null : onResultCtxItem(item)">
        <span class="ctx-label">{{ item.label }}</span>
        <span v-if="item.shortcut" class="ctx-shortcut">{{ item.shortcut }}</span>
      </div>
    </div>
  </teleport>

  <!-- SQL 编辑器右键菜单（与表格右键同一套样式，支持二级子菜单）
       注：Monaco 内置右键菜单未随按需加载引入（utils/monaco.js 的 contrib 白名单里没有它），
       所以这里由应用自己接管编辑器区域的 contextmenu -->
  <teleport to="body">
    <div v-if="edCtx.visible" class="grid-ctx-menu ed-ctx-menu" :style="{ left: edCtx.x + 'px', top: edCtx.y + 'px' }"
         @contextmenu.prevent @mousedown.stop>
      <div v-for="(item, i) in edCtx.items" :key="i"
           :class="['ctx-item', { 'ctx-disabled': item.disabled, 'ctx-sep': item.sep, 'ctx-active': item.sub && edCtxSub && edCtxSub.parentIndex === i }]"
           @click="(item.sep || item.sub || item.disabled) ? null : onEdCtxItem(item)"
           @mouseenter="onEdCtxItemHover(item, i, $event)">
        <span class="ctx-label">{{ item.label }}</span>
        <span v-if="item.shortcut" class="ctx-shortcut">{{ item.shortcut }}</span>
        <span v-if="item.sub" class="ctx-arrow">›</span>
      </div>
    </div>
    <div v-if="edCtx.visible && edCtxSub && edCtxSub.items && edCtxSub.items.length" class="grid-ctx-menu grid-ctx-sub"
         :style="{ left: edCtxSub.x + 'px', top: edCtxSub.y + 'px' }"
         @contextmenu.prevent @mousedown.stop>
      <div v-for="(item, i) in edCtxSub.items" :key="i"
           :class="['ctx-item', { 'ctx-disabled': item.disabled, 'ctx-sep': item.sep }]"
           @click="(item.sep || item.disabled) ? null : onEdCtxItem(item)">
        <span class="ctx-label">{{ item.label }}</span>
        <span v-if="item.shortcut" class="ctx-shortcut">{{ item.shortcut }}</span>
      </div>
    </div>
  </teleport>

  <TaskProgressDialog
    v-model:visible="exportTask.visible"
    task-kind="export"
    :target-name="props.scriptName || $t('sqlq.queryResult')"
    :status="exportTask.status"
    :done="exportTask.done"
    :total="exportTask.total"
    :phase="exportTask.phase"
    :message="exportTask.message"
    :logs="exportTask.logs"
    :canceling="exportTask.canceling"
    @cancel="exportTask.cancel(selectedConnId || props.conn.id)"
    @close="exportTask.close"
  />

  <!-- SQL 快捷验证：AI 给的 SQL 先看执行计划 / 试跑一次（不消耗 AI 调用） -->
  <SqlProbeDialog v-model="probeVisible" :sql="probeSql" :conn-id="selectedConnId || props.conn?.id"
                  :database="props.database" :mode="probeMode" @insert="onProbeInsert" />
  <!-- 模板变量填参：SQL 里的 :name 占位符在执行前收值（值按名称记忆） -->
  <el-dialog v-model="varDialog.visible" :title="$t('sqlq.varDialogTitle')" width="430" append-to-body
             :close-on-click-modal="false" @keyup.enter="onVarConfirm">
    <div class="var-row" v-for="v in varDialog.vars" :key="v">
      <span class="var-name">:{{ v }}</span>
      <el-input v-model="varDialog.values[v]" :placeholder="$t('sqlq.varValuePh')" clearable />
    </div>
    <div class="var-hint">{{ $t('sqlq.varHint') }}</div>
    <template #footer>
      <el-button @click="onVarCancel">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" @click="onVarConfirm">{{ $t('sqlq.varRun') }}</el-button>
    </template>
  </el-dialog>
  <!-- 查询结果数据透视：复用当前结果网格做分组汇总 / 计数 / 下钻，纯前端不消耗后端 -->
  <DataPivotDialog v-model="pivotVisible" :columns="pivotColumns" :rows="pivotRows" />
  <!-- CSV 导入向导：文件解析 / 列映射 / 分批 INSERT（全 SQL 数据源） -->
  <CsvImportDialog v-model="csvVisible" :conn-id="selectedConnId || props.conn?.id"
                   :database="txDatabaseOf()" :kind="connectionKind" @imported="onCsvImported" />
  <!-- 行详情：双击 / 右键行号查看整行字段明细 -->
  <CellDetailDialog v-model="rowDetail.visible" :title="rowDetail.title" :text="rowDetail.text" />
</template>

<script setup>
import { ref, computed, onMounted, onBeforeUnmount, watch, nextTick, h } from 'vue'
import { t } from '../../utils/i18n'
import VueMonacoEditor from '@guolao/vue-monaco-editor'
import { ensureMonaco } from '../../utils/monaco'
import TaskProgressDialog from '../../common/TaskProgressDialog.vue'
import CsvImportDialog from '../../common/CsvImportDialog.vue'
import SqlProbeDialog from '../../common/SqlProbeDialog.vue'
import CellDetailDialog from '../../common/CellDetailDialog.vue'
import DataPivotDialog from './DataPivotDialog.vue'
import { useExportTask, saveExportBlob } from '../../utils/useExportTask'
import { exportData } from '../../api'
import { ElMessage, ElMessageBox, ElSelect, ElOption } from 'element-plus'
import { editorSettingsLive, getEditorSettings, getQuerySettings, querySettingsLive } from '../../utils/settings'
import { getResolvedTheme, monacoTheme, onResolvedThemeChange } from '../../utils/theme'
import { formatSql as smartFormatSql, connDialectOf } from '../../utils/sqlFormat'
import { builtinFunctions, functionInsertText, smartCase } from '../../utils/sqlCompletions'
import { formatDbValue, nullDisplay } from '../../utils/cellValue'
import { useShortcutScope } from '../../utils/useShortcuts'
import { loadShortcuts } from '../../utils/shortcuts'
import { errMsg } from '../../utils/errMsg'
import { renderMarkdown, extractCodeBlocks } from '../../utils/markdown'
import { splitSqlStatements, splitSqlStatementRanges } from '../../utils/sqlSplit'
import { useExcelSelection } from '../../utils/excelSelection'
import { cellAlignClass } from '../../utils/cellAlign'
import { readSchemaCache, writeSchemaCache } from '../../utils/schemaCache'
import {
  CaretRight, Download, MagicStick, TrendCharts, Loading,
  Close, CircleCloseFilled, Coin, Brush, Clock, Files,
  Document, VideoPause, Connection, Folder, DataAnalysis, EditPen,
  Cpu, ArrowDown, Select, Histogram, Calendar, Switch as SwitchIcon, Tickets, Grid, Operation,
  Sort, SortUp, SortDown, Upload, Collection
} from '@element-plus/icons-vue'
import { executeSql, executeSqlCount, executeSqlBatch, executeNoSql, cancelSql, aiExplain, aiOptimize, aiFix, aiDiagnose, aiChat, listDatabases, listCatalogs, noSqlDatabases, listSchemas, listTables, listProcedures, listTriggers, listConnections, listColumns, getColumnComments, getAiConfig, txControl } from '../../api'
import { isNoSql as isNoSqlType, schemaLevelOf, byType } from '../../types'
import DbLogo from '../../common/DbLogo.vue'

const props = defineProps({ conn: Object, database: String, scriptName: String, tabConnId: String, tabDatabase: String, initialSql: String, autoRun: Boolean })
const emit = defineEmits(['save', 'dirty-change', 'conn-change', 'db-change', 'sql-change'])

const sql = ref(props.initialSql || '')
const lastSavedSql = ref(props.initialSql || '')
const isDirty = ref(false)

// 内容变化 → 防抖通知父级：父级据此更新页签快照，刷新后才不会退回「打开这份页签时」的内容
let sqlChangeTimer = null
watch(sql, (val) => {
  const dirty = val !== lastSavedSql.value
  if (dirty !== isDirty.value) {
    isDirty.value = dirty
    emit('dirty-change', dirty)
  }
  clearStmtError() // 内容改了：上一轮的报错标红已过期
  if (sqlChangeTimer) clearTimeout(sqlChangeTimer)
  sqlChangeTimer = setTimeout(() => { sqlChangeTimer = null; emit('sql-change', sql.value) }, 600)
  scheduleAutoSave(val, dirty)
})
const result = ref({ columns: [], rows: [], success: true, message: '', executeTime: 0 })
// 列显示/隐藏：hiddenResultCols 存被隐藏的列名，resultVisibleCols 为实际渲染的列（带原始下标）
const hiddenResultCols = ref(new Set())
const resultVisibleCols = computed(() => {
  const cols = result.value?.columns || []
  return cols.map((name, idx) => ({ name, idx })).filter(c => !hiddenResultCols.value.has(c.name))
})
const toggleResultColVisible = (name) => {
  const s = new Set(hiddenResultCols.value)
  if (s.has(name)) {
    s.delete(name)
  } else {
    if ((result.value?.columns?.length || 0) - s.size <= 1) { ElMessage.warning(t('sqlq.keepOneCol')); return }
    s.add(name)
  }
  hiddenResultCols.value = s
}
const showAllResultCols = () => { hiddenResultCols.value = new Set() }

// 结果表头字段类型（类型来自后端 columnTypes；**查询端点经常不带** ——
// 此时按首个非空值推断，保证 SQL 编辑器结果与表预览/NoSQL 的表头图标一致）
const resultTypeOf = (ci) => (result.value?.columnTypes || [])[ci] || ''
const resultKindOf = (ci) => {
  const t = resultTypeOf(ci).toLowerCase()
  if (/bool/.test(t)) return 'bool'
  if (/json/.test(t)) return 'json'
  if (/(blob|binary|bytea|image|raw|byte)/.test(t)) return 'blob'
  if (/^(int|bigint|smallint|tinyint|mediumint|decimal|numeric|float|double|real|number|bit|money|serial)/.test(t)) return 'num'
  if (/^(date|time|datetime|timestamp|year)/.test(t)) return 'date'
  if (t) return 'text'
  const col = (result.value?.columns || [])[ci]
  for (const r of (result.value?.rows || [])) {
    const v = r ? r[col] : null
    if (v === null || v === undefined || v === '') continue
    if (typeof v === 'number') return 'num'
    if (typeof v === 'object') return 'json'
    if (/^\d{4}-\d{2}-\d{2}/.test(String(v))) return 'date'
    return 'text'
  }
  return 'text'
}
// 类型族 → 徽章配色类（与 resultTypeIcon 同一套判定；全局 CSS 按类着色）
const KIND_CLASS = { bool: 'th-t-bool', json: 'th-t-json', blob: 'th-t-blob', num: 'th-t-num', date: 'th-t-date', text: 'th-t-text' }
const KIND_ICON = { bool: SwitchIcon, json: Tickets, blob: Document, num: Histogram, date: Calendar, text: Document }
const resultTypeClass = (ci) => KIND_CLASS[resultKindOf(ci)] || 'th-t-text'
const resultTypeIcon = (ci) => KIND_ICON[resultKindOf(ci)] || Grid
// 多段 SQL 结果：一次执行返回多条结果集时，以「结果1/结果2…」tab 逐条展示
const resultItems = ref([]) // [{ label, res }]
const activeResultIdx = ref(0)
const showResultTabs = computed(() => resultItems.value.length > 1)
const running = ref(false)
const loading = ref(false)
const resultVisible = ref(false)
const editorRatio = ref(1)

// 执行取消
const execId = ref('')
const cancelRequested = ref(false)
let cancelController = null

// 编辑器 / 查询设置（提前读取，避免在 setup 中访问时出现 TDZ）。
// editorSettings 用**共享响应式快照**：设置页保存后即时生效（wrapper 会 updateOptions），
// 以前是 setup 一次性快照 —— 改字号/换行对已打开的标签毫无作用，得重开。
const editorSettings = editorSettingsLive
const querySettings = getQuerySettings()
// 编辑器配色跟随应用主题
const editorTheme = ref(monacoTheme())
let offEditorTheme = null

// 分页（默认每页行数取设置项t('settings.query.pageSize')）
const pageSize = ref(querySettings.pageSize)
const currentPage = ref(1)
// 分页选项：保证当前默认值始终可选，其余保留大数据量浏览档位
const queryPageSizes = computed(() => {
  const base = [1000, 2000, 5000, 10000]
  if (querySettings.pageSize > 0 && !base.includes(querySettings.pageSize)) base.unshift(querySettings.pageSize)
  return base.sort((a, b) => a - b)
})
const elapsedTime = ref(0)
let queryTimer = null
const formatElapsed = (ms) => {
  if (ms < 1000) return `${ms}ms`
  return `${(ms / 1000).toFixed(2)}s`
}
// 后端已分页，直接展示返回的 rows 即可
const paginatedRows = computed(() => result.value?.rows || [])
// 真实总条数（后端统计；未知时返回 null）
const displayTotal = computed(() => {
  const t = result.value.totalCount
  return typeof t === 'number' && t >= 0 ? t : null
})
// 本页已加载行数
const loadedRows = computed(() => result.value?.rows?.length || 0)
// 分页 total：真实总条数优先。
//
// 未知时退回「本页行数」只是为了给分页器一个能算页码的数；模板那边会把自带的
// 「共 N 条」去掉，免得把本页行数当成总数展示。
const pageTotal = computed(() => {
  if (displayTotal.value !== null) return displayTotal.value
  // 总数未知：给分页器一个「刚好能翻下一页」的虚拟值 —— 当前页装满就多给 1 行的余量，
  // 没装满说明已是末页。真实总数由异步计数回填后覆盖（见 fetchCountFor）。
  const base = (currentPage.value - 1) * pageSize.value + loadedRows.value
  return loadedRows.value >= pageSize.value ? base + 1 : base
})

// ========== 总数异步补齐 ==========
// 主执行接口已把 COUNT 移出主链路（大 JOIN 的计数三层兜底串行能拖百秒级，
// 真机：数据 1s 就绪却等计数等了 163s+）：数据先回（totalCount=-1 表示未知），
// 这里再拿原句单独发计数请求，回来后回填分页器。后端带 10s 总预算，算不出保持 -1。
// seq 守卫：期间又跑了新查询/翻页的话，过期的计数结果直接丢弃。
// countPending：计数在途时底栏给一条「总数统计中…」状态；回填/放弃/被新查询作废时熄灭。
//
// **res 必须传响应式代理**（result.value / resultItems[i].res 暴露出来的那个），
// 不能传 HTTP 返回的原始对象：ref 深层代理后页面读的是 proxy，直接改原始对象
// 不触发任何更新 —— 表现就是「总数早就算完了，分页器却一直不动」。
let countSeq = 0
const countPending = ref(false)
const invalidateCount = () => { countSeq++; countPending.value = false }
// 当前展示结果对应的计数 SQL（fetchCountFor 每次都会刷新）：
// 「点击总数重新统计」靠它知道要重数哪条语句
const lastCountSql = ref('')

// 计数缓存：同一份 SQL 翻页时不重算（大表 COUNT 要秒级，每翻一页重跑一遍纯浪费）。
// 键 = 连接|库|归一化 SQL（压空白、去尾分号）；10 分钟过期（期间数据可能变了）。
// 只缓存**成功**的计数（>=0）：失败/-1 不缓存，下次还重试。
const countCache = new Map()
const COUNT_TTL = 10 * 60 * 1000
const countCacheKey = (connId, db, sqlText) =>
  `${connId}|${db || ''}|${String(sqlText).replace(/\s+/g, ' ').trim().replace(/;+\s*$/, '')}`

const fetchCountFor = (res, sqlText) => {
  if (!res || !res.success) return
  // 先记 SQL 再走后面的早退分支：切到「总数已知」的 tab 时也要更新，
  // 否则「重新统计」会拿上一条语句去数当前结果
  if (sqlText) lastCountSql.value = sqlText
  if (typeof res.totalCount === 'number' && res.totalCount >= 0) { countPending.value = false; return }
  if (!sqlText || !(res.rows && res.rows.length)) return
  const connId = selectedConnId.value || props.conn.id
  const db = selectedSchema.value
    ? `${selectedDatabase.value}.${selectedSchema.value}`
    : selectedDatabase.value || undefined
  const key = countCacheKey(connId, db, sqlText)
  const hit = countCache.get(key)
  if (hit && Date.now() - hit.t < COUNT_TTL) {
    // 命中缓存：立即回填，不发请求、不亮「统计中」
    res.totalCount = hit.n
    return
  }
  const seq = ++countSeq
  countPending.value = true
  executeSqlCount(connId, sqlText, db, (res.columns || []).length)
    .then((r) => {
      if (seq !== countSeq) return
      const n = r && typeof r.totalCount === 'number' ? r.totalCount : -1
      res.totalCount = n
      countPending.value = false
      if (n >= 0) {
        countCache.set(key, { t: Date.now(), n })
        // 简单容量上限：超了按插入序丢最老的（Map 迭代序 = 插入序）
        if (countCache.size > 100) countCache.delete(countCache.keys().next().value)
      }
    })
    .catch(() => { if (seq === countSeq) countPending.value = false })
}

/** 点击「共 N 条」重新统计：绕过缓存强制重数一次（表数据被别人改过时手动刷）。 */
const recountTotal = () => {
  const res = result.value
  const sqlText = lastCountSql.value
  if (!res || !res.success || !sqlText) return
  const connId = selectedConnId.value || props.conn.id
  const db = selectedSchema.value
    ? `${selectedDatabase.value}.${selectedSchema.value}`
    : selectedDatabase.value || undefined
  countCache.delete(countCacheKey(connId, db, sqlText))
  res.totalCount = -1
  fetchCountFor(res, sqlText)
}

// ========== 结果表：选中区汇总（底栏状态区）==========
// 优先级：单元格区域 > 选中行 > 选中列（同一时刻只会存在一块选区，见 focusResult* 那几个函数）。
// 计数按"格子数"给（和 Excel 一致），求和/均值只统计**数值类型**的列 —— 把字符串硬加起来没有意义。
const NUMERIC_TYPE_RE = /^(int|bigint|smallint|tinyint|mediumint|decimal|numeric|float|double|real|number|bit|money|serial)/i
// 类型缺失时的兜底：值是**严格数字面量**才算（有的源列类型拿不到，汇总不能跟着哑掉）
const NUMERIC_LITERAL_RE = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/
const isNumericResultCol = (idx) => NUMERIC_TYPE_RE.test(String(resultTypeOf(idx) || '').toLowerCase())
/** 数字显示：整数不带小数点，小数最多两位（汇总值常常是除出来的） */
const fmtNum = (n) => (n == null || !Number.isFinite(n)) ? '' : (Number.isInteger(n) ? String(n) : Number(n.toFixed(2)).toLocaleString())

const resultSelectionSummary = computed(() => {
  const rows = result.value?.rows || []
  const cols = result.value?.columns || []
  // gridSel.range 是框选模块暴露的响应式矩形，框选/键盘扩选都会更新它
  const rect = gridSel.range ? gridSel.range.value : null
  const acc = { cells: 0, nums: 0, sum: 0, min: null, max: null }
  const addCell = (r, c) => {
    if (r === 0) return                      // gkey 的行 0 是表头行，不计入
    const name = cols[c]
    if (name === undefined) return
    const row = rows[r - 1]                  // 与 valueAt 同一套映射：数据行 = rows[r-1]
    if (!row) return
    acc.cells++
    const v = row[name]
    if (v == null) return
    // 类型已知按类型；类型缺失看值本身（严格数字面量），普通文本列不会被误加
    const typed = String(resultTypeOf(c) || '')
    if (typed) {
      if (!NUMERIC_TYPE_RE.test(typed.toLowerCase())) return
    } else if (!NUMERIC_LITERAL_RE.test(String(v).trim())) return
    const n = Number(v)
    if (!Number.isFinite(n)) return
    acc.nums++
    acc.sum += n
    acc.min = acc.min == null ? n : Math.min(acc.min, n)
    acc.max = acc.max == null ? n : Math.max(acc.max, n)
  }
  if (rect) {
    for (let r = Math.max(1, rect.r1); r <= rect.r2; r++) {
      for (let c = rect.c1; c <= rect.c2; c++) addCell(r, c)
    }
  } else if (resultSelectedSet.value && resultSelectedSet.value.size) {
    for (const ri of resultSelectedSet.value) {
      if (typeof ri !== 'number') continue   // 行选集合若是别的形态就不硬算，宁可不出汇总
      for (let c = 0; c < cols.length; c++) addCell(ri + 1, c)
    }
  } else if (selectedCols.value && selectedCols.value.size) {
    for (let r = 0; r < rows.length; r++) {
      for (let c = 0; c < cols.length; c++) if (selectedCols.value.has(cols[c])) addCell(r + 1, c)
    }
  } else {
    return null
  }
  if (!acc.cells) return null
  return {
    cells: acc.cells,
    nums: acc.nums,
    sum: acc.sum,
    avg: acc.nums ? acc.sum / acc.nums : null,
    min: acc.min,
    max: acc.max
  }
})
// 翻到批量结果某一段的第 N 页：**只重跑这一段**（executeSql 带页码走后端分页+统计），
// 其余段的结果原样保留。整批重跑会把写入类语句再执行一遍，绝对不行。
const loadSegment = async (item, p, size) => {
  if (!item || !item.segmentSql) return
  // 有未提交的单元格修改：分段翻页同样使行号失效 —— 确认放弃才继续
  if (pendingEdits.value.length) {
    try {
      await ElMessageBox.confirm(t('sqlq.editsLoseWarn', { n: pendingEdits.value.length }), t('sqlq.editsLoseTitle'), {
        confirmButtonText: t('sqlq.discardEdits'),
        cancelButtonText: t('common.cancel'),
        type: 'warning'
      })
    } catch { return }
    discardEdits()
  }
  running.value = true
  loading.value = true
  cancelRequested.value = false
  invalidateCount()
  cancelController = new AbortController()
  try {
    const connId = selectedConnId.value || props.conn.id
    const db = selectedSchema.value
      ? `${selectedDatabase.value}.${selectedSchema.value}`
      : selectedDatabase.value || undefined
    const res = await executeSql(connId, item.segmentSql, db, execId.value, cancelController.signal, p, size)
    item.res = res
    item.page = p
    result.value = res
    // 总数未知时异步补齐（后端已不同步 COUNT，见 fetchCountFor）
    fetchCountFor(result.value, item.segmentSql)
  } catch (e) {
    if (!cancelRequested.value) ElMessage.error(e?.message || t('common.unknownError'))
  } finally {
    cancelController = null
    running.value = false
    loading.value = false
  }
}
const onPageChange = (p) => {
  const item = resultItems.value[activeResultIdx.value]
  if (item && item.segmentSql) { loadSegment(item, p, pageSize.value); return }
  runSql(p, pageSize.value, false)
}
const onPageSizeChange = (s) => {
  const item = resultItems.value[activeResultIdx.value]
  if (item && item.segmentSql) { loadSegment(item, 1, s); return }
  runSql(1, s, false)
}

// ========== 结果表格列宽拖拽（任意竖线均可拖动） ==========
const resultTableWrapRef = ref(null)
const resultColWidths = ref({})
const resultColResizing = ref(false)
const RESULT_LEAD = 1 // 首列复选框列，数据列 DOM 索引需 +RESULT_LEAD
let resultDrag = null
let resultLastResizeAt = 0
let resultLastDragAt = 0
let resultLastHover = null

const MIN_RESULT_COL_WIDTH = 60
const MAX_RESULT_COL_WIDTH = 480
// 表头那一格除文字外还要放下：左右内边距 20px + 左右边框 2px + 类型/主键图标 ~18px + 排序图标 ~20px
const RESULT_HEADER_EXTRA = 60
// 数据格：左右内边距 20px + 边框 2px + 一点余量
const RESULT_CELL_EXTRA = 24
// 首列（复选框 / 行号）宽度，与 CSS .row-sel-col / .row-sel-td 的 40px 保持一致
const RESULT_LEAD_COL_WIDTH = 40

const resultDefaultColWidth = (name) => {
  let w = 0
  for (const ch of String(name || '')) w += /[\u3000-\u9fff\uff00-\uffef]/.test(ch) ? 13 : 7
  // 兜底也按「表头放得下」给宽：测量完成前后不会先闪一下被截断的表头
  return Math.max(MIN_RESULT_COL_WIDTH, Math.min(MAX_RESULT_COL_WIDTH, Math.round(w) + RESULT_HEADER_EXTRA))
}

// 表格总宽 = 首列 + 各可见列宽之和，用内联 width 显式给出。
// 不能依赖 CSS 的 width: max-content：table-layout: fixed 下浏览器是按「内容」算 max-content 的，
// 再把多出来的空间摊回到各列上，于是 <col> 上写的小宽度（比如拖到 8px）会被撑回去，
// 表现就是「列宽拖不窄」。
const resultTableWidth = computed(() => {
  let w = RESULT_LEAD_COL_WIDTH
  for (const c of resultVisibleCols.value) w += resultColWidths.value[c.idx] || resultDefaultColWidth(c.name)
  return Math.round(w)
})

// 用 canvas 精确测量文本宽度，避免 table-layout:fixed 下 scrollWidth 被当前列宽污染
let resultMeasureCtx = null
let resultMeasureFont = ''
// 表格真实字体（含字号），由 measureResultColumns 从 DOM 现取。
// 原来写死 12px，而 .data-table 渲染 13px —— 文本被低估约 8%，表头正好差几像素被截。
let resultMeasureFontSpec = { size: 13, family: 'sans-serif' }
const measureResultText = (text, bold, fontFamily) => {
  if (!resultMeasureCtx) resultMeasureCtx = document.createElement('canvas').getContext('2d')
  const family = fontFamily || resultMeasureFontSpec.family
  const font = `${bold ? 600 : 400} ${resultMeasureFontSpec.size}px ${family}`
  if (font !== resultMeasureFont) { resultMeasureCtx.font = font; resultMeasureFont = font }
  return resultMeasureCtx.measureText(text == null ? '' : String(text)).width
}

// 用户手动拖拽过的列（按索引），自动测量不再覆盖
const resultManualCols = ref(new Set())

// ========== 列宽的**唯一一条公式** ==========
// 自动测量（新查询/换列后）、双击表头右缘、右键「列宽自适应」三条路径全部走它。
// 以前自适应那条单独留着一份实现（旧的 +42 预算 + 800 上限 + 50 行采样），
// 于是"双击自适应"得到的宽度和"默认宽度"对不上，看着像没生效。
const RESULT_SAMPLE_ROWS = 30

/** 测量用的字体必须与表格真实字体一致（字号曾被写死 12px，比实际的 13px 小一号） */
const syncResultFontSpec = (wrap) => {
  const tableEl = wrap ? wrap.querySelector('table') : null
  const cs = tableEl ? getComputedStyle(tableEl) : null
  const fontFamily = cs ? cs.fontFamily : 'sans-serif'
  const fontSize = cs ? (parseFloat(cs.fontSize) || 13) : 13
  if (fontSize !== resultMeasureFontSpec.size || fontFamily !== resultMeasureFontSpec.family) {
    resultMeasureFontSpec = { size: fontSize, family: fontFamily }
    resultMeasureFont = ''
  }
  return fontFamily
}

/** 一列「正好放得下」的宽度：表头按最大值、内容按 **P95**，取大者
 *  （一行里的超长值不该把整列撑到上限、把别的列挤出屏幕） */
const RESULT_CONTENT_PERCENTILE = 0.95
const naturalResultColWidth = (col, fontFamily, rows) => {
  const headerW = measureResultText(col, true, fontFamily) + RESULT_HEADER_EXTRA
  const widths = []
  for (const row of rows) {
    const v = row[col]
    widths.push(measureResultText(v == null ? 'NULL' : String(v), false, fontFamily))
  }
  let cellW = 0
  if (widths.length) {
    widths.sort((a, b) => a - b)
    const idx = Math.min(widths.length - 1, Math.floor(widths.length * RESULT_CONTENT_PERCENTILE))
    cellW = widths[idx]
  }
  const need = Math.max(headerW, cellW + RESULT_CELL_EXTRA)
  return Math.max(MIN_RESULT_COL_WIDTH, Math.min(MAX_RESULT_COL_WIDTH, Math.ceil(need)))
}

// 智能列宽：clamp(表头与前 30 行内容的真实文本宽 + 内边距, MIN, MAX)；不刻意撑满容器，右边留白
const measureResultColumns = () => {
  const wrap = resultTableWrapRef.value
  const cols = result.value?.columns || []
  const count = cols.length
  if (!wrap || !count) return
  const fontFamily = syncResultFontSpec(wrap)
  const sample = (result.value?.rows || []).slice(0, RESULT_SAMPLE_ROWS)

  // 与双击 / 右键「列宽自适应」共用同一个 naturalResultColWidth：默认宽度与自适应宽度必然相等
  const natural = cols.map((col) => naturalResultColWidth(col, fontFamily, sample))

  // 列宽只取内容自然宽度（手动拖过 / 右键自适应的列保持不动），
  // 不再为「撑满容器」而放大列 —— 字段少时表格就窄一些，右边留白
  const next = {}
  cols.forEach((c, i) => {
    next[i] = Math.round(resultManualCols.value.has(i) ? (resultColWidths.value[i] || natural[i]) : natural[i])
  })
  resultColWidths.value = next
}
// 新查询（列发生变化）时重新测量
watch(() => (result.value?.columns || []).join('\u0001'), async () => {
  resultColWidths.value = {}
  resultManualCols.value = new Set()
  hiddenResultCols.value = new Set()
  if ((result.value?.rows || []).length) {
    await nextTick()
    measureResultColumns()
  }
})
// 字段显示/隐藏变化后重新按可见列测量宽度
watch(() => resultVisibleCols.value.map(c => c.idx).join(','), () => {
  nextTick(() => { if ((result.value?.rows || []).length) measureResultColumns() })
})

// ========== 结果表格虚拟滚动：仅渲染可视区行，大幅减少 DOM 与内存 ==========
const VT_ROW_H = 32
const VT_BUFFER = 12
const vtStart = ref(0)
const vtEnd = ref(0)
const vtGapTop = ref(0)
const vtGapBottom = ref(0)
const vtVisibleRows = computed(() => {
  const rows = result.value?.rows || []
  return rows.slice(vtStart.value, vtEnd.value)
})
let vtRaf = 0
const onResultTableScroll = () => {
  if (vtRaf) return
  vtRaf = requestAnimationFrame(() => {
    vtRaf = 0
    const wrap = resultTableWrapRef.value
    const total = (result.value?.rows || []).length
    if (!wrap || !total) return
    const s = wrap.scrollTop
    const clientH = wrap.clientHeight || 1
    const first = Math.max(0, Math.floor(s / VT_ROW_H) - VT_BUFFER)
    const count = Math.ceil(clientH / VT_ROW_H) + VT_BUFFER * 2
    const last = Math.min(total, first + count)
    vtStart.value = first
    vtEnd.value = last
    vtGapTop.value = first * VT_ROW_H
    vtGapBottom.value = (total - last) * VT_ROW_H
  })
}
// 新结果集（行数变化）时回到顶部并初始化可视区间
watch(() => (result.value?.rows || []).length, () => {
  const wrap = resultTableWrapRef.value
  if (wrap) wrap.scrollTop = 0
  const n = result.value?.rows?.length || 0
  vtStart.value = 0
  vtEnd.value = Math.min(n, 100)
  vtGapTop.value = 0
  vtGapBottom.value = (n - vtEnd.value) * VT_ROW_H
  clearResultRowSelection()
  resultActiveCell.value = null
  resultSortColumn.value = ''
  resultSortDir.value = 'ASC'
  // 快照原始行序：取消排序（第三次点击）时还原
  unsortedRows.value = (result.value?.rows || []).slice()
  nextTick(() => onResultTableScroll())
})
// 结果区显示时初始化可视区间
watch(resultVisible, (v) => {
  if (v) nextTick(() => onResultTableScroll())
})
// 结果容器尺寸变化（拖拽分栏等）时重算可视区间
let vtObserver = null
watch(() => resultTableWrapRef.value, (wrap) => {
  if (vtObserver) { vtObserver.disconnect(); vtObserver = null }
  if (!wrap) return
  vtObserver = new ResizeObserver(() => onResultTableScroll())
  vtObserver.observe(wrap)
})
onBeforeUnmount(() => {
  if (vtRaf) cancelAnimationFrame(vtRaf)
  if (vtObserver) { vtObserver.disconnect(); vtObserver = null }
})

// 列宽拖拽热区：只在**表头**上判定，且热区很窄（Excel / DataGrip 同款）。
// 之前在数据格上也按 ±8~10px 判定，窄列两侧热区几乎盖满整格，
// 鼠标"放进单元格里"就会变成左右箭头，和普通单元格（手型）不一致
const RESULT_EDGE_GAP = 5
const resultEdgeColIdx = (e) => {
  const cell = e.target.closest('th')
  if (!cell || !cell.closest('table')) return -1
  const rect = cell.getBoundingClientRect()
  const x = e.clientX
  const vis = resultVisibleCols.value
  const lead = cell.cellIndex - RESULT_LEAD // 可见列位置
  let k = -1
  if (x >= rect.right - RESULT_EDGE_GAP && x <= rect.right + RESULT_EDGE_GAP) k = lead
  else if (lead - 1 >= 0 && x >= rect.left - RESULT_EDGE_GAP && x <= rect.left + RESULT_EDGE_GAP) k = lead - 1
  if (k < 0 || k >= vis.length) return -1
  return vis[k].idx
}
const onResultTableMove = (e) => {
  const wrap = resultTableWrapRef.value
  if (!wrap) return
  if (isResultColDragging()) return
  if (resultDrag) { wrap.style.cursor = 'col-resize'; return }
  const cell = e.target.closest('th, td')
  if (resultLastHover && resultLastHover !== cell) {
    resultLastHover.style.cursor = ''
  }
  const ci = resultEdgeColIdx(e)
  if (ci >= 0 && cell) {
    cell.style.cursor = 'col-resize'
    resultLastHover = cell
  } else {
    resultLastHover = null
  }
  wrap.style.cursor = ci >= 0 ? 'col-resize' : ''
}
const onResultTableLeave = () => {
  if (resultTableWrapRef.value) {
    if (!resultDrag) resultTableWrapRef.value.style.cursor = ''
    if (resultLastHover) { resultLastHover.style.cursor = ''; resultLastHover = null }
  }
}
const onResultTableDown = (e) => {
  if (e.button !== 0) return
  const ci = resultEdgeColIdx(e)
  if (ci < 0) {
    // 在数据格上按下 = 开始「单元格区域」这块选区：先清掉行/列选择。
    // Shift+按下 = 扩展已有区域（由框选模块处理），保留当前选区，这里不清
    if (!e.shiftKey && e.target && e.target.closest && e.target.closest('td[data-gkey]')) focusResultCells()
    return
  }
  e.preventDefault()
  const wrap = resultTableWrapRef.value
  // 首次拖拽前完成测量，避免列宽跳动
  if (!Object.keys(resultColWidths.value).length) measureResultColumns()
  const startW = resultColWidths.value[ci] || resultDefaultColWidth((result.value?.columns || [])[ci])
  // 同 TableDataView：**不再创建竖直参考线**。列宽实时跟随光标，已经足够说明拖动结果，
  // 再叠一根蓝线只会喧宾夺主（两处一起去掉，保持一致）。
  resultDrag = { ci, startW, startX: e.clientX, lastX: e.clientX }
  // 拖拽一开始就标记为「用户手动列」，后续自动测量不再覆盖（避免松手后被测量打回自然宽度）
  resultManualCols.value = new Set(resultManualCols.value).add(ci)
  document.body.style.cursor = 'col-resize'
  document.body.style.userSelect = 'none'
  if (wrap) wrap.classList.add('col-resizing')
  document.addEventListener('mousemove', onResultDragMove)
  document.addEventListener('mouseup', onResultDragEnd)
}
const onResultDragMove = (e) => {
  if (!resultDrag) return
  resultDrag.lastX = e.clientX
  // 实时应用列宽：列跟随光标变化，松手即最终值（不再回落到原始宽度）
  const liveW = Math.max(8, Math.round(resultDrag.startW + (e.clientX - resultDrag.startX)))
  resultColWidths.value = { ...resultColWidths.value, [resultDrag.ci]: liveW }
}
const onResultDragEnd = () => {
  if (resultDrag) {
    const finalW = Math.max(8, Math.round(resultDrag.startW + (resultDrag.lastX - resultDrag.startX)))
    resultColWidths.value = { ...resultColWidths.value, [resultDrag.ci]: finalW }
    resultManualCols.value = new Set(resultManualCols.value).add(resultDrag.ci)
  }
  resultDrag = null
  resultLastResizeAt = Date.now()
  resultColResizing.value = false
  document.body.style.cursor = ''
  document.body.style.userSelect = ''
  const wrap = resultTableWrapRef.value
  if (wrap) {
    wrap.classList.remove('col-resizing')
    wrap.style.cursor = ''
    if (resultLastHover) { resultLastHover.style.cursor = ''; resultLastHover = null }
  }
  document.removeEventListener('mousemove', onResultDragMove)
  document.removeEventListener('mouseup', onResultDragEnd)
}
onBeforeUnmount(() => { onResultDragEnd() })

// 框选区域复制完成（Ctrl+C 由框选模块自己的 copy 事件处理，这里只负责提示）
const onGridCopied = ({ rows, cols, header }) => {
  ElMessage.success(`已复制选区（${rows} 行 × ${cols} 列${header ? '，含列名' : ''}）`)
}

// 结果表格框选复制（类 Excel）：排除列宽拖拽边缘与表头（表头留给拖拽换列）
const gridSel = useExcelSelection({
  container: resultTableWrapRef,
  // 无修饰键拖动 = 新起一块区域；Shift+点击/拖动 = 扩展已有区域（模块内部先处理，见 onDown）
  shouldStart: (e) => !e.shiftKey && !e.target.closest('th') && resultEdgeColIdx(e) < 0,
  // 复制按数据取值（NULL 记为空串）：窗口化渲染下跨屏大选区也能完整复制
  valueAt: (r, c) => {
    if (r === 0) return (result.value?.columns || [])[c] || ''
    const row = (result.value?.rows || [])[r - 1]
    const name = (result.value?.columns || [])[c]
    if (!row || name === undefined) return ''
    return row[name] == null ? '' : String(row[name])
  },
  // 复制完成 → 按场景提示
  onCopied: onGridCopied
})
// 框选存在时不再单独画活动单元格描边，否则屏幕上会同时出现「两块选区」
const hasResultGridSelection = computed(() => !!gridSel?.range?.value)
/** 「当前没有行 / 列 / 框选」——活动单元格描边的前提条件（收成一个布尔，别让每格读 .size） */
const noResultBulkSelection = computed(() =>
  !resultSelectedSet.value.size && !selectedCols.value.size && !hasResultGridSelection.value)
// 三类选区（行 / 列 / 单元格框选）只能同时存在一块：框选区域一出现就清掉行、列选中。
// 否则「行选中」和「框选」会各自画各自的边线，看起来像碎成好几块
// （典型症状：全选若干行后又在某一行拖出一个小框，那一行会套一个额外的框）
watch(() => gridSel?.range?.value, (r) => {
  if (!r) return
  if (resultSelectedSet.value.size) resultSelectedSet.value = new Set()
  if (headerSelected.value) headerSelected.value = false
  if (selectedCols.value.size) clearColSelect()
})

// 点击结果表以外的界面 → 取消当前选中（活动单元格描边 / 选中列；框选由框选模块自己清）
const onDocClearResultPick = (e) => {
  const t = e.target
  if (!t || !t.closest) return
  if (t.closest('.grid-ctx-menu, .el-popper')) return // 菜单 / 弹层内部不处理
  // 只有落在「单元格 / 表头」上才算表格内点击（交给表格自己的逻辑切换选区）；
  // 滚动容器的空白处（表格右侧、下方留白）算外部 → 取消选中
  if (t.closest('.data-table-wrap') && t.closest('th, td')) return
  if (resultActiveCell.value) resultActiveCell.value = null
  if (selectedCols.value.size) clearColSelect()
  // 点结果表以外的界面：整块选中状态一起消失。
  // 例外只有「操作栏上的交互控件」——按钮 / 下拉 / 复选 / 分页，它们要用当前选择
  // （导出、列显示、分页、收起…），其余位置（标题空白、页脚空白、输入框…）一律取消
  if ((resultSelectedSet.value.size || headerSelected.value) &&
      !t.closest('button, .el-button, .el-dropdown, .el-checkbox, .el-radio, .el-switch, .el-pagination')) {
    clearResultRowSelection()
  }
}
document.addEventListener('mousedown', onDocClearResultPick)
onBeforeUnmount(() => document.removeEventListener('mousedown', onDocClearResultPick))

// ========== 结果表：行多选 ==========
const resultSelectedSet = ref(new Set())
// 行选择锚点：null = 无，-1 = 标题行，>=0 = 结果行下标
const resultLastAnchor = ref(null)
const headerSelected = ref(false) // 标题行是否也被选中（点/拖左侧行号列最上面那格）
const toggleResultRow = (idx) => {
  const s = new Set(resultSelectedSet.value)
  if (s.has(idx)) s.delete(idx); else s.add(idx)
  resultSelectedSet.value = s
}
const onResultRowClick = (idx, e) => {
  if (e.shiftKey && resultLastAnchor.value != null) {
    // Shift 连选：只做扩展，**锚点保持不动** ——
    // 否则第二次 Shift+点会以「上一次点到的那行」为起点（1~8 后再点 9 会变成 8~9）
    // 连选可能从标题行(-1)开始，交给 selectResultRowRange（它会正确设置 headerSelected）
    selectResultRowRange(resultLastAnchor.value, idx)
    return
  }
  if (e.ctrlKey || e.metaKey) {
    // 加减选也算「重新选数据行」：标题行的高亮要一起撤掉
    headerSelected.value = false
    toggleResultRow(idx)
  } else {
    // 单击选中数据行：标题行不能还亮着（否则看着像两块选区）
    headerSelected.value = false
    resultSelectedSet.value = new Set([idx])
  }
  resultLastAnchor.value = idx
}

// ========== 结果表：行号列按住拖动连选多行（Excel 行头）==========
let resRowDrag = null
// a/b 为结果行下标：-1 表示标题行
const selectResultRowRange = (a, b) => {
  const n = result.value?.rows?.length || 0
  const lo = Math.min(a, b)
  const hi = Math.max(a, b)
  headerSelected.value = lo <= -1 // 范围含标题行 → 标题行一起选中
  const i1 = Math.max(0, lo)
  const i2 = Math.min(n - 1, hi)
  // 拖选时增量增删（只碰区间两端变化的那几行）：每帧重建整段 Set 并替换 ref，
  // 会让所有读过它的单元格（可视区上千格）每帧重渲染一次。
  // 区间状态挂在本次拖拽对象上，每轮拖拽都是新对象，不会串到上一次的区间。
  const drag = resRowDrag
  if (drag && drag.lo != null && drag.count === n) {
    const s = resultSelectedSet.value
    for (let i = drag.lo; i < i1; i++) s.delete(i)
    for (let i = Math.max(i2 + 1, drag.lo); i <= drag.hi; i++) s.delete(i)
    for (let i = i1; i <= i2; i++) s.add(i)
    drag.lo = i1
    drag.hi = i2
    drag.count = n
    return
  }
  const s = new Set()
  for (let i = i1; i <= i2; i++) s.add(i)
  resultSelectedSet.value = s
  if (drag) { drag.lo = i1; drag.hi = i2; drag.count = n }
}
// 左上角（原全选复选框位置）= 标题行的行头：单击选中标题行，按住往下拖连选数据行
const onResultHeaderRowDown = (e) => {
  if (e.button !== 0) return // 右键交给 contextmenu 处理，别当成左键单击去打散选中
  if (gridSel && gridSel.suppressClick()) return
  focusResultRows()
  if (e.ctrlKey || e.metaKey) {
    headerSelected.value = !headerSelected.value
    resultLastAnchor.value = -1
    return
  }
  if (e.shiftKey && resultLastAnchor.value != null) {
    selectResultRowRange(resultLastAnchor.value, -1)
  } else {
    headerSelected.value = true
    resultSelectedSet.value = new Set()
    resultLastAnchor.value = -1
    resRowDrag = { anchor: -1, last: -1 }
    document.addEventListener('mousemove', onResultRowNumDragMove)
    document.addEventListener('mouseup', onResultRowNumDragEnd, { once: true })
  }
  if (resultTableWrapRef.value?.focus) resultTableWrapRef.value.focus({ preventScroll: true })
}
/**
 * 框选区域是否盖到标题行（第 0 行）。
 *
 * selEdges 只依赖这个布尔值，**不能让它直接读 range 对象**：
 * 框选拖动/自动滚动时 paint() 每帧都会重绘，range 的新旧值一旦参与依赖，
 * selEdges 就会每帧重算并返回新对象，进而把可视区上千个单元格全部重渲染一次。
 * 布尔值在「没变」时不会传播（Vue 的 computed 值稳定性），所以这里挡得住。
 */
const rangeCoversHeader = computed(() => {
  const r = gridSel?.range?.value
  return !!r && r.r1 === 0
})

// 行/列选中的「外沿」：连续选中的行段、列段只给首尾两端加边线，整块合起来是一个框
// （与单元格框选同一套观感），中间不画内部线
const selEdges = computed(() => {
  const n = result.value?.rows?.length || 0
  const rowTop = new Set()
  const rowBottom = new Set()
  // 标题行算「同一块选区的一部分」的两种情况：点/拖标题行行头选中它，
  // 或者框选区域覆盖到了第 0 行（Shift+点表头就会这样）——
  // 两种都要让首行数据的上边线让给标题行，否则会画出上下两条线、看着像两块
  const headOn = headerSelected.value || rangeCoversHeader.value
  // 只扫「当前真正渲染出来的那一段」：结果集可能上万行，全表扫描每次重绘都要走一遍，
  // 拖选/滚动时按帧重算就是白烧 CPU（扫窗口 ±1 行是为了让首尾行能读到相邻行）
  const from = Math.max(0, vtStart.value - 1)
  const to = Math.min(n, Math.max(vtEnd.value + 1, from + 1))
  for (let i = from; i < to; i++) {
    if (!resultSelectedSet.value.has(i)) continue
    // 上方相邻为「标题行（也选中）」时，首行的上边线让给标题行
    if (i === 0 ? !headOn : !resultSelectedSet.value.has(i - 1)) rowTop.add(i)
    if (i === n - 1 || !resultSelectedSet.value.has(i + 1)) rowBottom.add(i)
  }
  const cols = resultVisibleCols.value
  const colLeft = new Set()
  const colRight = new Set()
  let prevOn = false
  cols.forEach((c, i) => {
    const on = selectedCols.value.has(c.name)
    if (on && !prevOn) colLeft.add(c.name)
    if (!on && prevOn) colRight.add(cols[i - 1].name)
    prevOn = on
  })
  if (prevOn && cols.length) colRight.add(cols[cols.length - 1].name)
  // 标题行被选中：上边线永远在它自己身上；下边线只在「紧邻的首个数据行没被选中」时收口
  const headerBottom = headOn && !resultSelectedSet.value.has(0)
  return { rowTop, rowBottom, colLeft, colRight, headerBottom }
})
// 列选中时，只在「最后一行」收底边（中间行不封口）
const lastResultRowIdx = computed(() => (result.value?.rows?.length || 0) - 1)

// ===== 结果表也只有一块选中区域：三类选择互斥（行 / 列 / 单元格框选）=====
const clearResultRowSelection = () => {
  if (resultSelectedSet.value.size) resultSelectedSet.value = new Set()
  if (headerSelected.value) headerSelected.value = false
}
const clearResultColSelection = () => { if (selectedCols.value.size) clearColSelect() }
const clearResultGridRange = () => { if (gridSel && gridSel.hasSelection()) gridSel.clearRange() }
// 选中行 / 列：同时清掉活动单元格 —— 任何时候都只能有一块选区
const focusResultRows = () => { clearResultColSelection(); clearResultGridRange(); resultActiveCell.value = null }
const focusResultCols = () => { clearResultRowSelection(); clearResultGridRange(); resultActiveCell.value = null }
const focusResultCells = () => { clearResultRowSelection(); clearResultColSelection() }

const onResultRowNumDown = (idx, e) => {
  if (e.button !== 0) return // 右键交给 contextmenu 处理，别当成左键单击去打散选中
  if (gridSel && gridSel.suppressClick()) return
  focusResultRows()
  const prevAnchor = resultLastAnchor.value
  onResultRowClick(idx, e) // 复用：Shift 连选 / Ctrl 切换 / 单击单选
  if (resultTableWrapRef.value?.focus) resultTableWrapRef.value.focus({ preventScroll: true })
  if (e.ctrlKey || e.metaKey) return // Ctrl 单击是加减选，不进入拖拽
  resRowDrag = { anchor: e.shiftKey && prevAnchor != null ? prevAnchor : idx }
  document.addEventListener('mousemove', onResultRowNumDragMove)
  document.addEventListener('mouseup', onResultRowNumDragEnd, { once: true })
}
let resRowDragRaf = 0
let resRowDragMoved = false
const onResultRowNumDragMove = (e) => {
  if (!resRowDrag) return
  resRowDragPoint = { x: e.clientX, y: e.clientY }
  resRowDragMoved = true
  // 自动滚动的调度每次都走：它到边缘才开始循环、离开边缘就停，重新贴回边缘要能再被唤醒
  if (!resRowScrollRaf) resRowScrollRaf = requestAnimationFrame(resRowAutoScroll)
  if (resRowDragRaf) return
  // 合并到一帧：mousemove 可能远高于刷新率，而 elementFromPoint 会强制同步布局
  resRowDragRaf = requestAnimationFrame(() => {
    resRowDragRaf = 0
    if (resRowDrag) extendResultRowDragTo(resRowDragPoint.x, resRowDragPoint.y)
  })
}
// 指针下的行 → 从锚点连选到该行；指针拖出表格（拖到编辑器/分页栏）时按首/末行处理
const extendResultRowDragTo = (x, y) => {
  if (!resRowDrag) return
  const n = result.value?.rows?.length || 0
  if (!n) return
  const el = document.elementFromPoint(x, y)
  const tr = el && el.closest ? el.closest('tr[data-rid]') : null
  const rid = tr ? tr.getAttribute('data-rid') : null
  let idx = rid == null ? -1 : Number(rid) // 结果表的 data-rid 就是行下标
  if (!Number.isFinite(idx) || idx < 0 || idx >= n) {
    const host = resultTableWrapRef.value
    if (!host) return
    const rect = host.getBoundingClientRect()
    if (y >= rect.bottom) idx = n - 1
    else if (y <= rect.top) idx = 0
  }
  if (idx < 0 || idx === resRowDrag.last) return
  resRowDrag.last = idx
  selectResultRowRange(resRowDrag.anchor, idx)
}
// 拖到结果表上下边缘附近时自动滚动，长结果集也能一路拖到底
let resRowScrollRaf = 0
let resRowDragPoint = { x: 0, y: 0 }
const resRowAutoScroll = () => {
  resRowScrollRaf = 0
  if (!resRowDrag) return
  const host = resultTableWrapRef.value
  if (!host) return
  const rect = host.getBoundingClientRect()
  const EDGE = 26
  let dy = 0
  if (resRowDragPoint.y < rect.top + EDGE) dy = -1
  else if (resRowDragPoint.y > rect.bottom - EDGE) dy = 1
  if (!dy) return
  host.scrollTop += dy * 24
  extendResultRowDragTo(resRowDragPoint.x, resRowDragPoint.y)
  resRowScrollRaf = requestAnimationFrame(resRowAutoScroll)
}
const onResultRowNumDragEnd = () => {
  // 松手时补上还没落到选区里的最后一帧（拖得快时鼠标先松开、帧还没跑）
  if (resRowDragRaf) {
    cancelAnimationFrame(resRowDragRaf)
    resRowDragRaf = 0
    if (resRowDrag && resRowDragMoved) extendResultRowDragTo(resRowDragPoint.x, resRowDragPoint.y)
  }
  resRowDrag = null
  resRowDragMoved = false
  if (resRowScrollRaf) { cancelAnimationFrame(resRowScrollRaf); resRowScrollRaf = 0 }
  document.removeEventListener('mousemove', onResultRowNumDragMove)
}
onBeforeUnmount(onResultRowNumDragEnd)

// ========== 结果表：列拖拽排序（鼠标事件实现，避免原生 drag 在 sticky 表头上的卡顿） ==========
let resultColDragState = null // { fromIdx, startX, startY, moved, sourceEl, overEl, toIdx }
const RESULT_DRAG_THRESHOLD = 5
const isResultColDragging = () => !!(resultColDragState && resultColDragState.moved)
const onResultColDragStart = (ci, e) => {
  if (e.button !== 0) return
  // Shift+点击表头 = 扩展「单元格区域」（含标题行），交给框选模块，别切到列选中
  if (e.shiftKey && gridSel && gridSel.hasSelection()) return
  if (resultEdgeColIdx(e) >= 0) return // 靠近列右缘交给列宽拖拽
  // 直接拖动 = 连选多列（Excel 表头行为）；Alt+拖动 = 调整列顺序
  if (!e.altKey) { onResColSelectDown(ci, e); return }
  resultColDragState = { fromIdx: ci, startX: e.clientX, startY: e.clientY, moved: false, sourceEl: e.currentTarget, overEl: null, toIdx: -1 }
  document.addEventListener('mousemove', onResultColDragMoveDoc)
  document.addEventListener('mouseup', onResultColDragEndDoc)
}
const onResultColDragMoveDoc = (e) => {
  const st = resultColDragState
  if (!st) return
  if (!st.moved) {
    if (Math.abs(e.clientX - st.startX) < RESULT_DRAG_THRESHOLD && Math.abs(e.clientY - st.startY) < RESULT_DRAG_THRESHOLD) return
    st.moved = true
    if (st.sourceEl) st.sourceEl.classList.add('col-dragging')
    document.body.style.cursor = 'grabbing'
  }
  const el = document.elementFromPoint(e.clientX, e.clientY)
  const th = el && el.closest ? el.closest('th') : null
  if (st.overEl && st.overEl !== th) { st.overEl.classList.remove('col-drag-over'); st.overEl = null }
  const gk = th && th.getAttribute ? th.getAttribute('data-gkey') : null
  if (th && gk && gk.startsWith('0:')) {
    const toIdx = Number(gk.slice(2))
    st.toIdx = toIdx
    if (toIdx !== st.fromIdx) { th.classList.add('col-drag-over'); st.overEl = th }
  }
  e.preventDefault()
}
const onResultColDragEndDoc = () => {
  const st = resultColDragState
  if (!st) return
  if (st.overEl) st.overEl.classList.remove('col-drag-over')
  if (st.sourceEl) st.sourceEl.classList.remove('col-dragging')
  if (st.moved) {
    document.body.style.cursor = ''
    if (st.toIdx >= 0 && st.toIdx !== st.fromIdx) reorderResultColumns(st.fromIdx, st.toIdx)
  }
  resultColDragState = null
  resultLastDragAt = Date.now()
  document.removeEventListener('mousemove', onResultColDragMoveDoc)
  document.removeEventListener('mouseup', onResultColDragEndDoc)
}
const reorderResultColumns = (from, to) => {
  const r = result.value
  if (!r || !r.columns) return
  const oldCols = [...r.columns]
  const vis = [...resultVisibleCols.value] // [{ name, idx }]，idx 为原下标
  const fromPos = vis.findIndex(c => c.idx === from)
  const toPos = vis.findIndex(c => c.idx === to)
  if (fromPos < 0 || toPos < 0 || fromPos === toPos) return
  const moved = vis.splice(fromPos, 1)[0]
  vis.splice(toPos, 0, moved)
  // 生成新的整列顺序：隐藏列保持原位，可见槽位按新顺序填充
  const hidden = oldCols.map(n => hiddenResultCols.value.has(n))
  const order = [] // 新位置 -> 旧下标
  let vi = 0
  oldCols.forEach((n, i) => { order.push(hidden[i] ? i : vis[vi++].idx) })
  const newCols = order.map(i => oldCols[i])
  // 用下标置换重映射列宽与「手动列」标记，跟随列移动（对重名列也安全）
  const oldW = { ...resultColWidths.value }
  const newW = {}
  const newManual = new Set()
  order.forEach((oldIdx, newIdx) => {
    if (oldW[oldIdx] != null) newW[newIdx] = oldW[oldIdx]
    if (resultManualCols.value.has(oldIdx)) newManual.add(newIdx)
  })
  result.value = { ...r, columns: newCols }
  resultColWidths.value = newW
  resultManualCols.value = newManual
}

// ========== 结果表：列宽双击右缘自适应 ==========
// 它算出来的就是「默认宽度」：和自动测量共用 naturalResultColWidth，两条路不可能再各算一个值。
// （历史差异：这里以前是 +42 的旧预算、上限 800、采样 50 行 —— 所以双击完跟默认宽度不一样。）
// 按内容自适应列宽（右键菜单「列宽自适应」与双击表头右缘共用）
const autoFitResultCol = (ci) => {
  const wrap = resultTableWrapRef.value
  const col = (result.value?.columns || [])[ci]
  if (!wrap || !col) return
  const fontFamily = syncResultFontSpec(wrap)
  const rows = (result.value?.rows || []).slice(0, RESULT_SAMPLE_ROWS)
  const w = naturalResultColWidth(col, fontFamily, rows)
  resultColWidths.value = { ...resultColWidths.value, [ci]: w }
  resultManualCols.value = new Set(resultManualCols.value).add(ci)
}
// 选中多列时一起自适应
const autoFitSelectedResultCols = () => {
  for (const c of resultVisibleCols.value) {
    if (selectedCols.value.has(c.name)) autoFitResultCol(c.idx)
  }
}
const onResultHeaderDblClick = (e, ci) => {
  const rect = e.currentTarget.getBoundingClientRect()
  if (e.clientX < rect.right - 12) return // 只有双击右缘才自适应，避免误触
  autoFitResultCol(ci)
}

// ========== 结果表：活动单元格 + 键盘导航（类 Excel）==========
const resultActiveCell = ref(null) // { rowIdx, col }
const onResultCellClick = (rowIdx, col) => {
  // 左键点单元格 = 离开「选中列」上下文，回到单元格上下文（否则右键菜单会一直是列操作）
  if (selectedCols.value.size) clearColSelect()
  resultActiveCell.value = { rowIdx, col }
  if (resultTableWrapRef.value?.focus) resultTableWrapRef.value.focus({ preventScroll: true })
}
// 可见列位置 ↔ 原始列下标（data-gkey 用的是原始下标 c.idx，隐藏列会跳号）
const resVisibleIdxOf = (name) => resultVisibleCols.value.findIndex(v => v.name === name)
const resGkeyColOf = (name) => {
  const i = resVisibleIdxOf(name)
  return i < 0 ? -1 : resultVisibleCols.value[i].idx
}
const resGkeyOf = (rowIdx, name) => {
  const c = resGkeyColOf(name)
  return (rowIdx < 0 || c < 0) ? null : { r: rowIdx + 1, c }
}
// 把活动单元格落到 (rIdx, visIdx)（rIdx 为结果行下标，visIdx 为可见列下标）
const applyResActive = (rIdx, visIdx, extend) => {
  const rows = result.value?.rows || []
  const vis = resultVisibleCols.value
  if (!rows.length || !vis.length) return
  const r = Math.max(0, Math.min(rows.length - 1, rIdx))
  const ci = Math.max(0, Math.min(vis.length - 1, visIdx))
  const from = resultActiveCell.value
  const anchorG = extend ? (gridSel.anchor() || (from ? resGkeyOf(from.rowIdx, from.col) : null)) : null
  if (extend) focusResultCells() // Shift+方向键扩出的区域也是「唯一那块选区」，清掉行/列选择
  resultActiveCell.value = { rowIdx: r, col: vis[ci].name }
  if (anchorG) gridSel.setRange(anchorG.r, anchorG.c, r + 1, vis[ci].idx)
  else gridSel.clearRange()
  ensureResultActiveVisible()
}
// 移动活动单元格：edge = 跳到数据边缘（Ctrl+方向键），extend = 扩展选区（Shift+方向键）
const moveResultActive = (dr, dc, { edge = false, extend = false } = {}) => {
  const from = resultActiveCell.value
  if (!from) return
  const rows = result.value?.rows || []
  const vis = resultVisibleCols.value
  const c0 = resVisibleIdxOf(from.col)
  if (c0 < 0) return
  let r = from.rowIdx + dr
  let c = c0 + dc
  if (edge) {
    if (dr) r = dr > 0 ? rows.length - 1 : 0
    if (dc) c = dc > 0 ? vis.length - 1 : 0
  }
  applyResActive(r, c, extend)
}
// Tab / Shift+Tab：左右移动，行末换行
const tabMoveResult = (dir) => {
  const from = resultActiveCell.value
  if (!from) return
  const vis = resultVisibleCols.value
  let c = resVisibleIdxOf(from.col) + dir
  let r = from.rowIdx
  if (c >= vis.length) { c = 0; r += 1 }
  else if (c < 0) { c = vis.length - 1; r -= 1 }
  applyResActive(r, c, false)
}
const pageRowStepResult = () => Math.max(1, Math.floor((resultTableWrapRef.value?.clientHeight || 400) / 32) - 1)
const ensureResultActiveVisible = () => {
  nextTick(() => {
    const a = resultActiveCell.value
    if (!a || !resultTableWrapRef.value) return
    const ci = resGkeyColOf(a.col)
    if (ci < 0) return
    const el = resultTableWrapRef.value.querySelector(`[data-gkey="${(a.rowIdx + 1)}:${ci}"]`)
    if (el) el.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  })
}
const onResultKeyDown = (e) => {
  if (!rootRef.value) return
  const active = document.activeElement
  if (active && active.closest && active.closest('input, textarea, .el-select, .el-dropdown, .monaco-editor')) return
  // Ctrl/Cmd+A：全选结果行（右键菜单里的「全选」已移除，改走快捷键）
  if ((e.ctrlKey || e.metaKey) && (e.key === 'a' || e.key === 'A')) {
    const inRoot = rootRef.value.contains(active)
    const onBodyVisible = active === document.body && rootRef.value.getClientRects().length > 0
    if (!inRoot && !onBodyVisible) return
    if (!resultVisible.value) return
    e.preventDefault()
    focusResultRows()
    headerSelected.value = false // 全选的是数据行，标题行不跟着亮
    const n = result.value?.rows?.length || 0
    resultSelectedSet.value = n ? new Set(Array.from({ length: n }, (_, i) => i)) : new Set()
    return
  }
  if (!rootRef.value.contains(active)) return
  // Esc：清空当前这块选区（行 / 列 / 框选）
  if (e.key === 'Escape' &&
      (resultSelectedSet.value.size || headerSelected.value || selectedCols.value.size || gridSel.hasSelection())) {
    e.preventDefault()
    clearResultRowSelection()
    clearResultColSelection()
    clearResultGridRange()
    return
  }
  if (!resultVisible.value) return
  const mod = e.ctrlKey || e.metaKey
  // Ctrl+C：复制当前这块选区。主路径是浏览器派发的 copy 事件（见 onDocCopy，任何焦点下都能用）；
  // 这里只兜底 —— 个别环境不派发 copy 事件时，用 navigator.clipboard 补写一次。
  // 注意不能 preventDefault，否则会把 copy 事件本身掐掉
  if (mod && !e.shiftKey && (e.key === 'c' || e.key === 'C')) {
    const startedAt = Date.now()
    setTimeout(() => {
      if (copyEventAt >= startedAt) return          // copy 事件已经处理过
      if (gridSel && gridSel.hasSelection()) return // 框选模块自己会处理
      if (inEditableFocus()) return
      const text = selectionAsTsv()
      if (text) writeClipboard(text, copyHint())
    }, 0)
    return
  }
  if (!resultActiveCell.value) return
  // 方向键：Shift 扩展选区，Ctrl 跳到数据边缘，两者可叠加
  if (e.key.startsWith('Arrow')) {
    e.preventDefault()
    const dr = e.key === 'ArrowUp' ? -1 : e.key === 'ArrowDown' ? 1 : 0
    const dc = e.key === 'ArrowLeft' ? -1 : e.key === 'ArrowRight' ? 1 : 0
    moveResultActive(dr, dc, { edge: mod, extend: e.shiftKey })
    return
  }
  // Home / End / PageUp / PageDown
  if (e.key === 'Home' || e.key === 'End' || e.key === 'PageUp' || e.key === 'PageDown') {
    e.preventDefault()
    const vis = resultVisibleCols.value
    const rows = result.value?.rows || []
    const cur = resultActiveCell.value.rowIdx
    if (e.key === 'Home') applyResActive(mod ? 0 : cur, 0, e.shiftKey)
    else if (e.key === 'End') applyResActive(mod ? rows.length - 1 : cur, vis.length - 1, e.shiftKey)
    else moveResultActive(e.key === 'PageDown' ? pageRowStepResult() : -pageRowStepResult(), 0, { extend: e.shiftKey })
    return
  }
  // Tab / Shift+Tab：左右移动，行末换行
  if (e.key === 'Tab') {
    e.preventDefault()
    tabMoveResult(e.shiftKey ? -1 : 1)
  }
}

// ========== 结果表：右键菜单 ==========
const resCtx = ref({ visible: false, x: 0, y: 0, items: [], rowIdx: -1, col: '' })
const resCtxSub = ref(null)
let resCtxFrom = 'cell' // 右键来源，决定菜单给「列 / 行 / 区域 / 单元格」哪一套
const closeResCtx = () => { resCtx.value = { ...resCtx.value, visible: false }; resCtxSub.value = null }
const closeResCtxSub = () => { resCtxSub.value = null }
// 二级子菜单：悬停带 sub 的项时在其右侧弹出
const onResCtxItemHover = (item, i, e) => {
  if (!item.sub || !item.sub.length) { closeResCtxSub(); return }
  const el = e && e.currentTarget
  if (!el) return
  const menuEl = el.closest('.grid-ctx-menu')
  const menuRect = menuEl ? menuEl.getBoundingClientRect() : { left: 0, right: 0, top: 0 }
  const itemRect = el.getBoundingClientRect()
  resCtxSub.value = { parentIndex: i, items: item.sub, x: menuRect.right + 2, y: itemRect.top, menuLeft: menuRect.left }
  nextTick(() => {
    const subEl = document.querySelector('.grid-ctx-sub')
    if (!subEl || !resCtxSub.value) return
    const r = subEl.getBoundingClientRect()
    let x = resCtxSub.value.x
    let y = resCtxSub.value.y
    if (x + r.width > window.innerWidth - 4) x = Math.max(4, resCtxSub.value.menuLeft - r.width - 2)
    if (y + r.height > window.innerHeight - 4) y = Math.max(4, window.innerHeight - r.height - 4)
    resCtxSub.value = { ...resCtxSub.value, x, y }
  })
}
const writeClipboard = (text, msg) => {
  if (!text) return
  const done = () => { if (msg) ElMessage.success(msg) }
  // 降级：非安全上下文（http 且非 localhost）里 navigator.clipboard 不可用，
  // 用临时 textarea + execCommand 兜底，保证「复制」在哪都能用
  const fallback = () => {
    try {
      const ta = document.createElement('textarea')
      ta.value = text
      ta.setAttribute('readonly', '')
      ta.style.cssText = 'position:fixed;left:-9999px;top:0;opacity:0'
      document.body.appendChild(ta)
      ta.select()
      const ok = document.execCommand('copy')
      document.body.removeChild(ta)
      if (ok) done()
      else if (msg) ElMessage.error(t('sqlq.copyFailed'))
    } catch { if (msg) ElMessage.error(t('sqlq.copyFailed')) }
  }
  if (navigator.clipboard?.writeText) navigator.clipboard.writeText(text).then(done).catch(fallback)
  else fallback()
}
const onResultGridContextMenu = (e) => { e.preventDefault(); openResCtx(e.clientX, e.clientY, -1, '', 'grid') }
const onResultHeaderContextMenu = (e, col) => { e.preventDefault(); openResCtx(e.clientX, e.clientY, -1, col, 'header') }
const onResultRowContextMenu = (e, rowIdx) => { e.preventDefault(); openResCtx(e.clientX, e.clientY, rowIdx, '', 'row') }
const onResultContextMenu = (e, rowIdx, col) => { e.preventDefault(); openResCtx(e.clientX, e.clientY, rowIdx, col, 'cell') }

// ========== 行详情：双击 / 右键行号查看整行字段明细（复用单元格详情弹窗） ==========
// ===== 单元格快捷编辑：双击数据格（或右键「编辑此单元格」）→ 改值 → 生成 UPDATE 回填编辑器 =====
// **不执行**，回填后由用户确认再跑（配合写操作影响行预览双保险）。
// 更新哪张表：单表查询直接用 FROM 的表；JOIN 结果列出 FROM/JOIN 的全部表让用户挑
// （列归属只有用户知道，工具不猜）。WHERE：行里有名为 id 的列按主键定位；
// 否则用「其余全部列」拼 AND 条件（NULL → IS NULL），总是精确命中本行，安全但可能较长。
const quickEditTablesOf = () => {
  const text = lastExecSql || sql.value || ''
  if (!text) return []
  const tables = []
  const re = /\b(?:from|join)\s+[`"']?([a-z_][\w$]*(?:\.[a-z_][\w$]*)?)/gi
  let m
  while ((m = re.exec(text))) {
    if (!tables.includes(m[1])) tables.push(m[1])
  }
  return tables
}
const pickUpdateTable = (tables) => new Promise((resolve) => {
  let sel = tables[0]
  ElMessageBox({
    title: t('sqlq.cellEditPickTitle'),
    message: h('div', null, [
      h('p', { style: 'margin:0 0 10px;font-size:12px;color:var(--dc-text-dim);line-height:1.6' }, t('sqlq.cellEditPickTip')),
      h(ElSelect, {
        modelValue: sel,
        'onUpdate:modelValue': (v) => { sel = v },
        style: 'width:100%', filterable: true
      }, () => tables.map((tb) => h(ElOption, { key: tb, value: tb, label: tb })))
    ]),
    confirmButtonText: t('common.confirm'),
    cancelButtonText: t('common.cancel'),
    showCancelButton: true,
    closeOnClickModal: false
  }).then(() => resolve(sel)).catch(() => resolve(null))
})
const sqlLiteralOf = (v) => {
  if (v === null || v === undefined) return 'NULL'
  if (typeof v === 'number') return String(v)
  const s = String(v)
  if (/^-?\d+(\.\d+)?$/.test(s)) return s
  return `'${s.replace(/'/g, "''")}'`
}
const onCellQuickEdit = async (rowIdx, colName, row) => {
  const tables = quickEditTablesOf()
  if (!tables.length) { ElMessage.warning(t('sqlq.cellEditNoTable')); return }
  let table = tables[0]
  if (tables.length > 1) {
    const picked = await pickUpdateTable(tables)
    if (!picked) return
    table = picked
  }
  const oldRaw = row[colName]
  const oldShown = oldRaw == null ? nullDisplay() : String(formatDbValue(oldRaw))
  let newVal = ''
  try {
    const r = await ElMessageBox.prompt(
      `${table}.${colName} = ${oldRaw === null || oldRaw === undefined ? 'NULL' : oldShown}`,
      t('sqlq.cellEditTitle'),
      {
        inputValue: oldRaw == null ? '' : String(formatDbValue(oldRaw)),
        inputPlaceholder: t('sqlq.cellEditPh'),
        confirmButtonText: t('sqlq.cellEditGen'),
        cancelButtonText: t('common.cancel'),
        closeOnClickModal: false
      }
    )
    newVal = String(r.value ?? '').trim()
  } catch { return }
  if (newVal === oldShown) { ElMessage.info(t('sqlq.cellEditNoChange')); return }
  const setLit = /^null$/i.test(newVal) ? 'NULL' : sqlLiteralOf(newVal)
  const keys = Object.keys(row)
  const whereParts = []
  const pk = keys.find((k) => k.toLowerCase() === 'id')
  if (pk) {
    whereParts.push(`${pk} = ${sqlLiteralOf(row[pk])}`)
  } else {
    keys.forEach((k) => {
      if (k === colName) return
      const v = row[k]
      if (v === null || v === undefined) whereParts.push(`${k} IS NULL`)
      else whereParts.push(`${k} = ${sqlLiteralOf(v)}`)
    })
  }
  if (!whereParts.length) { ElMessage.warning(t('sqlq.cellEditNoWhere')); return }
  // 记入**编辑缓冲**：不立即执行，底栏出现「提交修改 / 放弃」。提交时批量执行
  // （事务模式开着就落在事务里，由用户手动 COMMIT）。
  const parsed = /^null$/i.test(newVal) ? null : (/^-?\d+(\.\d+)?$/.test(newVal) ? Number(newVal) : newVal)
  pendingEdits.value.push({
    rowIdx, colName, table,
    setSql: `${colName} = ${setLit}`,
    whereSql: whereParts.join('\n  AND '),
    newParsed: parsed,
    snapshot: result.value,
    rowsRef: result.value?.rows
  })
}

// ===== 编辑缓冲与提交（结果集直接编辑）=====
// 快照守卫：编辑期间翻页/新查询/重排过（rows 数组被换掉）⇒ 行号不可信，提交直接拒绝。
const pendingEdits = ref([])
const editsBusy = ref(false)
const discardEdits = () => { pendingEdits.value = [] }
const commitEdits = async () => {
  if (editsBusy.value || !pendingEdits.value.length) return
  const first = pendingEdits.value[0]
  if (!first || result.value !== first.snapshot || result.value?.rows !== first.rowsRef) {
    ElMessage.warning(t('sqlq.editsStale'))
    pendingEdits.value = []
    return
  }
  const connId = selectedConnId.value || props.conn.id
  const db = selectedSchema.value
    ? `${selectedDatabase.value}.${selectedSchema.value}`
    : selectedDatabase.value || undefined
  const sqlText = pendingEdits.value
    .map(e => `UPDATE ${e.table}\nSET ${e.setSql}\nWHERE ${e.whereSql};`)
    .join('\n')
  editsBusy.value = true
  try {
    execId.value = 'e_' + Date.now() + '_' + Math.random().toString(36).slice(2, 8)
    const b = await executeSqlBatch(connId, sqlText, db, execId.value, null)
    const results = (b && Array.isArray(b.results)) ? b.results : []
    const failed = results.find(r => !r.success)
    if (failed) { ElMessageBoxWithFix(failed.message || t('ai.runFailed')); return }
    // 成功：新值写回行数据（界面立即更新），清空缓冲
    for (const e of pendingEdits.value) {
      const row = result.value?.rows?.[e.rowIdx]
      if (row) row[e.colName] = e.newParsed
    }
    pendingEdits.value = []
    ElMessage.success(t('sqlq.editsCommitted', { n: results.reduce((m, r) => m + (r.affectedRows || 0), 0) })
      + (txMode.value ? ' ' + t('sqlq.rememberCommit') : ''))
  } catch (e) {
    ElMessageBoxWithFix(e?.message || t('ai.runFailed'))
  } finally {
    editsBusy.value = false
  }
}

// ===== 事务模式 =====
// begin → 后端关掉编辑器会话（亲和泳道）的 autocommit，之后的写语句都挂在事务里；
// 提交/回滚 → 收尾并交回 autocommit。全 agent 数据源通用（宿主走 JDBC 标准接口）。
const txMode = ref(false)
const txDirty = ref(false)
const txBusy = ref(false)
// 事务动作要带「与执行查询相同的库」：后端据此解析同一目标（影子连接等），泳道才对得上
const txDatabaseOf = () => selectedSchema.value
  ? `${selectedDatabase.value}.${selectedSchema.value}`
  : selectedDatabase.value || ''
const txEnd = async (action) => {
  if (txBusy.value || !txMode.value) return
  const connId = selectedConnId.value || props.conn.id
  txBusy.value = true
  try {
    const r = await txControl(connId, action, txDatabaseOf())
    if (r && r.success) {
      txMode.value = false
      txDirty.value = false
      ElMessage.success(action === 'commit' ? t('sqlq.txCommitted') : t('sqlq.txRolledBack'))
    } else {
      ElMessageBoxWithFix((r && r.message) || t('sqlq.txFail'))
    }
  } catch (e) {
    ElMessageBoxWithFix(e?.message || t('sqlq.txFail'))
  } finally {
    txBusy.value = false
  }
}
const txCommit = () => txEnd('commit')
const txRollback = () => txEnd('rollback')
const toggleTxMode = async () => {
  if (txBusy.value) return
  const connId = selectedConnId.value || props.conn.id
  txBusy.value = true
  try {
    if (txMode.value) {
      // 关闭事务模式：挂着未提交事务先回滚，不留一个悬着的事务占着连接
      if (txDirty.value) {
        try { await txControl(connId, 'rollback', txDatabaseOf()) } catch { /* 连接可能已断 */ }
      }
      txMode.value = false
      txDirty.value = false
      ElMessage.success(t('sqlq.txOff'))
    } else {
      const r = await txControl(connId, 'begin', txDatabaseOf())
      if (r && r.success) {
        txMode.value = true
        ElMessage.success(t('sqlq.txOn'))
      } else {
        ElMessageBoxWithFix((r && r.message) || t('sqlq.txFail'))
      }
    }
  } catch (e) {
    ElMessageBoxWithFix(e?.message || t('sqlq.txFail'))
  } finally {
    txBusy.value = false
  }
}
// 切换连接：事务绑定在旧连接的会话上 —— 回滚收尾并把状态复位
watch(selectedConnId, (nv, ov) => {
  if (txMode.value && ov) {
    txControl(ov, 'rollback', txDatabaseOf()).catch(() => {})
    txMode.value = false
    txDirty.value = false
    ElMessage.info(t('sqlq.txReset'))
  }
  discardEdits()
})

// ===== CSV 导入向导 =====
const csvVisible = ref(false)
const connectionKind = computed(() => String(selectedConn.value?.type || props.conn?.type || '').toLowerCase())
const onCsvImported = ({ table, rows }) => {
  // 导入成功：提示用一条查询验证（不自动执行，避免大结果意外刷屏）
  lastExecSql = `SELECT * FROM ${table}`
  ElMessage.info(t('csv.verifyTip', { n: rows, table }))
}

// ===== SQL 片段库：命名保存常用 SQL，双击插入（localStorage，跨会话保留） =====
const SNIP_LS = 'dbmind_snippets'
const snippets = ref([])
const loadSnippets = () => {
  try { snippets.value = JSON.parse(localStorage.getItem(SNIP_LS) || '[]') } catch { snippets.value = [] }
}
loadSnippets()
const saveSnippet = async () => {
  const text = (selTextOf(editorInstance) || sql.value || '').trim()
  if (!text) { ElMessage.warning(t('sqlq.snipEmpty')); return }
  try {
    const firstLine = text.split('\n').find(l => l.trim()) || ''
    const r = await ElMessageBox.prompt(firstLine.slice(0, 60), t('sqlq.snipSaveTitle'), {
      inputValue: firstLine.trim().slice(0, 30),
      inputPlaceholder: t('sqlq.snipNamePh'),
      confirmButtonText: t('common.save'),
      cancelButtonText: t('common.cancel'),
      closeOnClickModal: false
    })
    const name = String(r.value || '').trim()
    if (!name) return
    snippets.value.unshift({ id: 's_' + Date.now(), name, sql: text, ts: Date.now() })
    if (snippets.value.length > 100) snippets.value.pop()
    localStorage.setItem(SNIP_LS, JSON.stringify(snippets.value))
    ElMessage.success(t('sqlq.snipSaved'))
  } catch { /* 取消 */ }
}
const removeSnippet = (i) => {
  snippets.value.splice(i, 1)
  localStorage.setItem(SNIP_LS, JSON.stringify(snippets.value))
}
/** 插入片段：编辑器有选区就替换，否则插到光标处 */
const applySnippet = (sn) => {
  const ed = editorInstance
  if (!ed) return
  const sel = ed.getSelection()
  const range = (sel && !sel.isEmpty()) ? sel : ed.getModel().getFullModelRange().setStartPosition(
    ed.getPosition().lineNumber, ed.getPosition().column
  ).setEndPosition(ed.getPosition().lineNumber, ed.getPosition().column)
  ed.executeEdits('dbmind-sql', [{ range, text: sn.sql, forceMoveMarkers: true }])
  ed.pushUndoStop()
  ed.focus()
}

const rowDetail = ref({ visible: false, title: '', text: '' })
const openRowDetail = (rowIdx) => {
  const rows = result.value?.rows || []
  const cols = result.value?.columns || []
  const row = rows[rowIdx]
  if (!row) return
  const lines = cols.map(c => {
    const v = row[c]
    // 与网格同口径：ISO 时间串的 `T` 换成空格（详情里全是原始值会看着割裂）
    return c + '：' + (v == null ? nullDisplay() : String(formatDbValue(v)))
  })
  rowDetail.value = { visible: true, title: t('sqlq.rowDetailTitle', { n: rowIdx + 1 }), text: lines.join('\n') }
}

// 右键目标的行/列集合：右键落在框选区域内用选区，否则用当前单元格/行
const resCtxSelection = () => {
  const { rowIdx, col } = resCtx.value
  const allCols = result.value?.columns || []
  const allRows = result.value?.rows || []
  const visCols = resultVisibleCols.value
  const range = gridSel?.range?.value || null
  // 只有拖出「多格」的框选才算区域；单击产生的单格不算，避免盖掉显式的选中列
  const rangeHasArea = !!range && (range.r1 !== range.r2 || range.c1 !== range.c2)
  // 数据区 / 表格空白处右键：只要存在框选区域就按框选取
  // （右键落在框选外也不降级成单元格 —— 有选中就按选中给功能）
  if (rangeHasArea && (resCtxFrom === 'cell' || resCtxFrom === 'grid')) {
    const rows = []
    for (let r = Math.max(range.r1, 1); r <= range.r2; r++) {
      const row = allRows[r - 1]
      if (row) rows.push(row)
    }
    const cols = visCols.filter(c => c.idx >= range.c1 && c.idx <= range.c2).map(c => c.name)
    if (rows.length && cols.length) return { rows, cols }
  }
  // 勾选行 / 选中列 决定复制范围（Ctrl+A 全选行即「复制全部」）
  const checkedIdx = [...resultSelectedSet.value].sort((a, b) => a - b)
  const pickedCols = visCols.filter(c => selectedCols.value.has(c.name)).map(c => c.name)
  if (checkedIdx.length || pickedCols.length) {
    const rows = checkedIdx.length ? checkedIdx.map(i => allRows[i]).filter(Boolean) : allRows
    const cols = pickedCols.length ? pickedCols : visCols.map(c => c.name)
    if (rows.length && cols.length) return { rows, cols }
  }
  const row = rowIdx >= 0 ? allRows[rowIdx] : null
  return row ? { rows: [row], cols: col ? [col] : visCols.map(c => c.name) } : { rows: [], cols: [] }
}
const resSqlVal = (v) => {
  if (v === null || v === undefined) return 'NULL'
  if (typeof v === 'number') return String(v)
  if (typeof v === 'boolean') return v ? '1' : '0'
  return `'${String(v).replace(/'/g, "''")}'`
}
// 从编辑器 SQL 中尽力猜出表名，猜不到用占位符（查询结果可能来自多表 JOIN）
const guessTableName = () => {
  const s = String(sql.value || '').replace(/\s+/g, ' ')
  const m = /\bfrom\s+([`"[\].\w]+)/i.exec(s)
  return m ? m[1].replace(/[`"[\]]/g, '') : 'your_table'
}
// 结果集排序（对当前已加载的结果行做前端排序，不改动 SQL / 后端分页）
const sortResultBy = (col, dir) => {
  const rows = result.value?.rows
  if (!col || !rows || !rows.length) return
  const sorted = [...rows].sort((a, b) => {
    const va = a[col]
    const vb = b[col]
    if (va == null && vb == null) return 0
    if (va == null) return 1
    if (vb == null) return -1
    const na = Number(va)
    const nb = Number(vb)
    const bothNum = !isNaN(na) && !isNaN(nb) && String(va).trim() !== '' && String(vb).trim() !== ''
    const c = bothNum ? na - nb : String(va).localeCompare(String(vb), 'zh')
    return dir === 'DESC' ? -c : c
  })
  // 原地改 rows，不换对象：结果对象被异步计数引用着（fetchCountFor 回填 totalCount），
  // 整个替换会让在途的计数写进旧对象 —— 总数就再也不显示了
  result.value.rows = sorted
  // 行序变化后，按索引维护的选中/活动单元格已失效，清空避免错位
  resultSelectedSet.value = new Set()
  resultActiveCell.value = null
  resultLastAnchor.value = -1
}
// 结果表表头点击排序：升序 → 降序 → **取消（恢复原始行序）**，与表预览同一套三击循环
const resultSortColumn = ref('')
const resultSortDir = ref('ASC')
// 载入时的原始行序快照：取消排序时还原
const unsortedRows = ref([])
// ===== 结果表头的**字段注释**：单表 SELECT 时后端按方言取（MySQL/Doris 走
// information_schema、PG/Kingbase 走 pg_description、SQL Server 走 extended_properties、
// Oracle/DM 走 all_col_comments、ClickHouse 走 system.columns……），表头第二行显示。
// 只对「无 JOIN / 聚合 / 去重的单表查询」生效 —— 猜错表名的注释张冠李戴比没有更糟。
const resultColComments = ref({})
const resComment = (name) => (result.value?.columnComments || {})[String(name).toLowerCase()] || ''
const loadResColumnComments = (execSql, connId, db) => {
  resultColComments.value = {}
  const text = String(execSql || '').replace(/\s+/g, ' ')
  const fromM = /\bfrom\s+([`"[\]\w.]+)\s*(?:;|$)/i.exec(text)
  const multi = /\bjoin\b|\bunion\b|\bgroup\s+by\b|\bdistinct\b/i.test(text)
  if (!fromM || multi) return
  const raw = fromM[1].replace(/[`"[\]]/g, '')
  const tableName = raw.split('.').pop()
  const snapshot = result.value
  getColumnComments(connId, db || '', tableName)
    .then((map) => {
      if (result.value !== snapshot || !map) return
      resultColComments.value = map
      // 原地写 columnComments，不换对象：注释请求与异步计数并发，整对象替换会让
      // 晚一步回来的计数写进旧对象 —— 大表计数慢（~1s）被注释（快）抢先替换，
      // 总数永远显示不出来（小表计数快、碰巧先到才显得"正常"）
      result.value.columnComments = map
    })
    .catch(() => {})
}
// ========== 选中整列（Excel 式）：单击表头选中整列，Ctrl/Cmd 加减选，Shift 连选一段 ==========
const selectedCols = ref(new Set())
const lastColAnchor = ref('')
const clearColSelect = () => { selectedCols.value = new Set(); lastColAnchor.value = '' }
const toggleColSelect = (name) => {
  const s = new Set(selectedCols.value)
  if (s.has(name)) s.delete(name); else s.add(name)
  selectedCols.value = s
  lastColAnchor.value = name
}
const selectSingleCol = (name) => {
  selectedCols.value = new Set([name])
  lastColAnchor.value = name
}
const selectColRange = (name) => {
  const cols = resultVisibleCols.value.map(c => c.name)
  const a = cols.indexOf(lastColAnchor.value)
  const b = cols.indexOf(name)
  if (a < 0 || b < 0) { selectSingleCol(name); return }
  selectedCols.value = new Set(cols.slice(Math.min(a, b), Math.max(a, b) + 1))
}
// 表头点击：单击选中整列、Ctrl/Cmd 加/减选、Shift 连选一段（排序改点表头右侧的按钮）
const onResultHeaderClickOrSelect = (name, e) => {
  if (Date.now() < resColSelClickUntil) return // 刚拖过连选，紧接着的这次 click 忽略
  // Shift+点击表头 = 扩展单元格区域（框选模块负责），不要再切列选中
  if (e.shiftKey && gridSel && gridSel.hasSelection()) return
  focusResultCols() // 列选择这块选区生效时，清掉行选择与框选
  if (e.ctrlKey || e.metaKey) { toggleColSelect(name); return }
  if (e.shiftKey) { selectColRange(name); return }
  selectSingleCol(name)
}

// ========== 结果表：列头拖动连选多列（Excel 表头行为）==========
let resColSelDrag = null
let resColSelClickUntil = 0
const onResColSelectDown = (ci, e) => {
  const cols = resultVisibleCols.value
  const hit = cols.find(c => c.idx === ci)
  if (!hit) return
  focusResultCols() // 拖表头连选列前先清掉行选择与框选
  // 结果表的 data-gkey 用的是原始列下标（c.idx），锚点也存 idx
  resColSelDrag = { anchor: hit.name, anchorC: hit.idx, mode: 'cols', moved: false }
  document.addEventListener('mousemove', onResColSelectMove)
  document.addEventListener('mouseup', onResColSelectEnd, { once: true })
  e.preventDefault() // 拖动时不要选中表头文字
}
const onResColSelectMove = (e) => {
  const st = resColSelDrag
  if (!st) return
  const el = document.elementFromPoint(e.clientX, e.clientY)
  if (!el || !el.closest) return
  const gkeyOf = (node) => {
    const g = node && node.getAttribute ? node.getAttribute('data-gkey') : null
    if (!g) return null
    const p = g.split(':')
    return { r: Number(p[0]), c: Number(p[1]) }
  }
  // 从表头往下拖进数据区 → 从「连选列」切成「含标题行(行0)的区域框选」
  if (st.mode === 'cols' && el.closest('td[data-gkey]')) {
    st.mode = 'cells'
    focusResultCells()
  }
  if (st.mode === 'cells') {
    const cell = el.closest('td[data-gkey]') || el.closest('th[data-gkey]')
    const g = gkeyOf(cell)
    if (!g || st.anchorC < 0) return
    st.moved = true
    gridSel.setRange(0, st.anchorC, g.r, g.c) // 锚点固定在标题行，范围自然包含列名
    e.preventDefault()
    return
  }
  // 表头内左右拖：连选多列
  const g = gkeyOf(el.closest('th[data-gkey]'))
  if (!g) return
  const cols = resultVisibleCols.value
  const a = cols.findIndex(c => c.name === st.anchor)
  const b = cols.findIndex(c => c.idx === g.c)
  if (a < 0 || b < 0) return
  st.moved = true
  selectedCols.value = new Set(cols.slice(Math.min(a, b), Math.max(a, b) + 1).map(c => c.name))
  lastColAnchor.value = st.anchor
  e.preventDefault()
}
const onResColSelectEnd = () => {
  const st = resColSelDrag
  resColSelDrag = null
  document.removeEventListener('mousemove', onResColSelectMove)
  if (st && st.moved) resColSelClickUntil = Date.now() + 250
}
onBeforeUnmount(() => {
  resColSelDrag = null
  document.removeEventListener('mousemove', onResColSelectMove)
})
const onResultHeaderClick = (name) => {
  if (!name) return
  if (Date.now() - resultLastResizeAt < 300) return // 拖列宽后的 click 不触发排序
  if (Date.now() - resultLastDragAt < 300) return   // 拖拽换列后的 click 不触发排序
  if (resultSortColumn.value === name) {
    if (resultSortDir.value === 'ASC') {
      resultSortDir.value = 'DESC'
    } else {
      // 第三击 = 取消排序：恢复该结果集的原始行序（与表预览同一套三击循环）
      resultSortColumn.value = ''
      resultSortDir.value = 'ASC'
      // 原地改 rows（同排序：保住对象身份，别把在途计数的回填落在旧对象上）
      result.value.rows = unsortedRows.value.slice()
      resultSelectedSet.value = new Set()
      resultActiveCell.value = null
      resultLastAnchor.value = -1
      return
    }
  } else {
    resultSortColumn.value = name
    resultSortDir.value = 'ASC'
  }
  sortResultBy(name, resultSortDir.value)
}
// 当前选区的 TSV 文本（Excel 可直接粘贴）：
// 列选中 = 列名 + 该列全部已加载行；行选中 = 所选行的全部可见列；否则 = 当前单元格
// 行选中：仅当标题行也在选中范围内时，第一行输出列名
const selectionAsTsv = () => {
  const rows = result.value?.rows || []
  const vis = resultVisibleCols.value
  if (selectedCols.value.size) {
    const picked = vis.filter(c => selectedCols.value.has(c.name))
    if (!picked.length) return ''
    const lines = [picked.map(c => c.name).join('\t')]
    for (const r of rows) lines.push(picked.map(c => r[c.name] == null ? '' : String(r[c.name])).join('\t'))
    return lines.join('\n')
  }
  if (resultSelectedSet.value.size || headerSelected.value) {
    const lines = []
    // 标题行被选中 → 第一行输出列名（与「含表头」复制一致）；没选中就不带
    if (headerSelected.value) lines.push(vis.map(c => c.name).join('\t'))
    const picked = [...resultSelectedSet.value].sort((a, b) => a - b).map(i => rows[i]).filter(Boolean)
    for (const r of picked) lines.push(vis.map(c => r[c.name] == null ? '' : String(r[c.name])).join('\t'))
    return lines.join('\n')
  }
  if (resultActiveCell.value) {
    const { rowIdx, col } = resultActiveCell.value
    const row = rows[rowIdx]
    if (row) return row[col] == null ? '' : String(row[col])
  }
  return ''
}
// 右键「复制」：与 Ctrl+C 完全同一条路 —— 有框选就复制框选区域（按数据全量取值），
// 否则复制当前行/列选中的区域（表头只在选中范围内才带）
const copyLikeCtrlC = () => {
  if (gridSel && gridSel.hasSelection()) {
    const t = gridSel.copy()
    if (t) writeClipboard(t, `已复制选区（${t.split('\n').length} 行）`)
    return
  }
  const text = selectionAsTsv()
  if (!text) { ElMessage.warning(t('sqlq.nothingToCopy')); return }
  writeClipboard(text, `已复制 ${text.split('\n').length} 行`)
}
// Ctrl+C 的场景化提示：让人明确这次到底复制了什么
const copyHint = () => {
  if (selectedCols.value.size) return `已复制 ${selectedCols.value.size} 列（含列名）`
  if (resultSelectedSet.value.size) return `已复制 ${resultSelectedSet.value.size} 行${headerSelected.value ? '（含列名）' : ''}`
  if (headerSelected.value) return t('sqlq.copyColNames')
  return t('sqlq.copyCell')
}
// 焦点是否在输入类控件（含 Monaco 编辑器）里：此时复制的是用户自己的文字，别抢
const inEditableFocus = () => {
  const ae = document.activeElement
  if (!ae || ae === document.body) return false
  if (ae.closest && ae.closest('.monaco-editor')) return true
  const tag = (ae.tagName || '').toLowerCase()
  return tag === 'input' || tag === 'textarea' || ae.isContentEditable === true
}
// Ctrl+C 的主路径：浏览器派发的 copy 事件。
// 不要求焦点在表格里 —— Ctrl+A 全选后焦点常在 body，只挂在 keydown 上会「按了没反应」
let copyEventAt = 0
const onDocCopy = (e) => {
  if (inEditableFocus()) { e.stopImmediatePropagation(); return } // 编辑器/输入框里的复制不抢
  if (gridSel && gridSel.hasSelection()) return // 框选交给框选模块（按数据全量取值）
  if (!resultVisible.value) return
  const text = selectionAsTsv()
  if (!text) return
  copyEventAt = Date.now()
  e.clipboardData.setData('text/plain', text)
  ElMessage.success(copyHint())
  e.preventDefault()
}
document.addEventListener('copy', onDocCopy)
onBeforeUnmount(() => document.removeEventListener('copy', onDocCopy))
const copyResCtx = (mode) => {
  const { rows, cols } = resCtxSelection()
  if (!rows.length || !cols.length) { ElMessage.warning(t('sqlq.nothingToCopy')); return }
  const table = guessTableName()
  const val = (r, c) => (r && r[c] != null) ? String(r[c]) : ''
  if (mode === 'csv') {
    const esc = (s) => /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s
    const lines = [cols.map(esc).join(',')]
    for (const r of rows) lines.push(cols.map(c => esc(val(r, c))).join(','))
    writeClipboard(lines.join('\n'), t('sqlq.copyCsv', { n: rows.length }))
    return
  }
  if (mode === 'json') {
    const obj = rows.map(r => {
      const o = {}
      for (const c of cols) o[c] = r[c] === undefined ? null : r[c]
      return o
    })
    writeClipboard(JSON.stringify(obj.length === 1 ? obj[0] : obj, null, 2), t('sqlq.copyJson', { n: rows.length }))
    return
  }
  if (mode === 'insert') {
    const text = rows.map(r => `INSERT INTO ${table} (${cols.join(', ')}) VALUES (${cols.map(c => resSqlVal(r[c])).join(', ')});`).join('\n')
    writeClipboard(text, t('sqlq.copyInsert', { n: rows.length, table: table }))
    return
  }
  if (mode === 'update') {
    // 结果集没有主键信息，用选中列做 WHERE（请自行核对）
    const text = rows.map(r => {
      const setClause = cols.map(c => `${c} = ${resSqlVal(r[c])}`).join(', ')
      const whereClause = cols.map(c => `${c} = ${resSqlVal(r[c])}`).join(' AND ')
      return `UPDATE ${table} SET ${setClause} WHERE ${whereClause};`
    }).join('\n')
    writeClipboard(text, t('sqlq.copyUpdate', { n: rows.length, table: table }))
  }
}
const copyResMarkdown = () => {
  const { rows, cols } = resCtxSelection()
  if (!rows.length || !cols.length) return
  const esc = (s) => String(s).replace(/\|/g, '\\|').replace(/\r?\n/g, ' ')
  const head = `| ${cols.map(esc).join(' | ')} |`
  const sep = `| ${cols.map(() => '---').join(' | ')} |`
  const body = rows.map(r => `| ${cols.map(c => esc(r[c] == null ? '' : r[c])).join(' | ')} |`)
  writeClipboard([head, sep, ...body].join('\n'), t('sqlq.copyMarkdown', { n: rows.length }))
}
// 结果集没有主键信息，用选中列做 WHERE，复制后请自行核对
const copyResDelete = () => {
  const { rows, cols } = resCtxSelection()
  if (!rows.length || !cols.length) return
  const table = guessTableName()
  const sql = rows.map(r => `DELETE FROM ${table} WHERE ${cols.map(c => `${c} = ${resSqlVal(r[c])}`).join(' AND ')};`).join('\n')
  writeClipboard(sql, t('sqlq.copyDelete', { n: rows.length, table: table }))
}
// 复制表头（列名）：有选中的列就复制选中的，否则复制右键那一列。
// 多列用制表符分隔 —— 与 Excel 一致，粘到 Excel 里是横向多个单元格（不是一整串文本）
const copyColHeader = () => {
  const picked = resultVisibleCols.value.filter(c => selectedCols.value.has(c.name)).map(c => c.name)
  const names = picked.length ? picked : (resCtx.value.col ? [resCtx.value.col] : [])
  if (!names.length) return
  writeClipboard(names.join('\t'), names.length > 1 ? t('sqlq.copyColNamesN', { n: names.length }) : t('sqlq.copyColNameOne', { name: names[0] }))
}

const hideResultColumn = (name) => {
  if (!name) return
  if (resultVisibleCols.value.length <= 1) { ElMessage.warning(t('sqlq.keepOneCol')); return }
  hiddenResultCols.value = new Set(hiddenResultCols.value).add(name)
}
// 「隐藏列」：有选中的列就隐藏全部选中列（单选/多选都支持）；没选任何列时隐藏右键那一列
const hideResultColumnSmart = (name) => {
  const picked = resultVisibleCols.value.filter(c => selectedCols.value.has(c.name)).map(c => c.name)
  if (!picked.length) { hideResultColumn(name); return }
  if (resultVisibleCols.value.length - picked.length < 1) { ElMessage.warning(t('sqlq.keepOneCol')); return }
  hiddenResultCols.value = new Set([...hiddenResultCols.value, ...picked])
  clearColSelect()
}
const openResCtx = (x, y, rowIdx, col, from = 'cell') => {
  resCtx.value = { visible: false, x, y, items: [], rowIdx, col }
  resCtxFrom = from
  // 表头右键：该列不在选中列里时，切换为只选这一列（Excel 行为）
  // 当前是否存在「真正的选区」：框选区域（单格不算）/ 选中行 / 选中列
  const fr = gridSel?.range?.value || null
  const frameOn = !!fr && (fr.r1 !== fr.r2 || fr.c1 !== fr.c2)
  const hasSelection = frameOn || resultSelectedSet.value.size > 0 || selectedCols.value.size > 0 || headerSelected.value
  // 表头 / 行头右键**只在完全没有选中时**才把选中切过去。已有选区就保持不动，
  // 这样「选中若干行后在数据区右键」与「在行头右键」拿到的菜单完全一致（菜单由选中决定）
  if (from === 'header' && col && !hasSelection) {
    focusResultCols() // 列选择是唯一选区：清掉行选择与框选
    selectSingleCol(col)
  }
  if (from === 'row' && rowIdx >= 0 && !hasSelection) {
    focusResultRows() // 行选择是唯一选区：清掉列选择与框选
    resultSelectedSet.value = new Set([rowIdx])
    resultLastAnchor.value = rowIdx
  }
  const sel = resCtxSelection()
  if (!col && !sel.rows.length) return
  const items = []
  // 只在确有内容时插分隔线，避免菜单顶部出现孤立分隔线
  const sep = () => { if (items.length && !items[items.length - 1].sep) items.push({ sep: true }) }
  // 排序走表头点击，不再占用右键菜单
  // 复制为 ▸：各上下文共用的格式项
  const formatSub = [
    { label: 'CSV', command: 'copy-csv' },
    { label: 'JSON', command: 'copy-json' },
    { label: 'MARKDOWN', command: 'copy-markdown' },
    { label: 'INSERT', command: 'copy-insert' },
    { label: 'UPDATE', command: 'copy-update' },
    { label: 'DELETE', command: 'copy-delete' }
  ]
  const selColCount = selectedCols.value.size
  const selRowCount = resultSelectedSet.value.size
  // 菜单内容由**当前选中**决定，与右键落在哪无关（选中行时，数据区与行头给的菜单必须一模一样）：
  //   框选区域 → 区域功能；选中行 → 行功能；选中列 → 列功能；
  //   完全没有选中时才看「点在哪里」：表头→列、行头→行、数据区→单元格。
  //   只选中一个单元格不算"有选区"，那时给的就是单元格功能。
  const mode = frameOn ? 'range'
    : (selRowCount || headerSelected.value) ? 'rows'
      : selColCount ? 'columns'
        : from === 'header' ? 'columns'
          : from === 'row' ? 'rows'
            : from === 'grid' ? 'range'
              : 'cell'
  // 表格空白处右键但没有任何框选 → 没东西可操作
  if (from === 'grid' && !frameOn) return

  if (mode === 'range') {
    // —— 区域操作 ——
    // 「复制」= 与 Ctrl+C 完全一致（照选中范围原样复制，表头只在选中范围内才带）
    items.push({ label: t('mdk.copy'), command: 'copy-sel', shortcut: 'Ctrl+C' })
    items.push({ label: t('sqlq.ctxCopyAs'), sub: formatSub })
  } else if (mode === 'columns') {
    // —— 列操作 ——
    items.push({ label: t('mdk.copy'), command: 'copy-sel', shortcut: 'Ctrl+C' })
    items.push({ label: t('sqlq.ctxCopyAs'), sub: formatSub })
    // 只要列名（不含数据）：贴进 SELECT / WHERE 用；选了几列就复制几列的名字
    items.push({ label: t('sqlq.ctxCopyHeader'), command: 'copy-col-header' })
    sep()
    items.push({ label: t('sqlq.ctxColFit'), command: 'col-fit' })
    items.push({ label: t('sqlq.ctxColFitAll'), command: 'col-fit-all' })
    items.push({ label: t('sqlq.ctxResetColW'), command: 'reset-colw' })
    items.push({ label: t('sqlq.ctxHideCol'), command: 'hide-col' })
    if (hiddenResultCols.value.size) items.push({ label: t('sqlq.ctxShowAllCols'), command: 'show-all-cols' })
  } else if (mode === 'rows') {
    // —— 行操作（结果表只读，没有编辑类操作） ——
    items.push({ label: t('sqlq.ctxRowDetail'), command: 'row-detail' })
    items.push({ label: t('mdk.copy'), command: 'copy-sel', shortcut: 'Ctrl+C' })
    items.push({ label: t('sqlq.ctxCopyAs'), sub: formatSub })
  } else {
    // —— 单元格操作（只放作用在这一格上的操作；行/列操作先选中行/列再右键） ——
    if (col) items.push({ label: t('mdk.copy'), command: 'copy-cell', shortcut: 'Ctrl+C' })
    if (col && rowIdx >= 0 && row) items.push({ label: t('sqlq.ctxCellEdit'), command: 'cell-edit' })
  }
  // 全选/取消全选走 Ctrl+A 与表头复选框，不再占用右键菜单
  if (!items.length) return
  // 只有一个子项的二级菜单没必要，直接提升为一级
  const flat = items.map(it => (it.sub && it.sub.length <= 1 ? it.sub[0] : it))
  resCtx.value = { visible: true, x, y, items: flat, rowIdx, col }
  resCtxSub.value = null
  nextTick(() => {
    const el = document.querySelector('.grid-ctx-menu')
    if (el) {
      const r = el.getBoundingClientRect()
      if (r.right > window.innerWidth) resCtx.value.x = window.innerWidth - r.width - 8
      if (r.bottom > window.innerHeight) resCtx.value.y = window.innerHeight - r.height - 8
    }
  })
}
const onResultCtxItem = (item) => {
  const { rowIdx, col } = resCtx.value
  const rows = result.value?.rows || []
  const row = rowIdx >= 0 ? rows[rowIdx] : null
  const val = (r, c) => (r && r[c] != null) ? String(r[c]) : ''
  switch (item.command) {
    case 'row-detail': if (rowIdx >= 0) openRowDetail(rowIdx); break
    case 'copy-cell': if (row) writeClipboard(val(row, col), t('sqlq.copyCell')); break
    case 'cell-edit': if (row) onCellQuickEdit(rowIdx, col, row); break
    case 'copy-sel': copyLikeCtrlC(); break
    case 'copy-col-header': copyColHeader(); break
    case 'copy-csv': copyResCtx('csv'); break
    case 'copy-json': copyResCtx('json'); break
    case 'copy-insert': copyResCtx('insert'); break
    case 'copy-update': copyResCtx('update'); break
    case 'copy-markdown': copyResMarkdown(); break
    case 'show-all-cols': showAllResultCols(); break
    case 'copy-delete': copyResDelete(); break
    case 'col-fit': {
      if (selectedCols.value.size > 1) { autoFitSelectedResultCols(); break }
      const ci = (result.value?.columns || []).indexOf(col)
      if (ci >= 0) autoFitResultCol(ci)
      break
    }
    case 'col-fit-all': {
      const cols = result.value?.columns || []
      const vis = new Set(resultVisibleCols.value.map(c => c.name))
      cols.forEach((name, i) => { if (vis.has(name)) autoFitResultCol(i) })
      break
    }
    case 'reset-colw':
      resultColWidths.value = {}
      resultManualCols.value = new Set()
      nextTick(() => measureResultColumns())
      break
    case 'hide-col': hideResultColumnSmart(col); break
  }
  closeResCtx()
}
const onDocResCtxClose = (e) => {
  const inMenu = e.target.closest && e.target.closest('.grid-ctx-menu')
  if (resCtx.value.visible && !inMenu) closeResCtx()
  if (edCtx.value.visible && !inMenu) closeEdCtx()
}
// Esc 关闭编辑器右键菜单（捕获阶段，避免先被 Monaco 内部处理掉）
const onEdCtxKeyDown = (e) => {
  if (e.key === 'Escape' && edCtx.value.visible) closeEdCtx()
}

// 连接 / 数据库 / Schema 选择
const allConnections = ref([])
const selectedConnId = ref('')
const databases = ref([])
const selectedDatabase = ref('')
// catalog 方言（Doris）：catalog 与库拆成两个下拉。databases 存储仍是全限定名
// `catalog.库`（历史/预设/useDatabase 全都不用动），下面三个只是下拉的显示状态。
// dbCatalogs 为空 = 普通方言，库下拉直接用 databases（行为与从前完全一致）。
const dbCatalogs = ref([])
const selectedCatalog = ref('')
const selectedDbShort = ref('')
const lastDbByCatalog = ref({}) // catalog → 上次选过的库：切回来回到原位，而不是字母序第一
const dbOptions = computed(() => {
  if (!dbCatalogs.value.length) return databases.value
  const p = selectedCatalog.value + '.'
  return databases.value.filter(d => d.startsWith(p)).map(d => d.slice(p.length))
})
/** 把全限定名 selectedDatabase 同步到（可选的）双下拉显示状态 */
const syncCatalogUi = () => {
  if (!dbCatalogs.value.length) { selectedDbShort.value = selectedDatabase.value; return }
  const q = String(selectedDatabase.value || '')
  const i = q.indexOf('.')
  const cat = i > 0 ? q.slice(0, i) : ''
  if (cat && dbCatalogs.value.includes(cat)) {
    selectedCatalog.value = cat
  } else if (!selectedCatalog.value || !dbCatalogs.value.includes(selectedCatalog.value)) {
    selectedCatalog.value = dbCatalogs.value[0] || ''
  }
  const p = selectedCatalog.value + '.'
  selectedDbShort.value = q.startsWith(p) ? q.slice(p.length) : (dbOptions.value[0] || '')
  if (q.startsWith(p) && q.slice(p.length)) lastDbByCatalog.value[selectedCatalog.value] = q
}
/** 库下拉（裸名）变更 → 还原成全限定名走原有 onDbChange */
const onDbShortChange = (short) => {
  onDbChange(dbCatalogs.value.length ? selectedCatalog.value + '.' + (short || '') : (short || ''))
}
/** catalog 下拉变更：优先回到该 catalog 上次用过的库，没有才选第一个 */
const onCatalogChange = () => {
  const p = selectedCatalog.value + '.'
  if (!String(selectedDatabase.value || '').startsWith(p)) {
    const remembered = lastDbByCatalog.value[selectedCatalog.value] || ''
    if (remembered && databases.value.includes(remembered)) {
      onDbShortChange(remembered.slice(p.length))
    } else {
      onDbShortChange(dbOptions.value[0] || '')
    }
  }
}
const loadingDbs = ref(false)
const schemas = ref([])
const selectedSchema = ref('')
const loadingSchemas = ref(false)
const tableNames = ref([]) // 当前库的表名缓存（用于编辑器补全）

const selectedConn = computed(() => allConnections.value.find(c => String(c.id) === String(selectedConnId.value)))
const connType = computed(() => selectedConn.value?.type || '')
const isNoSql = computed(() => isNoSqlType(connType.value))
// SQL 格式化方言：跟随当前连接类型自动识别（未知类型回退标准 SQL）
const fmtDialect = computed(() => connDialectOf(connType.value))
const showSchemaSelect = computed(() => schemaLevelOf(connType.value) === 'schema')
// 连接下拉：按目录（environment）分组（一级：目录名；二级：连接名）。
// **未分组的连接归入「开发」**—— 用户要求：下拉里不该出现「未分组」这种分组，
// 每个连接都应该落在 开发/测试/生产 之一（默认开发，后续可拖拽改）。
const connGroups = computed(() => {
const map = new Map()
for (const c of allConnections.value) {
const key = c.environment || 'DEV'
if (!map.has(key)) map.set(key, [])
map.get(key).push(c)
}
// 组顺序按预置环境：开发 → 测试 → 生产 → 自定义目录
const order = ['DEV', 'TEST', 'PROD']
return [...map.entries()]
.sort((a, b) => {
const ia = order.indexOf(a[0]), ib = order.indexOf(b[0])
return (ia === -1 ? 99 : ia) - (ib === -1 ? 99 : ib)
})
.map(([folder, items]) => ({ folder, items }))
})

const mounted = ref(false)
const editorRef = ref(null)
// Monaco 约 2.6MB，按需加载：就绪前渲染占位，首屏不再打包它
const monacoReady = ref(false)

// 导出走异步任务，显示进度条和实时日志
const exportTask = useExportTask()
ensureMonaco().then(() => { monacoReady.value = true })
  .catch((e) => ElMessage.error('编辑器加载失败：' + (e && e.message ? e.message : e)))
let editorInstance = null

// ===== 语句面包屑 + 报错语句标红（多段脚本的定位能力）=====
// 每条语句的原文区间（splitSqlStatementRanges 给字符偏移，转 Monaco 位置用）
const stmtBreadcrumbs = computed(() => {
  const text = sql.value || ''
  const ranges = splitSqlStatementRanges(text)
  if (ranges.length <= 1) return []
  return ranges.map((r, i) => ({
    i: i + 1, start: r.start, end: r.end,
    firstLine: (r.text.split('\n')[0] || '').slice(0, 120)
  }))
})
const gotoStatement = (s) => {
  const ed = editorInstance
  const model = ed && ed.getModel()
  if (!ed || !model) return
  const start = model.getPositionAt(s.start)
  const end = model.getPositionAt(s.end)
  const range = {
    startLineNumber: start.lineNumber, startColumn: start.column,
    endLineNumber: end.lineNumber, endColumn: end.column
  }
  ed.setSelection(range)
  ed.revealRangeInCenter(range)
  ed.focus()
}
/** 面包屑 ▶：选中该语句并只执行它（选中即 getExecutableSql 的取材口径） */
const runStatement = (s) => {
  gotoStatement(s)
  runSql()
}
// 报错语句标红：decorations 用裸对象 range（Monaco 内部 DOM 不带组件 scoped 属性，样式放全局块）
let stmtErrorDecorations = []
const clearStmtError = () => {
  const ed = editorInstance
  if (ed && stmtErrorDecorations.length) {
    try { ed.deltaDecorations(stmtErrorDecorations, []) } catch { /* 编辑器可能已销毁 */ }
  }
  stmtErrorDecorations = []
}
/** 执行失败后定位出错语句：不整条爆红 —— 只在语句首行标红点（悬停看完整报错），
 *  且尽量把「报错里提到的标识符」（如 Unknown column 'j' 的 j）单独划红线。
 *  preferSql：本轮实际执行的 SQL（执行选中场景 = 选中的那条语句），用于匹配目标 */
const markErrorStatement = (res, stmtIndex, preferSql) => {
  clearStmtError()
  const ed = editorInstance
  const model = ed && ed.getModel()
  if (!ed || !model) return
  const ranges = splitSqlStatementRanges(sql.value || '')
  if (!ranges.length) return
  let target = -1
  if (Number.isInteger(stmtIndex) && stmtIndex >= 0 && stmtIndex < ranges.length) target = stmtIndex
  if (target < 0) {
    const failedSql = String(res?.failedSql || '').trim()
    if (failedSql) {
      const norm = (s) => String(s).replace(/\s+/g, ' ').toLowerCase()
      target = ranges.findIndex((r) => norm(r.text) === norm(failedSql))
    }
  }
  if (target < 0 && preferSql) {
    // 执行选中场景：执行的 SQL 就是那条语句 —— 规范化后精确匹配；
    // 选了半条语句时按「包含」兜底
    const np = preferSql.replace(/\s+/g, ' ').toLowerCase()
    if (np) {
      target = ranges.findIndex((r) => r.text.replace(/\s+/g, ' ').toLowerCase() === np)
      if (target < 0) target = ranges.findIndex((r) => np.includes(r.text.replace(/\s+/g, ' ').toLowerCase()))
    }
  }
  if (target < 0 && ranges.length === 1) target = 0
  if (target < 0) return
  const r = ranges[target]
  const startPos = model.getPositionAt(r.start)
  const endPos = model.getPositionAt(r.end)
  const fullRange = {
    startLineNumber: startPos.lineNumber, startColumn: startPos.column,
    endLineNumber: endPos.lineNumber, endColumn: endPos.column
  }
  const msg = String(res?.message || '')
  const decos = []
  // ① 语句首行 glyph 红点：入口指示，悬停显示完整报错（不遮正文）
  decos.push({
    range: { startLineNumber: startPos.lineNumber, startColumn: 1, endLineNumber: startPos.lineNumber, endColumn: 1 },
    options: {
      isWholeLine: true,
      glyphMarginClassName: 'stmt-err-glyph',
      glyphMarginHoverMessage: { value: '**SQL 执行失败**\n\n' + msg.replace(/\n/g, '\n\n') },
      stickiness: 1
    }
  })
  // ② 精确到出错标识符：报错信息里第一个引号包住的词（如 'j'）在语句内按整词匹配
  const tm = /'([^'\n]{1,64})'/.exec(msg)
  if (tm && /[a-z_]/i.test(tm[1])) {
    const word = tm[1].replace(/^[`"'\[\]]+|[`"'\[\]]+$/g, '')
    if (word.length >= 1) {
      const stmtRaw = (sql.value || '').slice(r.start, r.end)
      const re = new RegExp('(?<![\\w$`"\'])' + word.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '(?![\\w$`"\'])', 'i')
      const m = re.exec(stmtRaw)
      if (m) {
        const tp = model.getPositionAt(r.start + m.index)
        const tp2 = model.getPositionAt(r.start + m.index + m[0].length)
        decos.push({
          range: { startLineNumber: tp.lineNumber, startColumn: tp.column, endLineNumber: tp2.lineNumber, endColumn: tp2.column },
          options: {
            inlineClassName: 'stmt-err-token',
            hoverMessage: { value: '**SQL 报错**\n\n' + msg.replace(/\n/g, '\n\n') },
            stickiness: 1
          }
        })
      }
    }
  }
  try {
    stmtErrorDecorations = ed.deltaDecorations([], decos)
  } catch { /* 编辑器销毁竞态忽略 */ }
  ed.revealRangeInCenter(fullRange)
}

const aiDialogVisible = ref(false)
const aiDialogTitle = ref('')
const aiLoading = ref(false)
const aiFixLoading = ref(false)
const aiResult = ref('')
const aiResultIsSql = ref(false)
const aiErrorReason = ref('')
const aiUsage = ref(null)

const aiModels = ref([])
const selectedAiModelId = ref('')

/** 对话框内容：一律按 Markdown 渲染。
 *  改写 / 修复返回的是裸 SQL，包成 ```sql 围栏后才能复用「每段 SQL 各自插入 / 复制」的按钮 */
const aiResultMd = computed(() => {
  const t = aiResult.value
  if (!t) return ''
  if (!aiResultIsSql.value || t.includes('```')) return t
  return '```sql\n' + t.trim() + '\n```'
})
/** AI 助手菜单分发：改写需要先问要求，其余直接发起 */
const onAiAction = (cmd) => {
  if (cmd === 'rewrite') askAiRewrite()
  else if (cmd) askAi(cmd)
}

/** SQL 快捷验证弹窗：plan = 执行计划，run = 试跑取前 100 行（都不消耗 AI 调用） */
const probeVisible = ref(false)
const probeMode = ref('run')
const probeSql = ref('')

// ===== SQL 执行历史（本地 localStorage 按连接隔离，最近执行一键回填）=====
const historyList = ref([])
const HISTORY_KEY = (cid) => 'xplore:sql-history:' + cid
const loadHistory = () => {
  const cid = selectedConnId.value || props.conn?.id
  if (!cid) { historyList.value = []; return }
  try { historyList.value = JSON.parse(localStorage.getItem(HISTORY_KEY(cid)) || '[]') } catch { historyList.value = [] }
}
const persistHistory = () => {
  const cid = selectedConnId.value || props.conn?.id
  if (!cid) return
  try { localStorage.setItem(HISTORY_KEY(cid), JSON.stringify(historyList.value)) } catch { /* 忽略配额 */ }
}
/** 执行成功后调用：把本次 SQL 记录到本地历史（连接内隔离，最多保留 50 条，同 SQL+同库去重） */
const recordHistory = (sqlText, db, cost) => {
  const text = (sqlText || '').trim()
  if (!text) return
  const cid = selectedConnId.value || props.conn?.id
  if (!cid) return
  const list = historyList.value.filter(h => !(h.sql === text && (h.db || '') === (db || '')))
  list.unshift({ id: Date.now() + '_' + Math.random().toString(36).slice(2, 6), sql: text, db: db || '', ts: Date.now(), cost: cost || 0 })
  historyList.value = list.slice(0, 50)
  persistHistory()
}
const applyHistory = (h) => {
  if (!h) return
  sql.value = h.sql
  // 若历史记录的目标库当前下拉里有，则顺带切过去，否则仅回填 SQL
  if (h.db && h.db !== selectedDatabase.value && databases.value.includes(h.db)) selectedDatabase.value = h.db
}
const removeHistory = (i) => { const list = historyList.value.slice(); list.splice(i, 1); historyList.value = list; persistHistory() }
const clearHistory = () => { historyList.value = []; persistHistory() }
const formatHistTime = (ts) => {
  const d = new Date(ts); const p = (n) => String(n).padStart(2, '0')
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}
watch([selectedConnId, () => props.conn?.id], loadHistory, { immediate: true })

// ===== 查询结果数据透视（复用当前结果集，纯前端分组汇总 / 计数 / 下钻）=====
const pivotVisible = ref(false)
const pivotColumns = ref([])
const pivotRows = ref([])
const openPivot = () => {
  const r = result.value
  if (!r || !r.rows || !r.rows.length) { ElMessage.warning(t('sqlq.noRowsToExport')); return }
  pivotColumns.value = (r.columns || []).slice()
  pivotRows.value = r.rows.map(row => ({ ...row })) // 快照，避免后续翻页/排序改动影响透视
  pivotVisible.value = true
}
/** 弹窗里点「插入编辑器」：只写入不自动执行，跑不跑由用户决定 */
const onProbeInsert = (text) => {
  probeVisible.value = false
  onInsert({ detail: text })
}

/** markdown 内 SQL 代码块的按钮：v-html 无法绑定 Vue 事件，这里做点击委托 */
const onAiMdAction = (e) => {
  const btn = e.target instanceof Element ? e.target.closest('[data-sql-act]') : null
  if (!btn) return
  e.preventDefault(); e.stopPropagation()
  const block = extractCodeBlocks(aiResultMd.value)[Number(btn.dataset.sqlIdx)]
  const sqlText = block ? block.code : ''
  if (!sqlText) return
  const act = btn.dataset.sqlAct
  if (act === 'run' || act === 'plan') {
    // 先验证再用：只读护栏在后端，这里只负责把结果摊给用户看
    probeMode.value = act === 'plan' ? 'plan' : 'run'
    probeSql.value = sqlText
    probeVisible.value = true
    return
  }
  if (act === 'copy') copySql(sqlText)
  else replaceAndRun(sqlText) // 插入并执行：写入编辑器后直接跑
}

const loadAiModels = async () => {
  try {
    const cfg = await getAiConfig()
    if (cfg && Array.isArray(cfg.models)) {
      aiModels.value = cfg.models
      // 默认使用 Auto 自动选择模式（后端自动挑选最优可用模型，失败自动切换）
      if (!selectedAiModelId.value) selectedAiModelId.value = 'auto'
    }
  } catch (e) { /* ignore */ }
}

/** 模型下拉按钮上显示的文案（Auto 或具体模型名） */
const currentAiModelLabel = computed(() => {
  if (!selectedAiModelId.value || selectedAiModelId.value === 'auto') return 'Auto'
  const m = aiModels.value.find(x => x.id === selectedAiModelId.value)
  return (m && (m.name || m.model)) || 'Auto'
})
/** 选择模型：command 即模型 id，auto 表示由后端自动挑选 */
const onPickAiModel = (cmd) => { if (cmd) selectedAiModelId.value = cmd }

// 脚本保存/加载（按 连接+数据库 隔离，保存在左侧树对应库的 Scripts 目录下）
const saveDialogVisible = ref(false)
const saveName = ref('')
const savedScripts = ref([])

// 当前脚本存储 key：dc_scripts:<connId>:<db>
const scriptKey = () => {
  // 库标识需与对象树一致：有 schema 层级时用 库.schema（SQL Server 树端按此隔离脚本）
  const db = selectedSchema.value
    ? `${selectedDatabase.value}.${selectedSchema.value}`
    : (selectedDatabase.value || props.database || '')
  return `dc_scripts:${selectedConnId.value || props.conn?.id || ''}:${db}`
}

// 保存/删除脚本后通知 MainView 刷新树中对应库的 Scripts 目录
const notifyScriptsChanged = () => {
  window.dispatchEvent(new CustomEvent('dc-scripts-changed', {
    detail: { connId: selectedConnId.value || props.conn?.id, db: selectedDatabase.value || props.database || '' }
  }))
}

const editorOptions = computed(() => ({
  automaticLayout: true,
  // 左侧 glyphMargin：报错语句的红点指示画在这里
  glyphMargin: true,
  // 空编辑器的引导提示（contrib/placeholderText）：一眼知道这里写什么、怎么执行
  placeholder: t('sqlq.editorPlaceholder'),
  // 补全列表**收归 provider 独家供给**：词建议（文档里出现过的词）混进来会让列表
  // 忽多忽少、看着「时而提示时而不提示」；provider 本身稳定返回关键字/函数/表名
  wordBasedSuggestions: 'off',
  // 显式固定触发行为：输入即弹、点号/触发符也弹，不随内置默认值漂移
  suggestOnTriggerCharacters: true,
  suggestSelection: 'first',
  // 括号自动补齐（设置可关）：SQL 语言定义里没有 autoClosingPairs（默认 languageDefined
  // 不生效），所以开着时强制 always —— 输入 ( [ ' 自动带上另一半，覆盖输入不会重复
  autoClosingBrackets: editorSettings.value.autoCloseBrackets ? 'always' : 'never',
  fontSize: editorSettings.value.fontSize,
  mouseWheelZoom: true,

  // 小地图已下线（用户反馈没啥用）：**必须显式关** —— Monaco 的 minimap 默认就是
  // enabled=true，当初误以为默认关、把显式配置删了，小地图反而全回来了（真机踩过）。
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  wordWrap: editorSettings.value.wordWrap ? 'on' : 'off',
  tabSize: editorSettings.value.tabSize,
  lineNumbers: editorSettings.value.lineNumbers ? 'on' : 'off',
  lineNumbersMinChars: 2,
  lineDecorationsWidth: 0,
  // 当前编辑行不做任何高亮：失焦时 'line'/'all' 会把当前行画成一个边框（用户不要这个框）
  renderLineHighlight: 'none',
  // snippetsPreventQuickSuggestions 默认 true：片段类建议（我们的函数补全是片段）
  // 会被排除在**自动弹出**之外 —— 表现就是「输入函数名没有任何提示」。关掉它。
  // 智能补全（设置可关）：关掉时连建议 widget 一起收（showSuggestions: false）
  suggest: editorSettings.value.quickSuggest
    ? { preview: true, showSnippets: false, snippetsPreventQuickSuggestions: false }
    : { showSuggestions: false },
  quickSuggestions: editorSettings.value.quickSuggest ? { other: true, comments: false, strings: false } : false,
  acceptSuggestionOnEnter: 'on',
  snippetSuggestions: 'bottom',
  fixedOverflowWidgets: true,
  folding: true,
  foldingHighlight: true,
  bracketPairColorization: { enabled: true },
  guides: { bracketPairs: true, indentation: true },
  matchBrackets: 'always',
  autoClosingQuotes: 'always',
  formatOnPaste: true,
  formatOnType: true,
  smoothScrolling: true,
  cursorBlinking: 'smooth',
  cursorSmoothCaretAnimation: 'on',
  renderWhitespace: 'boundary',
  // 让 Tab 键触发补全而不是插入制表符
  tabCompletion: 'on',
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
}))

// SQL 关键字列表（保留常用子集，避免补全列表过于庞杂）
const SQL_KEYWORDS = [
  'SELECT', 'FROM', 'WHERE', 'JOIN', 'LEFT', 'RIGHT', 'INNER', 'ON', 'GROUP', 'BY',
  'ORDER', 'HAVING', 'LIMIT', 'OFFSET', 'INSERT', 'INTO', 'VALUES', 'UPDATE', 'SET',
  'DELETE', 'CREATE', 'TABLE', 'ALTER', 'DROP', 'AS', 'AND', 'OR', 'NOT', 'IN', 'EXISTS',
  'BETWEEN', 'LIKE', 'IS', 'NULL', 'DISTINCT', 'UNION', 'ALL', 'CASE', 'WHEN', 'THEN',
  'ELSE', 'END', 'COUNT', 'SUM', 'AVG', 'MIN', 'MAX', 'ASC', 'DESC'
]

// 注册到全局 monaco.languages 上的 provider：返回值必须 dispose。
// 否则反复开关 SQL 页签会不断叠加（补全里出现重复项、每次输入被重复回调），长会话越来越卡
const monacoProviders = []

// ===== 列名懒加载缓存：补全「表.列」与「引用到的表的列」用 =====
// 键 = 小写表名。不在建库时预取全部列（几百张表就是几百次请求），谁被点号/被
// FROM 引用到了才取谁，取一次终身缓存
const columnsCache = {}
const columnsLoading = new Set()

// ===== 库名. 前缀补全：该库的表/视图/存储过程/函数/触发器 =====
// 键 = `${connId}:${db}`。首触发现场拉（表/视图 + 例程 + 触发器三路并发，不阻塞本轮），
// 结果留在会话缓存里，下一个字符继续输入时就能看到；表/视图清单顺带喂给对象树缓存。
const dbObjectsCache = {}
const loadDbObjects = (cid, db) => {
  const key = `${cid}:${db}`
  if (dbObjectsCache[key]) return dbObjectsCache[key]
  const entry = { tables: [], views: [], procs: [], fns: [], triggers: [] }
  dbObjectsCache[key] = entry
  const nameOf = (x) => (x && typeof x === 'object' ? (x.name || x.routineName || x.triggerName || x.table || '') : String(x || ''))
  listTables(cid, db).then((list) => {
    const arr = Array.isArray(list) ? list : []
    entry.tables = arr.filter((t) => (t.type || 'TABLE') === 'TABLE').map(nameOf).filter(Boolean)
    entry.views = arr.filter((t) => (t.type || '') === 'VIEW').map(nameOf).filter(Boolean)
    writeSchemaCache('tables:' + cid + ':' + db, arr) // 与对象树同一份缓存，展开树时直接复用
  }).catch(() => {})
  listProcedures(cid, db).then((list) => {
    const arr = Array.isArray(list) ? list : []
    entry.procs = arr.filter((r) => (r.routineType || 'PROCEDURE') === 'PROCEDURE').map(nameOf).filter(Boolean)
    entry.fns = arr.filter((r) => (r.routineType || '') === 'FUNCTION').map(nameOf).filter(Boolean)
  }).catch(() => {})
  listTriggers(cid, db).then((list) => {
    entry.triggers = (Array.isArray(list) ? list : []).map(nameOf).filter(Boolean)
  }).catch(() => {})
  return entry
}
const loadColumns = (table) => {
  const key = String(table || '').toLowerCase()
  if (!key || columnsCache[key] || columnsLoading.has(key)) return
  columnsLoading.add(key)
  listColumns(selectedConnId.value || props.conn?.id, selectedDatabase.value || props.database, table)
    .then((cols) => {
      columnsCache[key] = (Array.isArray(cols) ? cols : [])
        .map((c) => (typeof c === 'string' ? c : c.name || c.columnName))
        .filter(Boolean)
    })
    .catch(() => {})
    .finally(() => columnsLoading.delete(key))
}

// ON 子句关联列提示：把与「JOIN 另一侧表」**同名的列**排到最前（大概率是外键），
// detail 会标「同名列」。另一侧尚未缓存列时现场拉，下一轮补全即可置顶。
// 同时返回另一侧的前缀（otherPrefix），供「一键补全整段 a.x = b.x」用。
const decorateOnColumns = (cols, lineText, typedPrefix, aliasMap) => {
  const plain = (c) => ({ name: c, same: false })
  if (!/\bon\b[^\n]*$/i.test(lineText)) return { list: cols.map(plain), otherPrefix: null }
  const m = /\bon\s+([\w$]+)\s*\./i.exec(lineText)
  if (!m) return { list: cols.map(plain), otherPrefix: null }
  const typed = String(typedPrefix || '').toLowerCase()
  const p = m[1].toLowerCase()
  if (p === typed) return { list: cols.map(plain), otherPrefix: null } // 另一侧还没写到
  const other = aliasMap[p]
    || (tableNames.value.find((x) => String(x).toLowerCase() === p) || p)
  if (!other) return { list: cols.map(plain), otherPrefix: null, onStartCol: 0 }
  loadColumns(other)
  const oc = columnsCache[String(other).toLowerCase()]
  let list = cols.map(plain)
  if (oc && oc.length) {
    const os = new Set(oc.map((c) => String(c).toLowerCase()))
    list = cols
      .map((c) => ({ name: c, same: os.has(String(c).toLowerCase()) }))
      .sort((x, y) => (x.same === y.same ? 0 : x.same ? -1 : 1))
  }
  // ON 关键字之后在本行内的列号（1-based）：「一键整段」用它把已输入的条件整个替换
  return { list, otherPrefix: m[1], onStartCol: m.index + m[0].length + 1 }
}

const onEditorMount = (editor, monaco) => {
  editorInstance = editor
  // 注册自定义补全提供者
  if (monaco) {
    monacoProviders.push(monaco.languages.registerCompletionItemProvider('sql', {
      // 仅在点号处强制触发（库.schema.表 级联）；避免输入空格/换行时无谓弹出
      triggerCharacters: ['.'],
      provideCompletionItems: (model, position) => {
        const word = model.getWordUntilPosition(position)
        const range = {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: word.startColumn,
          endColumn: word.endColumn
        }

        // suggestions 提到 try 外：提供器中途抛错（哪怕一个候选计算出错）也不能
        // 让整轮补全报废 —— 真机症状就是「表名提示只剩文档里碰巧出现的那一个词」
        const suggestions = []
        try {

        // 0) 点号级联：`表.` → 该表的列；`库.` → 该库的表。
        //    列名没缓存就现场拉（fire-and-forget），下次触发时就能看到
        const lineText = model.getValueInRange({
          startLineNumber: position.lineNumber, startColumn: 1,
          endLineNumber: position.lineNumber, endColumn: position.column
        })
        // 别名映射：FROM t a / JOIN u AS b → 别名 → 表名。`别名.` 补该表的列
        //（排除保留字 —— `from x` 后跟换行 where 会被误当别名）
        const allText = model.getValue()
        const aliasMap = {}
        const ALIAS_RESERVED = new Set(['where', 'group', 'order', 'on', 'set', 'left', 'right', 'inner', 'outer', 'join', 'limit', 'union', 'select', 'as', 'into', 'update', 'values', 'having', 'cross', 'using', 'when', 'then'])
        const aliasRe = /(?:from|join|into|update)\s+[`"']?([\w.]+)[`"']?\s+(?:as\s+)?([a-z_][\w$]*)/gi
        let am
        while ((am = aliasRe.exec(allText))) {
          if (ALIAS_RESERVED.has(am[2].toLowerCase())) continue
          aliasMap[am[2].toLowerCase()] = am[1].split('.').pop()
        }
        const dotMatch = lineText.match(/([A-Za-z_][\w$]*)\.\w*$/)
        if (dotMatch) {
          const prefix = dotMatch[1].toLowerCase()
          const dbNames = databases.value.map((d) => String(d).toLowerCase())
          if (dbNames.includes(prefix)) {
            // `库名.` → **该库**的对象：表/视图/存储过程/函数/触发器。
            // 以前这里错误地弹了当前库的表名 —— 语义是按写下的库限定，不是当前下拉
            const dbOriginal = databases.value.find((d) => String(d).toLowerCase() === prefix) || prefix
            const cid = selectedConnId.value || props.conn?.id
            const objs = loadDbObjects(cid, dbOriginal)
            const push = (name, kind, detail) => {
              if (!name) return
              suggestions.push({
                label: name,
                kind: monaco.languages.CompletionItemKind[kind],
                insertText: name,
                range,
                sortText: '1' + name,
                detail
              })
            }
            objs.tables.forEach((n) => push(n, 'Class', t('tree.cat.tables')))
            objs.views.forEach((n) => push(n, 'Interface', t('tree.cat.views')))
            objs.procs.forEach((n) => push(n, 'Function', t('tree.cat.procs')))
            objs.fns.forEach((n) => push(n, 'Function', t('tree.cat.functions')))
            objs.triggers.forEach((n) => push(n, 'Event', t('tree.cat.triggers')))
            return { suggestions }
          }
          const tableHit = tableNames.value.find((tn) => String(tn).toLowerCase() === prefix)
            || Object.keys(columnsCache).find((k) => k === prefix)
          if (tableHit) {
            loadColumns(tableHit)
            const colDeco = decorateOnColumns(columnsCache[prefix] || [], lineText, prefix, aliasMap)
            colDeco.list.forEach((c) => suggestions.push({
              label: c.name,
              kind: monaco.languages.CompletionItemKind.Field,
              insertText: c.name,
              range,
              sortText: (c.same ? '0bbb' : '0ccc') + c.name,
              detail: (c.same ? t('sqlq.joinSameCol') + ' · ' : '') + t('sqlq.completionColumn')
            }))
            // 一键补全整段关联条件：把 `ON` 之后已输入的部分整个替换为 `a.x = b.x`
            const fs1 = colDeco.list.find((c) => c.same)
            if (fs1 && colDeco.otherPrefix && colDeco.onStartCol) {
              suggestions.push({
                label: `ON ${colDeco.otherPrefix}.${fs1.name} = ${prefix}.${fs1.name}`,
                kind: monaco.languages.CompletionItemKind.Snippet,
                insertText: `${colDeco.otherPrefix}.${fs1.name} = ${prefix}.${fs1.name}`,
                range: {
                  startLineNumber: position.lineNumber, startColumn: colDeco.onStartCol,
                  endLineNumber: position.lineNumber, endColumn: position.column
                },
                sortText: '0aaa' + fs1.name,
                detail: t('sqlq.joinFullSnippet')
              })
            }
            return { suggestions }
          }
          // 别名命中：`a.` → 别名对应的表 → 补该表的列（detail 标出来源表）
          const aliasTable = aliasMap[prefix]
          if (aliasTable) {
            loadColumns(aliasTable)
            const colDeco = decorateOnColumns(columnsCache[String(aliasTable).toLowerCase()] || [], lineText, prefix, aliasMap)
            colDeco.list.forEach((c) => suggestions.push({
              label: c.name,
              kind: monaco.languages.CompletionItemKind.Field,
              insertText: c.name,
              range,
              sortText: (c.same ? '0bbb' : '0ccc') + c.name,
              detail: aliasTable + ' · ' + (c.same ? t('sqlq.joinSameCol') + ' · ' : '') + t('sqlq.completionColumn')
            }))
            const fs2 = colDeco.list.find((c) => c.same)
            if (fs2 && colDeco.otherPrefix && colDeco.onStartCol) {
              suggestions.push({
                label: `ON ${colDeco.otherPrefix}.${fs2.name} = ${prefix}.${fs2.name}`,
                kind: monaco.languages.CompletionItemKind.Snippet,
                insertText: `${colDeco.otherPrefix}.${fs2.name} = ${prefix}.${fs2.name}`,
                range: {
                  startLineNumber: position.lineNumber, startColumn: colDeco.onStartCol,
                  endLineNumber: position.lineNumber, endColumn: position.column
                },
                sortText: '0aaa' + fs2.name,
                detail: t('sqlq.joinFullSnippet')
              })
            }
            return { suggestions }
          }
        }

        // 1) SQL 关键字（常用子集）：插入大小写跟随用户已敲的词
        SQL_KEYWORDS.forEach(kw => {
          suggestions.push({
            label: kw,
            kind: monaco.languages.CompletionItemKind.Keyword,
            insertText: smartCase(kw, word.word),
            range,
            sortText: '0fff' + kw,
            detail: t('settings.format.keyword')
          })
        })

        // 1.5) 内置函数（按连接方言）：代码片段插入 `FN(参数占位)`，
        //      光标落在第一个参数上，Tab 逐个跳；右括号已在片段里，无需手敲
        builtinFunctions(fmtDialect.value).forEach(fn => {
          suggestions.push({
            label: fn,
            kind: monaco.languages.CompletionItemKind.Function,
            insertText: functionInsertText(fn, word.word),
            insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
            range,
            sortText: '0eee' + fn,
            detail: t('sqlq.completionFunction')
          })
        })

        // 2) 表名：优先当前库真实表，其次从编辑器已引用的表提取
        const nameSet = new Set()
        tableNames.value.forEach(t => nameSet.add(t))
        // 对象树的结构缓存兜底：tableNames 还没就绪（刚切库/页签恢复中）时也能给全表名，
        // 否则补全列表里只剩文档里碰巧出现过的词（真机症状）
        if (!nameSet.size) {
          const cid = selectedConnId.value || props.conn?.id
          const hit = readSchemaCache('tables:' + cid + ':' + (selectedDatabase.value || props.database || ''))
          const cached = hit && Array.isArray(hit.value) ? hit.value : []
          cached.forEach((tb) => {
            const n = tb && typeof tb === 'object' ? (tb.name || tb.table || '') : String(tb || '')
            if (n) nameSet.add(n)
          })
        }
        const tableMatches = allText.match(/(?:FROM|JOIN|INTO|UPDATE|TABLE)\s+[`"']?([\w.]+)[`"']?/gi) || []
        const refTables = []
        tableMatches.forEach(m => {
          const name = m.replace(/(?:FROM|JOIN|INTO|UPDATE|TABLE)\s+/i, '').replace(/[`"']/g, '').trim()
          const base = name.split('.').pop()
          if (base) { nameSet.add(base); refTables.push(base) }
        })
        // 引用到的表顺带懒加载列（不阻塞本次补全）
        refTables.forEach(loadColumns)
        // 已缓存的列并入主列表：写 SELECT 的字段清单时不用再打「表.」
        const seenCols = new Set()
        refTables.forEach((t) => {
          ;(columnsCache[String(t).toLowerCase()] || []).forEach((c) => {
            if (seenCols.has(c)) return
            seenCols.add(c)
            suggestions.push({
              label: c,
              kind: monaco.languages.CompletionItemKind.Field,
              insertText: c,
              range,
              sortText: '0ddd' + c,
              detail: t('sqlq.completionColumn')
            })
          })
        })
        nameSet.forEach(name => {
          suggestions.push({
            label: name,
            kind: monaco.languages.CompletionItemKind.Class,
            insertText: name,
            range,
            sortText: '1aaa' + name,
            detail: t('tree.cat.tables')
          })
        })

        // 3) 数据库名
        databases.value.forEach(db => {
          suggestions.push({
            label: db,
            kind: monaco.languages.CompletionItemKind.Module,
            insertText: db,
            range,
            sortText: '1bbb' + db,
            detail: t('sqlq.snippetDatabase')
          })
        })

        // 4) 仅保留最常用的 SQL 片段，避免列表繁杂
        const snippets = [
          { label: 'SELECT * FROM', insertText: 'SELECT * FROM ${1:table_name};', detail: t('sqlq.snippetSelectAll') },
          { label: 'INSERT INTO', insertText: 'INSERT INTO ${1:table_name} (${2:columns}) VALUES (${3:values});', detail: '插入数据' },
          { label: 'UPDATE', insertText: 'UPDATE ${1:table_name} SET ${2:column} = ${3:value} WHERE ${4:condition};', detail: '更新数据' },
          { label: 'DELETE FROM', insertText: 'DELETE FROM ${1:table_name} WHERE ${2:condition};', detail: t('sqlq.snippetDelete') }
        ]
        snippets.forEach(s => {
          suggestions.push({
            label: s.label,
            kind: monaco.languages.CompletionItemKind.Snippet,
            insertText: s.insertText,
            insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
            range,
            sortText: '2' + s.label,
            detail: s.detail
          })
        })

        return { suggestions }
        } catch (err) {
          // 提供器抛错时 Monaco 会整轮丢弃补全、只剩文档词建议 —— 落日志并保住
          // 已生成的候选，绝不让「表名提示不行」变成无声的
          console.warn('[sqlq] SQL 补全提供器异常：', err)
        }
        return { suggestions }
      }
    }))

    // 注册格式化提供者
    monacoProviders.push(monaco.languages.registerDocumentFormattingEditProvider('sql', {
      provideDocumentFormattingEdits: (model) => {
        const formatted = smartFormatSql(model.getValue(), getEditorSettings(), fmtDialect.value)
        return [{
          range: model.getFullModelRange(),
          text: formatted
        }]
      }
    }))

    // 快捷键不再在此硬绑定：统一由下方 useShortcutScope 分发，
    // 键位可在「设置 → 快捷键」中自定义
  }

  // 光标 / 选中状态，供底部状态栏显示
  editor.onDidChangeCursorPosition((e) => {
    cursorLine.value = e.position.lineNumber
    cursorColumn.value = e.position.column
  })
  editor.onDidChangeCursorSelection((e) => updateEditorStatus(e.selection))
  updateEditorStatus(editor.getSelection())

  // 菜单开着时编辑器滚动（滚轮 / 光标移动带滚动）就把菜单关掉，避免菜单与代码行错位
  editor.onDidScrollChange(() => closeEdCtx())
}

// ========== 编辑器状态栏 ==========
const cursorLine = ref(1)
const cursorColumn = ref(1)
const selChars = ref(0)
const selLines = ref(0)
const hasEditorSelection = computed(() => selChars.value > 0)
const sqlLineCount = computed(() => (sql.value ? sql.value.split('\n').length : 1))
const updateEditorStatus = (selection) => {
  if (!selection || selection.isEmpty() || !editorInstance) {
    selChars.value = 0
    selLines.value = 0
    return
  }
  const text = editorInstance.getModel().getValueInRange(selection)
  selChars.value = text.length
  selLines.value = text ? text.split('\n').length : 0
}

// ========== 编辑器右键：SQL 常用操作 ==========
const selTextOf = (ed) => {
  if (!ed) return ''
  const s = ed.getSelection()
  if (!s || s.isEmpty()) return ''
  return ed.getModel().getValueInRange(s)
}
const replaceSelText = (ed, text) => {
  const s = ed.getSelection()
  if (!s || s.isEmpty()) return
  ed.executeEdits('dbmind-sql', [{ range: s, text, forceMoveMarkers: true }])
  ed.pushUndoStop()
  ed.focus()
}
/**
 * 只处理引号外的片段：'...' / "..." / `...` / [...] 内的内容原样保留。
 * 「转大写 / 转小写 / 压缩成一行」都走它，避免把字符串字面量里的数据一起改掉。
 */
const mapOutsideQuotes = (text, fn) => {
  let out = ''
  let i = 0
  while (i < text.length) {
    const ch = text[i]
    if (ch === "'" || ch === '"' || ch === '`') {
      let j = i + 1
      while (j < text.length) {
        if (text[j] === ch) {
          if (text[j + 1] === ch) { j += 2; continue } // '' / "" 转义
          break
        }
        j++
      }
      out += text.slice(i, Math.min(j + 1, text.length))
      i = j + 1
    } else if (ch === '[') {
      const j = text.indexOf(']', i)
      if (j < 0) { out += fn(text.slice(i)); break }
      out += text.slice(i, j + 1)
      i = j + 1
    } else {
      let j = i
      while (j < text.length && "'\"`[".indexOf(text[j]) < 0) j++
      out += fn(text.slice(i, j))
      i = j
    }
  }
  return out
}
const collapseSqlWs = (s) => String(s).replace(/[ \t]*\r?\n[ \t]*/g, ' ').replace(/[ \t]{2,}/g, ' ')
const transformSelectedSql = (fn, msg) => {
  const ed = editorInstance
  const t = selTextOf(ed)
  if (!t) { ElMessage.warning(t('sqlq.pickSqlFirst')); return }
  replaceSelText(ed, fn(t))
  if (msg) ElMessage.success(msg)
}
const formatSelectedSql = () => {
  const ed = editorInstance
  const t = selTextOf(ed)
  if (!t.trim()) { ElMessage.warning(t('sqlq.pickSqlToFormat')); return }
  try {
    replaceSelText(ed, smartFormatSql(t, getEditorSettings(), fmtDialect.value))
    ElMessage.success(t('sqlq.formattedSelection'))
  } catch (e) {
    ElMessage.error('格式化失败：' + (e?.message || e?.toString?.() || t('common.unknownError')))
  }
}
// 复制为 IN (...) 列表：按行取值，自动去掉行尾逗号与包裹引号（贴列名/值列表都能用）
const copySelectionAsInList = () => {
  const t = selTextOf(editorInstance)
  if (!t.trim()) { ElMessage.warning(t('sqlq.pickContentToConvert')); return }
  const items = t.split(/\r?\n/)
    .map(v => v.trim().replace(/,\s*$/, '').trim())
    .filter(Boolean)
    .map(v => v.replace(/^(['"`])([\s\S]*)\1$/, '$2').replace(/''/g, "'"))
  if (!items.length) return
  const body = items.map(v => `  '${String(v).replace(/'/g, "''")}'`).join(',\n')
  writeClipboard(`IN (\n${body}\n)`, `已复制 IN 列表（${items.length} 项）`)
}
// 选中光标所在的整条语句（按分号拆分，引号/注释内的分号不算）
const selectCurrentStatement = () => {
  const ed = editorInstance
  const model = ed && ed.getModel()
  if (!model) return
  const offset = model.getOffsetAt(ed.getPosition())
  const ranges = splitSqlStatementRanges(model.getValue())
  if (!ranges.length) return
  const hit = ranges.find(r => offset >= r.start && offset <= r.end) || ranges[ranges.length - 1]
  const start = model.getPositionAt(hit.start)
  const end = model.getPositionAt(hit.end)
  ed.setSelection({
    startLineNumber: start.lineNumber, startColumn: start.column,
    endLineNumber: end.lineNumber, endColumn: end.column
  })
  ed.revealRangeInCenterIfOutsideViewport(ed.getSelection())
  updateEditorStatus(ed.getSelection())
  ed.focus()
}
// ========== SQL 编辑器：右键菜单 ==========
// 说明：utils/monaco.js 是按需加载 Monaco（contrib 白名单），其中没有 contextmenu，
// 因此 Monaco 不会接管编辑器右键（浏览器原生菜单会直接弹出来）。这里由应用接管，
// 菜单样式与表格右键保持一致，并复用其二级子菜单交互。
const edCtx = ref({ visible: false, x: 0, y: 0, items: [] })
const edCtxSub = ref(null)
const closeEdCtxSub = () => { edCtxSub.value = null }
const closeEdCtx = () => {
  if (!edCtx.value.visible && !edCtxSub.value) return
  edCtx.value = { ...edCtx.value, visible: false }
  edCtxSub.value = null
}
// 二级子菜单：悬停带 sub 的项时在其右侧弹出（与结果表右键同一套逻辑）
const onEdCtxItemHover = (item, i, e) => {
  if (!item.sub || !item.sub.length) { closeEdCtxSub(); return }
  const el = e && e.currentTarget
  if (!el) return
  const menuEl = el.closest('.grid-ctx-menu')
  const menuRect = menuEl ? menuEl.getBoundingClientRect() : { left: 0, right: 0, top: 0 }
  const itemRect = el.getBoundingClientRect()
  edCtxSub.value = { parentIndex: i, items: item.sub, x: menuRect.right + 2, y: itemRect.top, menuLeft: menuRect.left }
  nextTick(() => {
    const subEl = document.querySelector('.grid-ctx-sub')
    if (!subEl || !edCtxSub.value) return
    const r = subEl.getBoundingClientRect()
    let x = edCtxSub.value.x
    let y = edCtxSub.value.y
    if (x + r.width > window.innerWidth - 4) x = Math.max(4, edCtxSub.value.menuLeft - r.width - 2)
    if (y + r.height > window.innerHeight - 4) y = Math.max(4, window.innerHeight - r.height - 4)
    edCtxSub.value = { ...edCtxSub.value, x, y }
  })
}
const onEditorContextMenu = (e) => {
  const ed = editorInstance
  if (!ed || !ed.getModel()) return
  // 右键点不在当前选中范围内 → 光标移到右键处（与主流编辑器一致，避免误操作到上一处选区）
  const target = ed.getTargetAtClientPoint(e.clientX, e.clientY)
  const pos = target && target.position
  const sel = ed.getSelection()
  const inSel = pos && sel && !sel.isEmpty() &&
    pos.lineNumber >= sel.startLineNumber && pos.lineNumber <= sel.endLineNumber
  if (pos && !inSel) {
    ed.setSelection({
      startLineNumber: pos.lineNumber, startColumn: pos.column,
      endLineNumber: pos.lineNumber, endColumn: pos.column
    })
  }
  ed.focus()
  updateEditorStatus(ed.getSelection())

  const text = selTextOf(ed)
  const hasSel = !!text
  // 菜单右侧的键位提示取自「设置 → 快捷键」的实时配置，改键后提示同步变化
  const sc = loadShortcuts()
  const key = (id) => sc[id] || ''
  const items = []
  const sep = () => { if (items.length && !items[items.length - 1].sep) items.push({ sep: true }) }
  // —— 执行 ——
  items.push({ label: t('sqlq.ctxRunSel'), command: 'run-sel', shortcut: key('query.run'), disabled: !hasSel })
  items.push({ label: t('shortcut.query.runAll.label'), command: 'run-all', shortcut: key('query.runAll') })
  // —— 处理选中片段 ——
  sep()
  items.push({ label: t('sqlq.ctxFmtSel'), command: 'format-sel', shortcut: key('query.format'), disabled: !hasSel })
  items.push({ label: t('shortcut.query.upper.label'), command: 'upper', shortcut: key('query.upper'), disabled: !hasSel })
  items.push({ label: t('shortcut.query.lower.label'), command: 'lower', shortcut: key('query.lower'), disabled: !hasSel })
  items.push({ label: t('shortcut.query.oneLine.label'), command: 'one-line', shortcut: key('query.oneLine'), disabled: !hasSel })
  items.push({ label: t('shortcut.query.comment.label'), command: 'comment', shortcut: key('query.comment') })
  // —— 复制 / 选中 ——
  sep()
  items.push({ label: t('sqlq.ctxCopyAs'), sub: [
    { label: t('sqlq.ctxInList'), command: 'copy-in', shortcut: key('query.copyInList'), disabled: !hasSel },
    { label: t('sqlq.ctxOneLine'), command: 'copy-one-line', shortcut: key('query.copyOneLine'), disabled: !hasSel }
  ] })
  items.push({ label: t('shortcut.query.selectStatement.label'), command: 'select-stmt', shortcut: key('query.selectStatement') })
  // —— 通用编辑（Monaco / 系统级键位，固定不可改，这里只做提示）——
  sep()
  items.push({ label: t('tdv.revert'), command: 'undo', shortcut: 'Ctrl+Z' })
  items.push({ label: t('common.redo'), command: 'redo', shortcut: 'Ctrl+Y' })
  items.push({ label: t('common.cut'), command: 'cut', shortcut: 'Ctrl+X', disabled: !hasSel })
  items.push({ label: t('mdk.copy'), command: 'copy', shortcut: 'Ctrl+C', disabled: !hasSel })
  items.push({ label: t('common.paste'), command: 'paste', shortcut: 'Ctrl+V' })
  items.push({ label: t('common.selectAll'), command: 'select-all', shortcut: 'Ctrl+A' })

  edCtxSub.value = null
  edCtx.value = { visible: true, x: e.clientX, y: e.clientY, items }
  // 贴近视口边缘时自动回退，避免菜单被裁掉
  nextTick(() => {
    const el = document.querySelector('.ed-ctx-menu')
    if (!el) return
    const r = el.getBoundingClientRect()
    let x = edCtx.value.x
    let y = edCtx.value.y
    if (x + r.width > window.innerWidth - 4) x = Math.max(4, window.innerWidth - r.width - 4)
    if (y + r.height > window.innerHeight - 4) y = Math.max(4, window.innerHeight - r.height - 4)
    edCtx.value = { ...edCtx.value, x, y }
  })
}
const onEdCtxItem = (item) => {
  if (!item || item.sep || item.sub || item.disabled) return
  const ed = editorInstance
  closeEdCtx()
  if (!ed) return
  // **先还焦点再执行**：粘贴/剪切等剪贴板操作要求编辑器持有焦点
  //（navigator.clipboard.readText 在无焦点时直接失败，此前是先 trigger 后 focus，
  // 于是「粘贴」从自定义菜单触发时永远静默无效）
  ed.focus()
  const trigger = (id) => ed.trigger('ed-ctx', id, null)
  switch (item.command) {
    case 'run-sel': runSql(); break
    case 'run-all': runSqlAll(); break
    case 'format-sel': formatSelectedSql(); break
    case 'upper': transformSelectedSql(t => mapOutsideQuotes(t, seg => seg.toUpperCase())); break
    case 'lower': transformSelectedSql(t => mapOutsideQuotes(t, seg => seg.toLowerCase())); break
    case 'one-line': transformSelectedSql(t => mapOutsideQuotes(t, collapseSqlWs).trim()); break
    case 'comment': {
      const a = ed.getAction('editor.action.commentLine')
      if (a) a.run()
      break
    }
    case 'copy-in': copySelectionAsInList(); break
    case 'copy-one-line': {
      const t = selTextOf(ed)
      if (t) writeClipboard(collapseSqlWs(t).trim(), t('sqlq.copyOneLineDone'))
      break
    }
    case 'select-stmt': selectCurrentStatement(); break
    case 'undo': trigger('undo'); break
    case 'redo': trigger('redo'); break
    case 'cut': trigger('editor.action.clipboardCutAction'); break
    case 'copy': trigger('editor.action.clipboardCopyAction'); break
    case 'paste': {
      // 自实现粘贴：Monaco 内置粘贴动作依赖浏览器原生右键链路，自定义菜单下
      // navigator.clipboard.readText 需要「焦点 + 权限」，上面已补焦点；首次会弹
      // 一次剪贴板权限询问（允许后记住），拒绝或失败则回落到内置动作
      navigator.clipboard?.readText?.()
        .then((text) => {
          if (text) {
            ed.executeEdits('ed-ctx', [{ range: ed.getSelection(), text, forceMoveMarkers: true }])
          } else {
            trigger('editor.action.clipboardPasteAction')
          }
        })
        .catch(() => trigger('editor.action.clipboardPasteAction'))
      break
    }
    case 'select-all': trigger('editor.action.selectAll'); break
    default: break
  }
  ed.focus()
}

// 获取当前要执行的 SQL（选中优先，无选中执行全部）
// reuseLast：翻页时沿用上次执行的 SQL。否则「执行全部 / 执行选中」之后翻页会再读一次
// 编辑器选区，出现「明明跑的是全部，翻到第 2 页却只跑了选中片段」的错位。
let lastExecSql = ''
const getExecutableSql = (reuseLast = false) => {
  if (reuseLast && lastExecSql) return lastExecSql
  if (forceRunAll || !editorInstance) return sql.value.trim()
  const selection = editorInstance.getSelection()
  const selectedText = editorInstance.getModel().getValueInRange(selection).trim()
  return selectedText || sql.value.trim()
}

// 「执行全部」：忽略当前选中，强制跑整段脚本（右键菜单用）
let forceRunAll = false
const runSqlAll = () => {
  if (running.value) return
  forceRunAll = true
  Promise.resolve(runSql()).finally(() => { forceRunAll = false })
}

/**
 * 空编辑器统一拦截。
 *
 * 执行 / 格式化 / AI 分析都依赖编辑器里有内容，原来这几处是静默 return ——
 * 点完没有任何反馈，最容易被当成「按钮坏了」。
 */
const requireSql = () => {
  if (sql.value.trim()) return true
  ElMessage.warning(t('sqlq.editorEmptyInput'))
  return false
}

// 危险 SQL 判定：DELETE FROM / DROP / TRUNCATE（与设置页文案一致，忽略注释）
const isDangerousSql = (s) => {
  // 去掉块注释与行注释后，仅检测危险关键字语句（避免被注释内容触发）
  const clean = (s || '').replace(/\/\*[\s\S]*?\*\//g, ' ').replace(/(--|#)[^\n\r]*/g, ' ').toUpperCase()
  // 关键字前必须紧跟语句边界（行首 / 分号 / 空白），防止匹配到字符串、标识符等
  return /(?:^|;|\s)(?:DELETE\s+FROM\b|DROP\s+(?:TABLE|DATABASE|VIEW|INDEX|SCHEMA|SEQUENCE|PROCEDURE|FUNCTION|TRIGGER|EVENT|TYPE)\b|TRUNCATE\s+(?:TABLE\s+)?)/.test(clean)
}

const setResultCancelled = () => {
  resultItems.value = []
  activeResultIdx.value = 0
  result.value = { columns: [], rows: [], success: false, message: t('sqlq.canceled'), executeTime: elapsedTime.value }
}

// 展示单条结果（原有执行路径）
const showSingleResult = (res) => {
  resultItems.value = []
  activeResultIdx.value = 0
  if (res && res.success) {
    result.value = res
    if (res.rowCount === 0 && !res.columns.length && res.affectedRows >= 0) {
      // 后端的 message 是英文（"Query OK, N rows affected"）—— 提示按界面语言走词典
      ElMessage.success(t('sqlq.affectedOk', { n: res.affectedRows }))
    }
  } else {
    result.value = res || { columns: [], rows: [], success: false, message: t('ai.runFailed'), executeTime: 0 }
    if (result.value.message === t('sqlq.canceled')) return
    // 执行失败不弹全局浮窗，完整错误交由下方结果面板展示（避免编辑器中部堆叠提示）
    ElMessageBoxWithFix(result.value.message || t('ai.runFailed'))
  }
}

// 展示批量（多段）结果：每段语句一个 tab，tab 之间切换仅切换数据源展示。
// 每段带 `sql`（段落原文）与真实 totalCount（后端统计）—— 支持按段翻页
const showBatchResult = (b) => {
  const results = (b && Array.isArray(b.results)) ? b.results : []
  if (!results.length) {
    showSingleResult(b || { columns: [], rows: [], success: false, message: t('ai.runFailed'), executeTime: 0 })
    return
  }
  resultItems.value = results.map((r, i) => ({
    label: t('sqlq.resultN', { n: i + 1 }),
    res: r,
    // 段落原文：翻到第 N 页时只重跑这一段（整批重跑会把写入语句再执行一遍）
    segmentSql: r.sql || '',
    page: 1
  }))
  activeResultIdx.value = 0
  result.value = results[0]
  // 首个 tab 若总数未知，异步补齐（后端批量路径也不再同步 COUNT）
  fetchCountFor(result.value, results[0].sql || '')
  // 编辑器文本含多段但实际仅拆出单段（例程块 / 注释等）：按单结果做收尾提示
  if (results.length === 1) {
    const only = results[0]
    if (only.success && only.rowCount === 0 && !only.columns.length && only.affectedRows >= 0) {
      // 与单条路径同口径：后端的英文回执按界面语言走词典
      ElMessage.success(t('sqlq.affectedOk', { n: only.affectedRows }))
    } else if (!only.success && only.message !== t('sqlq.canceled')) {
      ElMessageBoxWithFix(only.message || t('ai.runFailed'))
    }
  }
}

// 切换到第 i 条结果 tab（resultItems 数据源切换，表格列宽/虚拟滚动自动重算）
const selectResultTab = (i) => {
  const item = resultItems.value[i]
  if (!item) return
  if (i === activeResultIdx.value && result.value === item.res) return
  activeResultIdx.value = i
  result.value = item.res
  // 换结果集 ⇒ 行号基准变了，编辑缓冲作废（快照守卫也会拦过期提交）
  discardEdits()
  // 切到的 tab 若总数未知，异步补齐（只数当前展示的段，不并发数全部）
  fetchCountFor(result.value, item.segmentSql || item.res?.sql || '')
  // 不同结果集独立分页：切换 tab 恢复到该段自己的页码
  currentPage.value = item.page || 1
  clearResultRowSelection()
  resultActiveCell.value = null
  resultSortColumn.value = ''
  resultSortDir.value = 'ASC'
  unsortedRows.value = (result.value?.rows || []).slice()
  hiddenResultCols.value = new Set()
  clearColSelect()
}

// ===== 生产库保护（生产库二次确认）=====
// 与下面的「危险 SQL 确认」是**两条独立的闸**，别合并理解：
// - 危险 SQL 确认看的是**语句**（DELETE/DROP/TRUNCATE），用户可以在设置里关掉（天天写生产的人会关）；
// - 生产库确认看的是**连接**（环境标记为 PROD），只要在写就确认，**不跟随那个设置**
//   —— 关掉前者是"我知道自己在写什么"，不该顺带失去后者这层兜底。
const isProdConnection = () => String(props.conn?.env || '').toUpperCase() === 'PROD'
const WRITE_SQL_RE = /^\s*(insert|update|delete|replace|merge|truncate|drop|alter|create|rename|grant|revoke|call|exec|execute|load\s+data|set\s+global)\b/i
const isWriteSql = (text) => String(text || '')
  .split('\n')
  .map(line => line.replace(/--.*$/, '').trim())
  .filter(Boolean)
  .some(line => WRITE_SQL_RE.test(line))
const firstSqlLine = (text) => String(text || '').split('\n').map(s => s.trim()).filter(Boolean)[0] || ''

// ===== 写操作影响行预览：UPDATE/DELETE 执行前先算「将影响多少行」=====
// DELETE → 直接包一层 COUNT；UPDATE → 截取 WHERE 段包 COUNT（字符串里再出现 where 的
// 概率很低，估算口径，弹窗里标明「预估」）。无 WHERE 条件时特别提示全表影响。
const writeCountSql = (stmt) => {
  const s = stmt.replace(/;\s*$/, '')
  let m = s.match(/^delete\s+from\s+(`[^`]+`|"[^"]+"|\[[^\]]+\]|[\w.]+)\s*(where[\s\S]*)?$/i)
  if (m) return `select count(*) as cnt from ${m[1]} ${m[2] || ''}`.trim()
  m = s.match(/^update\s+(`[^`]+`|"[^"]+"|\[[^\]]+\]|[\w.]+)\s+set\s+[\s\S]*$/i)
  if (m) {
    const wm = s.match(/\swhere\s([\s\S]*)$/i)
    return wm
      ? `select count(*) as cnt from ${m[1]} where ${wm[1]}`
      : `select count(*) as cnt from ${m[1]}`
  }
  return null
}
const escHtmlLocal = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]))
/** 返回 true=继续执行；false=用户取消。非 UPDATE/DELETE（如 INSERT）不预览直接过。 */
const previewWriteRows = async (execSql, connId, db) => {
  const stmts = splitSqlStatements(execSql)
  const rows = []
  for (const stmt of stmts) {
    const countSql = writeCountSql(stmt)
    if (!countSql) continue
    let count = null
    try {
      const r = await executeSql(connId, countSql, db, null, null, 1, 10, true)
      const v = r && r.rows && r.rows[0] ? Object.values(r.rows[0])[0] : null
      if (typeof v === 'number') count = v
      else if (v != null && !isNaN(Number(v))) count = Number(v)
    } catch { /* 预估失败按未知处理 */ }
    rows.push({ stmt: stmt.replace(/\s+/g, ' ').trim().slice(0, 90), count })
  }
  if (!rows.length) return true
  const html = rows.map((r, i) => {
    const n = r.count == null
      ? '<b>无法预估</b>'
      : (r.count === 0 ? '<b>0</b> 行' : `<b>${r.count.toLocaleString()}</b> 行`)
    const noWhere = /delete\s+from\s+[^\s]+\s*;?\s*$/i.test(r.stmt) || /update\s+[^\s]+\s+set\b(?![\s\S]*\bwhere\b)/i.test(r.stmt)
    return `${i + 1}. <code>${escHtmlLocal(r.stmt)}</code><br>&nbsp;&nbsp;&nbsp;预估影响：${n}${noWhere ? ' —— <b style="color:#e34d4d">⚠ 无 WHERE 条件，将影响全表！</b>' : ''}`
  }).join('<br><br>')
  try {
    await ElMessageBox.confirm(
      `<div style="text-align:left">以下写语句执行前的<b>预估影响行数</b>（基于当前数据）：${''}<br><br>${html}</div>`,
      t('sqlq.writePreviewTitle'),
      { type: 'warning', dangerouslyUseHTMLString: true, confirmButtonText: t('sqlq.writePreviewRun'), cancelButtonText: t('common.cancel'), closeOnClickModal: false, closeOnPressEscape: false }
    )
    return true
  } catch { return false }
}

// ===== 模板变量：SQL 里带 :name 占位符时，执行前弹窗填参 =====
// 提取/替换都走 mapOutsideQuotes（字符串字面量里的 "12:30" 不会被当成变量）。
// 值按名称记忆（localStorage），数字直写、null 置 NULL、其余按字符串字面量转义。
let varValuesOnce = null
let pendingRunParams = null
const varDialog = ref({ visible: false, vars: [], values: {} })
const VARVALS_LS = 'dbmind_varvalues'
const readVarVals = () => { try { return JSON.parse(localStorage.getItem(VARVALS_LS) || '{}') } catch { return {} } }
const extractSqlVars = (text) => {
  const found = []
  const seen = new Set()
  mapOutsideQuotes(text, (seg) => {
    const re = /(?<![:@\w]):([a-z_][\w$]*)/gi
    let m
    while ((m = re.exec(seg))) {
      if (!seen.has(m[1].toLowerCase())) { seen.add(m[1].toLowerCase()); found.push(m[1]) }
    }
    return seg
  })
  return found
}
const substituteVars = (text, values) => mapOutsideQuotes(text, (seg) =>
  seg.replace(/(?<![:@\w]):([a-z_][\w$]*)/gi, (whole, name) => {
    const v = values[name]
    if (v === undefined || String(v).trim() === '') return whole
    const s = String(v).trim()
    if (/^null$/i.test(s)) return 'NULL'
    if (/^-?\d+(\.\d+)?$/.test(s)) return s
    return `'${s.replace(/'/g, "''")}'`
  })
)
const openVarDialog = (vars) => {
  const remembered = readVarVals()
  const values = {}
  vars.forEach((v) => { values[v] = remembered[v] || '' })
  varDialog.value = { visible: true, vars, values }
}
const onVarConfirm = () => {
  const d = varDialog.value
  const missing = d.vars.filter((v) => !(d.values[v] || '').trim())
  if (missing.length) { ElMessage.warning(t('sqlq.varMissing', { n: missing.length })); return }
  try {
    const m = readVarVals()
    d.vars.forEach((v) => { m[v] = d.values[v] })
    localStorage.setItem(VARVALS_LS, JSON.stringify(m))
  } catch { /* 存储不可用就算了 */ }
  varValuesOnce = { ...d.values }
  d.visible = false
  const p = pendingRunParams || {}
  pendingRunParams = null
  runSql(p.page || 1, p.size || pageSize.value, p.batchable !== false)
}
const onVarCancel = () => {
  varDialog.value.visible = false
  pendingRunParams = null
}

const runSql = async (page = 1, size = pageSize.value, batchable = true) => {
  if (running.value) return
  if (!requireSql()) return
  // 有未提交的单元格修改：翻页/重跑会使行号失效 —— 确认放弃才继续
  if (pendingEdits.value.length) {
    try {
      await ElMessageBox.confirm(t('sqlq.editsLoseWarn', { n: pendingEdits.value.length }), t('sqlq.editsLoseTitle'), {
        confirmButtonText: t('sqlq.discardEdits'),
        cancelButtonText: t('common.cancel'),
        type: 'warning'
      })
    } catch { return }
    discardEdits()
  }
  let execSql = getExecutableSql(page !== 1)
  if (!execSql) return
  // 模板变量：一次性值已在手（确认弹窗后重入）就替换；否则先弹窗收参
  if (varValuesOnce) {
    execSql = substituteVars(execSql, varValuesOnce)
    varValuesOnce = null
  } else {
    const vars = extractSqlVars(execSql)
    if (vars.length) {
      pendingRunParams = { page, size, batchable }
      openVarDialog(vars)
      return
    }
  }
  lastExecSql = execSql
  // 生产库保护：PROD 连接上写操作再确认一次（读操作不打扰）
  if (!isNoSql.value && isProdConnection() && isWriteSql(execSql)) {
    try {
      await ElMessageBox.confirm(
        '这条连接标记为「生产环境」，即将执行写操作：\n' + firstSqlLine(execSql) + '\n\n确认继续吗？',
        t('sqlq.prodWriteTitle'),
        { type: 'warning', confirmButtonText: t('sqlq.prodWriteConfirm'), cancelButtonText: t('tree.multiCancel'), closeOnClickModal: false, closeOnPressEscape: false }
      )
    } catch { return }
  }
  // 查询设置t('settings.query.confirmDanger')：危险 SQL 二次确认（仅关系型数据库）。
  // 每次执行时实时读取，确保设置页保存后立即生效（querySettings 为 setup 时快照）
  if (!isNoSql.value && getQuerySettings().confirmDanger && isDangerousSql(execSql)) {
    try {
      await ElMessageBox.confirm(t('sqlq.dangerBody'), t('sqlq.dangerTitle'), {
        type: 'warning',
        confirmButtonText: t('sqlq.dangerConfirm'),
        cancelButtonText: t('tree.multiCancel'),
        closeOnClickModal: false,
        closeOnPressEscape: false
      })
    } catch { return }
  }
  running.value = true
  loading.value = true
  resultVisible.value = true
  resultItems.value = []
  activeResultIdx.value = 0
  currentPage.value = page
  pageSize.value = size
  const queryStart = Date.now()
  elapsedTime.value = 0
  cancelRequested.value = false
  invalidateCount()
  clearStmtError() // 新一轮执行：上一轮的报错标红清掉
  execId.value = 'q_' + Date.now() + '_' + Math.random().toString(36).slice(2, 8)
  cancelController = new AbortController()
  if (queryTimer) clearInterval(queryTimer)
  queryTimer = setInterval(() => { elapsedTime.value = Date.now() - queryStart }, 100)
  // 首次出结果时把"只有编辑器"的布局切成上下分栏。
  // 现在比例 1 也可能是用户**亲手**拉到底（编辑器铺满）的结果，那种情况下别去动他。
  if (editorRatio.value === 1 && !ratioTouched) { editorRatio.value = 0.6; ratioCycle = 0 }
  try {
    const connId = selectedConnId.value || props.conn.id
    if (isNoSql.value) {
      const res = await executeNoSql(connId, selectedDatabase.value || undefined, execSql)
      if (cancelRequested.value) { setResultCancelled(); return }
      showSingleResult(res)
    } else {
      const db = selectedSchema.value
        ? `${selectedDatabase.value}.${selectedSchema.value}`
        : selectedDatabase.value || undefined
      // 写操作影响行预览：UPDATE/DELETE 执行前先确认将影响多少行（取消则不执行）
      if (isWriteSql(execSql)) {
        const go = await previewWriteRows(execSql, connId, db)
        if (!go) return
      }
      // 多段 SQL：批量执行（同连接顺序执行），每段结果以 tab 展示；
      // 分页翻页或单条语句仍走 executeSql，保留原分页能力
      const statements = batchable ? splitSqlStatements(execSql) : []
      if (statements.length > 1) {
        const b = await executeSqlBatch(connId, execSql, db, execId.value, cancelController.signal)
        if (cancelRequested.value) { setResultCancelled(); return }
        showBatchResult(b)
        // 事务模式：批量里有写语句成功 ⇒ 有未提交变更
        if (txMode.value && (b && Array.isArray(b.results)) && b.results.some((r) => r && r.success && (r.affectedRows || 0) > 0)) txDirty.value = true
        // 批量失败：定位并标红出错的那条语句（序号与前端切分一致）
        const failIdx = (b && Array.isArray(b.results)) ? b.results.findIndex((r) => r && r.success === false) : -1
        if (failIdx >= 0) markErrorStatement(b.results[failIdx], failIdx)
        const histCost = (b && Array.isArray(b.results)) ? b.results.reduce((m, r) => Math.max(m, (r.executeTime) || 0), 0) : 0
        recordHistory(execSql, db, histCost)
      } else {
        const res = await executeSql(connId, execSql, db, execId.value, cancelController.signal, page, size)
        if (cancelRequested.value) { setResultCancelled(); return }
        showSingleResult(res)
        // 事务模式：写语句成功 ⇒ 有未提交变更（提示条点亮，提醒 COMMIT/ROLLBACK）
        if (txMode.value && res && res.success && (res.affectedRows || 0) > 0) txDirty.value = true
        // 总数未知时异步补齐（后端已不同步 COUNT，见 fetchCountFor）
        fetchCountFor(result.value, execSql)
        // 失败 → 编辑器里把出错语句标红定位（execSql = 选中的那条或整段单条）
        if (!res.success) markErrorStatement(res, undefined, execSql)
        recordHistory(execSql, db, res.executeTime || 0)
        loadResColumnComments(execSql, connId, db)
      }
    }
  } catch (e) {
    if (cancelRequested.value) { setResultCancelled(); return }
    const msg = e?.message || e?.toString?.() || t('common.unknownError')
    resultItems.value = []
    activeResultIdx.value = 0
    result.value = { columns: [], rows: [], success: false, message: msg, executeTime: 0 }
  } finally {
    if (queryTimer) { clearInterval(queryTimer); queryTimer = null }
    cancelController = null
    running.value = false
    loading.value = false
  }
}

// 停止/取消正在执行的 SQL
const stopSql = async () => {
  if (!running.value) return
  cancelRequested.value = true
  const id = execId.value
  if (id) cancelSql(id).catch(() => {})
  if (cancelController) cancelController.abort()
  loading.value = false
  running.value = false
}

const formatSql = () => {
  // NoSQL 不适用 SQL 格式化，这里不是「内容为空」，不弹提示
  if (isNoSql.value) return
  if (!requireSql()) return
  // 有选中就只格式化选中片段（与「执行」的选中优先语义一致）
  const sel = editorInstance ? selTextOf(editorInstance) : ''
  if (sel.trim()) { formatSelectedSql(); return }
  try {
    sql.value = smartFormatSql(sql.value, getEditorSettings(), fmtDialect.value)
    ElMessage.success(t('sce.formatDone'))
  } catch (e) {
    ElMessage.error('格式化失败：' + (e?.message || e?.toString?.() || t('common.unknownError')))
  }
}

// 脚本保存/加载
const loadSavedScripts = () => {
  try {
    const raw = localStorage.getItem(scriptKey())
    savedScripts.value = raw ? JSON.parse(raw) : []
  } catch {
    savedScripts.value = []
  }
}

// 关闭带未保存改动的脚本时使用：返回 Promise<boolean>，true=已保存 / false=用户取消
let closeSaveResolve = null
const saveForClose = () => new Promise((resolve) => {
  if (props.scriptName) {
    resolve(doSaveScript(props.scriptName, true))
    return
  }
  closeSaveResolve = resolve
  saveDialogVisible.value = true
  saveName.value = '脚本 ' + new Date().toLocaleString('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })
})
const cancelSaveDialog = () => {
  saveDialogVisible.value = false
  if (closeSaveResolve) { closeSaveResolve(false); closeSaveResolve = null }
}

// nameOverride 可能来自 @click 直接绑定（会是 MouseEvent 对象），仅字符串才当作脚本名，否则用输入框
const doSaveScript = (nameOverride, silent = false) => {
  const name = (typeof nameOverride === 'string' && nameOverride ? nameOverride : saveName.value).trim()
  if (!name) { if (!silent) ElMessage.warning(t('sqlq.enterScriptName')); return false }
  if (!sql.value.trim()) { if (!silent) ElMessage.warning(t('sqlq.editorEmpty')); return false }
  const db = selectedDatabase.value || props.database
  if (!db) { if (!silent) ElMessage.warning(t('sqlq.pickDatabase')); return false }
  loadSavedScripts()
  // 同名脚本视为"覆盖更新"，避免重复条目（自动保存依赖此语义）
  const existing = savedScripts.value.find(x => x.name === name)
  if (existing) {
    existing.content = sql.value
    existing.time = Date.now()
  } else {
    savedScripts.value.unshift({ id: 'script_' + Date.now(), name, content: sql.value, time: Date.now() })
  }
  localStorage.setItem(scriptKey(), JSON.stringify(savedScripts.value.slice(0, 50)))
  saveDialogVisible.value = false
  saveName.value = ''
  lastSavedSql.value = sql.value
  isDirty.value = false
  emit('dirty-change', false)
  emit('save', name)
  if (closeSaveResolve) { closeSaveResolve(true); closeSaveResolve = null }
  if (!silent) ElMessage.success(t('sqlq.scriptSaved', { db: db }))
  notifyScriptsChanged()
  return true
}
// getSql 供父级在保存会话快照时读取实时内容（兜底：即使 sql-change 还没触发也能取到最新）
defineExpose({ saveForClose, getSql: () => sql.value })

// 自动保存：停止输入 1.5s 后静默保存。已命名脚本按原名覆盖更新；
// 未命名的新脚本在**第一次**自动保存时按「脚本 + 时间」自动建档（后续沿用同一个名字，
// 覆盖更新同一条 —— 否则开关打开也没东西可存，等于摆设）。
let autoSaveTimer = null
let autoDraftName = ''
const scheduleAutoSave = (val, dirty) => {
  if (autoSaveTimer) { clearTimeout(autoSaveTimer); autoSaveTimer = null }
  if (!dirty || !val.trim()) return
  if (!editorSettings.value.autoSave) return
  autoSaveTimer = setTimeout(() => {
    autoSaveTimer = null
    if (!sql.value || sql.value === lastSavedSql.value) return
    if (!props.scriptName && !autoDraftName) {
      autoDraftName = '脚本 ' + new Date().toLocaleString('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })
    }
    const name = props.scriptName || autoDraftName
    if (name) doSaveScript(name, true)
  }, 1500)
}

const onSaveClick = () => {
  if (!requireSql()) return
  if (props.scriptName) {
    doSaveScript(props.scriptName)
  } else {
    saveDialogVisible.value = true
    saveName.value = '脚本 ' + new Date().toLocaleString('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })
  }
}

const ElMessageBoxWithFix = (errMsg) => {
  lastError.value = errMsg
}

const lastError = ref('')

// AI 预检（所有 AI 入口共用）：没配置就直接一句提示返回，调用方不要再弹任何框
const ensureAiConfigured = async () => {
  try {
    const cfg = await getAiConfig()
    if (cfg && cfg.enabled && (cfg.models || []).length) return true
    ElMessageBox.confirm(t('sqlq.aiNotConfigured'), t('sqlq.aiNotConfiguredTitle'), {
      confirmButtonText: t('sqlq.gotoAiSettings'),
      cancelButtonText: t('common.cancel'),
      type: 'warning',
    }).then(() => window.dispatchEvent(new CustomEvent('dc-open-settings', { detail: { tab: 'ai' } }))).catch(() => {})
    return false
  } catch { return true /* 配置读不到就照旧走原流程，由后端给出准确原因 */ }
}

const askAi = async (mode) => {
  // 先拦在弹窗之前：没内容还弹出一个转圈的对话框，比不弹更让人困惑
  if (!requireSql()) return
  // AI 没配置时**连对话框都不弹**：空框 + 一条错误气泡，比单纯一句提示更让人困惑
  if (!(await ensureAiConfigured())) return
  aiDialogTitle.value = mode === 'explain' ? t('sqlq.aiTitleExplain')
    : mode === 'fix' ? t('sqlq.aiTitleFix')
    : mode === 'diagnose' ? t('sqlq.aiTitleDiagnose') : t('sqlq.aiTitleOptimize')
  aiDialogVisible.value = true
  aiLoading.value = true
  aiResult.value = ''
  aiUsage.value = null

  aiResultIsSql.value = false
  aiErrorReason.value = ''
  try {
    const payload = { sql: sql.value, connectionId: selectedConnId.value || props.conn.id, database: props.database, modelId: selectedAiModelId.value }
    const res = mode === 'explain' ? await aiExplain(payload)
      : mode === 'fix' ? await aiFix({ ...payload, error: result.value.message })
      : mode === 'diagnose' ? await aiDiagnose(payload)
      : await aiOptimize(payload)
    if (res && res.success) {
      if (mode === 'fix') { aiErrorReason.value = res.errorReason || ''; aiResult.value = res.sql || res.content || '' }
else { aiResult.value = res.content }
      aiResultIsSql.value = mode === 'fix'
          aiUsage.value = res.usage || null

    }
    else ElMessage.error(res?.message || t('ai.requestFailed'))
  } catch (e) { ElMessage.error(e?.message || e?.toString?.() || t('ai.requestFailed')) }
  aiLoading.value = false
}

// 行内 AI 改写：对选中内容（无选中则全文）按自然语言要求重写
const askAiRewrite = async () => {
  // 文案与其它入口统一：原来写「请先选中要改写的 SQL」会让人以为必须先划选，
  // 其实无选中时就是对全文改写，真正缺的是内容本身
  if (!requireSql()) return
  // 同 askAi：AI 没配置时改写输入框也不该弹出来
  if (!(await ensureAiConfigured())) return
  const target = getExecutableSql()
  if (!target) return
  let instruction = ''
  try {
    const r = await ElMessageBox.prompt(t('sqlq.rewriteAskBody'), t('sqlq.aiTitleRewrite'), {
      confirmButtonText: t('sqlq.rewriteAskConfirm'),
      cancelButtonText: t('tree.multiCancel'),
      inputValue: t('sqlq.rewriteAskDefault'),
      inputPlaceholder: t('sqlq.rewriteAskPlaceholder')
    })
    instruction = (r && r.value) || ''
  } catch (e) {
    return
  }
  aiDialogTitle.value = t('sqlq.aiTitleRewrite')
  aiDialogVisible.value = true
  aiLoading.value = true
  aiResult.value = ''
  aiErrorReason.value = ''
  aiResultIsSql.value = true
  try {
    const res = await aiChat({
      system: '你是一名资深数据库工程师。请按用户要求改写 SQL，只输出改写后的完整 SQL 语句，'
        + '不要任何解释、不要 Markdown 代码块标记。',
      prompt: t('sqlq.aiRewriteAsk') + instruction + '\n\n### 原始 SQL\n' + target,
      connectionId: selectedConnId.value || props.conn.id,
      database: props.database,
      modelId: selectedAiModelId.value
    })
if (res && res.success) { aiResult.value = res.content; aiUsage.value = res.usage || null }
    else ElMessage.error(res?.message || t('ai.requestFailed'))
  } catch (e) {
    ElMessage.error(e?.message || e?.toString?.() || t('ai.requestFailed'))
  }
  aiLoading.value = false
}

const askAiFix = async () => {
  aiFixLoading.value = true
  await askAi('fix')
  aiFixLoading.value = false
}

const onExport = async (cmd) => {
  const [scope, format] = String(cmd || '').split('-')
  if (!['csv', 'excel'].includes(format)) return
  const connId = selectedConnId.value || props.conn.id
  if (!connId) { ElMessage.warning(t('mv.pickConn')); return }
  const database = selectedSchema.value
    ? `${selectedDatabase.value}.${selectedSchema.value}`
    : (selectedDatabase.value || props.database || undefined)
  if (scope === 'all') {
    // 导出全部走异步任务，后端流式分页导出，前端实时显示进度与日志
    const payload = { sql: sql.value, format, database }
    await exportTask.start(connId, payload, props.scriptName || t('sqlq.queryResult'), format)
    return
  }
  // 当前页：走**同步单页接口**（后端只查这一页就回字节）。
  //
  // 不能走 exportTask.start —— 它默认提交到 /export/task（异步导出【全部】），
  // payload 里的 page/size 会被后端忽略，于是「导出当前页」变成「导出全表」。
  const payload = {
    sql: sql.value,
    format,
    database,
    page: Math.max(1, currentPage.value),
    size: Math.max(1, pageSize.value)
  }
  try {
    const blob = await exportData(connId, payload)
    const saved = await saveExportBlob(blob, props.scriptName || t('sqlq.queryResult'), format)
    if (saved.canceled) return
    ElMessage.success(t('mv.exportSavedAs', { name: saved.name }))
  } catch (e) {
    ElMessage.error(t('mv.exportFailed', { detail: errMsg(e) }))
  }
}

const copySql = async (text) => {
  try { await navigator.clipboard.writeText(text); ElMessage.success(t('common.copied')) } catch (e) { ElMessage.error(t('sqlq.copyFailed')) }
}

const replaceAndRun = (text) => {
  sql.value = text
  aiDialogVisible.value = false
  runSql()
}

const onInsert = (e) => {
  sql.value = e.detail
  // 注入已保存的脚本内容时以注入值为基线，避免误触发 dirty/自动保存
  lastSavedSql.value = e.detail
  // Monaco 编辑器可能尚未初始化完成，直接调用 setValue 确保内容写入
  if (editorInstance) {
    editorInstance.setValue(e.detail)
  }
}

/** 「插入并执行」：AI 结果里点代码块右上角的插入按钮时，写入后直接跑 */
const onRunSql = (e) => {
  onInsert(e)
  nextTick(() => runSql())
}

// 加载所有连接
const loadAllConnections = async () => {
  try {
    allConnections.value = await listConnections()
  } catch (e) {
    allConnections.value = []
  }
}

// 切换连接（仅改编辑器自身上下文，不触发全局连接切换，避免 tab 被清空）
const onConnChange = async (connId) => {
  selectedDatabase.value = ''
  databases.value = []
  dbCatalogs.value = []
  selectedCatalog.value = ''
  selectedDbShort.value = ''
  selectedSchema.value = ''
  schemas.value = []
  tableNames.value = []
  if (connId) {
    await loadDatabases()
  }
}

// 切换数据库（仅改编辑器自身上下文）
const onDbChange = (val) => {
  selectedDatabase.value = val || ''
  // 双下拉显示状态同步回全限定名（切 catalog 自动带第一个库就靠它回写裸名下拉；
  // 程序改 v-model 不会触发 el-select 的 @change，不会递归）
  syncCatalogUi()
  selectedSchema.value = ''
  schemas.value = []
  if (val && showSchemaSelect.value) {
    loadSchemas()
  }
  loadTableNames()
}

// 切换 Schema
const onSchemaChange = (val) => {
  selectedSchema.value = val || ''
}

// 数据库列表缓存（sessionStorage，60s）：切库/重开标签页时先用上次结果立刻渲染，
// 避免不可达连接（要等满 connectTimeout，往往 10s）时下拉一直转圈
const DB_LIST_TTL = 60 * 1000
const readDbListCache = (cid) => {
  try {
    const it = JSON.parse(sessionStorage.getItem('xplore.dblist.' + cid) || 'null')
    if (!it || !Array.isArray(it.list)) return null
    return {
      list: it.list,
      cats: Array.isArray(it.cats) ? it.cats : [],
      fresh: Date.now() - (it.ts || 0) < DB_LIST_TTL,
    }
  } catch { return null }
}
const writeDbListCache = (cid, list, cats) => {
  try { sessionStorage.setItem('xplore.dblist.' + cid, JSON.stringify({ ts: Date.now(), list, cats: cats || [] })) } catch { /* 忽略配额/隐私模式 */ }
}

/** 库清单：catalog 方言（Doris）按「catalog.库」全限定列出，与左侧树完全一致 ——
 *  只列默认 catalog 的裸名，右键生成的 internal.ods 在下拉里既显示不出也选不中
 *  （真机踩过：下拉全是 __internal_schema / ods 这种裸名）。其余类型原样返回裸名。 */
const loadDbList = async (cid) => {
  dbCatalogs.value = []
  try {
    const cats = await listCatalogs(cid)
    const names = Array.isArray(cats) ? cats : []
    // **用户建了自定义 catalog 才**出「catalog + 库」双下拉（全限定名，与树一致）。
    // 只有默认 internal 时不折腾 —— 单下拉裸库名，与"没建过 catalog"完全一样
    // （为一张 internal 名单多一个下拉，纯噪音）。两种形态并存于同一套页签逻辑。
    if (names.length > 1) {
      const all = []
      for (const c of names) {
        const dbs = await listDatabases(cid, c).catch(() => [])
        for (const d of (Array.isArray(dbs) ? dbs : [])) all.push(c + '.' + d)
      }
      if (all.length) { dbCatalogs.value = names; return all }
    }
  } catch { /* catalog 接口失败则回退默认清单 */ }
  return await listDatabases(cid)
}

const loadDatabases = async () => {
  const cid = selectedConnId.value || props.conn?.id
  if (!cid) { databases.value = []; selectedDatabase.value = ''; return }
  const cached = readDbListCache(cid)
  if (cached) {
    databases.value = cached.list               // 有缓存先渲染
    dbCatalogs.value = cached.cats              // catalog 拆分状态也要立刻就位
    syncCatalogUi()
  }
  loadingDbs.value = !cached                    // 只有真要等网络时才转圈
  try {
    if (!cached || !cached.fresh) {
      const list = isNoSql.value ? await noSqlDatabases(cid) : await loadDbList(cid)
      databases.value = list || []
      writeDbListCache(cid, databases.value, dbCatalogs.value)
    }
  } catch (e) {
    if (!cached) {
      databases.value = []
      const msg = e?.message || e?.toString?.() || t('common.unknownError')
      //「连接不存在或类型未知」对用户是句黑话，说白就是"这个连接已经不在连接列表里了"
      ElMessage.error(/连接不存在或类型未知/.test(String(msg))
        ? t('sqlq.connMissing')
        : '加载数据库失败：' + msg)
    }
  } finally {
    loadingDbs.value = false
  }
  if (!databases.value.length) return
  syncCatalogUi()
  if (!selectedDatabase.value || !databases.value.includes(selectedDatabase.value)) {
    // 页签带来的库可能**不在默认库名清单里**：Doris 是 catalog 方言，表节点带的库是
    // 全限定名 `internal.ods`，而清单来自默认 catalog 的 SHOW DATABASES（ods 这些裸名）。
    // 这种情况要**保留并补进清单** —— 后端按全限定名解析完全有效；兜底改选第一项
    // 会静默切到错误的库（真机踩过：选中了按字母序第一的 __internal_schema）。
    // 只有页签根本没带库时才选第一项。
    const want = selectedDatabase.value || props.database || ''
    if (want && !databases.value.includes(want)) databases.value.push(want)
    selectedDatabase.value = want || (databases.value[0] || '')
  }
  // schema 与表名互不依赖：并行加载。原来在这里串行 await loadSchemas()，
  // 导致“数据库下拉”要等 schema（新库首次连接 ~1s）才结束转圈。
  const tasks = [loadTableNames()]
  if (showSchemaSelect.value && selectedDatabase.value) tasks.push(loadSchemas())
  await Promise.all(tasks)
}

// 加载当前库表名（用于编辑器补全；NoSQL 无表概念，忽略）
const loadTableNames = async () => {
  const cid = selectedConnId.value || props.conn?.id
  const db = selectedDatabase.value
  if (isNoSql.value || !cid || !db) { tableNames.value = []; return }
  try {
    const list = await listTables(cid, db)
    // listTables 返回对象数组（{ name, type, ... }），这里统一提取成字符串，
    // 否则 Monaco 补全拿不到可渲染的表名文本
    tableNames.value = (Array.isArray(list) ? list : [])
      .map(t => (t && typeof t === 'object' ? t.name || t.table || '' : (t || '').toString()))
      .filter(Boolean)
  } catch (e) {
    tableNames.value = []
  }
}

const loadSchemas = async () => {
  const cid = selectedConnId.value || props.conn?.id
  const db = selectedDatabase.value
  if (!cid || !db) { schemas.value = []; selectedSchema.value = ''; return }
  loadingSchemas.value = true
  try {
    const list = await listSchemas(cid, db)
    schemas.value = list || []
    // 有默认 schema（如 SQL Server 的 dbo）则优先选中，否则选第一个
    const defSch = byType(connType.value).defaultSchema
    if (defSch) {
      selectedSchema.value = schemas.value.includes(defSch) ? defSch : (schemas.value[0] || '')
    } else {
      selectedSchema.value = schemas.value[0] || ''
    }
  } catch (e) {
    schemas.value = []
  }
  loadingSchemas.value = false
}

const rootRef = ref(null)

// 可自定义快捷键分发（键位在「设置 → 快捷键」中调整），
// 仅在当前视图可见且焦点在其中时响应；原 Monaco addCommand 硬绑定已移除
useShortcutScope(rootRef, {
  'query.run': () => runSql(),
  'query.runAll': () => runSqlAll(),
  'query.format': () => formatSql(),
  'query.save': () => onSaveClick(),
  'query.stop': () => stopSql(),
  // 选中片段类操作（键位可在「设置 → 快捷键」中改，右键菜单里的提示同步跟随）
  'query.upper': () => transformSelectedSql(t => mapOutsideQuotes(t, seg => seg.toUpperCase())),
  'query.lower': () => transformSelectedSql(t => mapOutsideQuotes(t, seg => seg.toLowerCase())),
  'query.oneLine': () => transformSelectedSql(t => mapOutsideQuotes(t, collapseSqlWs).trim()),
  'query.comment': () => {
    const a = editorInstance && editorInstance.getAction('editor.action.commentLine')
    if (a) a.run()
  },
  'query.copyInList': () => copySelectionAsInList(),
  'query.copyOneLine': () => {
    const t = selTextOf(editorInstance)
    if (t) writeClipboard(collapseSqlWs(t).trim(), t('sqlq.copyOneLineDone'))
  },
  'query.selectStatement': () => selectCurrentStatement(),
  // 结果表（只读）也能用的表格动作
  'data.autoFit': () => {
    if (selectedCols.value.size) autoFitSelectedResultCols()
    else resultVisibleCols.value.forEach(c => autoFitResultCol(c.idx))
  }
})
// 编辑器区高度占比的两个端点：拖动与双击**共用**这两个数字，免得两处各写一套。
// 这里**不设软限制**：允许一路贴到两端（0 = 编辑器完全收起 / 1 = 结果区完全收起）。
// 敢放开的前提是 grid 模板把那 6px 分隔条**排除在比例之外**
// （见 .sql-query-view.with-result 里的 calc(... * (100% - 6px))）——
// 否则比例取到 1 时分隔条会被顶出容器，用户再也抓不回来。
const EDITOR_RATIO_MIN = 0
const EDITOR_RATIO_MAX = 1
// 双击循环的状态：0 = 不在循环内，1 = 已到最上，2 = 已到最下
let ratioCycle = 0
let ratioBeforeCycle = 0.6      // 进入循环前的比例，第三次双击还原到这里
let ratioTouched = false        // 用户是否亲手调过比例（决定 runSql 还要不要套用"首次出结果 0.6"）
let dragState = null
const onResizerStart = (e) => {
  if (!rootRef.value) return
  const rect = rootRef.value.getBoundingClientRect()
  dragState = { startY: e.clientY, startRatio: editorRatio.value, totalH: rect.height }
  document.body.style.cursor = 'row-resize'
  document.body.style.userSelect = 'none'
  window.addEventListener('mousemove', onResizerMove)
  window.addEventListener('mouseup', onResizerEnd)
  e.preventDefault()
}
const onResizerMove = (e) => {
  if (!dragState) return
  const { startY, startRatio, totalH } = dragState
  if (totalH <= 0) return
  const dy = e.clientY - startY
  let ratio = startRatio + dy / totalH
  ratio = Math.min(EDITOR_RATIO_MAX, Math.max(EDITOR_RATIO_MIN, ratio))
  // 真的拖动了才退出双击循环（注意不能放在 mousedown 里：双击也会先来一次 mousedown，
  // 在那里重置会让循环永远停在第一步）
  ratioCycle = 0
  ratioTouched = true
  editorRatio.value = ratio
}

/**
 * 双击分隔条：在「结果最大化（贴最上）→ 编辑器最大化（贴最下）→ 还原」之间循环。
 *
 * 两端用的是**与拖动相同的钳制值**：编辑器至少要留得下工具栏与几行 SQL，
 * 结果区至少要留得下表头，所以"最上 / 最下"就是 0.15 / 0.9，而不是 0 / 1。
 *
 * 用一个小状态机，而不是"按当前比例就近取极值"——后者在 0.6 处双击会跳到最下，
 * 与"双击就是想让结果区铺满"的直觉正好相反。
 */
const onResizerDblClick = () => {
  ratioTouched = true
  if (ratioCycle === 0) {
    ratioBeforeCycle = editorRatio.value || 0.6
    editorRatio.value = EDITOR_RATIO_MIN
    ratioCycle = 1
    return
  }
  if (ratioCycle === 1) {
    editorRatio.value = EDITOR_RATIO_MAX
    ratioCycle = 2
    return
  }
  editorRatio.value = ratioBeforeCycle
  ratioCycle = 0
}
const onResizerEnd = () => {
  dragState = null
  document.body.style.cursor = ''
  document.body.style.userSelect = ''
  window.removeEventListener('mousemove', onResizerMove)
  window.removeEventListener('mouseup', onResizerEnd)
}

// 库标识可能是「库.schema」复合串：schema 层级类型（SQL Server / PostgreSQL / KingbaseES 等）
// 的树里，schema 节点及其下的对象节点都用这个复合串作为库标识。
// 「筛选器」按数据库类型决定是否拆出 schema 下拉；非 schema 层级类型的库名本身可能含点号，原样保留。
const splitDbSchema = (raw, type) => {
  const s = String(raw || '')
  if (schemaLevelOf(type) !== 'schema') return { db: s, schema: '' }
  const i = s.indexOf('.')
  if (i <= 0 || i === s.length - 1) return { db: s, schema: '' }
  return { db: s.slice(0, i), schema: s.slice(i + 1) }
}

// 目标连接的类型：优先查连接列表（onMounted 已先加载连接），兜底用当前 conn
const connTypeOfId = (connId) =>
  (allConnections.value.find(c => String(c.id) === String(connId))?.type) || connType.value

// 按上下文带出「连接 / 库 / Schema」筛选器：
// 右键对象「编辑」打开脚本、切换脚本页签时，筛选器自动对齐到该对象所属的连接与库
// （schema 层级类型同时对齐 schema，例如 SQL Server 的 dbo）
const applyContext = async (rawConnId, rawDb) => {
  const wanted = rawConnId ? String(rawConnId) : ''
  // 页签带来的连接 id 可能已经失效（首页「最近查询」里存的历史连接、刷新后还原的页签快照，
  // 它们引用的连接被删掉重建过就会换 id）：**列表非空却查不到**时不要拿它去请求 ——
  // 那会一路报「连接不存在或类型未知」，连接下拉还会显示一串原始 id。
  // 退回 props.conn（父级已用 connFor 兜过底），兜不到就留空，让用户自己选。
  const stale = !!wanted && allConnections.value.length > 0
    && !allConnections.value.some(c => String(c.id) === wanted)
  const targetConnId = stale ? String(props.conn?.id || '') : wanted
  if (stale) ElMessage.warning(t('sqlq.staleConn'))
  const { db, schema } = splitDbSchema(rawDb, connTypeOfId(targetConnId))

  if (targetConnId && targetConnId !== String(selectedConnId.value)) {
    selectedConnId.value = targetConnId
    // 预置目标库：loadDatabases 内部会保留仍在列表中的预置值（否则会兜底成列表首个库）
    if (db) { selectedDatabase.value = db; selectedSchema.value = ''; schemas.value = [] }
    await loadDatabases()
  }

  if (db && databases.value.includes(db) && db !== selectedDatabase.value) {
    selectedDatabase.value = db
    if (showSchemaSelect.value) { schemas.value = []; selectedSchema.value = '' }
    loadTableNames()
    if (showSchemaSelect.value) await loadSchemas()
  }

  if (schema && showSchemaSelect.value) {
    if (!schemas.value.length) await loadSchemas()
    if (schemas.value.includes(schema)) selectedSchema.value = schema
  }
}

// 初始化选择器：优先使用 tab 自带的上下文，其次用当前主界面状态
const initSelectors = async () => {
  await applyContext(props.tabConnId || props.conn?.id, props.tabDatabase || props.database)
}

onMounted(() => {
  mounted.value = true
  lastSavedSql.value = sql.value
  window.addEventListener('dc-insert-sql', onInsert)
  window.addEventListener('dc-run-sql', onRunSql)
  loadAllConnections().then(() => {
    initSelectors().then(() => {
      // 新建脚本并要求自动执行（AI 结果里点「插入并执行」）：等筛选器就绪后再跑
      if (props.autoRun && sql.value.trim()) nextTick(() => runSql())
    })
  })
  loadAiModels()
  window.addEventListener('keydown', onResultKeyDown)
  window.addEventListener('keydown', onEdCtxKeyDown, true)
  document.addEventListener('mousedown', onDocResCtxClose)
  // 编辑器配色跟随应用主题
  offEditorTheme = onResolvedThemeChange((r) => {
    editorTheme.value = monacoTheme()
  })
})

// 当 tab 的上下文变化时（新建查询后），重新初始化选择器
watch(() => props.tabConnId, (val) => {
  if (val) initSelectors()
})
watch(() => props.tabDatabase, (val) => {
  if (val) applyContext(props.tabConnId || props.conn?.id, val)
})

watch(() => props.conn?.id, (val) => {
  if (val && String(val) !== String(selectedConnId.value) && !props.tabConnId) {
    selectedConnId.value = String(val)
    loadDatabases()
  }
})
watch(() => props.database, (val) => {
  if (!val || props.tabDatabase) return
  const { db, schema } = splitDbSchema(val, connType.value)
  if (db && databases.value.includes(db) && !selectedDatabase.value) {
    selectedDatabase.value = db
    if (showSchemaSelect.value) {
      loadSchemas().then(() => {
        if (schema && schemas.value.includes(schema)) selectedSchema.value = schema
      })
    }
  }
})
onBeforeUnmount(() => {
  // 摘掉注册到全局 monaco.languages 的补全/格式化 provider（见 onEditorMount）
  monacoProviders.splice(0).forEach((d) => { try { d?.dispose?.() } catch { /* ignore */ } })
  if (queryTimer) { clearInterval(queryTimer); queryTimer = null }
  if (running.value) stopSql() // 卸载时中止进行中的查询并清理计时器
  // 事务模式还开着：回滚收尾（fire-and-forget），别留一个悬着的事务占着会话
  if (txMode.value) txControl(selectedConnId.value || props.conn.id, 'rollback', txDatabaseOf()).catch(() => {})
  if (autoSaveTimer) { clearTimeout(autoSaveTimer); autoSaveTimer = null }
  // sql-change 的防抖定时器也得摘：否则页签关掉后 600ms 内还会向父级 emit
  if (sqlChangeTimer) { clearTimeout(sqlChangeTimer); sqlChangeTimer = null }
  window.removeEventListener('dc-insert-sql', onInsert)
  window.removeEventListener('dc-run-sql', onRunSql)
  window.removeEventListener('keydown', onResultKeyDown)
  window.removeEventListener('keydown', onEdCtxKeyDown, true)
  document.removeEventListener('mousedown', onDocResCtxClose)
  document.removeEventListener('mousemove', onResultColDragMoveDoc)
  document.removeEventListener('mouseup', onResultColDragEndDoc)
  window.removeEventListener('mousemove', onResizerMove)
  window.removeEventListener('mouseup', onResizerEnd)
  document.body.style.cursor = ''
  document.body.style.userSelect = ''
  if (offEditorTheme) offEditorTheme()
})
</script>

<style scoped>
.sql-query-view { height: 100%; display: grid; grid-template-rows: 1fr; gap: 6px; padding: 10px; }
.dlg-title { display: flex; align-items: center; gap: 8px; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}
.sql-query-view.no-conn { padding: 0; }
.sql-no-conn { display: flex; align-items: center; justify-content: center; color: var(--dc-text-dim); font-size: 14px; height: 100%; }
/* 两段高度按 (100% - 5px) 分配：把分隔条那 5px 排除在比例之外。
   好处一：比例拉到 0 / 1 时（两端拉满）分隔条仍留在可视区内，不会"贴出去"再也抓不到；
   好处二：三行加起来正好 100%，不再多出那 5px 把底部顶出去。
   （5px 是产品要求的缝宽：与「修改表结构」页的视觉一致，比原来的 6px 再收 1px。） */
.sql-query-view.with-result {
  /* `gap: 0` 是**必须**的：外层 .sql-query-view 上有 `gap: 6px`，而 grid 的 gap 会在
     **每两行之间**都生效 —— 于是实际缝 = 6(gap) + 5(分隔条那行) + 6(gap) = 17px，
     比设计值宽出两倍多（这就是截图里那条偏宽的缝）。缝宽只由中间那一行决定。 */
  gap: 0;
  grid-template-rows:
    minmax(0, calc(var(--editor-ratio, 0.6) * (100% - 6px)))
    6px
    minmax(0, calc((1 - var(--editor-ratio, 0.6)) * (100% - 6px)));
}
.editor-area { min-height: 0; display: flex; flex-direction: column; background: var(--dc-bg-card); border: 1px solid var(--dc-border); border-radius: var(--dc-radius); overflow: hidden; box-shadow: var(--dc-shadow-sm); }
.editor-head { display: flex; justify-content: space-between; align-items: center; padding: 6px 10px; border-bottom: 1px solid var(--dc-border); gap: 10px; }
.editor-title { font-weight: 600; font-size: 14px; }

/* 选择器组 */
.selector-group { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
.conn-select { width: 180px; }
.db-catalog-select { width: 130px; }
.db-select { width: 160px; }
.schema-select { width: 140px; }

.sel-logo { margin-right: 4px; }
.sel-icon { font-size: 14px; color: var(--dc-primary); margin-right: 4px; }

/* 下拉选项样式 */
.opt-row { display: flex; align-items: center; gap: 8px; }
.opt-name { flex: 1; font-size: 14px; }
/* 分组标题：数据库类型名 */
.conn-select :deep(.el-select-group__title) { font-size: 12px; letter-spacing: .5px; }

/* 覆盖 el-select 前缀样式 */
.conn-select :deep(.el-input__prefix) { display: flex; align-items: center; margin-left: 6px; }
.db-select :deep(.el-input__prefix) { display: flex; align-items: center; margin-left: 6px; }
.schema-select :deep(.el-input__prefix) { display: flex; align-items: center; margin-left: 6px; }

.head-actions { display: flex; align-items: center; gap: 6px; }
/* 下拉触发按钮（模型 / AI 助手）：与同行 el-button（24px 高）同规格，图标用主色；
   弹层样式复用全局 .ai-model-dropdown（与 AI 面板的三个下拉完全一致） */
.model-btn,
.ai-act-btn {
  display: inline-flex; align-items: center; gap: 5px; flex-shrink: 0;
  height: 24px; padding: 0 8px; border-radius: 6px; cursor: pointer;
  border: 1px solid var(--dc-border); background: var(--dc-bg-card); color: var(--dc-text);
  font-size: 13px; font-family: inherit;
  transition: color .15s ease, border-color .15s ease, background .15s ease;
}
.model-btn:hover,
.ai-act-btn:hover { color: var(--dc-primary); border-color: var(--dc-primary); }
.model-btn-ic,
.ai-act-btn-ic { color: var(--dc-primary); font-size: 14px; }
.model-btn-tx { max-width: 96px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.model-btn-caret,
.ai-act-btn-caret { font-size: 13px; color: var(--dc-text-weak); }
.kbd { background: var(--dc-bg-soft); border: 1px solid var(--dc-border); border-radius: 4px; padding: 1px 5px; font-size: 11px; margin-left: 6px; }
.editor-body { flex: 1; min-height: 0; padding: 6px; position: relative; }
/* 编辑器底部状态栏 */
.editor-status {
  flex: 0 0 auto; display: flex; align-items: center; gap: 14px;
  padding: 4px 10px; border-top: 1px solid var(--dc-border);
  background: var(--dc-bg-soft); font-size: 11.5px; color: var(--dc-text-mid);
  white-space: nowrap; overflow: hidden;
}
.editor-status .st-item { flex: 0 0 auto; }
.editor-status .st-dim { color: var(--dc-text-dim); }
.editor-status .st-hl { color: var(--dc-primary); font-weight: 600; }
.editor-loading { height: 100%; display: flex; align-items: center; justify-content: center; color: var(--dc-text-dim); font-size: 13px; }
.result-area { min-height: 0; display: flex; flex-direction: column; background: var(--dc-bg-card); border: 1px solid var(--dc-border); border-radius: var(--dc-radius); overflow: hidden; box-shadow: var(--dc-shadow-sm); }
/* 与「修改表结构」页的 SQL 预览分隔条（TableDetailView 的 .sql-resizer）**完全同一套**：
   48×4 的细条、平时 45% 透明（很淡，不抢视觉）、悬停才提亮成主色。
   之前这里少了 opacity、条更粗（5px），容器还带了 `margin: -2px 0` —— 于是它比对面那条
   实、比对面那条粗，还会浮到相邻面板上，看着像"压着内容"。 */
.resizer { height: 6px; cursor: row-resize; flex: 0 0 6px; position: relative; z-index: 2; }
.resizer::before {
  content: ''; position: absolute; left: 50%; top: 50%;
  transform: translate(-50%, -50%);
  width: 48px; height: 4px; border-radius: 3px;
  background: var(--dc-text-dim); opacity: .45;
  transition: background .15s ease, opacity .15s ease;
}
.resizer:hover::before { background: var(--dc-primary); opacity: 1; }
.result-head { display: flex; align-items: center; gap: 10px; padding: 8px 12px; border-bottom: 1px solid var(--dc-border); }
.result-title { font-weight: 600; font-size: 14px; }
.result-tabs { display: flex; align-items: center; gap: 6px; padding: 6px 12px; border-bottom: 1px solid var(--dc-border); background: var(--dc-bg-soft); flex-shrink: 0; overflow-x: auto; }
.result-tab { display: inline-flex; align-items: center; gap: 6px; padding: 4px 14px; font-size: 13px; line-height: 1; border: 1px solid var(--dc-border); border-radius: 999px; background: var(--dc-bg); color: var(--dc-text-dim); cursor: pointer; transition: all .15s; white-space: nowrap; }
.result-tab:hover { color: var(--dc-primary); border-color: var(--dc-primary-soft); background: var(--dc-primary-soft); }
.result-tab.active { color: var(--dc-primary); border-color: var(--dc-primary); background: var(--dc-primary-soft); font-weight: 600; }
.result-tab-dot { width: 6px; height: 6px; border-radius: 50%; flex-shrink: 0; }
.result-tab-dot.ok { background: var(--dc-success); }
.result-tab-dot.err { background: var(--dc-danger); }
.result-tab-ms { font-size: 11px; color: var(--dc-text-dim); font-weight: 400; }
.result-tab.active .result-tab-ms { color: var(--dc-primary); opacity: .8; }
.flex-spacer { flex: 1; }
.loading-text { display: flex; align-items: center; gap: 6px; color: var(--dc-primary); font-size: 13px; }
.result-grid { flex: 1; min-height: 0; position: relative; overflow: hidden; display: flex; flex-direction: column; }
.result-grid .el-empty { flex: 1; display: flex; flex-direction: column; justify-content: center; }
/* 加载遮罩（与数据表一致） */
.grid-loading-overlay {
  position: absolute; inset: 0; z-index: 20;
  display: flex; align-items: center; justify-content: center;
  background: var(--dc-bg-card);
  background: color-mix(in srgb, var(--dc-bg-card) 78%, transparent);
}
.grid-loading-box {
  display: flex; align-items: center; gap: 10px;
  padding: 12px 18px; border-radius: 10px;
  background: var(--dc-bg-card); border: 1px solid var(--dc-border);
  box-shadow: var(--dc-shadow-sm, 0 2px 12px rgba(0, 0, 0, 0.08));
  color: var(--dc-primary);
}
.grid-loading-text { font-size: 14px; color: var(--dc-text-mid); }
.data-table-wrap { flex: 1; overflow: auto; contain: layout paint; }
.result-chart-wrap { flex: 1; min-height: 0; display: flex; flex-direction: column; }
.data-table .fill-col { padding: 0; border: none; background: transparent !important; min-width: 1px; }
.data-table-wrap.col-resizing, .data-table-wrap.col-resizing * { cursor: col-resize !important; user-select: none; }
/* 底栏布局与数据表一致：耗时 + 选中统计**同在左侧**，分页器 margin-left:auto 靠右
   （以前 space-between 把统计挤到正中间，两边看起来不是一个产品） */
.result-footer { display: flex; align-items: center; padding: 6px 12px; border-top: 1px solid var(--dc-border); background: var(--dc-bg-soft); flex-shrink: 0; gap: 12px; }
.result-footer > .el-pagination { margin-left: auto; }
.result-time { font-size: 13px; color: var(--dc-text-dim); font-weight: 500; }
/* 总数统计中：紧贴分页器左侧，弱化显示（计数是后台补的，别抢注意力） */
.count-pending {
  margin-left: auto; display: inline-flex; align-items: center; gap: 4px;
  font-size: 12px; color: var(--dc-text-dim);
}
.count-pending + .el-pagination { margin-left: 0; }
/* 可点击的总数：悬停变主色 + 提示「点我重新统计」，点击强制重数一次。
   margin-left:auto 把自己推到右侧、紧贴分页器（分页器遇它就让出 auto 边距） */
.total-refresh {
  margin-left: auto; display: inline-flex; align-items: center;
  font-size: 13px; color: var(--dc-text-dim); cursor: pointer;
  user-select: none; border-radius: 4px; padding: 0 4px;
  transition: color .12s, background .12s;
}
.total-refresh:hover { color: var(--dc-primary); background: var(--dc-bg-hover); }
.total-refresh + .el-pagination { margin-left: 0; }
/* 选中区汇总：夹在耗时与分页之间，弱化显示、数字加粗，避免抢分页的注意力 */
.result-summary { display: inline-flex; align-items: center; gap: 10px; font-size: 12px; color: var(--dc-text-dim); flex-wrap: wrap; }
.result-summary .rs-item { white-space: nowrap; }
.result-summary .rs-item b { color: var(--dc-text); font-weight: 600; font-variant-numeric: tabular-nums; }
/* 表格宽度 = 首列 + 各列宽度之和，由模板上的内联 width（resultTableWidth）给出，不再 width:100%：
   —— 字段少时不会被拉伸去"撑满"容器（序号列也稳定在 40px）
   —— 字段多时自然超出容器，由 .data-table-wrap 横向滚动
   注意：这里不能写 width: max-content —— 固定布局下浏览器按「内容」计算 max-content，
   多出的空间会摊回各列，列宽就永远拖不窄（详见 resultTableWidth 的注释） */
/* 字号来自设置（编辑器页签的「结果表格字号」），变量挂在 .data-table-wrap 上 */
.data-table { position: relative; table-layout: fixed; border-collapse: collapse; font-size: var(--grid-fs, 13px); }
/* 吸顶表头：`top: -1px` 盖住滚动时表头上方那道 1px 的缝 ——
   border-collapse: collapse 下上边框属于 table，不跟着吸顶走，那 1px 会露出下层底色。 */
.data-table thead { position: sticky; top: -1px; z-index: 2; }
/* 表头：保留竖向分隔线（用户口径）；右侧多留 26px 给绝对定位的排序图标，
   字段名省略号在图标前收住（窄列不重叠）。有注释时表头两行，高度交给内容 */
.data-table th { position: relative; background: var(--dc-bg-table-head); color: var(--dc-text-strong); font-weight: 600; text-align: left; padding: 5px 26px 5px 10px; height: auto; line-height: 1.3; vertical-align: middle; border: 1px solid var(--dc-border); white-space: nowrap; overflow: hidden; }
/* 表头文字块：竖排两行 —— 第一行「类型图标 + 字段名」，第二行注释顶格 */
.data-table th .th-text {
  display: inline-flex; flex-direction: column; justify-content: center;
  vertical-align: middle; overflow: hidden; min-width: 0; max-width: 100%;
}
.data-table th .th-line1 { display: flex; align-items: center; min-width: 0; }
.data-table th .th-label { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
.data-table th .th-comment {
  font-size: 11px; font-weight: 400; color: var(--dc-text-dim);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;
}
.data-table td { padding: 0 10px; height: 32px; line-height: 32px; border: 1px solid var(--dc-border); color: var(--dc-text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
/* 表头字段类型图标：裸图标（无底色/无固定盒子，与表预览/NoSQL 统一）；
   vertical-align: middle 让图标与字段名垂直居中在同一条线上。
   颜色由全局 th-t-* 按类型族给，这里不写 color（scoped 优先级会压掉全局配色） */
.data-table th .th-type-ic { display: inline-flex; align-items: center; flex: 0 0 auto; vertical-align: middle; margin-right: 5px; cursor: default; }
.data-table th .th-type-ic .el-icon { font-size: 12px; }
/* 表头排序按钮：固定在列头右缘垂直居中（右侧 26px 已预留，与表预览/NoSQL 同款） */
.data-table th .th-sort {
  position: absolute; right: 8px; top: 50%; transform: translateY(-50%);
  display: inline-flex; align-items: center;
  font-size: 13px; color: var(--dc-text-dim); z-index: 3;
  opacity: .35; cursor: pointer; transition: opacity .12s, color .12s;
}
.data-table th:hover .th-sort { opacity: .9; }
.data-table th .th-sort:hover { opacity: 1; color: var(--dc-primary); }
.data-table th .th-sort.is-sorted { opacity: 1; color: var(--dc-primary); }
/* 已排序列：文字主色 + 底部 2px 主色条（与表预览/NoSQL 同款） */
.data-table th.sort-asc, .data-table th.sort-desc { color: var(--dc-primary); box-shadow: inset 0 -2px 0 var(--dc-primary); }
.data-table tbody tr.vt-gap td { padding: 0; height: auto; line-height: 0; border: none; background: transparent !important; font-size: 0; }
/* 斑马纹：一律用 background-color —— background 简写会把行/列选中的外沿渐变线（background-image）清掉 */
.data-table tbody tr:nth-child(even) td { background-color: var(--dc-bg-soft); }
/* ===== 行 / 列选中：与单元格框选同一套观感（整块淡色填充 + 沿整块外沿画 2px 主色边线）
   边线只画在整块的外沿：连续选中的行段/列段由 selEdges 标出首尾两端，中间不画内部线 ===== */
.data-table tbody tr.selected td,
.data-table thead tr.selected th,
.data-table tbody tr td.col-selected,
.data-table th.col-selected {
  --sel-t: 0px; --sel-b: 0px; --sel-l: 0px; --sel-r: 0px;
  background-image:
    linear-gradient(var(--dc-primary), var(--dc-primary)),
    linear-gradient(var(--dc-primary), var(--dc-primary)),
    linear-gradient(var(--dc-primary), var(--dc-primary)),
    linear-gradient(var(--dc-primary), var(--dc-primary));
  background-position: top, bottom, left, right;
  background-size: 100% var(--sel-t), 100% var(--sel-b), var(--sel-l) 100%, var(--sel-r) 100%;
  background-repeat: no-repeat;
}
/* 行选中（含标题行）：整行淡色底，整块四周一个框 */
.data-table tbody tr.selected td,
.data-table thead tr.selected th { background-color: var(--dc-primary-soft) !important; }
.data-table tbody tr.selected.row-sel-top td,
.data-table thead tr.selected.row-sel-top th { --sel-t: 2px; }
.data-table tbody tr.selected.row-sel-bottom td,
.data-table thead tr.selected.row-sel-bottom th { --sel-b: 2px; }
.data-table tbody tr.selected td:first-child,
.data-table thead tr.selected th:first-child { --sel-l: 2px; }
.data-table tbody tr.selected td:last-child,
.data-table thead tr.selected th:last-child { --sel-r: 2px; }
/* 列选中：表头与该列单元格同底色，表头就是这块的顶边，到「最后一行」收底边 */
.data-table tbody tr td.col-selected { background-color: var(--dc-primary-soft); }
.data-table th.col-selected { --sel-t: 2px; background-color: var(--dc-primary-soft); }
/* 选中整列：四边都收口成完整矩形（与表预览/NoSQL 同一套框线语言，用户口径） */
.data-table th.col-selected.col-sel-l, .data-table td.col-selected.col-sel-l { --sel-l: 2px; }
.data-table th.col-selected.col-sel-r, .data-table td.col-selected.col-sel-r { --sel-r: 2px; }
.data-table tr.col-sel-bottom td.col-selected { --sel-b: 2px; }
.data-table tbody tr:hover td { background-color: var(--dc-primary-wash); }
/* NULL：灰 + 斜体，明显区别于真实数据。
   对齐不在这里定死 —— 跟着字段类型走（见 utils/cellAlign.js 的 alignByType），
   所以下面的 .al-r / .al-c 必须写在本条之后，靠后者压住它 */
.data-table td.null-cell {
  color: var(--dc-text-weak, var(--dc-text-dim));
  font-style: italic;
}
/* Excel 对齐规则：数字 / 日期右对齐、布尔居中（判定见 utils/cellAlign.js）；
   NULL 单元格按字段类型拿到同样的类，于是和同列的真实值对齐一致 */
.data-table td.al-r { text-align: right; }
.data-table td.al-c { text-align: center; }
/* 结果表首列复选框列：提高选择器优先级覆盖 .data-table th/td 的 padding 与 line-height，
   否则复选框会停在 34px 行高的基线上，看起来偏上（没居中） */
.row-sel-col { width: 40px; }
.data-table th.row-sel-th, .data-table td.row-sel-td {
  width: 40px; min-width: 40px; max-width: 40px;
  padding: 0; text-align: center; vertical-align: middle; line-height: 1;
}
.data-table th.row-sel-th { background-color: var(--dc-bg-table-head); cursor: pointer; }
.data-table th.row-sel-th :deep(.el-checkbox), .data-table td.row-sel-td :deep(.el-checkbox) { margin-right: 0; }
.lead-check { height: 18px; display: inline-flex; }
/* 行号列（Excel 行头）：显示序号，点/拖选行；选中行时行号一起高亮
   注意：这里必须用 background-color，不能用 background 简写 ——
   简写会把「行/列选中」那套外沿渐变线（background-image）一并清掉，
   表现就是序号列缺边线、看着没被选中 */
.data-table td.row-sel-td {
  cursor: pointer; user-select: none;
  color: var(--dc-text-dim); font-size: 12px;
}
.data-table td.row-sel-td:hover { background-color: var(--dc-primary-wash); color: var(--dc-text); }
.data-table td.row-sel-td.row-num-on {
  background-color: var(--dc-primary-soft);
  color: var(--dc-primary); font-weight: 600;
}
.row-num-tx { display: inline-block; }
/* 拖拽列排序反馈 */
.data-table th.col-drag-over { box-shadow: inset 2px 0 0 var(--dc-primary); background: var(--dc-primary-wash); }
/* 选中的列（Ctrl/Cmd 点表头加选、Shift 连选）：表头高亮 + 底部主色条 */
/* 选中的列：底色与外沿边线见上方「行 / 列选中」样式块 */
.data-table th.col-dragging { opacity: 0.5; }
/* 行选中的填充与边线见上方「行 / 列选中」样式块（这里不能再写 background 简写，会清掉外沿线） */
.data-table td.active-cell { outline: 2px solid var(--dc-primary); outline-offset: -2px; }
.data-table-wrap:focus { outline: none; }
.error-block { margin: 14px; padding: 14px 16px; background: var(--dc-danger-wash); border: 1px solid var(--dc-danger); border-radius: 8px; color: var(--dc-danger); font-size: 13px; line-height: 1.7; }
.error-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-bottom: 10px; }
.error-head-left { display: flex; align-items: center; gap: 8px; }
.error-icon { color: var(--dc-danger); font-size: 18px; }
.error-title { font-weight: 600; color: var(--dc-danger); font-size: 14px; }
.error-time { color: var(--dc-text-dim); font-size: 13px; margin-left: auto; }
.error-message { font-family: "SF Mono", Consolas, monospace; white-space: pre-wrap; word-break: break-all; background: var(--dc-bg-soft); padding: 12px 14px; border-radius: 6px; color: var(--dc-danger); font-size: 14px; line-height: 1.8; max-height: 420px; overflow: auto; }
.has-more-alert { margin: 6px 10px 0; flex-shrink: 0; }
.ai-dialog-body { min-height: 200px; max-height: 440px; overflow: auto; }
.ai-loading { display: flex; flex-direction: column; align-items: center; gap: 10px; padding: 40px 0; color: var(--dc-text-dim); }
.ai-error-reason { background: var(--dc-warning-wash); border: 1px solid var(--dc-warning); border-radius: 6px; padding: 10px 12px; margin-bottom: 12px; font-size: 14px; color: var(--dc-warning); line-height: 1.6; }
.ai-reason-label { font-weight: 600; }
/* 结论统一走 .ai-markdown（全局样式），这里只补对话框内的滚动留白 */
.ai-dialog-body .ai-markdown { font-size: 14px; }

/* 脚本列表 */
.script-list { max-height: 320px; overflow: auto; }
.script-item { display: flex; align-items: center; gap: 10px; padding: 10px 12px; border-radius: 6px; cursor: pointer; transition: background .15s; }
.script-item:hover { background: var(--dc-bg-hover); }
.script-name { flex: 1; font-size: 14px; color: var(--dc-text); }
.script-time { font-size: 12px; color: var(--dc-text-dim); }

/* SQL 执行历史下拉 */
.hist-dropdown .el-dropdown-menu { padding: 0; }
.hist-head { display: flex; align-items: center; justify-content: space-between; padding: 8px 12px; border-bottom: 1px solid var(--dc-border); font-size: 14px; font-weight: 600; }
.hist-list { max-height: 62vh; overflow: auto; min-width: 480px; max-width: 720px; }
.hist-item { padding: 8px 12px; border-bottom: 1px solid var(--dc-border); cursor: pointer; }
.hist-item:hover { background: var(--dc-hover); }
.hist-meta { display: flex; gap: 10px; align-items: center; font-size: 12px; color: var(--dc-text-dim); margin-bottom: 4px; }
.hist-db { color: var(--dc-primary); font-weight: 600; }
.hist-sql { font-family: monospace; font-size: 13px; color: var(--dc-text); display: -webkit-box; -webkit-line-clamp: 8; -webkit-box-orient: vertical; overflow: hidden; white-space: pre-wrap; word-break: break-all; }
.hist-ops { margin-top: 4px; text-align: right; }
.hist-empty { padding: 20px; text-align: center; color: var(--dc-text-dim); font-size: 14px; }
</style>

<style>
/* 结果表右键菜单（teleport 到 body，需全局样式） */
.grid-ctx-menu {
  position: fixed; z-index: 3000; min-width: 168px;
  max-height: calc(100vh - 16px); overflow: auto;
  background: var(--dc-bg-card, #fff);
  border: 1px solid var(--dc-border, #dcdfe6);
  border-radius: 8px; padding: 4px;
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.16);
  font-size: 13px; color: var(--dc-text, #303133);
  user-select: none;
}
.grid-ctx-menu .ctx-item { display: flex; align-items: center; gap: 10px; padding: 6px 10px; border-radius: 5px; cursor: pointer; white-space: nowrap; }
.grid-ctx-menu .ctx-item:hover,
.grid-ctx-menu .ctx-item.ctx-active { background: var(--dc-primary-wash, #ecf5ff); }
.grid-ctx-menu .ctx-item.ctx-disabled { opacity: 0.45; cursor: not-allowed; }
.grid-ctx-menu .ctx-item.ctx-disabled:hover { background: transparent; }
.grid-ctx-menu .ctx-item.ctx-sep { height: 1px; padding: 0; margin: 4px 6px; background: var(--dc-border, #dcdfe6); cursor: default; }
.grid-ctx-menu .ctx-item.ctx-sep:hover { background: var(--dc-border, #dcdfe6); }
.grid-ctx-menu .ctx-label { flex: 1 1 auto; }
.grid-ctx-menu .ctx-shortcut { flex: 0 0 auto; font-size: 12px; color: var(--dc-text-dim, #909399); }
.grid-ctx-menu .ctx-arrow { flex: 0 0 auto; color: var(--dc-text-dim, #909399); }
.grid-ctx-menu.grid-ctx-sub { z-index: 3100; }


/* 「选择显示字段」下拉（popper 挂到 body，需全局样式，与数据表一致） */
.col-vis-dropdown .col-vis { max-height: 340px; overflow: auto; padding: 4px; min-width: 190px; }
.col-vis-dropdown .col-vis-head {
  display: flex; align-items: center; justify-content: space-between;
  padding: 2px 8px 8px; font-size: 13px; font-weight: 600; color: var(--dc-text-mid, #606266);
  border-bottom: 1px solid var(--dc-border, #ebeef5); margin-bottom: 4px;
}
.col-vis-dropdown .col-vis-item {
  display: flex; width: 100%; margin: 0; padding: 4px 8px; box-sizing: border-box;
  border-radius: 4px; height: auto;
}
.col-vis-dropdown .col-vis-item:hover { background: var(--dc-bg-hover, #f5f7fa); }
.col-vis-dropdown .col-vis-item .el-checkbox__label {
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px;
}
</style>

<!-- 报错语句标红 + 语句面包屑：decorations/chips 落在 Monaco 内部 DOM 与全局层，须用非 scoped 样式 -->
<style>
/* 报错语句：首行 glyph 红点（悬停显示完整报错）+ 出错标识符红波浪线 */
.stmt-err-glyph {
  background: radial-gradient(circle, #e34d4d 0 45%, transparent 50%);
  cursor: pointer;
}
.stmt-err-token {
  text-decoration: underline wavy #e34d4d;
  text-underline-offset: 3px;
}
.stmt-bar {
  display: flex; align-items: center; gap: 4px; flex-wrap: wrap;
  padding: 3px 8px; flex-shrink: 0;
  border-bottom: 1px solid var(--dc-border);
  background: var(--dc-bg-soft);
  overflow-x: auto;
}
.stmt-bar-label { font-size: 12px; color: var(--dc-text-dim); margin-right: 2px; white-space: nowrap; }
.stmt-chip {
  font-size: 12px; line-height: 1; padding: 4px 8px;
  border: 1px solid var(--dc-border); border-radius: 10px;
  background: var(--dc-bg-card); color: var(--dc-text-mid);
  cursor: pointer; white-space: nowrap;
  transition: color .12s, border-color .12s;
}
.stmt-chip:hover { color: var(--dc-primary); border-color: var(--dc-primary); }
/* 面包屑 ▶：悬停该 chip 时才出现，点击只执行那一条 */
.stmt-run {
  display: none; align-items: center; justify-content: center;
  margin-left: 2px; width: 14px; height: 14px; border-radius: 50%;
  background: var(--dc-primary); color: #fff; vertical-align: -2px;
}
.stmt-chip:hover .stmt-run { display: inline-flex; }
.stmt-run:hover { opacity: .8; }
/* 模板变量填参弹窗 */
.var-row { display: flex; align-items: center; gap: 10px; margin-bottom: 10px; }
.var-name { font-family: monospace; font-size: 13px; color: var(--dc-primary); width: 110px; text-align: right; flex-shrink: 0; }
.var-hint { font-size: 12px; color: var(--dc-text-dim); line-height: 1.6; }
/* 事务模式：未提交状态点（绿=干净、橙=有未提交写） */
.tx-dot { width: 8px; height: 8px; border-radius: 50%; background: var(--el-color-success); flex-shrink: 0; }
.tx-dot.dirty { background: var(--el-color-warning); box-shadow: 0 0 6px var(--el-color-warning); }
/* 单元格编辑缓冲条：贴在底栏上方，弱底色提醒还有改动没落地 */
.edit-bar { display: flex; align-items: center; gap: 10px; padding: 4px 12px; border-top: 1px solid var(--dc-border); background: var(--el-color-warning-light-9); flex-shrink: 0; }
.edit-bar-text { font-size: 12px; color: var(--dc-text); }
</style>
