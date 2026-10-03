<template>
  <div v-loading="loading" class="table-edit-page">
    <!-- 顶部标题栏 -->
    <div class="form-tab-header">
      <div class="header-title">
        <el-icon :size="16" class="header-icon"><Grid /></el-icon>
        <span class="header-text">{{ $t('tdet.editStructure') }} · {{ table }}</span>
        <el-tag size="small" effect="plain" type="info">{{ database }}</el-tag>
      </div>
      <!-- 操作按钮统一挪到下方 SQL 预览栏（与「保存修改」同一行）：
           它们本就是围绕 SQL 预览的动作，摆在标题栏右侧离预览最远。这里只留 DDL 模式的提示。 -->
      <div class="header-actions">
        <el-tag v-if="ddlMode" size="small" effect="plain" type="warning">{{ $t('tdet.viewingRawDdl') }}</el-tag>
      </div>
    </div>

    <!-- 主体：左编辑 + 右预览 -->
    <div class="form-tab-body">
      <!-- 左侧编辑区 -->
      <div class="form-area">
        <!-- 参考 Navicat：页签是纯文字（激活项加粗加深），不用边框卡片 -->
        <el-tabs v-model="tab" class="table-tabs">
          <!-- 基本信息（表选项回显，改动自动进入 SQL 预览） -->
          <el-tab-pane :label="$t('tdet.tabBasic')" name="basic">
            <div class="tab-inner basic-tab">
              <el-alert v-if="isCH" type="info" :closable="false" show-icon class="ted-alert">
                <template #title>{{ $t('tdet.chTip') }}</template>
              </el-alert>
              <el-alert v-if="isDoris" type="info" :closable="false" show-icon class="ted-alert">
                <template #title>{{ $t('tdet.dorisTip') }}</template>
              </el-alert>
              <!-- 与「字段定义」同一张表：同一张卡 + 表头浅底 + 横竖网格 + 29px 紧凑行。
                   左列是「项目」、右列是值，控件与字段表同尺寸。 -->
              <div class="field-table-wrap">
                <table class="field-table basic-table">
                  <colgroup>
                    <col style="width:180px" />
                    <col />
                  </colgroup>
                  <thead>
                    <tr><th>{{ $t('tdet.item') }}</th><th>{{ $t('tdet.value') }}</th></tr>
                  </thead>
                  <tbody>
                    <tr>
                      <td class="basic-label">{{ $t('tdet.tableName') }}</td>
                      <td><el-input :model-value="table" size="small" disabled /></td>
                    </tr>
                    <!-- 表概况：只放**所有数据库都成立**的四项 ——
                         行数走真实 COUNT(*)（各驱动同一套接口，拿不到时显示 — 而不是 0）；
                         列数 / 索引数 / 主键取自已加载的结构，无需额外请求。 -->
                    <!-- 这四行原来是**纯文字**，与上下那些「禁用输入框」的行不是一套外观
                         （行高一高一低、文字起始位置也不同）。统一套上禁用输入框：
                         仍是只读，但整张表的行高与对齐完全一致。 -->
                    <tr>
                      <td class="basic-label">{{ $t('tdet.rowCount') }}</td>
                      <td><el-input :model-value="rowCountText" size="small" disabled /></td>
                    </tr>
                    <tr>
                      <td class="basic-label">{{ $t('tdet.colCount') }}</td>
                      <td><el-input :model-value="displayCols.length" size="small" disabled /></td>
                    </tr>
                    <tr>
                      <td class="basic-label">{{ $t('tdet.indexCount') }}</td>
                      <td><el-input :model-value="displayIdxRows.length" size="small" disabled /></td>
                    </tr>
                    <!-- 主键行：数据库没有主键概念时不显示（如 Doris，主键就是建表时的 Key 列，
                         这里恒为「—」，摆一行占位只会让人困惑） -->
                    <tr v-if="pkText && pkText !== '—'">
                      <td class="basic-label">{{ $t('tdet.primaryKey') }}</td>
                      <td><el-input :model-value="pkText" size="small" disabled /></td>
                    </tr>
                    <!-- ClickHouse 建表属性：引擎 / 排序键 / 分区键建表后不可修改，
                         只读展示当前值（数据来自 system.tables，见后端 table_options） -->
                    <tr v-if="isCH">
                      <td class="basic-label">{{ $t('tdet.engine') }}</td>
                      <td><el-input :model-value="tableForm.engine" size="small" disabled placeholder="—" /></td>
                    </tr>
                    <tr v-if="isCH">
                      <td class="basic-label">{{ $t('tdet.orderKey') }}</td>
                      <td><el-input :model-value="tableForm.sortingKey" size="small" disabled placeholder="—" /></td>
                    </tr>
                    <tr v-if="isCH">
                      <td class="basic-label">{{ $t('tdet.partitionKey') }}</td>
                      <td><el-input :model-value="tableForm.partitionKey" size="small" disabled placeholder="—" /></td>
                    </tr>
                    <!-- Doris 建表属性：建表后不可修改，只读展示当前值（输入框禁用） -->
                    <tr v-if="isDoris">
                      <td class="basic-label">{{ $t('tdet.dataModel') }}</td>
                      <td><el-input :model-value="tableForm.dorisModel" size="small" disabled placeholder="—" /></td>
                    </tr>
                    <tr v-if="isDoris">
                      <td class="basic-label">{{ $t('tdet.distributedBy') }}</td>
                      <td><el-input :model-value="tableForm.dorisDistCol" size="small" disabled placeholder="—" /></td>
                    </tr>
                    <tr v-if="isDoris">
                      <td class="basic-label">{{ $t('tdet.buckets') }}</td>
                      <td><el-input :model-value="tableForm.dorisBuckets" size="small" disabled placeholder="—" /></td>
                    </tr>
                    <tr v-if="isDoris">
                      <td class="basic-label">{{ $t('tdet.replicas') }}</td>
                      <td><el-input :model-value="tableForm.dorisReplication" size="small" disabled placeholder="—" /></td>
                    </tr>
                    <tr v-if="showCommentField">
                      <td class="basic-label">{{ $t('tdet.tableComment') }}</td>
                      <!-- ClickHouse（MODIFY COMMENT）与 Doris（MODIFY COMMENT）的表注释都可改；
                           引擎/字符集那些才是 Doris 真正动不了的。 -->
                      <td><el-input v-model="tableForm.comment" size="small" clearable :placeholder="$t('tdet.commentPlaceholder')" /></td>
                    </tr>
                    <tr v-if="showEngineField">
                      <td class="basic-label">{{ $t('tdet.engine') }}</td>
                      <td>
                        <!-- allow-create：清单是常用值，不在名单里的（新版本引擎/自建字符集）
                             照样能手输 —— 有下拉但不被下拉限制 -->
                        <el-select v-if="engines.length" v-model="tableForm.engine" size="small" :disabled="isCH" clearable filterable allow-create>
                          <el-option v-for="e in engines" :key="e" :label="e" :value="e" />
                        </el-select>
                        <!-- 后端暂未提供引擎清单时退化为文本输入：**能改**，好过整行消失 -->
                        <el-input v-else v-model="tableForm.engine" size="small" clearable :disabled="isCH" :placeholder="$t('tdet.enginePlaceholder')" />
                      </td>
                    </tr>
                    <tr v-if="showCharsetField">
                      <td class="basic-label">{{ $t('tdet.charset') }}</td>
                      <td>
                        <el-select v-if="charsets.length" v-model="tableForm.charset" size="small" clearable filterable allow-create @change="onCharsetChange">
                          <el-option v-for="c in charsets" :key="c" :label="c" :value="c" />
                        </el-select>
                        <el-input v-else v-model="tableForm.charset" size="small" clearable :placeholder="$t('tdet.charsetPlaceholder')" @change="onCharsetChange" />
                      </td>
                    </tr>
                    <tr v-if="showCharsetField">
                      <td class="basic-label">{{ $t('tdet.collation') }}</td>
                      <td>
                        <el-select v-if="collations.length" v-model="tableForm.collation" size="small" clearable filterable allow-create>
                          <el-option v-for="c in collations" :key="c" :label="c" :value="c" />
                        </el-select>
                        <el-input v-else v-model="tableForm.collation" size="small" clearable :placeholder="$t('tdet.collationPlaceholder')" />
                      </td>
                    </tr>
                    <tr v-if="showAutoIncField">
                      <td class="basic-label">{{ $t('tdet.autoIncrement') }}</td>
                      <td><el-input v-model="tableForm.autoIncrement" size="small" :placeholder="$t('tdet.autoIncPlaceholder')" /></td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </div>
          </el-tab-pane>

          <!-- 字段编辑 -->
          <el-tab-pane :label="$t('tdet.tabColumns', { n: fieldCount })" name="columns">
            
            <div class="tab-inner col-tab">
              <div class="card-actions">
                <div class="card-left">
                  <span class="sum-tags">
                    <el-tag v-if="colStat.added" size="small" type="success" effect="plain" round>{{ $t('tdet.statAdded', { n: colStat.added }) }}</el-tag>
                    <el-tag v-if="colStat.modified" size="small" type="warning" effect="plain" round>{{ $t('tdet.statModified', { n: colStat.modified }) }}</el-tag>
                    <el-tag v-if="colStat.deleted" size="small" type="danger" effect="plain" round>{{ $t('tdet.statDeleted', { n: colStat.deleted }) }}</el-tag>
                  </span>
                </div>
                <!-- 参考 Navicat：搜索框放在操作条右侧、紧挨新增按钮 -->
                <el-input v-model="colSearch" size="small" clearable :prefix-icon="Search"
                          class="col-search" :placeholder="$t('tdet.searchCols')" />
                <el-button size="small" type="primary" :icon="Plus" @click="addColRow">{{ $t('tdet.addColumn') }}</el-button>
              </div>
              <div class="field-table-wrap">
                <table class="field-table">
                  <!-- 列宽写在 colgroup 上：`table-layout: fixed` 只认**第一行**的列宽，
                       而第一行是那个 colspan 的空行（页签与工具栏住的地方），
                       于是所有列只能**均分** —— 序号列、删除列因此都被撑成 83px。
                       colgroup 是让"表头里写的宽度"真正生效的写法。 -->
                  <colgroup>
                    <!-- 列宽全部用**百分比**（按当前显示的列归一化到 100%，见 colWidths）：
                         写死像素时，容器比列宽之和大右侧就留白（真机反复出现）；百分比与容器
                         宽度无关，合计恒为 100% ⇒ 表格必然严丝合缝铺满。 -->
                    <col :style="{ width: colWidths.idx }" />
                    <col :style="{ width: colWidths.name }" />
                    <col :style="{ width: colWidths.type }" />
                    <col :style="{ width: colWidths.len }" />
                    <col :style="{ width: colWidths.prec }" />
                    <col :style="{ width: colWidths.nullable }" />
                    <col v-if="showPkCol" :style="{ width: colWidths.pk }" />
                    <col v-if="showOrderKey" :style="{ width: colWidths.sort }" />
                    <col :style="{ width: colWidths.def }" />
                    <col v-if="showAutoIncrement" :style="{ width: colWidths.auto }" />
                    <col v-if="showComment" :style="{ width: colWidths.comment }" />
                    <col :style="{ width: colWidths.ops }" />
                  </colgroup>
                  <thead>
                    <!-- 表格自己的"空行"：一行属于表格的空白（与表头同底色、随表头吸顶），
                         不是外部 padding —— 所以它的底色和宽度都与表格完全一致。 -->
                    <tr class="thead-spacer"><th :colspan="headSpan"></th></tr>
                    <tr>
                      <th style="width:40px">#</th>
                      <th>{{ $t('tdet.colName') }}</th>
                      <th style="width:120px">{{ $t('tdet.colType') }}</th>
                      <th style="width:64px">{{ $t('tdet.colLength') }}</th>
                      <th style="width:64px">{{ $t('tdet.colPrecision') }}</th>
                      <th class="c-center" style="width:76px">{{ $t('tdet.nullable') }}</th>
                      <th v-if="showPkCol" class="c-center" style="width:56px">{{ $t('tdet.primaryKey') }}</th>
                      <th v-if="showOrderKey" class="c-center" style="width:70px" :title="$t('tdet.sortKeyTip')">{{ $t('tdet.sortKeyCol') }}</th>
                      <th style="width:132px">{{ $t('tdet.defaultValue') }}</th>
                      <th v-if="showAutoIncrement" class="c-center" style="width:56px">{{ $t('tdet.autoIncCol') }}</th>
                      <th v-if="showComment">{{ $t('tdet.comment') }}</th>
                      <th class="c-center" style="width:56px">{{ $t('tdet.actions') }}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr v-for="(c, i) in visibleCols" :key="c._uid"
                        :class="{ 'r-new': c._state === 'new', 'r-mod': c._state === 'loaded' && colChanged(c) }">
                      <td><span class="row-idx">{{ i + 1 }}</span></td>
                      <td>
                        <el-input v-model="c.name" size="small" clearable
                                  :disabled="nameDisabled(c)"
                                  :placeholder="c._state === 'new' ? $t('tdet.newColName') : ''" />
                      </td>
                      <td>
                        <el-select v-model="c.typeBase" size="small" filterable allow-create
                                   :disabled="typeDisabled(c)" class="type-select">
                          <el-option v-for="t in typeOptions" :key="t" :label="t" :value="t" />
                        </el-select>
                      </td>
                      <td>
                        <el-input v-model="c.length" size="small" :disabled="lenDisabled(c)"
                                  :placeholder="lenPlaceholder(c)" :title="lenPlaceholder(c)" />
                      </td>
                      <td>
                        <el-input v-model="c.scale" size="small" :disabled="lenDisabled(c)"
                                  :placeholder="needsLength(c.typeBase) ? $t('tdet.colPrecision') : ''" :title="needsLength(c.typeBase) ? $t('tdet.scaleTip') : ''" />
                      </td>
                      <td class="c-center">
                        <!-- 主键列必然非空：勾选框在这里禁用，而不是等数据库把 ALTER 拒掉。
                             参考 Navicat：勾选框后跟「是/否」文字，比一个孤零零的方框好读得多。
                             列序也照参考：可为空在前、主键在后。 -->
                        <span class="bool-cell">
                          <span class="bool-text">{{ c.nullable ? $t('tdet.yes') : $t('tdet.no') }}</span>
                          <el-checkbox v-model="c.nullable" :disabled="inputDisabled(c) || !!c.primaryKey"
                                       :title="c.primaryKey ? $t('tdet.pkNotNull') : ''" />
                        </span>
                      </td>
                      <td v-if="showPkCol" class="c-center">
                        <el-checkbox v-if="canEditPkOf(c)" v-model="c.primaryKey" @change="onPkChange(c)" :title="$t('tdet.setPk')" />
                        <el-icon v-else-if="c.primaryKey" color="#f5b34d" :size="15"><StarFilled /></el-icon>
                        <span v-else class="pk-none">—</span>
                      </td>
                      <td v-if="showOrderKey" class="c-center">
                        <el-tag v-if="c.sortKey" size="small" type="success" effect="plain">{{ $t('tdet.yes') }}</el-tag>
                        <span v-else class="pk-none">—</span>
                      </td>
                      <td>
                        <!-- 始终渲染输入框，默认值为 NULL 时以占位提示；输入即覆盖（原实现只显示 NULL 标签，导致无法编辑） -->
                        <el-input v-model="c.defaultValue" size="small" :disabled="inputDisabled(c)"
                                  :placeholder="c.defaultIsNull ? 'NULL' : ''" @input="c.defaultIsNull = false" />
                      </td>
                      <td v-if="showAutoIncrement" class="c-center">
                        <el-checkbox v-if="canEditAutoOf(c)" v-model="c.autoIncrement" @change="onAutoChange(c)" :title="$t('tdet.autoIncTip')" />
                        <el-checkbox v-else :model-value="c.autoIncrement" disabled :title="autoTitle(c)" />
                      </td>
                      <td v-if="showComment">
                        <!-- 注释列只有 110px，长注释在格子里看不全：悬停该行时右侧浮出「展开」，
                             双击输入框也一样 —— 两者都打开多行大编辑框（见模板末尾的 el-dialog）。
                             按钮绝对定位，所以出现/消失都不挤动输入框。 -->
                        <div class="cmt-cell">
                          <el-input v-model="c.comment" size="small" :disabled="inputDisabled(c)"
                                    @dblclick="openComment(c)" />
                          <el-button v-if="!inputDisabled(c)" text size="small" :icon="EditPen"
                                     class="cmt-expand" :title="$t('tdet.zoomComment')"
                                     @click.stop="openComment(c)" />
                        </div>
                      </td>
                      <td class="c-center">
                        <el-button text size="small" :icon="Delete" class="dc-del" :title="c._state === 'new' ? $t('tdet.remove') : $t('tdet.deleteColumn')" @click="removeColRow(c)" />
                      </td>
                    </tr>
                    <tr v-if="!visibleCols.length">
                      <td :colspan="headSpan" class="empty-row">
                        {{ displayCols.length ? $t('tdet.noMatchingCol', { q: colSearch }) : $t('tdet.noColumns') }}
                      </td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </div>
          </el-tab-pane>

          <!-- 索引编辑 -->
          <el-tab-pane :label="$t('tdet.tabIndexes', { n: displayIdxRows.length })" name="indexes">
            <div class="tab-inner idx-tab">
              <div class="card-actions">
                <div class="card-left">
                  <span class="sum-tags">
                    <el-tag v-if="idxStat.added" size="small" type="success" effect="plain" round>{{ $t('tdet.statAdded', { n: idxStat.added }) }}</el-tag>
                    <el-tag v-if="idxStat.modified" size="small" type="warning" effect="plain" round>{{ $t('tdet.statModified', { n: idxStat.modified }) }}</el-tag>
                    <el-tag v-if="idxStat.deleted" size="small" type="danger" effect="plain" round>{{ $t('tdet.statDeleted', { n: idxStat.deleted }) }}</el-tag>
                  </span>

                </div>
                <el-button v-if="allowIndexEdit" size="small" type="primary" :icon="Plus" @click="addIdxRow">{{ $t('tdet.addIndex') }}</el-button>
              </div>
              <div class="idx-body">
                <!-- 空状态：不再用一张大空表格撑场面 —— 一行表头 + 一大片空白最难看。
                     没有索引时渲染一个居中的虚线卡片，把"怎么加"直接说清楚。 -->
                <div v-if="!displayIdxRows.length" class="idx-empty">
                  <el-icon :size="30" class="idx-empty-icon"><DocumentCopy /></el-icon>
                  <div class="idx-empty-title">{{ $t('tdet.noIndex') }}</div>
                  <div class="idx-empty-text">{{ $t('tdet.noIndexHint') }}</div>
                  <el-button v-if="allowIndexEdit" size="small" type="primary" :icon="Plus" @click="addIdxRow">{{ $t('tdet.addIndex') }}</el-button>
                </div>
                <el-table v-else :data="displayIdxRows" size="small" :row-class-name="idxRowClass" height="100%">
                  <el-table-column :label="$t('tdet.indexName')" min-width="140">
                    <template #default="{ row }"><el-input v-model="row.name" size="small" :placeholder="$t('tdet.indexName')" /></template>
                  </el-table-column>
                  <el-table-column :label="$t('tdet.unique')" width="64">
                    <template #default="{ row }"><el-switch v-model="row.unique" size="small" /></template>
                  </el-table-column>
                  <el-table-column v-if="indexTypes.length" :label="$t('tdet.colType')" width="130">
                    <template #default="{ row }">
                      <el-select v-model="row.indexType" size="small" clearable>
                        <el-option v-for="t in indexTypes" :key="t" :label="t" :value="t" />
                      </el-select>
                    </template>
                  </el-table-column>
                  <el-table-column :label="$t('tdet.indexCols')" min-width="200">
                    <template #default="{ row }">
                      <el-select v-model="row.columns" size="small" multiple filterable collapse-tags>
                        <el-option v-for="c in displayCols.filter(x => String(x.name || '').trim())" :key="c._uid" :label="c.name" :value="String(c.name).trim()" />
                      </el-select>
                    </template>
                  </el-table-column>
                  <el-table-column :label="$t('tdet.actions')" width="110">
                    <template #default="{ row }">
                      <div class="idx-ops">
                        <el-button v-if="has('supportsIndexRebuild') && row._state === 'loaded'" text size="small" :icon="Refresh" class="idx-rebuild" :title="$t('tdet.rebuildThisIndex')" @click="rebuildIndex(row)" />
                        <el-button text size="small" :icon="Delete" class="dc-del" :title="row._state === 'new' ? $t('tdet.remove') : $t('tdet.deleteIndex')" @click="removeIdxRow(row)" />
                      </div>
                    </template>
                  </el-table-column>
                </el-table>
              </div>
            </div>
          </el-tab-pane>
        </el-tabs>
      </div>

      <!-- 分隔条：**它就是两块卡片之间的那道缝**（6px；透明底 → 露出页面白底），
           细条上下居中 → 正好落在缝的中间。向上拖 = 预览变高，双击在最大/最小之间切换。
           位置很关键：放在 .form-area 与 .sql-area **之间**，不再放进 .sql-area 内部 ——
           原来它在卡片里铺了一条 6px 的 --dc-bg 灰带，于是细条看着贴在卡片顶边、
           "缝"出现在卡片内部，而不是上下两块卡片之间。 -->
      <div v-show="!sqlCollapsed" class="sql-resizer"
           :title="$t('tdet.resizerTitle')"
           @mousedown.prevent="onSqlResizeStart" @dblclick="onSqlResizeDblClick"></div>

      <!-- SQL 预览：在编辑区**下方**（原来是右侧栏），点标题行即可展开/收起。
           收起只藏 SQL 正文，**底部的「保存修改」始终留着** —— 不然收起后找不到保存入口。 -->
      <div ref="sqlAreaRef" class="sql-area" :class="{ collapsed: sqlCollapsed }"
           :style="sqlCollapsed ? null : { height: sqlHeight + 'px' }">
        <div class="sql-head" @click="sqlCollapsed = !sqlCollapsed"
             :title="sqlCollapsed ? $t('tdet.expandSql') : $t('tdet.collapseSql')">
          <el-icon class="sql-fold" :class="{ folded: sqlCollapsed }"><CaretRight /></el-icon>
          <el-icon><DocumentCopy /></el-icon>
          <span>{{ $t('tdet.sqlPreview') }}</span>
          <!-- 展开/收起跟在标题后（同一处切换），状态标签才推到最右 -->
          <span class="sql-fold-tx">{{ sqlCollapsed ? $t('tdet.expand') : $t('tdet.collapse') }}</span>
          <!-- 状态标签只在**有内容可说**时出现：无改动时正文已经写着
               「-- 暂无改动 / -- 调整字段/索引/表选项后…」，再挂一个「暂无修改」是重复。 -->
          <el-tag v-if="showStateTag" size="small" effect="plain" :type="previewTag.type"
                  class="sql-state">
            {{ previewTag.text }}
          </el-tag>
        </div>
        <div v-show="!sqlCollapsed" class="sql-body">
          <pre><code>{{ previewBody }}</code></pre>
        </div>
        <div class="sql-foot">
          <span class="sql-tip">{{ ddlMode ? $t('tdet.ddlTip') : $t('tdet.changeTip') }}</span>
          <!-- 原来在标题栏右侧的三个按钮：它们都是围绕 SQL 预览的动作，挪到预览这一行更顺手 -->
          <div class="sql-foot-actions">
            <el-button size="small" text :icon="Document" @click="showDdl">{{ ddlMode ? $t('tdet.backToPreview') : $t('tdet.rawDdl') }}</el-button>
            <el-button size="small" text :icon="CopyDocument" :disabled="!canCopy" @click="copySql">{{ $t('tdet.copySql') }}</el-button>
            <el-button size="small" text :icon="Refresh" @click="resetAll">{{ $t('common.reset') }}</el-button>
            <el-button type="primary" :icon="CaretRight" size="small" :loading="saving" :disabled="ddlMode || !canSave" @click="save">
              {{ $t('tdet.saveChanges') }}
            </el-button>
          </div>
        </div>
      </div>
    </div>
  </div>

  <!-- 注释放大编辑：点「确定」才写回该字段的 comment（与格子里的输入框同一个数据源，
       因此改完会照常进入下方 ALTER SQL 预览）。弹窗内容用内联样式，
       因为它被 teleport 到 body，组件里 scoped 的样式够不着。 -->
  <el-dialog v-model="commentDlg.visible" :title="$t('tdet.editComment')" width="520px" append-to-body>
    <div style="display:flex;align-items:center;gap:8px;margin-bottom:8px">
      <el-tag size="small" effect="plain" type="info">{{ commentDlg.field }}</el-tag>
      <span style="font-size:12px;color:var(--dc-text-dim)">{{ $t('tdet.commentMultiLine') }}</span>
      <!-- 自己数一下字数：Element 的 show-word-limit 必须配 maxlength 才显示，
           而注释长度因库而异（MySQL 列注释上限 1024，PG 无限制），不该硬塞一个上限。 -->
      <span style="margin-left:auto;font-size:12px;color:var(--dc-text-dim)">{{ $t('tdet.charCount', { n: commentDlg.text.length }) }}</span>
    </div>
    <el-input v-model="commentDlg.text" type="textarea" :autosize="{ minRows: 4, maxRows: 12 }"
              :placeholder="$t('tdet.commentFieldPh')" />
    <template #footer>
      <el-button size="small" @click="commentDlg.visible = false">{{ $t('common.cancel') }}</el-button>
      <el-button size="small" type="primary" @click="applyComment">{{ $t('common.confirm') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, reactive, computed, watch, onMounted, onBeforeUnmount } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { t } from '../../utils/i18n'
// 说明：本文件多个 DDL 生成函数把局部变量起名为 t（带引号的表名），会遮蔽翻译函数 t()；
// 这里另存一个引用 tr，专供那些函数内部使用（见 buildColumnSql 里的 sp_rename 注释）。
const tr = t
import { Plus, Delete, Refresh, CopyDocument, CaretRight, Grid, DocumentCopy, StarFilled, Document, EditPen, Search } from '@element-plus/icons-vue'
import { listColumns, listIndexes, listTables, alterTable, getFeatures, executeSql, getTableDdl, getTableCounts } from '../../api'

const props = defineProps({ conn: Object, database: String, table: String })

// ==================== 方言能力（features 由后端下发） ====================

const features = ref({})
const has = (k) => !!features.value[k]
const ddlStyle = computed(() => features.value.ddlStyle || 'generic')
const isCH = computed(() => ddlStyle.value === 'clickhouse')
const isDoris = computed(() => has('doris') || ddlStyle.value === 'doris')

const typeOptions = computed(() => features.value.columnTypes || [])
const indexTypes = computed(() => features.value.indexTypes || [])
const engines = computed(() => features.value.engines || [])
const charsets = computed(() => features.value.charsets || [])
const collationMap = computed(() => features.value.collations || {})
const collations = computed(() => collationMap.value[tableForm.value.charset] || [])

// 基本信息各字段是否展示 / 可编辑
// Doris 的表注释**可以改**（实测 2.x：`ALTER TABLE … MODIFY COMMENT '…'` 成功），
// 之前跟着「表选项只读」一起禁掉了 —— 表注释行不再显示，用户改不了。
const showCommentField = computed(() => has('supportsComment'))
/* 表选项三行（存储引擎 / 编码 / 排序规则）：只看**后端是否声明支持表选项**，
   不再要求"能力清单非空" —— 后端目前没给 MySQL 的 engines/charsets/collations 清单，
   而按清单非空来决定显隐，后果是**整行消失、用户改都改不了**。
   清单为空时模板会退化成文本输入（见各行的 v-if/v-else），照旧能改。 */
const showEngineField = computed(() => has('supportsTableOptions') && !isDoris.value)
const showCharsetField = computed(() => has('supportsTableOptions') && !isCH.value && !isDoris.value)
const showAutoIncField = computed(() => has('supportsTableOptions') && has('supportsAutoIncrement') && !isCH.value && !isDoris.value)

const loadFeatures = async () => {
  try { features.value = await getFeatures(props.conn.id) }
  catch (e) {
    console.warn('加载方言能力失败，使用通用默认：', e?.message || e)
    features.value = {}
  }
}

// ==================== 基础工具 ====================

// 标识符转义
const qt = (name) => {
  const s = String(name)
  const style = features.value.quoteStyle
  if (style === 'BACKTICK') return '`' + s.replace(/`/g, '``') + '`'
  if (style === 'BRACKET') return '[' + s.replace(/\]/g, ']]') + ']'
  return '"' + s.replace(/"/g, '""') + '"'
}
const sq = (v) => String(v).replace(/'/g, "''")

const tab = ref('basic')
const columns = ref([])          // 加载时的原始列（作为差异基线）
const indexes = ref([])
const loading = ref(false)
const saving = ref(false)
const ddlMode = ref(false)       // 原始 DDL 查看模式
const ddlText = ref('')

// ==================== SQL Server 专属辅助 ====================

const mssqlParts = () => {
  const t = String(props.table || '')
  if (t.includes('.')) { const i = t.indexOf('.'); return { schema: t.slice(0, i), table: t.slice(i + 1) } }
  return { schema: 'dbo', table: t }
}
const mssqlCommentSql = (kind, comment, columnName) => {
  const { schema, table } = mssqlParts()
  const level0 = `'SCHEMA', N'${sq(schema)}'`
  const level1 = `'TABLE', N'${sq(table)}'`
  const level2 = columnName ? `'COLUMN', N'${sq(columnName)}'` : 'NULL, NULL'
  const args = `${level0}, ${level1}, ${level2}`
  return `IF EXISTS(SELECT 1 FROM ::fn_listextendedproperty('MS_Description', ${args})) ` +
    `EXEC sp_updateextendedproperty 'MS_Description', N'${sq(comment)}', ${args} ` +
    `ELSE EXEC sp_addextendedproperty 'MS_Description', N'${sq(comment)}', ${args}`
}
const mssqlPkName = async () => {
  const { table } = mssqlParts()
  try {
    const res = await executeSql(props.conn.id,
      `SELECT kc.name AS name FROM sys.key_constraints kc JOIN sys.tables t ON kc.parent_object_id = t.object_id WHERE t.name = N'${sq(table)}' AND kc.type = 'PK'`,
      props.database, null, null, null, null, true)
    const row = res && res.rows && res.rows[0]
    return row && row.name ? String(row.name) : ''
  } catch (e) { return '' }
}
const mssqlPk = ref('')

/**
 * 这些类型**必须**带长度/精度。
 *
 * 不填的后果不是「不写参数」这么轻：`nvarchar` 在 SQL Server 上会报错或退化成 1 字符宽，
 * `decimal` 在 MySQL 上默认 (10,0) —— 精度被悄悄改掉。所以按方言挡在保存之前。
 *
 * PG 不在此列：`varchar` / `numeric` 不带参数分别是「不限长」与「任意精度」，都是合法语义。
 */
const LEN_REQUIRED = {
  mysql: ['VARCHAR', 'CHAR', 'VARBINARY', 'BINARY', 'DECIMAL', 'NUMERIC'],
  mssql: ['VARCHAR', 'NVARCHAR', 'CHAR', 'NCHAR', 'VARBINARY', 'BINARY', 'DECIMAL', 'NUMERIC'],
  oracle: ['VARCHAR2', 'NVARCHAR2', 'CHAR', 'NCHAR', 'DECIMAL', 'NUMERIC']
}
const needsLength = (typeBase) => {
  const list = LEN_REQUIRED[ddlStyle.value]
  if (!list) return false
  const base = String(typeBase || '').trim().toUpperCase().replace(/\(.*/, '').trim()
  return list.includes(base)
}
/** 长度输入框的占位：必填的给「必填」，可选的给「可选」，都不填也无妨的留空。 */
const lenPlaceholder = (r) => {
  if (r.opaque) return ''
  return needsLength(r.typeBase) ? t('tdet.required') : t('tdet.optional')
}

const buildType = (t, len) => {
  const base = (t || '').trim() || 'VARCHAR'
  if (base.includes('(')) return base
  const l = String(len ?? '').trim()
  if (!l) return base
  if (l.includes(',') && !/DECIMAL|NUMERIC|NUMBER|FLOAT|DOUBLE|REAL/i.test(base)) return `${base}(${l.split(',')[0].trim()})`
  return `${base}(${l})`
}
const typeParam = (o) => {
  const l = String(o.length ?? '').trim()
  const s = String(o.scale ?? '').trim()
  if (l && s) return `${l},${s}`
  return l || s
}

// ==================== 字段表格（与新建表一致的回显编辑） ====================

const normType = (s) => String(s || '').toLowerCase().replace(/\s+/g, '')

const clickhouseType = (t, len) => {
  const raw = String(t ?? '').trim()
  if (!raw) return 'String'
  const head = raw.replace(/\(.*/, '').trim().toUpperCase()
  const l = String(len ?? '').trim()
  const map = {
    VARCHAR: 'String', VARCHAR2: 'String', CHAR: 'String', TEXT: 'String',
    LONGTEXT: 'String', MEDIUMTEXT: 'String', TINYTEXT: 'String',
    TINYBLOB: 'String', BLOB: 'String', MEDIUMBLOB: 'String', LONGBLOB: 'String', CLOB: 'String',
    TINYINT: 'Int8', SMALLINT: 'Int16', MEDIUMINT: 'Int32', INT: 'Int32', INTEGER: 'Int32', BIGINT: 'Int64',
    FLOAT: 'Float32', REAL: 'Float64', DOUBLE: 'Float64', 'DOUBLE PRECISION': 'Float64',
    BOOL: 'Boolean', BOOLEAN: 'Boolean', TIMESTAMP: 'DateTime', DATETIME: 'DateTime',
    NUMERIC: 'Decimal', DECIMAL: 'Decimal', FIXEDSTRING: 'FixedString', FIXED_STRING: 'FixedString'
  }
  const mapped = map[head]
  if (mapped === 'Decimal') return l ? `Decimal(${l})` : 'Decimal(38, 6)'
  if (mapped === 'DateTime') return l ? `DateTime64(${l})` : 'DateTime'
  if (mapped === 'FixedString') return `FixedString(${l || '16'})`
  if (mapped) return mapped
  const nativeMap = {
    UINT8: 'UInt8', UINT16: 'UInt16', UINT32: 'UInt32', UINT64: 'UInt64',
    INT8: 'Int8', INT16: 'Int16', INT32: 'Int32', INT64: 'Int64',
    FLOAT32: 'Float32', FLOAT64: 'Float64', STRING: 'String',
    BOOLEAN: 'Boolean', DATE: 'Date', UUID: 'UUID', IPV4: 'IPv4', IPV6: 'IPv6'
  }
  if (nativeMap[head]) return nativeMap[head]
  if (/^datetime64$/i.test(head)) return l ? `DateTime64(${l})` : 'DateTime64(3)'
  if (/^decimal$/i.test(head)) return l ? `Decimal(${l})` : 'Decimal(38, 6)'
  if (/^fixedstring$/i.test(head)) return `FixedString(${l || '16'})`
  if (raw.includes('(') || /^(array|map|nullable|lowcardinality|enum|tuple|json)$/i.test(head)) return raw
  return l ? `${head}(${l})` : head
}

/** CH 类型剥 `Nullable(...)` 包装（可多层）。包装由「可空」勾选列表达，不该塞进类型格。
 *  括号必须自平衡才剥（防 `Nullable(a)b(c)` 这类被误剥）；LowCardinality/Array/Map 等
 *  有类型语义的包装**不剥** —— 剥了重建会丢语义（如 LowCardinality），维持整串 opaque。 */
const stripChNullable = (raw) => {
  let cur = String(raw || '').trim()
  let changed = false
  for (;;) {
    const m = cur.match(/^Nullable\s*\((.*)\)\s*$/i)
    if (!m) break
    let depth = 0
    let ok = true
    for (const ch of m[1]) {
      if (ch === '(') depth++
      else if (ch === ')') { depth--; if (depth < 0) { ok = false; break } }
    }
    if (!ok || depth !== 0) break
    cur = m[1].trim()
    changed = true
  }
  return { text: cur, changed }
}

const splitTypeText = (raw) => {
  const s = (raw || '').trim()
  const m = s.match(/^([A-Za-z_]+)\s*\(([^)]*)\)(.*)$/)
  if (m) {
    const base = m[1].toUpperCase()
    if (!/^\d+(,\d+)?$/.test(m[2].trim().replace(/\s+/g, ''))) {
      return { base: s, length: '', opaque: true }
    }
    if (!(m[3] || '').trim()) return { base, length: m[2].trim(), opaque: false }
    return { base: s, length: '', opaque: true }
  }
  if (!/\s/.test(s)) return { base: s.toUpperCase(), length: '', opaque: false }
  return { base: s, length: '', opaque: true }
}

let colUid = 0
const colRows = ref([])

/**
 * Doris 的列属性：列名(小写) → { agg: 聚合类型, key: 是否 key 列 }。
 *
 * 为什么单独存一张表、而不是直接写到字段行上：`buildColRows()` 是
 * `columns.value.map(… => ({…新对象}))`，**每次调用都会重建整排行对象**，
 * 而它会被多处触发（含 watch）。直接把属性挂在行上，任何一次重建都会把它冲掉 ——
 * 实测就是如此：挂上去的那一刻诊断显示 aggKey=true ✓，生成 SQL 时却读不到 ✗，
 * 于是 `MODIFY COLUMN` 永远缺少 `KEY`，Doris 报 Can not change aggregation type。
 */
const dorisColAttrs = new Map()

/** 把 Doris 列属性补到当前字段行上（重建后要再调一次，见下面的 watch） */
const applyDorisColAttrs = () => {
  if (!dorisColAttrs.size) return
  for (const r of colRows.value) {
    const hit = dorisColAttrs.get(String(r.name || '').trim().toLowerCase())
    if (!hit) continue
    if (hit.agg) r.aggType = hit.agg
    r.aggKey = !!hit.key
  }
}
// colRows 每次被重建（buildColRows / watch 触发）之后，都重补一遍属性。
// flush:'post' 保证它在赋值与渲染之后跑，不会被同一轮的其它赋值盖掉。
watch(colRows, () => applyDorisColAttrs(), { flush: 'post' })

const isLoadedRow = (r) => r._state === 'loaded'
// 支持在线修改已有字段的方言
// Doris 走 MySQL 协议、`ALTER TABLE … MODIFY COLUMN` 是它的原生语法，后端能力位也是
// supportsColumnModify=true —— 之前名单里漏了它，于是整个字段表被灰成只读：
// 勾选框灰掉看着像纯文本、输入框全禁用，同一张表在 MySQL 上却是可编辑的，
// 「不同数据库样式不一样」就是这么来的。
const canEditLoaded = computed(() =>
  // derby 也在内：后端 edit_capabilities 已声明 Derby 支持改列（实测过 10.16），
  // 名单里漏了它的话，后端说"支持"而这里把整张字段表灰成只读 —— 两处口径不一致。
  has('supportsColumnModify') && ['mysql', 'pg', 'oracle', 'mssql', 'clickhouse', 'doris', 'derby'].includes(ddlStyle.value))
// 新增行可内联主键的方言
const canSetPkOnAdd = computed(() => ['mysql', 'oracle', 'mssql'].includes(ddlStyle.value))
// 修改主键/自增权限（与旧实现保持一致）
const canEditPkOf = (row) => {
  // Doris 的主键即建表时确定的 Key 列（DUPLICATE/UNIQUE KEY），不支持通过 ALTER 新增/修改
  // （会报 "Can not change aggregation type"），编辑态一律只读展示
  if (isDoris.value) return false
  if (row._state === 'new') return canSetPkOnAdd.value
  return canEditLoaded.value && (ddlStyle.value === 'mysql' || ddlStyle.value === 'mssql')
}
const canEditAutoOf = (row) => {
  // Doris 不支持通过 ALTER 修改/新增列的自增属性（自增只能在建表时指定），编辑态一律禁用
  if (isDoris.value) return false
  if (row._state === 'new') return ['mysql', 'mssql', 'oracle'].includes(ddlStyle.value)
  return canEditLoaded.value && ddlStyle.value === 'mysql'
}
/**
 * 自增勾选框禁用时的**原因**。
 *
 * 原来这里是统一的一句「当前状态不可修改」—— 用户看到的就是「为什么不能点？」。
 * 真实原因是**数据库不支持**（SQL Server 的 IDENTITY 无法通过 ALTER 添加/移除），
 * 必须说清楚，否则会被读成「这个功能你们没做」。
 */
const autoTitle = (row) => {
  if (isDoris.value) return t('tdet.autoDoris')
  if (row._state === 'new') return t('tdet.autoNewCol')
  if (ddlStyle.value === 'mssql') return t('tdet.autoMssql')
  if (ddlStyle.value === 'oracle') return t('tdet.autoOracle')
  if (ddlStyle.value === 'mysql') return t('tdet.autoMysql')
  return t('tdet.autoGeneric')
}

const inputDisabled = (r) => isLoadedRow(r) && !canEditLoaded.value
const typeDisabled = (r) => inputDisabled(r)
const lenDisabled = (r) => r.opaque || inputDisabled(r)
/**
 * 字段名能否编辑。
 *
 * 原来这里把 oracle / mssql 钉成「不支持在线重命名」——**这与事实不符**：
 * SQL Server 有 `sp_rename '…', '…', 'COLUMN'`，Oracle 有 `ALTER TABLE … RENAME COLUMN`。
 * （Oracle 分支本来就已经会生成改名语句，只是被这道闸门挡住，用户永远碰不到。）
 * 真正会生成不出来的只有下面这些没有改名语法的方言。
 */
const nameDisabled = (r) => {
  // 支持改名的方言：字段名**始终可编辑**（不再落到 inputDisabled —— 那会把
  // supportsColumnModify=false 的 SQLite / Derby 也一并灰掉，而它们只是不能改
  // 类型/默认值/注释，改名是支持的）
  if (isLoadedRow(r) && renameSupported.value) return false
  if (isLoadedRow(r) && !renameSupported.value) return true
  return inputDisabled(r)
}
/**
 * 该方言有没有「改列名」语法。名单 = **下面 `buildColumnSql` 确实会生成出改名语句**的方言；
 * 生成不出来的（SQLite 只能重建表）才按不支持处理。
 *
 * - Derby：`RENAME COLUMN 表.旧 TO 新`（**实测** 10.16 可用），所以它在名单里；
 * - ClickHouse：`ALTER TABLE … RENAME COLUMN a TO b`（20.4+）—— 它**不是** SQLite 那种
 *   「只能重建表」的情况：下面的 clickhouse 分支（见 `buildColumnSql`）一直能生成改名语句，
 *   只是名字不在这份名单里，于是「字段名」输入框永远是灰的、用户点不动（实测如此）。
 */
const renameSupported = computed(() =>
  // doris：`ALTER TABLE … RENAME COLUMN 旧 新`（实测 2.x 可用，**连分桶/Key 列都能改**，
  // 语法是空格连接、没有 TO —— 生成器按这个写）；
  // sqlite（3.25+ RENAME COLUMN）/ generic（DB2 / DuckDB 的 ANSI 形式）：只放开改名一种。
  ['mysql', 'doris', 'pg', 'oracle', 'mssql', 'derby', 'clickhouse', 'sqlite', 'generic'].includes(ddlStyle.value))

const buildColRows = () => {
  const isCh = ddlStyle.value === 'clickhouse'
  colRows.value = (columns.value || []).map((c, i) => {
    // CH：先剥 Nullable(...) 包装再拆参数 —— 不剥的话 `Nullable(Decimal(10, 2))` 整串
    // 变成 opaque 类型格，长度/精度永远空白（真机）。包装由「可空」勾选列表达，
    // _orig.type 也存**剥完的内核类型**：与 buildType/typeParam 的重建结果同口径，
    // 未修改的行不会因包装差异被误判「已修改」而生成假 ALTER。
    let innerType = null
    let rawType = c.type
    if (isCh) {
      const u = stripChNullable(rawType)
      if (u.changed) innerType = u.text
    }
    const sp = splitTypeText(innerType ?? rawType)
    const lp = String(sp.length || '')
    const comma = lp.indexOf(',')
    const lenVal = comma < 0 ? lp : lp.slice(0, comma)
    const scaleVal = comma < 0 ? '' : lp.slice(comma + 1)
    const dv = c.defaultValue
    const dvNull = dv === null || dv === undefined
    return {
      _uid: 'c' + (++colUid) + '_' + i,
      _state: 'loaded',
      name: c.name,
      typeBase: sp.base, length: lenVal, scale: scaleVal, opaque: sp.opaque,
      nullable: !!c.nullable,
      defaultIsNull: dvNull, defaultValue: dvNull ? '' : String(dv),
      autoIncrement: !!c.autoIncrement, primaryKey: !!c.primaryKey,
      sortKey: !!c.sortKey,
      comment: c.comment || '', extra: c.extra || '',
      _orig: {
        name: c.name, type: normType(innerType ?? c.type), nullable: !!c.nullable,
        defaultIsNull: dvNull, defaultValue: dvNull ? null : String(dv),
        comment: c.comment || '',
        primaryKey: !!c.primaryKey, autoIncrement: !!c.autoIncrement,
        length: lenVal, scale: scaleVal
      }
    }
  })
}

const colChanged = (r) => {
  if (!isLoadedRow(r) || !r._orig) return false
  const o = r._orig
  if (String(r.name || '').trim() !== String(o.name || '').trim()) return true
  if (normType(buildType(r.typeBase, typeParam(r))) !== o.type) return true
  if (!!r.nullable !== o.nullable) return true
  const curDv = r.defaultIsNull ? null : String(r.defaultValue ?? '')
  if (curDv !== o.defaultValue) return true
  if ((r.comment || '').trim() !== o.comment.trim()) return true
  if (!!r.primaryKey !== !!o.primaryKey) return true
  if (!!r.autoIncrement !== !!o.autoIncrement) return true
  return false
}
/** 除「字段名」外，其它属性（类型/可空/默认值/注释/主键/自增）有没有变。
 *  Doris 改名专用：只改名时**不能**再发 MODIFY（Doris 会对 Nothing is changed 报错）。 */
const colAttrsChanged = (r) => {
  if (!isLoadedRow(r) || !r._orig) return false
  const o = r._orig
  if (normType(buildType(r.typeBase, typeParam(r))) !== o.type) return true
  if (!!r.nullable !== o.nullable) return true
  const curDv = r.defaultIsNull ? null : String(r.defaultValue ?? '')
  if (curDv !== o.defaultValue) return true
  if ((r.comment || '').trim() !== o.comment.trim()) return true
  if (!!r.primaryKey !== !!o.primaryKey) return true
  if (!!r.autoIncrement !== !!o.autoIncrement) return true
  return false
}

const newColRow = () => {
  const base = typeOptions.value[0] || 'VARCHAR'
  return {
    _uid: 'n' + (++colUid),
    _state: 'new', name: '',
    typeBase: base, length: /char|text/i.test(base) ? '255' : '', scale: '', opaque: false,
    nullable: true, defaultIsNull: false, defaultValue: '',
    autoIncrement: false, primaryKey: false, sortKey: false, comment: '', extra: '',
    _orig: null
  }
}
const addColRow = () => { colRows.value.push(newColRow()) }
const removeColRow = (r) => {
  if (r._state === 'new') colRows.value = colRows.value.filter(x => x !== r)
  else r._state = 'deleted'
}

const displayCols = computed(() => colRows.value.filter(r => r._state !== 'deleted'))
const fieldCount = computed(() => displayCols.value.filter(c => String(c.name || '').trim()).length)
/** 字段搜索：**只影响渲染**，不参与差异统计、不影响生成的 SQL —— 过滤是"看"，不是"改" */
const colSearch = ref('')
const visibleCols = computed(() => {
  const kw = colSearch.value.trim().toLowerCase()
  if (!kw) return displayCols.value
  return displayCols.value.filter((c) =>
    String(c.name || '').toLowerCase().includes(kw)
    || String(c.comment || '').toLowerCase().includes(kw)
    || String(c.typeBase || '').toLowerCase().includes(kw))
})
/* 注：`extra`（数据库返回的额外属性，如 on update CURRENT_TIMESTAMP）仍然保留在
   提交给后端的列信息里（见 toPayload），只是**不再单独占一列显示** —— 那一列绝大多数
   行都是空的，146px 的宽度换来的信息量不值；自增这类关键属性另有「自增」勾选列表达。 */

/* ==================== 注释放大编辑 ====================
   注释列只有 110px，长注释在格子里只能看到一小截。
   这里给一个多行编辑弹窗：打开时把当前值抄进草稿，点「确定」才写回，取消不动原值。 */
const commentDlg = reactive({ visible: false, text: '', field: '', row: null })
const openComment = (row) => {
  commentDlg.row = row
  commentDlg.field = String(row.name || '').trim() || t('tdet.newField')
  commentDlg.text = String(row.comment || '')
  commentDlg.visible = true
}
const applyComment = () => {
  if (commentDlg.row) commentDlg.row.comment = commentDlg.text
  commentDlg.visible = false
}

/**
 * 字段表格的列宽（百分比，合计恒为 100%）。
 *
 * 为什么用百分比而不是像素：像素总和是固定的（实测 860~880px），容器比它宽时右侧必然
 * 留一条空白（真机反复出现、换过好几种写法都按浏览器各自的分配规则跑）；百分比与容器宽度
 * 无关、合计 100% ⇒ 无论窗口多宽多窄，表格都精确铺满。权重只表达相对比例，最终统一归一化，
 * 所以「某方言少了主键/自增列」也不会破坏铺满。
 */
// 权重（相对比例，最终归一化为百分比）：字段名收窄、注释放大（用户实测反馈：
// 字段名一屏都放得下还占大片宽度，注释反而挤在 100px 里看不全）
const COL_WEIGHTS = { idx: 3, name: 13, type: 11, len: 5, prec: 5, nullable: 7, pk: 6, sort: 7, def: 12, auto: 5, comment: 30, ops: 5 }
const colWidths = computed(() => {
  const keys = ['idx', 'name', 'type', 'len', 'prec', 'nullable']
  if (showPkCol.value) keys.push('pk')
  if (showOrderKey.value) keys.push('sort')
  keys.push('def')
  if (showAutoIncrement.value) keys.push('auto')
  if (showComment.value) keys.push('comment')
  keys.push('ops')
  const total = keys.reduce((sum, k) => sum + COL_WEIGHTS[k], 0)
  const out = {}
  for (const k of keys) out[k] = ((COL_WEIGHTS[k] / total) * 100).toFixed(3) + '%'
  return out
})
const showOrderKey = computed(() => isCH.value)
// 主键列：Doris 没有「每列主键」概念（主键=建表时的 Key 列，界面上恒为「—」）、
// ClickHouse 同理 —— 整列不显示，比一列无意义的「—」干净
const showPkCol = computed(() => !isDoris.value && !isCH.value)
// 自增列：Doris 的自增只能在建表时指定、编辑器对它无任何可操作项（一列禁用的勾选框），
// 不显示
const showAutoIncrement = computed(() => has('supportsAutoIncrement') && !isDoris.value && !isCH.value)
const showComment = computed(() => has('supportsComment'))
const headSpan = computed(() => 9
  + (showOrderKey.value ? 1 : 0)
  + (showAutoIncrement.value ? 1 : 0)
  + (showComment.value ? 1 : 0))

const colStat = computed(() => {
  let added = 0, modified = 0, deleted = 0
  for (const r of colRows.value) {
    if (r._state === 'new') added++
    else if (r._state === 'deleted') deleted++
    else if (colChanged(r)) modified++
  }
  return { added, modified, deleted }
})

const onPkChange = (r) => { if (r.primaryKey && r.nullable) r.nullable = false }
const onAutoChange = (r) => {
  if (r.autoIncrement) {
    if (!r.primaryKey) r.primaryKey = true
    if (r.nullable) r.nullable = false
  }
}

// ==================== 字段 SQL 生成 ====================

const rowToForm = (r, isEdit) => ({
  name: String(r.name || '').trim(),
  oldName: isEdit && r._orig ? r._orig.name : '',
  type: r.typeBase || 'VARCHAR',
  length: r.opaque ? '' : r.length,
  scale: r.opaque ? '' : r.scale,
  nullable: !!r.nullable,
  defaultValue: r.defaultIsNull ? '' : String(r.defaultValue ?? ''),
  autoIncrement: !!r.autoIncrement,
  primaryKey: !!r.primaryKey,
  autoIncrementStart: 1,
  comment: r.comment || '',
  position: 'last', afterColumn: ''
})

const buildMySqlColumnDef = (f, includePk = true) => {
  let s = `${qt(f.name)} ${buildType(f.type, typeParam(f))}`
  if (includePk && f.primaryKey) s += ' PRIMARY KEY'
  s += (!f.primaryKey && f.nullable) ? '' : ' NOT NULL'
  if (f.autoIncrement) s += ' AUTO_INCREMENT'
  const dv = String(f.defaultValue ?? '').trim()
  if (dv !== '') s += /^null$/i.test(dv) ? ' DEFAULT NULL' : ` DEFAULT '${sq(dv)}'`
  if (f.comment) s += ` COMMENT '${sq(f.comment)}'`
  return s
}

const posPart = (f) => {
  if (f.position === 'first') return ' FIRST'
  if (f.position === 'after' && f.afterColumn) return ` AFTER ${qt(f.afterColumn)}`
  return ''
}

// 生成 新增/修改 字段 SQL；不可用返回 null（生成预览前已有 validate 拦截）
const buildColumnSql = (f, isEdit) => {
  // SQLite：不能改已有列的类型/默认值/注释（被 supportsColumnModify=false 禁掉），
  // 但 **3.25+ 支持 RENAME COLUMN** —— 字段名放开，只生成改名这一条。
  if (ddlStyle.value === 'sqlite' && isEdit) {
    if (f.name.trim() !== String(f.oldName || '').trim()) {
      return `ALTER TABLE ${qt(props.table)} RENAME COLUMN ${qt(f.oldName)} TO ${qt(f.name.trim())}`
    }
    return null
  }
  if (isEdit && (ddlStyle.value === 'oracle' || ddlStyle.value === 'mssql') && f.name !== f.oldName) return null
  // generic（DB2 / DuckDB 等暂未逐条实测的 SQL 方言）：只放开「改名」这一种标准操作
  // （RENAME COLUMN 旧 TO 新 是 ANSI 形式）；其它属性按 supportsColumnModify=false 保持只读。
  if (ddlStyle.value === 'generic' && isEdit) {
    if (f.name.trim() !== String(f.oldName || '').trim()) {
      return `ALTER TABLE ${qt(props.table)} RENAME COLUMN ${qt(f.oldName)} TO ${qt(f.name.trim())}`
    }
    return null
  }
  const t = qt(props.table)
  const dv = String(f.defaultValue ?? '').trim()
  const defNull = (v) => (/^null$/i.test(v) ? ' NULL' : ` '${sq(v)}'`)
  // Doris：`MODIFY COLUMN` **必须**带上列的聚合类型 —— key 列写 `KEY`，value 列写它自己的
  // 聚合（SUM / REPLACE / MAX…）。不带就被当成"改聚合类型"，Doris 直接报
  // `Can not change aggregation type`（实测：连只改注释也会撞上）。
  // 聚合类型与 key 标记来自 `DESC … ALL`（见 load() 里那一段）；拿不到时按 value 列处理，
  // 语句退化成不带聚合类型的样子（至少是原来那句，而不是乱加）。
  if (ddlStyle.value === 'doris') {
    const typeText = `${qt(f.name.trim())} ${buildType(f.type, typeParam(f))}`
    if (isEdit) {
      // 改列名：`ALTER TABLE … RENAME COLUMN 旧 新`（实测 2.x 可用，连分桶/Key 列都能改；
      // 语法是空格连接、**没有 TO**）。改名与改属性是两条语句 —— 改名用新名，
      // 随后的 MODIFY（若有属性变化）也用新名，顺序不能反。
      // 只改名、属性没动时**只发 RENAME**：多余的 MODIFY 会被 Doris 以
      // Nothing is changed 拒掉（caller 经 `f._attrsChanged` 告知属性是否另有变化）。
      const renamed = f.name.trim() !== String(f.oldName || '').trim()
      const stmts = []
      if (renamed) stmts.push(`ALTER TABLE ${t} RENAME COLUMN ${qt(f.oldName)} ${qt(f.name.trim())}`)
      if (!renamed || f._attrsChanged) {
        let def = typeText
        const agg = String(f.aggType || '').trim()
        if (f.aggKey) def += ' KEY'
        else if (agg && !/^none$/i.test(agg)) def += ' ' + agg
        def += f.nullable ? ' NULL' : ' NOT NULL'
        if (dv !== '') def += /^null$/i.test(dv) ? ' DEFAULT NULL' : ` DEFAULT '${sq(dv)}'`
        if (f.comment) def += ` COMMENT '${sq(f.comment)}'`
        stmts.push(`ALTER TABLE ${t} MODIFY COLUMN ${def}`)
      }
      return stmts.join(';\n')
    }
    let add = `ALTER TABLE ${t} ADD COLUMN ${typeText}`
    if (dv !== '') add += /^null$/i.test(dv) ? ' DEFAULT NULL' : ` DEFAULT '${sq(dv)}'`
    if (f.comment) add += ` COMMENT '${sq(f.comment)}'`
    return add
  }
  if (ddlStyle.value === 'mysql') {
    const pos = posPart(f)
    if (isEdit) {
      return f.name !== f.oldName
        ? `ALTER TABLE ${t} CHANGE COLUMN ${qt(f.oldName)} ${buildMySqlColumnDef(f, false)}${pos}`
        : `ALTER TABLE ${t} MODIFY COLUMN ${buildMySqlColumnDef(f, false)}${pos}`
    }
    let sql = `ALTER TABLE ${t} ADD COLUMN ${buildMySqlColumnDef(f, false)}${pos}`
    const start = Number(f.autoIncrementStart)
    if (f.autoIncrement && Number.isInteger(start) && start > 1) sql += `;\nALTER TABLE ${t} AUTO_INCREMENT=${start}`
    return sql
  }
  if (ddlStyle.value === 'pg') {
    const stmts = []
    const n = f.name.trim()
    if (isEdit) {
      if (n !== f.oldName) stmts.push(`ALTER TABLE ${t} RENAME COLUMN ${qt(f.oldName)} TO ${qt(n)}`)
      stmts.push(`ALTER TABLE ${t} ALTER COLUMN ${qt(n)} TYPE ${buildType(f.type, typeParam(f))}`)
      stmts.push(`ALTER TABLE ${t} ALTER COLUMN ${qt(n)} ${f.nullable ? 'DROP' : 'SET'} NOT NULL`)
      if (dv !== '') stmts.push(`ALTER TABLE ${t} ALTER COLUMN ${qt(n)} SET DEFAULT${defNull(dv)}`)
      else stmts.push(`ALTER TABLE ${t} ALTER COLUMN ${qt(n)} DROP DEFAULT`)
      if (f.comment && has('supportsComment')) stmts.push(`COMMENT ON COLUMN ${t}.${qt(n)} IS '${sq(f.comment)}'`)
    } else {
      let add = `ALTER TABLE ${t} ADD COLUMN ${qt(n)} ${buildType(f.type, typeParam(f))}`
      add += f.nullable ? '' : ' NOT NULL'
      if (dv !== '') add += ` DEFAULT${defNull(dv)}`
      stmts.push(add)
      if (f.comment && has('supportsComment')) stmts.push(`COMMENT ON COLUMN ${t}.${qt(n)} IS '${sq(f.comment)}'`)
    }
    return stmts.join(';\n')
  }
  if (ddlStyle.value === 'oracle') {
    const stmts = []
    const n = f.name.trim()
    let def = `${qt(n)} ${buildType(f.type, typeParam(f))}`
    if (f.primaryKey) def += ' PRIMARY KEY'
    const ostart = Number(f.autoIncrementStart)
    if (f.autoIncrement) {
      def += ostart > 1 ? ` GENERATED BY DEFAULT AS IDENTITY (START WITH ${ostart})` : ' GENERATED BY DEFAULT AS IDENTITY'
    }
    def += f.nullable ? '' : ' NOT NULL'
    if (dv !== '') def += ` DEFAULT${defNull(dv)}`
    stmts.push(`ALTER TABLE ${t} ${isEdit ? 'MODIFY' : 'ADD'} (${def})`)
    if (f.comment && has('supportsComment')) stmts.push(`COMMENT ON COLUMN ${t}.${qt(n)} IS '${sq(f.comment)}'`)
    return stmts.join(';\n')
  }
  if (ddlStyle.value === 'mssql') {
    const n = f.name.trim()
    const def = `${qt(n)} ${buildType(f.type, typeParam(f))}`
    const stmts = []
    if (isEdit) {
      if (n !== f.oldName) {
        // SQL Server 改列名走 sp_rename，而不是 ALTER COLUMN（后者改不了名字）。
        // 顺手把它的坑写进 SQL 预览：sp_rename **不会**更新引用该列的视图/存储过程/约束，
        // 这条提醒只在语句上方留一句注释，不替用户做决定。
        const { schema, table } = mssqlParts()
        stmts.push(
          tr('tdet.spRenameNote') + '\n' +
          `EXEC sp_rename N'${sq(schema)}.${sq(table)}.${sq(f.oldName)}', N'${sq(n)}', 'COLUMN'`)
      }
      stmts.push(`ALTER TABLE ${t} ALTER COLUMN ${def} ${f.nullable ? 'NULL' : 'NOT NULL'}`)
    } else {
      let add = `ALTER TABLE ${t} ADD ${def}`
      if (f.autoIncrement) add += ' IDENTITY(1,1)'
      add += f.nullable ? ' NULL' : ' NOT NULL'
      if (dv !== '' && !f.autoIncrement) add += ` DEFAULT${defNull(dv)}`
      stmts.push(add)
      if (f.primaryKey) {
        const pkName = 'PK_' + String(props.table).replace(/[^\w]/g, '').slice(0, 90) + '_' + n.replace(/[^\w]/g, '').slice(0, 20)
        stmts.push(`ALTER TABLE ${t} ADD CONSTRAINT ${qt(pkName)} PRIMARY KEY (${qt(n)})`)
      }
    }
    if (f.comment && has('supportsComment')) stmts.push(mssqlCommentSql('COLUMN', f.comment, n))
    return stmts.join(';\n')
  }
  if (ddlStyle.value === 'clickhouse') {
    const n = f.name.trim()
    const baseT = clickhouseType(f.type, typeParam(f))
    const defT = f.nullable && !/^Nullable\(/i.test(baseT) ? `Nullable(${baseT})` : baseT
    let def = `${qt(n)} ${defT}`
    if (dv !== '') def += /^null$/i.test(dv) ? ' DEFAULT NULL' : ` DEFAULT '${sq(dv)}'`
    if (f.comment) def += ` COMMENT '${sq(f.comment)}'`
    if (isEdit) {
      return n !== f.oldName
        ? `ALTER TABLE ${t} RENAME COLUMN ${qt(f.oldName)} TO ${qt(n)};\nALTER TABLE ${t} MODIFY COLUMN ${def}`
        : `ALTER TABLE ${t} MODIFY COLUMN ${def}`
    }
    return `ALTER TABLE ${t} ADD COLUMN ${def}`
  }
  if (ddlStyle.value === 'derby') {
    /*
     * Derby 的列变更语法自成一派：`ALTER COLUMN c SET DATA TYPE …` / `SET DEFAULT …` /
     * `NOT NULL | NULL`，改名是 `RENAME COLUMN`。**实测**（Derby 10.16）这几条都可用。
     *
     * 两处如实说明：
     * - 列注释 Derby 没有 → `supportsComment=false`，界面上不会出现那个输入框；
     * - Derby 改类型时会**校验已有数据**能否转换，且被索引/约束引用的列可能被拒绝 ——
     *   报错交给数据库给（它说的比我们猜的准），这里不预先拦。
     */
    const stmts = []
    const n = f.name.trim()
    if (isEdit) {
      // Derby 的改名**不是** `ALTER TABLE … RENAME COLUMN`（实测 10.16 直接报
      // `Syntax error: Encountered "RENAME"`），而是一条独立语句：`RENAME COLUMN 表.旧 TO 新`。
      if (n !== f.oldName) stmts.push(`RENAME COLUMN ${t}.${qt(f.oldName)} TO ${qt(n)}`)
      stmts.push(`ALTER TABLE ${t} ALTER COLUMN ${qt(n)} SET DATA TYPE ${buildType(f.type, typeParam(f))}`)
      stmts.push(`ALTER TABLE ${t} ALTER COLUMN ${qt(n)} ${f.nullable ? 'NULL' : 'NOT NULL'}`)
      if (dv !== '') stmts.push(`ALTER TABLE ${t} ALTER COLUMN ${qt(n)} SET DEFAULT${defNull(dv)}`)
      else stmts.push(`ALTER TABLE ${t} ALTER COLUMN ${qt(n)} DROP DEFAULT`)
    } else {
      // 顺序照 Derby 的 columnDefinition：类型 → DEFAULT → NOT NULL
      let add = `ALTER TABLE ${t} ADD COLUMN ${qt(n)} ${buildType(f.type, typeParam(f))}`
      if (dv !== '') add += ` DEFAULT${defNull(dv)}`
      add += f.nullable ? '' : ' NOT NULL'
      stmts.push(add)
    }
    return stmts.join(';\n')
  }
  // SQLite / 通用方言：仅支持新增字段
  let add = `ALTER TABLE ${t} ADD COLUMN ${qt(f.name.trim())} ${buildType(f.type, typeParam(f))}`
  add += f.nullable ? '' : ' NOT NULL'
  if (dv !== '') add += ` DEFAULT${defNull(dv)}`
  return add
}

// 待执行修改的校验（生成/保存前调用），返回问题列表
const validate = () => {
  const issues = []
  for (const r of colRows.value) {
    const nm = String(r.name || '').trim()
    if (r._state === 'new') {
      if (!nm) { issues.push(t('tdet.issueNoName')); continue }
      if (!r.typeBase) issues.push(t('tdet.issueNoType', { name: nm }))
      if (r.autoIncrement && !r.primaryKey) issues.push(t('tdet.issueAutoIncNotPk', { name: nm }))
      if (ddlStyle.value === 'sqlite' && !r.nullable && !String(r.defaultValue ?? '').trim()) {
        issues.push(t('tdet.issueSqliteNotNull', { name: nm }))
      }
      // Derby 同一条限制：往**非空表**里加 NOT NULL 列必须带 DEFAULT，否则数据库直接拒绝
      if (ddlStyle.value === 'derby' && !r.nullable && !String(r.defaultValue ?? '').trim()) {
        issues.push(t('tdet.issueDerbyNotNull', { name: nm }))
      }
    } else if (r._state === 'loaded' && colChanged(r)) {
      // SQL Server：改已有列的默认值必须先删默认约束再建，当前生成器只出 ALTER COLUMN。
      // 不拦的话会「预览有 SQL、保存成功、但默认值没变」—— 静默失效比报错更难查。
      if (ddlStyle.value === 'mssql') {
        const cur = r.defaultIsNull ? null : String(r.defaultValue ?? '')
        if (cur !== (r._orig.defaultValue ?? null)) {
          issues.push(t('tdet.issueSqlServerDefault', { name: nm }))
        }
      }
      if (r.autoIncrement && !r.primaryKey && !r._orig.primaryKey && !r._orig.autoIncrement) {
        issues.push(t('tdet.issueAutoIncNeedsPk', { name: nm }))
      }
      if (r.autoIncrement && !r.primaryKey && r._orig.primaryKey) {
        issues.push(t('tdet.issueStillAutoInc', { name: nm }))
      }
    }
  }
  if (ddlStyle.value === 'mysql') {
    const ais = colRows.value.filter(r => r.autoIncrement && (r._state === 'new' || colChanged(r)))
    if (ais.length > 1) issues.push(t('tdet.issueOneAutoInc'))
  }
  // 注：Doris 的分桶列（DISTRIBUTED BY）数据库不允许修改（实测
  // `Can not modify distribution column[…]`）。这里**不拦** —— 语句照常生成、
  // 照常发出去，被拒时把数据库的原话显示出来；用户要的是"看得见 SQL"，不是被静默跳过。
  // 需要长度却没填：生成出来的 DDL 没有参数，在 SQL Server / MySQL / Oracle 上
  // 要么直接报错，要么把列悄悄变成极窄宽度（甚至 1 字符）。这是最该拦在保存之前的一类改动。
  for (const r of colRows.value) {
    if (r.opaque || r._state === 'deleted') continue
    if (r._state === 'loaded' && !colChanged(r)) continue
    const ln = String(r.length ?? '').trim()
    if (needsLength(r.typeBase) && !ln) {
      const nm = String(r.name || '').trim() || t('tdet.unnamedField')
      const base = String(r.typeBase || '').trim().toUpperCase()
      const tip = ['DECIMAL', 'NUMERIC'].includes(base) ? t('tdet.decimalTip') : ''
      issues.push(t('tdet.issueColLength', { col: nm, type: base, tip }))
    }
  }
  for (const r of idxRows.value) {
    if (r._state === 'new' || (r._state === 'loaded' && idxChanged(r))) {
      if (!String(r.name || '').trim()) issues.push(t('tdet.issueIndexName'))
      if (!(r.columns || []).length) issues.push(t('tdet.issueIndexNoCols', { name: String(r.name || '').trim() || t('tdet.unnamed') }))
    }
  }
  return issues
}
const issues = computed(() => validate())

// ==================== 变更 SQL 汇总（实时预览） ====================

const samePk = (a, b) => a.length === b.length && a.every((x, i) => x === b[i])

// 索引：先输出需要 DROP 的（供删除/重建），供列变更前清理。
// **只改名字**的已存在索引不在这里删 —— 见下面 idxCreateParts 的改名分支：
// 删掉再建会让撑着自增列的那条索引短暂消失，MySQL 直接拒绝（errorCode=1075）。
const idxDropParts = () => {
  const parts = []
  const renamable = (r) => canRenameIndex.value && idxPureRename(r)
  const removed = idxRows.value.filter(r => r._state === 'deleted')
  const modified = idxRows.value.filter(r => r._state === 'loaded' && idxChanged(r) && !renamable(r))
  for (const r of [...removed, ...modified]) {
    const oldName = r._orig && r._orig.name
    if (oldName) parts.push(dropIndexSql(oldName))
  }
  return parts
}
// 索引：列变更之后再重建/新增
const idxCreateParts = () => {
  const parts = []
  const modified = idxRows.value.filter(r => r._state === 'loaded' && idxChanged(r))
  const added = idxRows.value.filter(r => r._state === 'new')
  for (const r of [...modified, ...added]) {
    if (!String(r.name || '').trim() || !(r.columns || []).length) continue
    // 只改名字 → 走原生 RENAME：索引不消失，自增列的键也就不受影响
    if (canRenameIndex.value && idxPureRename(r)) {
      parts.push(renameIndexSql(String(r._orig.name).trim(), String(r.name).trim()))
      continue
    }
    const s = createIndexSql(r)
    if (s) parts.push(s)
  }
  return parts
}

// 字段变更：按方言生成完整 ALTER（与旧的逐条保存顺序一致）
const colParts = () => {
  const removed = colRows.value.filter(r => r._state === 'deleted')
  const added = colRows.value.filter(r => r._state === 'new' && String(r.name || '').trim() !== '')
  const modified = colRows.value.filter(r => isLoadedRow(r) && colChanged(r))
  if (!removed.length && !added.length && !modified.length) return []
  const parts = []
  const t = qt(props.table)
  // Doris 与 MySQL 同一套：它走 MySQL 协议，DROP/ADD/MODIFY COLUMN 都一致。
  // （分支里的自增相关处理对 Doris 是空转 —— Doris 没有 auto_increment，能力位也是 false。）
  if (ddlStyle.value === 'mysql' || ddlStyle.value === 'doris') {
    for (const r of removed) parts.push(`ALTER TABLE ${t} DROP COLUMN ${qt(r._orig ? r._orig.name : r.name)}`)
    // ① 修改：先不带 AUTO_INCREMENT（避免 DROP PRIMARY KEY 被自增列阻断，最后统一恢复）
    // Doris 分桶列（DISTRIBUTED BY）：数据库**一律拒绝** `MODIFY COLUMN` ——
    // 改类型、改注释、什么都改不了（实测 `Can not modify distribution column[id]`）。
    // 不让它拖垮整批变更：该列的 ALTER 不生成，脚本里留一行注释说明原因，
    // 保存成功后再弹一条警告（见 save() 里的 dorisDistSkipped）。
    const distCol = ddlStyle.value === 'doris' ? dorisDistColName() : ''
    for (const r of modified) {
      if (distCol && String(r.name || '').trim().toLowerCase() === distCol) {
        // 注意：本函数里 `t` 已被上面的 `const t = qt(props.table)`（引号表名）遮住，
        // 这里必须用 i18n 别名 tr（见文件头）—— 直接写 t(...) 等于调字符串，必炸（真机踩过）
        parts.push(`-- ${tr('tdet.dorisDistSkip', { col: String(r.name || '').trim() })}`)
        continue
      }
      const f = rowToForm(r, true); f.autoIncrement = false
      // Doris：把 `DESC … ALL` 取回的**聚合类型 / key 标记带进 form**。
      // `rowToForm()` 是「行对象 → 普通表单对象」的转换，只搬固定字段 ——
      // 不补这一步，生成器读到的 f.aggKey 永远是 undefined，
      // `MODIFY COLUMN` 就永远缺 `KEY`，Doris 报 Can not change aggregation type（实测）。
      f.aggKey = r.aggKey
      f.aggType = r.aggType
      // Doris：只改名时不能发多余的 MODIFY（见 buildColumnSql 的 doris 分支）
      f._attrsChanged = colAttrsChanged(r)
      const sql = buildColumnSql(f, true)
      if (sql) parts.push(sql)
    }
    // ② 新增
    for (const r of added) {
      const f = rowToForm(r, false); f.autoIncrement = false
      const sql = buildColumnSql(f, false)
      if (sql) parts.push(sql)
    }
    // ③ 表级主键 diff（删除列后 MySQL 会自动摘除被删列上的主键，避免多余的 DROP PRIMARY KEY）
    const oldPk = (columns.value || []).filter(c => c.primaryKey).map(c => String(c.name))
    const newPk = displayCols.value.filter(r => r.primaryKey && String(r.name || '').trim()).map(r => String(r.name).trim())
    const removedNames = new Set(removed.map(r => String(r._orig ? r._orig.name : r.name)))
    const remainPk = oldPk.filter(n => !removedNames.has(n))
    if (!samePk(oldPk, newPk) && !samePk(remainPk, newPk)) {
      if (remainPk.length) parts.push(`ALTER TABLE ${t} DROP PRIMARY KEY`)
      if (newPk.length) parts.push(`ALTER TABLE ${t} ADD PRIMARY KEY (${newPk.map(qt).join(', ')})`)
    }
    // ④ 恢复自增
    for (const r of [...modified, ...added]) {
      if (r.autoIncrement) {
        parts.push(`ALTER TABLE ${t} MODIFY COLUMN ${buildMySqlColumnDef(rowToForm(r, true), false)}`)
      }
    }
    return parts
  }
  // 非 MySQL：删除 → 修改 → 新增（SQL Server 若改动主键列，须先摘除主键约束）
  const oldPk = (columns.value || []).filter(c => c.primaryKey).map(c => String(c.name))
  const newPk = displayCols.value.filter(r => r.primaryKey && String(r.name || '').trim()).map(r => String(r.name).trim())
  const pkTouched = oldPk.some(n =>
    removed.some(r => String(r._orig ? r._orig.name : r.name) === n) ||
    modified.some(r => String(r._orig ? r._orig.name : r.name) === n))
  if (ddlStyle.value === 'mssql' && pkTouched && oldPk.length && mssqlPk.value) {
    parts.push(`ALTER TABLE ${t} DROP CONSTRAINT ${qt(mssqlPk.value)}`)
  }
  for (const r of removed) parts.push(`ALTER TABLE ${t} DROP COLUMN ${qt(r._orig ? r._orig.name : r.name)}`)
  for (const r of modified) {
    const sql = buildColumnSql(rowToForm(r, true), true)
    if (sql) parts.push(sql)
  }
  for (const r of added) {
    const sql = buildColumnSql(rowToForm(r, false), false)
    if (sql) parts.push(sql)
  }
  if (ddlStyle.value === 'mssql') {
    if ((pkTouched || !samePk(oldPk, newPk)) && newPk.length) {
      const pkName = 'PK_' + String(props.table).replace(/[^\w]/g, '').slice(0, 90)
      parts.push(`ALTER TABLE ${t} ADD CONSTRAINT ${qt(pkName)} PRIMARY KEY (${newPk.map(qt).join(', ')})`)
    }
  }
  return parts
}

// 表选项变更
const optsBase = { engine: '', charset: '', collation: '', autoIncrement: '', comment: '' }
// 数据库现状的**权威基线**（listTables 的表清单）—— 快照可能被并发载入/DDL 回填踩脏，
// 判定「用户有没有改」最终以这里的值为准（真机踩过：打开即误报注释有改动）
const tablesRef = ref([])
const optsChanged = computed(() => {
  const f = tableForm.value
  // Doris：引擎/字符集/自增等建表后不可改（界面只读），**只有表注释**能改
  // （实测 `ALTER TABLE … MODIFY COMMENT` 可用）—— 只比注释，别把只读项算成改动
  // （之前恒为 false 又会漏掉注释这个真能改的）。
  // 基线用**权威值**（表清单里这张表的注释 = 数据库现状）—— 快照可能被并发载入踩脏，
  // 用它会「打开即误报注释有改动」（真机踩过多次）。
  if (isDoris.value) {
    const base = tablesRef.value.find(x => x.name === props.table)?.comment ?? (optsBase.comment || '')
    return (f.comment || '') !== (base || '')
  }
  return f.engine !== optsBase.engine || f.charset !== optsBase.charset ||
    f.collation !== optsBase.collation || String(f.autoIncrement).trim() !== String(optsBase.autoIncrement).trim() ||
    (f.comment || '') !== (optsBase.comment || '')
})
const optParts = () => {
  // Doris 的表选项（引擎/字符集/自增…）建表后完全不可改 → 短路；
  // **表注释除外**：实测 `ALTER TABLE t MODIFY COMMENT '…'` 可用（与 ClickHouse 同语法）
  if (!optsChanged.value) return []
  const f = tableForm.value
  const t = qt(props.table)
  if (isDoris.value) {
    // 同 optsChanged：与**权威基线**（表清单现状）比较，不用可能被踩脏的快照
    const base = tablesRef.value.find(x => x.name === props.table)?.comment ?? (optsBase.comment || '')
    if ((f.comment || '') === (base || '')) return []
    return [`ALTER TABLE ${t} MODIFY COMMENT '${sq(f.comment || '')}'`]
  }
  // ClickHouse：表选项里**只有表注释**能改（引擎 / 排序键 / 分区键建表时定死，界面只读展示）。
  // 语法是 `ALTER TABLE t MODIFY COMMENT '…'`（21.3+）—— 更老的服务端会直接报语法错，
  // 界面把数据库的原话显示出来，不替它猜。
  if (ddlStyle.value === 'clickhouse') {
    if (!has('supportsComment') || (f.comment || '') === (optsBase.comment || '')) return []
    return [`ALTER TABLE ${t} MODIFY COMMENT '${sq(f.comment || '')}'`]
  }
  if (ddlStyle.value === 'mssql') {
    return [mssqlCommentSql('TABLE', f.comment || '')]
  }
  if (has('supportsTableOptions')) {
    const p = []
    if (f.engine && f.engine !== optsBase.engine) p.push(`ENGINE=${f.engine}`)
    // 编码与排序规则必须写成**同一个**表选项：MySQL 的语法是
    //   [DEFAULT] CHARACTER SET [=] x [COLLATE [=] y]
    // 拆成两个逗号分隔的选项（DEFAULT CHARACTER SET=x, COLLATE=y）会直接语法错，
    // 于是"改了却保存不成功"。两个都变就一起写，只变一个就写一个。
    const csChanged = !!f.charset && f.charset !== optsBase.charset
    const coChanged = !!f.collation && f.collation !== optsBase.collation
    if (csChanged || coChanged) {
      let cs = ''
      if (csChanged) cs += `DEFAULT CHARACTER SET=${f.charset}`
      if (coChanged) cs += (cs ? ' ' : '') + `COLLATE=${f.collation}`
      p.push(cs)
    }
    if (has('supportsAutoIncrement') && String(f.autoIncrement).trim() !== '' && String(f.autoIncrement).trim() !== String(optsBase.autoIncrement).trim()) {
      p.push(`AUTO_INCREMENT=${String(f.autoIncrement).trim()}`)
    }
    if (has('supportsComment') && (f.comment || '') !== (optsBase.comment || '')) {
      p.push(`COMMENT='${sq(f.comment || '')}'`)
    }
    if (!p.length) return []
    return [`ALTER TABLE ${t} ${p.join(' ')}`]
  }
  // PG / Oracle 等：仅注释
  if (has('supportsComment') && (f.comment || '') !== (optsBase.comment || '')) {
    return [`COMMENT ON TABLE ${t} IS '${sq(f.comment || '')}'`]
  }
  return []
}

const pendingCount = computed(() => {
  const c = colStat.value
  const i = idxStat.value
  return c.added + c.modified + c.deleted + i.added + i.modified + i.deleted + (optsChanged.value ? 1 : 0)
})

// 完整变更脚本：先清理索引 → 字段变更 → 重建/新增索引 → 表选项
const scriptParts = computed(() => [
  ...idxDropParts(),
  ...colParts(),
  ...idxCreateParts(),
  ...optParts()
])
/** SQL 预览是否收起（预览已移到编辑区下方，可折叠；默认展开） */
const sqlCollapsed = ref(false)

/* ==================== SQL 预览高度（拖动 / 双击） ====================
   与 SQL 查询界面的结果区分隔条同一套手感：往上拖变高、双击在最大/最小之间切换。
   默认给 260px —— 预览本身是要读的，贴着内容高度只能看到两行。 */
const SQL_H_DEFAULT = 260
const SQL_H_MIN = 96   // 标题行 + 底部按钮行，再低就只剩两条杠
/** 预览区的 DOM 引用：算高度上限时要拿它所在容器的**实际**可用高度 */
const sqlAreaRef = ref(null)
/**
 * 高度上限：**所在容器的实际可用高度**，不再按视口 70% 拍脑袋。
 *
 * 原来写死视口 70%，于是往上拖到一定程度就顶住不动了（"到不了最上"）。
 * 改成量容器（.form-tab-body）：编辑区能收成 0，所以拖到最上时预览可以占满整块主体。
 * 下限仍是标题行 + 底栏 —— 再低按钮会被切掉；要"只剩标题行"请点标题行收起。
 */
const sqlMaxH = () => {
  const parent = sqlAreaRef.value?.parentElement
  const available = parent ? parent.clientHeight : Math.round(window.innerHeight * 0.7)
  return Math.max(SQL_H_MIN, available)
}
const sqlHeight = ref(SQL_H_DEFAULT)
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

const previewSql = computed(() => scriptParts.value.length ? scriptParts.value.join(';\n') + ';' : '')
const previewBody = computed(() => {
  if (ddlMode.value && ddlText.value) return ddlText.value
  if (previewSql.value) return previewSql.value
  return t('tdet.ddlEmpty')
})
const canCopy = computed(() => !!(ddlMode.value ? ddlText.value : previewSql.value))
const canSave = computed(() => !ddlMode.value && !!previewSql.value && issues.value.length === 0)
const previewTag = computed(() => {
  if (ddlMode.value && ddlText.value) return { text: t('tdet.stViewingDdl'), type: 'info' }
  if (issues.value.length) return { text: t('tdet.stNeedsFix'), type: 'warning' }
  return { text: t('tdet.stExecutable'), type: 'success' }
})
/** 是否显示这个状态标签：**以「实际生成了 SQL」为准**（previewSql），而不是待保存计数 ——
    两者可能不一致（如 Doris 表选项被计入待保存、但一条 ALTER 都生成不了），
    拿计数当依据会凭空挂个「可执行」（真机踩过）。正文「暂无改动」/ 空时不挂标签。 */
const showStateTag = computed(() =>
  ddlMode.value ? !!ddlText.value : !!previewSql.value
)

// ==================== 索引操作 ====================

let idxUid = 0
const idxRows = ref([])
const isLoadedIdx = (r) => r._state === 'loaded'
const allowIndexEdit = computed(() => ddlStyle.value !== 'clickhouse')

const buildIdxRows = () => {
  idxRows.value = (indexes.value || []).map((ix, i) => ({
    _uid: 'ix' + (++idxUid) + '_' + i, _state: 'loaded',
    name: ix.name, unique: !!ix.unique, indexType: ix.type || (indexTypes.value[0] || ''),
    columns: [...(ix.columns || [])],
    _orig: { name: ix.name, unique: !!ix.unique, indexType: ix.type || '', columns: [...(ix.columns || [])] }
  }))
}
const newIdxRow = () => ({
  _uid: 'ixn' + (++idxUid), _state: 'new',
  // 默认名按约定用 <字段名>_idx（原来是 idx_<字段名>）；没有可用字段时退回 col_idx
  name: ((displayCols.value.find(c => c.name && c.name.trim()) || {}).name || 'col').trim() + '_idx',
  unique: false, indexType: indexTypes.value[0] || '', columns: [], _orig: null
})
// 选好「包含字段」后，把默认索引名跟着改（只在名字还是空/自动生成时改，不覆盖用户手写的）
const onIdxColumnsChange = (r) => {
  const first = (r.columns || []).map(c => String(c || '').trim()).filter(Boolean)[0]
  if (!first) return
  const cur = String(r.name || '').trim()
  if (!cur || /_idx$/.test(cur)) r.name = first + '_idx'
}
const addIdxRow = () => { idxRows.value.push(newIdxRow()) }
const removeIdxRow = (r) => {
  if (r._state === 'new') idxRows.value = idxRows.value.filter(x => x !== r)
  else r._state = 'deleted'
}
const displayIdxRows = computed(() => idxRows.value.filter(r => r._state !== 'deleted'))

/* ==================== 表概况（跨库通用） ====================
   只统计"任何数据库都成立"的四项：行数 / 列数 / 索引数 / 主键。
   · 行数：走 /table-count（真实 COUNT(*)，树里用的同一接口；返回按表名索引的对象）。
     拿不到给 null → 显示「—」（**不显示 0**，0 会被读成"空表"）；
     正在请求是 undefined → 显示「统计中…」。
   · 列数 / 索引数 / 主键：完全来自已加载的结构，不发额外请求。 */
const rowCount = ref(undefined)
const rowCountText = computed(() => {
  if (rowCount.value === undefined) return t('tdet.counting')
  if (rowCount.value === null) return '—'
  const n = Number(rowCount.value)
  return Number.isFinite(n) ? n.toLocaleString('zh-CN') : String(rowCount.value)
})
const pkText = computed(() => {
  const pks = displayCols.value
    .filter(c => c.primaryKey && String(c.name || '').trim())
    .map(c => String(c.name).trim())
  return pks.length ? pks.join(', ') : '—'
})
const loadRowCount = async () => {
  const connId = props.conn?.id
  rowCount.value = undefined
  if (!connId || !props.database || !props.table) { rowCount.value = null; return }
  try {
    const counts = await getTableCounts(connId, props.database, [props.table])
    const v = counts ? counts[props.table] : null
    rowCount.value = (v === undefined || v === null || v === '') ? null : v
  } catch {
    rowCount.value = null
  }
}
onMounted(loadRowCount)
watch(() => [props.conn?.id, props.database, props.table], loadRowCount)

const idxChanged = (r) => {
  if (!isLoadedIdx(r) || !r._orig) return false
  const o = r._orig
  return (r.name.trim() !== o.name.trim()) || (!!r.unique !== o.unique) ||
    ((r.indexType || '') !== (o.indexType || '')) ||
    (JSON.stringify([...(r.columns || [])].sort()) !== JSON.stringify([...(o.columns || [])].sort()))
}
const idxStat = computed(() => {
  let added = 0, modified = 0, deleted = 0
  for (const r of idxRows.value) {
    if (r._state === 'new') added++
    else if (r._state === 'deleted') deleted++
    else if (idxChanged(r)) modified++
  }
  return { added, modified, deleted }
})
const idxRowClass = ({ row }) => {
  if (row._state === 'new') return 'row-is-new'
  if (row._state === 'loaded' && idxChanged(row)) return 'row-is-modified'
  return ''
}

const createIndexSql = (f) => {
  const cols = (f.columns || []).map(x => qt(x)).join(', ')
  const unique = f.unique ? 'UNIQUE ' : ''
  if (ddlStyle.value === 'mysql') {
    const ft = f.indexType === 'FULLTEXT' ? 'FULLTEXT ' : (f.indexType === 'SPATIAL' ? 'SPATIAL ' : '')
    const using = (f.indexType === 'BTREE' || f.indexType === 'HASH') ? ` USING ${f.indexType}` : ''
    return `CREATE ${unique}${ft}INDEX ${qt(f.name)}${using} ON ${qt(props.table)} (${cols})`
  }
  if (ddlStyle.value === 'pg') {
    return `CREATE ${unique}INDEX ${qt(f.name)} ON ${qt(props.table)} USING ${f.indexType || 'btree'} (${cols})`
  }
  return `CREATE ${unique}INDEX ${qt(f.name)} ON ${qt(props.table)} (${cols})`
}

/**
 * 方言有没有「改索引名」的原生语句。有就走 RENAME，没有才退回「先删再建」。
 *
 * 为什么在意「只改名字」这一种：MySQL 上「先删再建」有个硬伤 —— 索引若正撑着
 * AUTO_INCREMENT 列（本应用自己加的 `_上游_auto_<列>` 就是这种），DROP 的那一瞬间
 * 自增列就没有键了，MySQL 拒绝整条语句：
 *   「Incorrect table definition. there can be only one auto column and it must be defined as a key」
 * （errorCode=1075）—— 于是「改个索引名」变成永远保存失败。
 * `RENAME INDEX` 是 MySQL 5.7+ 的原生语法：索引一直在、自增列的键始终成立。
 */
const canRenameIndex = computed(() => ['mysql', 'pg', 'oracle'].includes(ddlStyle.value))

/** 是否**只改了名字**（列、唯一性、索引类型都没动）—— 这种情况才适合直接改名 */
const idxPureRename = (r) => {
  if (!isLoadedIdx(r) || !r._orig) return false
  const o = r._orig
  const name = String(r.name || '').trim()
  const oldName = String(o.name || '').trim()
  if (!name || name === oldName) return false
  // PRIMARY 不能改名（MySQL 上会直接报错），这类保持原来的删建路径
  if (oldName.toUpperCase() === 'PRIMARY') return false
  if (!!r.unique !== !!o.unique) return false
  if ((r.indexType || '') !== (o.indexType || '')) return false
  return JSON.stringify([...(r.columns || [])].sort()) === JSON.stringify([...(o.columns || [])].sort())
}

/** 改索引名的原生语句（调用前先用 canRenameIndex 判断方言） */
const renameIndexSql = (from, to) => {
  const t = qt(props.table)
  if (ddlStyle.value === 'mysql') return `ALTER TABLE ${t} RENAME INDEX ${qt(from)} TO ${qt(to)}`
  return `ALTER INDEX ${qt(from)} RENAME TO ${qt(to)}`
}
const autoIncrementCol = () => (columns.value || []).find(c => /auto_increment/i.test(String(c.extra || '')))

// MySQL 删除包含自增列的主键时，需先给自增列补一个索引，否则 DROP PRIMARY KEY 会报 1075
const mySqlDropPrimaryKeySql = () => {
  const autoCol = autoIncrementCol()
  if (!autoCol) return `ALTER TABLE ${qt(props.table)} DROP PRIMARY KEY`

  // 若已有其他索引包含该自增列，可直接删主键
  const hasOtherIdx = idxRows.value.some(r =>
    r._state !== 'deleted' &&
    String(r.name).toUpperCase() !== 'PRIMARY' &&
    (r.columns || []).some(c => String(c).trim().toLowerCase() === String(autoCol.name).trim().toLowerCase())
  )
  if (hasOtherIdx) return `ALTER TABLE ${qt(props.table)} DROP PRIMARY KEY`

  const fallback = qt('_上游_auto_' + String(autoCol.name).replace(/[^a-zA-Z0-9_]/g, '').slice(0, 50))
  const colName = qt(autoCol.name)
  return `ALTER TABLE ${qt(props.table)} ADD INDEX ${fallback} (${colName});\n` +
    `ALTER TABLE ${qt(props.table)} DROP PRIMARY KEY`
}

const dropIndexSql = (name) => {
  if (ddlStyle.value === 'mysql') {
    if (String(name).toUpperCase() === 'PRIMARY') return mySqlDropPrimaryKeySql()
    return `ALTER TABLE ${qt(props.table)} DROP INDEX ${qt(name)}`
  }
  if (ddlStyle.value === 'mssql') return `DROP INDEX ${qt(name)} ON ${qt(props.table)}`
  return `DROP INDEX ${qt(name)}`
}

const rebuildIndex = async (row) => {
  try {
    await ElMessageBox.confirm(
      t('tdet.rebuildIndexBody', { name: (row.name || '') }),
      t('tdet.rebuildIndex'),
      { confirmButtonText: t('tdet.rebuildConfirm'), cancelButtonText: t('common.cancel'), type: 'warning' }
    )
  } catch {
    return
  }
  saving.value = true
  try {
    let sql
    if (ddlStyle.value === 'pg') {
      sql = `REINDEX INDEX ${qt(row.name)}`
    } else if (ddlStyle.value === 'oracle') {
      sql = `ALTER INDEX ${qt(row.name)} REBUILD`
    } else if (ddlStyle.value === 'mysql') {
      const isPk = String(row.name).toUpperCase() === 'PRIMARY'
      const cols = (row.columns || []).map(c => qt(c)).join(', ')
      // MySQL 重建主键时，若主键包含自增列，直接 DROP PRIMARY KEY 会导致自增列失去索引而报 1075。
      // 需要先把自增属性摘掉，重建完主键后再恢复。
      const autoCol = (columns.value || []).find(c => /auto_increment/i.test(String(c.extra || '')))
      const includesAuto = isPk && autoCol && (row.columns || []).some(c =>
        String(c).trim().toLowerCase() === String(autoCol.name).trim().toLowerCase())
      if (includesAuto) {
        // MySQL 自增列必须始终被索引。重建主键时先给自增列加一个临时唯一索引，
        // 再交换主键，最后删掉临时索引，避免 MODIFY COLUMN 导致注释/默认值等属性丢失。
        const tmpName = qt('_上游_tmp_ai_' + String(autoCol.name).replace(/[^a-zA-Z0-9_]/g, '').slice(0, 50))
        const colName = qt(autoCol.name)
        sql = `ALTER TABLE ${qt(props.table)} ADD UNIQUE INDEX ${tmpName} (${colName});\n` +
          `ALTER TABLE ${qt(props.table)} DROP PRIMARY KEY;\n` +
          `ALTER TABLE ${qt(props.table)} ADD PRIMARY KEY (${cols});\n` +
          `ALTER TABLE ${qt(props.table)} DROP INDEX ${tmpName}`
      } else {
        const unique = row.unique ? 'UNIQUE ' : ''
        const ft = row.indexType === 'FULLTEXT' ? 'FULLTEXT ' : (row.indexType === 'SPATIAL' ? 'SPATIAL ' : '')
        const using = (row.indexType === 'BTREE' || row.indexType === 'HASH') ? ` USING ${row.indexType}` : ''
        sql = `ALTER TABLE ${qt(props.table)} DROP INDEX ${qt(row.name)};\n` +
          `CREATE ${unique}${ft}INDEX ${qt(row.name)}${using} ON ${qt(props.table)} (${cols})`
      }
    } else {
      ElMessage.warning(t('tdet.rebuildUnsupported'))
      return
    }
    const r = await alterTable(props.conn.id, props.database, sql)
    if (!r.success) throw new Error(r.message || t('tdet.execFailed'))
    ElMessage.success(t('tdet.rebuilt'))
    await load()
  } catch (e) { ElMessage.error(t('tdet.rebuildFailed', { detail: (e?.message || e) })) }
  finally { saving.value = false }
}

// ==================== 表选项（基本信息） ====================

// sortingKey / partitionKey 只有 ClickHouse 会填（来自后端 system.tables，只读展示）；
// engine 对 ClickHouse 也由后端给出真实值（MergeTree 等），用于只读回显。
const tableForm = ref({ engine: '', charset: '', collation: '', autoIncrement: '', comment: '', sortingKey: '', partitionKey: '', dorisModel: '', dorisDistCol: '', dorisBuckets: '', dorisReplication: '' })

/** 解析 Doris 建表 DDL 中的只读属性：数据模型 / 分桶列 / 分桶数 / 副本数（均建于表创建时，不可改）。
 *  顺带把**表注释**也解析出来 —— Doris 的表清单接口不回 comment，表注释行要靠这里回显。 */
const parseDorisDdl = (ddl) => {
  const out = { dorisModel: '', dorisDistCol: '', dorisBuckets: '', dorisReplication: '', comment: '' }
  const s = String(ddl || '')
  if (!s) return out
  const m = s.match(/\b(DUPLICATE|UNIQUE|AGGREGATE)\s+KEY/i)
  if (m) out.dorisModel = m[1].toUpperCase() + ' KEY'
  const d = s.match(/DISTRIBUTED\s+BY\s+(HASH|RANDOM)\s*(?:\(([^)]*)\))?/i)
  if (d) out.dorisDistCol = String(d[1]).toUpperCase() === 'RANDOM' ? 'RANDOM' : String(d[2] || '').replace(/[`\s]/g, '')
  const b = s.match(/\bBUCKETS\s+(\d+)/i)
  if (b) out.dorisBuckets = b[1]
  // 副本数：老版本用 replication_num，新版本(3.x)用 replication_allocation="tag.location.default: N"
  const rn = s.match(/["']?replication_num["']?\s*=\s*["']?(\d+)/i)
  const ra = s.match(/["']?replication_allocation["']?\s*=\s*["']?[^"']*?(\d+)/i)
  if (rn) out.dorisReplication = rn[1]
  else if (ra) out.dorisReplication = ra[1]
  // 表级注释取**最后一个** COMMENT：列注释都挂在列定义里（在前），表级注释
  // 在右括号之后、PROPERTIES 之前。Doris 的 DDL 用**单引号**（列注释同），双引号是
  // PROPERTIES 里的键值 —— 两种都要认。
  let cm = null
  const re = /COMMENT\s+(?:"([^"]*)"|'([^']*)')/gi
  let mm
  while ((mm = re.exec(s))) cm = mm[1] !== undefined ? mm[1] : mm[2]
  if (cm !== null) out.comment = cm
  return out
}
/** 当前表的 Doris 分桶列名（RANDOM 分桶 / 非 Doris 表返回 ''，比对用小写） */
const dorisDistColName = () => {
  const c = String(tableForm.value.dorisDistCol || '').trim()
  return (!c || /^random$/i.test(c)) ? '' : c.toLowerCase()
}
/** 本次变更里被跳过的 Doris 分桶列（ALTER 生成不了，保存成功后要提示用户） */
const dorisDistSkipped = computed(() => {
  if (ddlStyle.value !== 'doris') return []
  const dist = dorisDistColName()
  if (!dist) return []
  return colRows.value
    .filter(r => isLoadedRow(r) && colChanged(r) && String(r.name || '').trim().toLowerCase() === dist)
    .map(r => String(r.name || '').trim())
})
const onCharsetChange = () => {
  if (collations.value.length && !collations.value.includes(tableForm.value.collation)) {
    tableForm.value.collation = collations.value[0]
  }
}

// ==================== 数据加载 ====================

let featuresLoaded = false
const ensureFeatures = async () => {
  if (!featuresLoaded) { await loadFeatures(); featuresLoaded = true }
}

const load = async () => {
  loading.value = true
  ddlMode.value = false
  ddlText.value = ''
  try {
    await ensureFeatures()
    const [cols, idxRes, tables] = await Promise.all([
      listColumns(props.conn.id, props.database, props.table),
      listIndexes(props.conn.id, props.database).catch(() => []),
      listTables(props.conn.id, props.database).catch(() => [])
    ])
    columns.value = cols || []
    indexes.value = (idxRes || []).filter(i => i.table === props.table)
    tablesRef.value = tables || []
    buildColRows()
    buildIdxRows()
    // Doris：列的**聚合类型**与"是否 key 列"只有 `DESC … ALL` 才给（JDBC 的列元数据里没有），
    // 而 `MODIFY COLUMN` 必须把聚合类型原样带回去 —— 不带就被当成"改聚合类型"，
    // Doris 直接报 `Can not change aggregation type`（实测：改注释也会撞上）。
    // 用现成的 executeSql 取一次（`mssqlPkName()` 也是这么做的）；拿不到就留空，
    // 生成器会退化成不带聚合类型的样子，至少不会生成一句注水的 SQL。
    if (isDoris.value) {
      try {
        // 两种取法都试：`DESC … ALL` 会多给 AggregationType（我们要的就是它）；
        // 某些版本/权限下它不可用，退到 `SHOW FULL COLUMNS`（至少能拿到 Field，key 列能判出来）。
        // 失败**不再静默吞掉**：控制台打出失败的那条 SQL 与原因 —— 上一版就是静默回退，
        // 结果预览里没带 KEY、只能靠猜是"查询失败"还是"解析没对上"。
        // 取法按"实际能通"的顺序排：**不带引号**的 `desc t all` 是实测跑通过的
        // （引号形式在某些 Doris 版本上会被 DESC 拒绝）；再退到 `show full columns`。
        const plain = String(props.table || '').trim()
        let attrRows = []
        for (const sql of [`desc ${plain} all`, `desc ${qt(plain)} all`, `show full columns from ${plain}`]) {
          try {
            const res = await executeSql(props.conn.id, sql, props.database, null, null, null, null, true)
            const rows = (res && res.rows) || []
            if (rows.length) { attrRows = rows; break }
          } catch (err) {
            console.warn('[doris] 列属性查询失败：', sql, err)
          }
        }
        if (!attrRows.length) {
          // 这里**故意用弹窗而不是 console.warn**：控制台默认只显示 Error 级别，
          // 上一版只打 warn，界面上看不出"到底是查询失败还是列名没对上"，只能靠猜。
          ElMessage.warning(t('tdet.dorisNoColProps'))
        }
        const pick = (row, key) => {
          const found = Object.keys(row).find(x => x.toLowerCase() === key.toLowerCase())
          return found ? String(row[found] ?? '').trim() : ''
        }
        const attrMap = new Map()
        for (const row of attrRows) {
          const col = pick(row, 'Field').toLowerCase()
          if (col) {
            attrMap.set(col, {
              // 实测（`desc ods.ppl_loan_case all`）：Doris 返回的列是
              // IndexName / IndexKeyType / Field / Type / InternalType / Null / Key / Default /
              // Extra / Visible / … —— **没有** AggregationType，聚合类型在 `Extra` 里
              // （NONE / SUM / REPLACE / MAX…）；是不是 key 列看 `Key` = true/false。
              agg: pick(row, 'Extra') || pick(row, 'AggregationType'),
              key: pick(row, 'Key').toLowerCase() === 'true',
            })
          }
        }
        // 存进**表**而不是直接写行对象：行对象会被 `buildColRows()` 重建，
        // 写上去的字段活不到生成 SQL 那一刻（实测：诊断弹窗里是 true，预览里却没有 KEY）。
        dorisColAttrs.clear()
        for (const [col, hit] of attrMap) dorisColAttrs.set(col, hit)
        // 顺手用同一次 `DESC … ALL` 的 `Type` 列**补全类型文本**：
        // Doris 经 JDBC 给的 TYPE_NAME 常常只有 `decimal`，精度/长度全丢了 ——
        // 于是界面上「长度」空着、被校验拦住，生成的 DDL 也少参数。
        // `DECIMALV3(18,6)` 要还原成 `DECIMAL(18,6)`：前端的类型解析用 `[A-Za-z_]+` 取类型名，不含数字。
        const typePatched = (() => {
          let changed = false
          for (const c of (columns.value || [])) {
            const col = String(c.name || '').trim().toLowerCase()
            const text = pick((attrRows.find(row => pick(row, 'Field').toLowerCase() === col) || {}), 'Type').replace(/V3\s*\(/i, '(')
            if (text && text !== c.type) { c.type = text; changed = true }
          }
          return changed
        })()
        // 类型变了要重建行对象；重建后 watch 会自动补属性，这里再显式补一次保证同步可见
        if (typePatched) buildColRows()
        applyDorisColAttrs()
      } catch { /* 拿不到就留空 */ }
    }
    // 表选项回显（先校正排序规则，再对基线快照，避免加载即产生差异）
    const t = (tables || []).find(x => x.name === props.table)
    const coll = t?.charset || ''
    tableForm.value = {
      engine: t?.engine || (engines.value[0] || ''),
      charset: coll ? String(coll).split('_')[0] : (charsets.value[0] || ''),
      collation: coll || '',
      autoIncrement: '',
      comment: t?.comment || '',
      // ClickHouse 的排序键 / 分区键（以及 engine）：后端从 system.tables 回填，只读展示
      sortingKey: t?.sortingKey || '',
      partitionKey: t?.partitionKey || '',
      dorisModel: '', dorisDistCol: '', dorisBuckets: '', dorisReplication: ''
    }
    // Doris 建表属性（模型/分桶/副本）建表后不可改：从 DDL 解析后只读回显
    if (isDoris.value) {
      try {
        const ddlRes = await getTableDdl(props.conn.id, props.database, props.table)
        const parsed = parseDorisDdl(ddlRes?.ddl || '')
        // 表清单的 comment 是数据库现状的权威值 —— DDL 解析（取「最后一个 COMMENT」）
        // 偶尔会因 DDL 形状差异取到别的注释，不能让它覆盖现状（否则打开就误报「注释有改动」）
        parsed.comment = t?.comment || parsed.comment
        Object.assign(tableForm.value, parsed)
        // DDL 回填后重照基线快照：保证刚打开时基线与显示值一致（不产生假差异）
        optsBase.comment = tableForm.value.comment
      } catch { /* DDL 取不到时留空 */ }
    }
    onCharsetChange()
    const f = tableForm.value
    optsBase.engine = f.engine
    optsBase.charset = f.charset
    optsBase.collation = f.collation
    optsBase.autoIncrement = ''
    optsBase.comment = f.comment
    if (ddlStyle.value === 'mssql') mssqlPk.value = await mssqlPkName()
  } catch (e) {
    ElMessage.error(t('tdet.loadSchemaFailed', { detail: (e?.message || e) }))
  } finally {
    loading.value = false
  }
}

const resetAll = async () => {
  ddlMode.value = false
  ddlText.value = ''
  await load()
  ElMessage.success(t('tdet.resetDone'))
}

// ==================== 保存 / 复制 ====================

/**
 * 生产库保护：环境标记为 PROD 的连接上，改结构的 DDL 也要确认一次。
 *
 * 与 SQL 页那道闸同源（看连接不看语句）：结构页保存的**全是写操作**，所以这里不做语句判定，
 * 只要连接是 PROD 就确认 —— 一次 DROP COLUMN 的后果不比一条 DELETE 小。
 */
const confirmProdWrite = async (summary) => {
  if (String(props.conn?.env || '').toUpperCase() !== 'PROD') return true
  try {
    await ElMessageBox.confirm(
      t('tdet.prodAlterBody', { summary }),
      t('tdet.prodAlterTitle'),
      { type: 'warning', confirmButtonText: t('tdet.prodAlterConfirm'), cancelButtonText: t('common.cancel'), closeOnClickModal: false, closeOnPressEscape: false }
    )
    return true
  } catch (e) {
    return false
  }
}

const save = async () => {
  if (!pendingCount.value) return ElMessage.info(t('tdet.noChangesToSave'))
  const probs = issues.value
  if (probs.length) return ElMessage.warning(probs[0])
  const script = scriptParts.value.join(';\n') + ';'
  if (!script.trim()) return ElMessage.info(t('tdet.noChangesToSave'))
  // 脚本剥掉注释行后一条语句都不剩 = 唯一的改动是 Doris 分桶列（生成不了 ALTER）：
  // 没必要再发一遍必然失败/无意义的请求，直接把原因说清楚
  const executable = script.split('\n').filter(l => l.trim() && !l.trim().startsWith('--')).join('\n')
  if (!executable.trim()) return ElMessage.warning(t('tdet.dorisDistSkip', { col: dorisDistSkipped.value.join(', ') || '—' }))
  // 生产库保护：DDL 前二次确认（读一下脚本首行，让用户知道要动什么）
  const firstLine = script.split('\n').map(s => s.trim()).filter(Boolean)[0] || ''
  if (!(await confirmProdWrite(firstLine))) return
  saving.value = true
  try {
    const res = await alterTable(props.conn.id, props.database, script)
    if (!res.success) throw new Error(res.message || t('tdet.execFailed'))
    ElMessage.success(t('tdet.savedOk'))
    // Doris 分桶列的修改生成不了 ALTER（数据库一律拒绝），保存成功后单独说清楚
    if (dorisDistSkipped.value.length) {
      ElMessage.warning(t('tdet.dorisDistSkip', { col: dorisDistSkipped.value.join(', ') }))
    }
    await load()
  } catch (e) {
    ElMessage.error(t('tdet.saveFailed', { detail: (e?.message || e) }))
  } finally {
    saving.value = false
  }
}

const copySql = async () => {
  const text = previewBody.value
  if (!text || text.startsWith(t('tdet.noChangesComment'))) return
  try {
    await navigator.clipboard.writeText(text)
    ElMessage.success(t('tdet.sqlCopied'))
  } catch (e) { ElMessage.warning(t('tdet.copyFailedDetail', { detail: (e?.message || e) })) }
}

const showDdl = async () => {
  if (ddlMode.value) { ddlMode.value = false; return }
  if (ddlText.value) { ddlMode.value = true; return }
  try {
    const res = await getTableDdl(props.conn.id, props.database, props.table)
    ddlText.value = res?.ddl || ''
    ddlMode.value = true
  } catch (e) { ElMessage.error(t('tdet.fetchDdlFailed', { detail: (e?.message || e) })) }
}

// 一旦产生结构改动，自动退出“原始 DDL”模式回到可编辑的 ALTER 预览
watch(pendingCount, (n) => { if (n > 0 && ddlMode.value) ddlMode.value = false })

onMounted(async () => {
  await load()
  // 首屏就把预览高度收敛到容器可用高度内：窗口不高时默认 260px 会顶出去
  sqlHeight.value = Math.min(sqlHeight.value, sqlMaxH())
})
watch(() => [props.conn?.id, props.database, props.table], () => {
  tab.value = 'basic'
  featuresLoaded = false
  features.value = {}
  columns.value = []
  colRows.value = []
  indexes.value = []
  idxRows.value = []
  tableForm.value = { engine: '', charset: '', collation: '', autoIncrement: '', comment: '', dorisModel: '', dorisDistCol: '', dorisBuckets: '', dorisReplication: '' }
  optsBase.engine = optsBase.charset = optsBase.collation = optsBase.autoIncrement = optsBase.comment = ''
  mssqlPk.value = ''
  load()
})
</script>

<style scoped>
/* ===== 页面骨架（与"新建表"一致：顶栏 + 左编辑右预览） ===== */
.table-edit-page {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--dc-bg-deep);
  /* 整块区域四周留 5px：三张卡（标题行 / 表格卡 / SQL 预览）都不再贴着面板边缘。
     这一圈留白是必须的 —— 贴着边缘时，卡片的 8px 圆角会被面板边线"压平"，
     看起来就像没做圆角（用户反馈"都加圆角啊"正是这个）。 */
  padding: 5px;
  box-sizing: border-box;
}
/* 顶部标题行：**独立成一张小卡**（四角 8px 圆角 + 完整边框 + 同款轻阴影），
   与下面的表格卡之间留 5px 缝隙（竖直间距见 .form-area 的 padding，
   四周留白见 .table-edit-page 的 padding）。
   整块区域自上而下并列三张小卡：标题行 / 表格卡 / SQL 预览，外观完全一致。 */
.form-tab-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  /* 独立成一张卡（与表格卡之间留 5px 缝隙）：四角同圆角 + 完整边框，
     不再充当"表格卡的标题行"，所以下边框也用常规色，不用较浅的 soft。 */
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
  min-width: 0;
}
.header-title .header-text { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.header-icon { color: var(--dc-primary); flex-shrink: 0; }
.header-actions { display: flex; gap: 6px; flex-shrink: 0; }

/* 编辑区在上、SQL 预览在下（原来预览是右侧 380px 固定栏）。
   纵向堆叠后两者都按 flex 分配高度，因此都必须是可收缩的（min-height: 0）。 */
.form-tab-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  min-height: 0;
}
.form-area {
  flex: 1;
  min-width: 0;
  /* 纵向堆叠时同样要 min-height: 0，否则内容会把下方的 SQL 预览挤出可视区 */
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  /* 左右**不留内边距**：下方的预览区（.sql-area）边框与底栏是通栏的，
     编辑区若两侧各缩进 12px，页签卡就比它窄一圈（实测 890 vs 914），
     看起来像两块宽度不同的面板。去掉左右缩进后两者同宽（实测同为 914），
     卡内字段内容也自然与预览正文对齐。上下留白保留。 */
  /* 上面留 5px = 标题行与表格卡之间的间距。
     下面**不留**：表格卡与 SQL 预览块之间的缝，由中间那条 .sql-resizer 自己撑开（6px）——
     两边都留的话缝会变成 5+6=11px，细条就落在缝的偏下位置，不是"缝的中间"了。 */
  /* 这块区域**不铺底色**（原先是 --dc-bg 的浅灰 #EEF1F6，用户要求去掉）：
     不铺底就透出下层卡片的底色，与表格卡自身一致，整块看起来是一张白卡。 */
  padding: 5px 0 0;
}
.form-area .table-tabs { flex: 1; }

/* ===== 页签条（参考 Navicat）：纯文字，激活项加粗加深 =====
   去掉 border-card 之后：没有外框、没有灰底标题带，页签退成一行文字 + 一条细线，
   内容区（表格卡片）自己承担边界 —— 与参考的"文字页签 + 独立表格框"一致。 */
.table-tabs {
  display: flex;
  flex-direction: column;
  min-height: 0;
  /* 关键：**页签条 + 工具栏 + 表格是同一张卡**（与参考一致）。
     整块只画一层边框/圆角/卡片底，页签条相当于这张卡的"标题行"，
     表格与工具栏都不再各自套边框 —— 否则看起来就是三个独立块拼起来的。 */
  /* 独立的一张卡：四角 8px 圆角 + 完整边框。
     与上方的 .form-tab-header 之间留 5px 缝隙（见 .form-area 的 padding-top）。 */
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  /* 白底主卡 + 轻阴影：整块浮在页面上，边界清楚但不压手 */
  background: var(--dc-bg-card);
  box-shadow: 0 1px 3px rgba(16, 24, 40, .06), 0 1px 2px rgba(16, 24, 40, .04);
  overflow: hidden;
  /* 操作条要绝对定位到**页签这一行**，需要把"定位祖先"抬到这张卡上来
     （Element 的 .el-tabs__content 自带 position: relative，会在它那层就把锚点吃掉）。 */
  position: relative;
}
/* Element 给 content 预置了 position: relative + overflow: hidden：
   前者会把操作条的定位锚点拉回内容区（于是它落在页签**下面**），
   后者会把上浮出去的部分裁掉。这里两条都放开 —— 卡片自身 overflow: hidden 仍兜底，
   页面内部（.tab-inner）各自也都有 overflow: hidden。 */
/* 内容区**不留底色**：Element 给 border-card 的内容区铺了一层
   `background: var(--el-color-info-light-9)`（本主题实测 rgb(228,233,242) 的淡灰蓝），
   整块表格区被它染成一块灰蓝；表体/表头都是白的，于是四周、下方透出这块底色，
   看着就像"表格底下压着一个背景块"。这里显式盖回透明，整块卡片恢复纯白。 */
.table-tabs :deep(.el-tabs__content) {
  flex: 1; padding: 0; position: static; overflow: visible;
  background: transparent !important;
}
.table-tabs :deep(.el-tabs__header) {
  /* 页签要"住进表格的第一行"：这里把 Element 的表头压成 0 高（不占位），
     页签改为绝对定位，覆盖在表格首行（那一行 35px 的空行）之上 ——
     于是它看起来就是表格的第一行，而不是表格上面另起的一行。 */
  height: 0;
  margin: 0;
  padding: 0;
  border-bottom: none;
  background: transparent;
  /* Element 给 header 设了 position: relative，会把绝对定位的页签"基准"带偏，
     改成 static 之后，页签以卡片（.table-tabs，relative）为准。 */
  position: static;
}
/* 页签**自己**坐在一个圆角框里，只占左边 —— 不是铺满整行。
   （之前把它当成通栏色带，就成了"另起一块"。） */
/* 让绝对定位的页签以"卡片"为基准：nav-wrap 也不参与定位 */
.table-tabs :deep(.el-tabs__nav-wrap) { height: 0; position: static; }
.table-tabs :deep(.el-tabs__nav) {
  /* 页签住进表格第一行：绝对定位到卡片顶部（= 表格首行），高 35px 与那一行一致，
     align-items: center 让三段文字在行内垂直居中。 */
  position: absolute;
  top: 0;
  left: 12px;
  height: 35px;
  display: flex;
  align-items: center;
  background: transparent;
  border: none;
  padding: 0;
  z-index: 5;
}
.table-tabs :deep(.el-tabs__nav-wrap::after) { display: none; }  /* 去掉 Element 自带的下划双层线 */
/* 参考里的激活页签是**主色文字**（不带下划线），所以把这条指示条藏掉 */
.table-tabs :deep(.el-tabs__active-bar) { display: none; }
.table-tabs :deep(.el-tabs__item) {
  /* height 与 border-radius **都必须 !important**：MainView 的全局 .dc-tabs 规则
     同样命中我们这排页签（本组件就渲染在那棵树下），实测把 height 压成 34px、
     border-radius 压成 0px —— 于是激活项那个白色滑块变成了**方角**（看着就不圆）。
     这里按设计值锁死：高 30px（在 35px 的空行里留出上下呼吸）、圆角 6px。 */
  height: 30px !important;
  line-height: 30px;
  padding: 0 14px !important;
  font-size: 13px;
  color: var(--dc-text-mid);
  border-radius: 6px !important;
  /* 必须 !important：MainView 里有一条全局规则
       .dc-tabs .el-tabs__item.is-active { background: var(--dc-primary-wash) !important }
     而本组件正渲染在那棵树下（任何层级的后代都被匹配到），不带就压不过它。 */
  background: transparent !important;
  transition: background .15s ease, color .15s ease, box-shadow .15s ease;
}
.table-tabs :deep(.el-tabs__item:hover) { color: var(--dc-text-strong); }
.table-tabs :deep(.el-tabs__item.is-active) {
  color: var(--dc-primary) !important;
  font-weight: 600;
  background: var(--dc-bg-card) !important;
  box-shadow: 0 1px 2px rgba(16, 24, 40, .10);
}
.table-tabs :deep(.el-tab-pane) { height: 100%; }
.tab-inner { height: 100%; padding: 10px 12px; display: flex; flex-direction: column; overflow: hidden; }
/* 表格那两个页签**不留内边距**：.tab-inner 的 12px 左右 + 10px 下留白会在表格外侧撑出
   一圈白边（表格的框线离卡片边框还有 14px 的白色间隙）。去掉后表格直接贴到卡片内缘，
   左右两侧的"框线"就是卡片自身那条边框，下方由最后一行的下边框收口。
   （基本信息页签保留内边距 —— 那里的表单需要留白。） */
/* 注意选择器写法：模板里是 `<div class="tab-inner col-tab">` —— 两个类在**同一个元素**上，
   所以要用 `.tab-inner.col-tab`（同伴），写成 `.col-tab .tab-inner`（后代）匹配不到。 */
.tab-inner.col-tab,
.tab-inner.idx-tab { padding: 0; }
/* 表格自己的"空行"（thead 里的一行 spacer）：**白底**（和数据行一致，才是"空行"），
   随表头吸顶、高度 14px、不画线 —— 之前用表头的浅底色，会和表头糊成一条，
   看起来像把表头加高了。 */
.field-table tr.thead-spacer th {
  /* 与标题行（表头）等高：表头行实测 35px（9px 上下内边距 + 一行文字） */
  height: 35px;
  padding: 0;
  border-bottom: none;
  background: var(--dc-bg-card);
}
/* 表头是**两行**（空行 + 标题行），滚动时两行都要固定：
   空行吸附在 top:0（默认规则），标题行则吸附到空行**下方** —— 35px。
   两行若都写 top:0，滚动时标题行会盖住空行，看起来就只固定了一行。 */
.field-table thead tr:last-child th { top: 35px; }
/* 有了表格内的空行，页签行与表格之间就不再需要额外的 padding */
.col-tab,
.idx-tab { padding-top: 0; }
/* 表格容器**贴着内容长**，不再用 flex:1 撑满整块面板：
   12 行的表原来会在下面拖出一大片带边框的空白（截图里最扎眼的就是它），
   像没加载完。现在高度随行数走；表比面板高时才出现滚动条（表头依旧吸顶）。 */
.col-tab .field-table-wrap {
  flex: 0 1 auto;
  max-height: 100%;
  /* 列宽总和（882px）已小于容器（886px），但 separate 边框模式下表格自身
     仍会比容器宽出约 2px，平白多出一条横向滚动条 —— 这里不产生横向滚动。
     代价：窗口极窄（< 860px）时超出部分会被裁掉，可接受。 */
  overflow-x: hidden;
  overflow-y: auto;
  /* **不铺底色、不带圆角、左右也不单画线**：表格已贴满卡片，四周的框线由
     卡片自身那条边框承担（左右分别是它的左右边，上边是它的上边，
     下边由最后一行的下边框收口）。之前单画的左右线会紧挨着卡片的边框，
     变成 2px 粗线。 */
  border: none;
  border-radius: 0;
  background: transparent;
  min-height: 0;
}
.col-tab .card-actions,
.idx-tab .card-actions {
  display: flex;
  align-items: center;
  /* 按钮恒定贴右：说明文字删掉后，左侧只剩变更统计（常常是空的），
     靠 justify-content 收口，比依赖某个子元素把按钮"顶"过去更稳 */
  justify-content: flex-end;
  gap: 8px;
  /* 固定高度的浅色操作条：按钮在条内**垂直居中**，上下留白必然相等
     （24px 的按钮落在 40px 的条里 = 上 8 / 下 8）。
     之前用 padding 拼"对称"只差 4px，等于看不出来 —— 换成有形状的条子，
     既把"上下不一样"这件事从根上消掉，也让操作行与下面的表格分开。 */
  /* **上浮到页签这一行**（参考的布局：左边页签、右边搜索/新增字段）。
     绝对定位 + 与页签等高的 34px，行内 align-items: center 保证纵向居中；
     max-width 给左侧页签留出位置，窄屏时也不会叠到页签上（提示文字自己省略号）。
     这样下面直接就是表格，不再单独占一行 —— 这才是"页签与表格像一个整体"。 */
  position: absolute;
  /* 与页签分段控件同高同层：控件 = 30px 页签 + 3×2 padding = 36px，
     操作条取同样的 36px，居中后与控件中线重合。 */
  top: 0;
  /* 右缩进 13px：表格贴满卡片后，容器右缘 = 卡片内缘，纵向滚动条占最后 8px
     （见 styles/index.css 的 ::-webkit-scrollbar { width: 8px }）→ 13px 刚好让按钮
     右缘落在滚动条左侧几个像素处，既不压住滚动条，也与表格右缘基本齐平。 */
  right: 13px;
  height: 35px;
  /* 左侧留给页签（实测页签框宽 275px + 12px 缩进）。原来留 300px 太保守：
     提示 + 搜索框 + 按钮 需要约 640px，可用宽度不够时搜索框会被挤窄（截图里右端被切）。
     现在放宽到 286px（左侧仍不会被页签压到），搜索框用 flex-shrink:0 保底不被压缩。 */
  max-width: calc(100% - 286px);
  padding: 0;
  margin: 0;
  border-radius: 0;
  background: transparent;
  z-index: 4;
  flex-shrink: 0;
}
.card-actions .card-left {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-right: auto;
  min-width: 0;
  overflow: hidden;
}
.card-actions .sum-tags {
  display: inline-flex;
  gap: 4px;
  align-items: center;
  flex-shrink: 0;
}

/* 索引空状态：虚线圆角卡片居中，替代"表头 + 一大片空白"的默认空表 */
.idx-empty {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  border: 1px dashed var(--dc-border);
  border-radius: 8px;
  background: var(--dc-bg-card);
}
.idx-empty-icon { color: var(--dc-text-dim); opacity: .65; }
.idx-empty-title { font-size: 13.5px; font-weight: 600; color: var(--dc-text-mid); }
.idx-empty-text { font-size: 12.5px; color: var(--dc-text-dim); margin-bottom: 8px; }
/* 索引表格容器：外层 .table-tabs 已是一张卡，这里不再套第二层边框 */
.idx-body {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  border: none;
  border-radius: 0;
  background: transparent;
  /* 顶部留 35px：与字段表那行"空行"同高 —— 页签与工具条（绝对定位 top:0）
     就住在卡片最上面那一条。字段表自己带一行 35px 的空行，而 el-table 没有，
     不留这段距离的话，表头会被工具条**直接压住**（实测截图上两者重叠）。
     border-box 保证加上这段 padding 后总高不变。 */
  padding-top: 35px;
  box-sizing: border-box;
}

/* 基本信息网格：整块收成一张卡片（原来表单直接铺在面板上、下面又是整片空白，
   看着像"没加载完"）。页签已经写着"基本信息"，所以不再加重复的标题行。 */
.form-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 0 16px;
  align-items: start;
  padding: 14px 14px 2px;
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border-soft);
  border-radius: 10px;
}
.form-grid .grid-item { min-width: 0; }
.form-grid :deep(.el-form-item) { margin-bottom: 12px; }
/* 字段标签统一 12px，并去掉 Element 为"标签在右侧"预留的内边距（这里是标签在上方） */
.form-grid :deep(.el-form-item__label) {
  font-size: 12px;
  font-weight: 500;
  color: var(--dc-text-dim);
  line-height: 18px;
  margin-bottom: 4px;
  padding: 0;
}
.form-grid :deep(.el-select) { width: 100%; }
.ted-alert { margin-bottom: 12px; }
.ted-alert :deep(.el-alert__title) { font-size: 13px; }

/* ===== 字段表格（HTML 表格，与新建表一致） ===== */
.field-table {
  width: 100%;
  /* **不能用 collapse**：collapsed 边框 + `position: sticky` 表头是 Chromium 的老毛病 ——
     滚动后表头会留残影，下面几行的勾选框/输入框"透过"表头显示（本项目实测必现）。
     separate + border-spacing: 0 让每个单元格各自画下边框，观感与 collapse 一致，
     但粘性表头的重绘就正常了。 */
  border-collapse: separate;
  border-spacing: 0;
  table-layout: fixed;
  /* 各列宽度之和必须 ≤ 容器宽（实测容器 886px）：之前加起来 916px，
     表格横向溢出 30px —— 最后一列被顶出去、还多出一条横向滚动条，
     右边缘也就和上面的操作条对不上了。这里把下限放宽到 860，让 100% 生效。 */
  min-width: 860px;
}
.field-table th {
  position: sticky;
  top: 0;
  /* **必须高于行内控件**：Element 给 .el-checkbox__inner 设了 z-index: 1，
     与表头原本的 1 **同级**，而 tbody 在 thead 之后 → 同级时后出现的赢，
     于是滚动时勾选框会盖在表头上（实测：表头里透出下面几行的勾选框）。
     抬到 3 之后表头稳定在最上层；行内控件最高也只到 1。 */
  z-index: 3;
  /* 表头用一档浅底：与白底数据行形成对比，标题行一眼可辨 */
  background: var(--dc-bg-soft);
  color: var(--dc-text-strong);
  font-weight: 600;
  font-size: 12px;
  letter-spacing: .02em;
  text-align: left;
  /* 左右 9px：列与列的区分靠留白。上下 9px → 6px：标题行随数据行一起收
     （35px → 约 29px），两者比例保持不变。
     注意 thead 里那个 35px 的空行**不动** —— 页签与工具栏住在那儿，
     而且标题行的 sticky top: 35px 正是按它算出来的。 */
  padding: 6px 9px;
  border-bottom: 1px solid var(--dc-border);
  white-space: nowrap;
}
/* 行高 5px、左右 9px：纵向松一点不贴横线，横向给足留白（列与列不靠竖线区分）。
   行分隔线用 --dc-border（不是最浅的 soft）—— soft 在部分屏幕上几乎看不见，
   用户反馈"框线都没了"，这里统一提到和表头下边框同色。
   单元格给白底：表格在浅灰蓝面板上是一张"白纸"，页签行则留在面板底色上。 */
/* 行高压缩：上下内边距 5px → 2px（配合下面的控件压缩，行高从 43px 收到约 27px）。
   左右 8px → 9px：与表头的 9px 对齐（差 1px 在高分屏上就能看出输入框比表头偏左）。
   首列例外，见下面 :first-child 的 4px（序号列居中，不需要 9px）。 */
.field-table td { padding: 2px 9px; background: var(--dc-bg-card); border-bottom: 1px solid var(--dc-border); vertical-align: middle; }
/* 列竖线：标题行与数据行逐格画右边框（separate 边框模式各画各的，不会叠加变粗）。
   最后一列不画 —— 否则与容器外框并成两条线。
   表格的第一行（thead 里那个 35px 的"空行"）**不加竖线**：页签与工具栏就住在那一行，
   画了会从页签/搜索框中间穿过去。 */
.field-table thead tr:last-child th,
.field-table tbody td { border-right: 1px solid var(--dc-border); }
.field-table thead tr:last-child th:last-child,
.field-table tbody td:last-child { border-right: none; }
/* 注：最后一行**保留**下边框 —— 容器已不再画外框，这条就是表格自己的底边线。 */

/* 序号列（第一列）：居中；左右内边距从 10px 收到 4px ——
   列宽只有 32px（colgroup），不收紧内边距的话数字会被挤到换行。 */
.field-table thead tr:last-child th:first-child,
.field-table tbody td:first-child { text-align: center; }
.field-table tbody td:first-child { padding-left: 4px; padding-right: 4px; }

/* ===== 行内控件统一压到 22px（配合 row 的 2px 内边距） =====
   输入框/下拉/小按钮 Element 的 small 尺寸都是 24px，这里一并收到 22px，
   否则它们会成为新的"行高天花板"。 */
.field-table :deep(.el-input__inner) { height: 22px; }
.field-table :deep(.el-input__wrapper) { min-height: 22px; }
.field-table :deep(.el-select__wrapper) { min-height: 22px; }
.field-table :deep(.el-button--small) { height: 22px; padding: 0 6px; }
/* 主键 / 可空 / 自增等开关列：表头与内容居中，其余列左对齐 */
.field-table th.c-center,
.field-table td.c-center { text-align: center; }
/* 勾选框自带 margin-right，居中后会偏 —— 去掉才真正居中；标签文本这里不需要 */
/* height: 32px → 22px：**这是行高 43px 的真正来源**（Element 的勾选框默认 32px 高，
   比同一行里的输入框还高 8px）。压到 22px 后与输入框齐平。 */
.field-table :deep(.el-checkbox) { margin-right: 0; height: 22px; }
/* 双保险：把表格内勾选框的层级压回 auto（Element 默认给 __inner 设了 z-index: 1，
   那正是它盖住粘性表头的原因）。乘号/勾号的绘制不受影响，这里只动层叠。 */
.field-table :deep(.el-checkbox),
.field-table :deep(.el-checkbox__inner),
.field-table :deep(.el-switch) { z-index: auto; }
.field-table :deep(.el-checkbox__label) { display: none; }
/* ↓↓↓ 行底色统一画在 td 上（悬停、新增/改动），按顺序覆盖：
   悬停 → 新增/改动 → 它们在悬停时的高亮。全部同优先级，靠顺序决定胜出。
   注：**不做斑马纹** —— 参考里的行全是同色，只靠分隔线 + 悬停/当前行高亮区分；
   几十行同色时，斑马纹反而会和"新增/改动"的底色打架。 */
.field-table tbody tr:hover td { background: var(--dc-bg-hover); }
.field-table tbody tr.r-new td { background: rgba(61, 220, 151, .06); }
.field-table tbody tr.r-mod td { background: rgba(245, 179, 77, .07); }
.field-table tbody tr.r-new:hover td { background: rgba(61, 220, 151, .12); }
.field-table tbody tr.r-mod:hover td { background: rgba(245, 179, 77, .13); }
.row-idx {
  color: var(--dc-text-dim); font-size: 12.5px;
  font-variant-numeric: tabular-nums;
}
/* ↓↓↓ 让表格"像表格"的关键：单元格里的输入控件默认**隐形**（透明底 + 无描边），
   整张表读起来是一列列文字；鼠标移到该行才浮出输入框的样子，聚焦时给主色描边，
   编辑态一眼可辨。原来每个格子都是常驻输入框，看上去就是一排盒子。 */
.field-table :deep(.el-input__wrapper),
.field-table :deep(.el-select__wrapper) {
  background-color: transparent !important;
  box-shadow: none !important;
  transition: background .15s ease, box-shadow .15s ease;
}
.field-table tbody tr:hover :deep(.el-input__wrapper),
.field-table tbody tr:hover :deep(.el-select__wrapper) {
  /* 用 --dc-bg-hover 而不是 --dc-bg-input：后者在浅色主题下是**纯白**，
     一格一格亮得晃眼；这个是有色阶的中性底（浅色 #e8edf6 / 深色 #2c3140）。 */
  background-color: var(--dc-bg-hover) !important;
  box-shadow: 0 0 0 1px var(--dc-border) inset !important;
}
.field-table :deep(.el-input__wrapper.is-focus),
.field-table :deep(.el-select__wrapper.is-focused) {
  background-color: var(--dc-bg-hover) !important;
  box-shadow: 0 0 0 1px var(--dc-primary) inset !important;
}
/* 注：以上必须带 !important —— index.css 里有一套全局的 `.el-input__wrapper { background-color:
   var(--dc-bg-input) !important; box-shadow: … !important }` 主题覆盖，不带就压不过它
   （第一版就是这样静默失效的）。选择器同时提到 .field-table 之后，优先级也高于 Element 自身。 */
.pk-none { color: var(--dc-text-dim); font-size: 14px; }
.dv-null { font-family: "SF Mono", Consolas, monospace; }
.empty-row { text-align: center; color: var(--dc-text-dim); padding: 18px 0 !important; font-size: 13px; }
.type-select { width: 100%; }
/* 注释格：输入框 + 悬停浮出的「展开」按钮。
   按钮绝对定位 —— 出现/消失都不挤动输入框（否则鼠标一扫，文字就跳位置）；
   底色用 inherit，跟随所在行（斑马纹/悬停底色），所以压在文字尾部也不会露出一块白底。 */
.cmt-cell { position: relative; display: flex; align-items: center; background: inherit; }
.cmt-cell :deep(.el-input) { width: 100%; }
.cmt-expand {
  position: absolute;
  right: 0;
  top: 50%;
  transform: translateY(-50%);
  padding: 2px;
  border-radius: 4px;
  background: inherit;
  color: var(--dc-text-dim);
  opacity: 0;
  transition: opacity .15s ease, color .15s ease;
}
.field-table tbody tr:hover .cmt-expand { opacity: 1; }
.cmt-expand:hover { color: var(--dc-primary); }

/* ===== 参考 Navicat 那版的细节 ===== */
/* 搜索框（放在操作条右侧，紧挨「添加字段」） */
/* flex-shrink: 0：宁可让左侧提示文字省略号，也不要把搜索框压窄（压窄后输入区就看不全了） */
.col-search { width: 190px; flex-shrink: 0; }
/* 可空：勾选框 + 「是/否」文字，比孤零零一个方框好读 */
.bool-cell { display: inline-flex; align-items: center; gap: 5px; }
.bool-text { font-size: 12px; color: var(--dc-text-dim); }
/* 正在编辑的行给一个主色左标 + 浅底：一屏几十行时，"光标在哪一行"一眼可见
   （用左标 + 浅底，不干扰斑马纹的读法）。
   位置必须在这几条斑马纹规则**之后**，同优先级下靠顺序取胜。 */
.field-table tbody tr:focus-within td { background: var(--dc-primary-wash); }
.field-table tbody tr:focus-within td:first-child { box-shadow: inset 3px 0 0 var(--dc-primary); }
.dc-del { color: var(--dc-text-dim); }
.dc-del:hover { color: var(--dc-danger); background: rgba(255, 97, 97, .1); }

/* 索引表格主题 */
.idx-tab :deep(.el-table) {
  --el-table-bg-color: transparent;
  --el-table-tr-bg-color: transparent;
  /* 表头与字段表格同色（原来是 --dc-bg-code，和字段页签不一致） */
  --el-table-header-bg-color: var(--dc-bg-soft);
  --el-table-header-text-color: var(--dc-text-strong);
  --el-table-border-color: var(--dc-border-soft);
  --el-table-row-hover-bg-color: var(--dc-primary-wash);
  --el-table-border: 1px solid var(--dc-border-soft);
}
.idx-tab :deep(.el-table__inner-wrapper::before) { display: none; }
.idx-tab :deep(.el-table th.el-table__cell) {
  background: var(--dc-bg-soft) !important;
  color: var(--dc-text-mid) !important;
  /* 与字段表同色（原来是 soft，看着比字段表浅一档） */
  border-bottom: 1px solid var(--dc-border) !important;
  /* 列竖线：el-table 默认**没有**列分隔线，和字段表并排看就是两张不同的表 */
  border-right: 1px solid var(--dc-border) !important;
  /* 内边距挪到 .cell 上（Element 的 td 自己带 4px 0，会把行撑高） */
  padding: 0 !important;
}
/* 索引表表头/内容左对齐 */
.idx-tab :deep(.el-table th.el-table__cell > .cell),
.idx-tab :deep(.el-table td.el-table__cell > .cell) { text-align: left; }
/* 尺寸与字段表对齐：表头 cell 上下 3px（+内容约 24px ≈ 30px，字段表 29px）、
   数据 cell 上下 2px（实测行高 30px，字段表 29px）。 */
.idx-tab :deep(.el-table th.el-table__cell > .cell) { padding: 3px 9px !important; }
/* 左右 4px → 9px：与表头的 9px 对齐（原来输入框的框左缘比列表头偏左 5px） */
.idx-tab :deep(.el-table td.el-table__cell > .cell) { padding: 2px 9px !important; }
.idx-tab :deep(.el-table td.el-table__cell) {
  background: transparent !important;
  border-bottom: 1px solid var(--dc-border) !important;
  border-right: 1px solid var(--dc-border) !important;
  padding: 0 !important;
}
/* 最后一列不画右边框（否则与外框并成双线） */
.idx-tab :deep(.el-table th.el-table__cell:last-child),
.idx-tab :deep(.el-table td.el-table__cell:last-child) { border-right: none !important; }
/* 操作列：表头「操作」与内容都**居中**（原来是跟着全局的左对齐走的）。
   .idx-ops 是 inline-flex，所以父级 .cell 的 text-align: center 就能把它整体居中。 */
.idx-tab :deep(.el-table th.el-table__cell:last-child > .cell),
.idx-tab :deep(.el-table td.el-table__cell:last-child > .cell) { text-align: center !important; }
/* 操作列里的「重建 + 删除」并排一行（合成一列后的内部排布） */
.idx-ops { display: inline-flex; align-items: center; gap: 2px; }
/* 重建按钮：图标化后与删除按钮同一套观感 —— 平时是主色图标，悬停给一层浅底 */
.idx-rebuild { color: var(--dc-primary); padding: 2px; }
.idx-rebuild:hover { background: var(--dc-primary-wash); }
.idx-tab :deep(.el-table .el-table__row.row-is-new > td.el-table__cell) { background: rgba(61, 220, 151, .08) !important; }
.idx-tab :deep(.el-table .el-table__row.row-is-modified > td.el-table__cell) { background: rgba(245, 179, 77, .1) !important; }
.idx-tab :deep(.el-table .el-input__wrapper) { padding: 0 6px; }
.idx-tab :deep(.el-table .el-input__inner) { font-size: 13px; }
/* 索引表与字段表同一套观感：控件默认隐形、悬停该行才浮出、聚焦给主色描边。
   否则在「字段定义 / 索引」之间切换时，像换了一套界面。 */
.idx-tab :deep(.el-input__wrapper),
.idx-tab :deep(.el-select__wrapper) {
  background-color: transparent !important;
  box-shadow: none !important;
  transition: background .15s ease, box-shadow .15s ease;
}
.idx-tab :deep(.el-table__row:hover .el-input__wrapper),
.idx-tab :deep(.el-table__row:hover .el-select__wrapper) {
  background-color: var(--dc-bg-hover) !important;
  box-shadow: 0 0 0 1px var(--dc-border) inset !important;
}
.idx-tab :deep(.el-input__wrapper.is-focus),
.idx-tab :deep(.el-select__wrapper.is-focused) {
  background-color: var(--dc-bg-hover) !important;
  box-shadow: 0 0 0 1px var(--dc-primary) inset !important;
}

/* ===== SQL 预览（编辑区**下方**，可展开/收起）=====
   原来在右侧：宽 380px + 左边框。改到下方后宽度撑满、上边框替代左边框，
   并给一个高度上限（40vh），免得把上面的编辑区挤没；内容超出时 .sql-body 内部滚动。 */
.sql-area {
  width: 100%;
  /* 高度由拖动/双击控制（内联 style 给 height，默认 260px），这里不再设 max-height，
     否则会把它顶住；收起时高度由内容决定（见 .collapsed）。 */
  /* 与上面的表格卡**同一套外观**：1px 细边框 + 8px 圆角 + 卡片底 + 同一层轻阴影，
     上下两块叠在一起才像一组。overflow: hidden 是必须的 —— 不裁的话，
     头部那条浅灰底和底栏会把圆角顶成直角（正文自身的滚动不受影响，
     .sql-body 自己 overflow: auto）。 */
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  background: var(--dc-bg-card);
  box-shadow: 0 1px 3px rgba(16, 24, 40, .06), 0 1px 2px rgba(16, 24, 40, .04);
  overflow: hidden;
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  min-height: 0;
}
/* 收起时高度交给内容（标题行 + 底部按钮行）；展开时的高度由内联 style 给。
   收起时分隔条不显示（v-show），这条 6px 缝改由 margin-top 补上 ——
   否则两块卡片会贴在一起，看着像一整块。 */
.sql-area.collapsed { height: auto; margin-top: 6px; }
/* 分隔条：与 SQL 查询界面的结果区分隔条同一套手感（细条，悬停变主色）。
   它**本身就是上下两块卡片之间那道缝**：6px 高 + 透明底（不铺底色 → 露出页面白底，
   与「去掉编辑区灰底」的诉求一致），4px 细条在这 6px 里上下居中 → 正好落在缝中间。
   原来它铺 --dc-bg 灰底、又待在 .sql-area 卡片**内部**，细条于是贴在卡片顶边。 */
.sql-resizer { height: 6px; flex: 0 0 6px; cursor: row-resize; position: relative; z-index: 2; background: transparent; }
.sql-resizer::before {
  content: ''; position: absolute; left: 50%; top: 50%;
  transform: translate(-50%, -50%);
  width: 48px; height: 4px; border-radius: 3px;
  background: var(--dc-text-dim); opacity: .45;
  transition: background .15s ease, opacity .15s ease;
}
.sql-resizer:hover::before { background: var(--dc-primary); opacity: 1; }
/* 底部按钮组（原始 DDL / 复制 SQL / 重置 / 保存修改） */
.sql-foot-actions { display: flex; align-items: center; gap: 2px; flex-shrink: 0; }
.sql-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 10px 12px;
  font-size: 13px;
  color: var(--dc-text-dim);
  border-bottom: 1px solid var(--dc-border-soft);
  flex-shrink: 0;
  cursor: pointer;
  user-select: none;
}
/* 预览标题栏：一档浅底 + 下边框，把"标题 / 代码 / 底栏"三层分开 */
.sql-head { background: var(--dc-bg-soft); border-bottom: 1px solid var(--dc-border-soft); }
.sql-head:hover { background: var(--dc-bg-hover); color: var(--dc-text); }
/* 折叠箭头：展开时朝下、收起时朝右 */
.sql-fold { transition: transform .18s ease; transform: rotate(90deg); }
.sql-fold.folded { transform: rotate(0deg); }
.sql-fold-tx { font-size: 12px; color: var(--dc-text-weak); }
/* 收起时把「执行前请确认…」一并收掉，只留保存按钮 */
.sql-area.collapsed .sql-tip { display: none; }
.sql-state { font-size: 12px; margin-left: auto; }
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
  padding: 8px 12px;
  border-top: 1px solid var(--dc-border);
  display: flex;
  align-items: center;
  /* 按钮**恒定贴右**：用 flex-end + 提示文字 margin-right:auto 撑开。
     这里不能用 space-between —— 收起预览时 .sql-tip 会 display:none（见下一条），
     只剩一个子元素，space-between 会把它推到**左侧**，整组按钮就"跑"了。 */
  justify-content: flex-end;
  gap: 8px;
  flex-shrink: 0;
}
.sql-tip { font-size: 12px; color: var(--dc-text-dim); margin-right: auto; }
/* ===== 基本信息：与字段定义共用同一套表格外观 =====
   容器不留内边距（顶部 35px 让开"页签 + 工具条"那一条，与 .col-tab / .idx-tab 同理）；
   表格里**没有**字段表那行 35px 的"空行"，所以表头 sticky top 改回 0。
   这几条必须写在 .field-table 规则**之后**：选择器优先级相同，靠顺序取胜。 */
.tab-inner.basic-tab { padding: 35px 0 0; }
.tab-inner.basic-tab .field-table-wrap {
  flex: 0 1 auto;
  max-height: 100%;
  overflow: auto;
  min-height: 0;
}
.basic-table thead tr:last-child th { top: 0 !important; }
/* 左侧「项目」列：读作"行标题"，表头与内容都要盖掉字段表那条"首列居中"
   （那条是给字段表的 # 序号列用的，会连带把这里的「项目」也居中）。 */
.basic-table tbody td:first-child,
.basic-table thead tr:last-child th:first-child { text-align: left; }
/* padding-left 必须 !important：字段表那条"序号列"规则
   （.field-table tbody td:first-child { padding-left: 4px }）优先级更高（多一个 :first-child），
   会把这里的 9px 压成 4px —— 行标签因此比表头「项目」往左偏 5px，两列对不齐。 */
.basic-table td.basic-label {
  font-size: 12.5px;
  color: var(--dc-text-mid);
  padding-left: 9px !important;
  white-space: nowrap;
}
/* （原「表概况只读值」那条纯文字样式已随标记一起去掉：
     行数 / 列数 / 索引数 / 主键 现在也是禁用输入框，与其它行同一套外观，
     内边距由上面的 .basic-table tbody td:not(:first-child) 统一提供。） */
/* 「值」列：左右内边距也统一到 9px。
   原来单元格是 4px、表头是 9px —— 输入框的框左缘因此比表头「值」往左偏 5px（实测 556 vs 561），
   加上输入框自身的 6px 内边距，框里的文字更往右，看着就"没对齐"。 */
.basic-table tbody td:not(:first-child) { padding-left: 9px !important; padding-right: 9px !important; }
</style>
