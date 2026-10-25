<template>
  <el-dialog :model-value="modelValue" width="min(1280px, 96vw)" top="3vh" class="sync-dialog"
             :class="{ 'run-mode': step === 5 }"
             :close-on-click-modal="false" :close-on-press-escape="!taskRunning"
             :before-close="onBeforeClose"
             append-to-body @update:model-value="$emit('update:modelValue', $event)">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Promotion /></el-icon></span>
        <span>{{ $t('sync.title') }}</span>
        <!-- 标题旁的说明文字已按需求删掉（sync.subtitle 词典保留但不再渲染） -->
      </div>
    </template>

    <!-- 顶部「源 → 目标」摘要条已按需求删除（信息在步骤 1 与摘要页里都有） -->

    <!-- ==================== 步骤条（与数据对比一致的向导式交互） ==================== -->
    <el-steps v-if="step <= 4" :active="step - 1" align-center finish-status="success" class="sync-steps">
      <el-step :title="$t('sync.stepSource')" />
      <el-step :title="$t('sync.stepObjects')" />
      <el-step :title="$t('sync.stepOptions')" />
      <el-step :title="$t('sync.stepSummary')" />
      </el-steps>

    <!-- ==================== 步骤 1：选择源 / 目标 ==================== -->
    <div v-if="step === 1" class="step-pane step-source">
      <div class="flow">
        <div class="side">
          <div class="side-head"><span class="dot src"></span>{{ $t('sync.source') }}</div>
          <!-- 连接按**分组**列：连接名长起来以后（「原生产-xxx_kechuan」这类）只按名字排是一锅粥，
               分组头沿用左侧对象树的目录名，和树里看到的位置对得上。 -->
          <el-select v-model="src.connectionId" size="small" :disabled="taskRunning" popper-class="conn-opt-popper" @change="onSrcConnChange">
            <el-option-group v-for="g in connGroups" :key="g.label" :label="g.label">
              <el-option v-for="c in g.options" :key="c.id" :label="c.name + ' (' + c.type + ')'" :value="c.id" />
            </el-option-group>
          </el-select>
          <div class="pair">
            <el-select v-if="!srcHideDb" v-model="src.database" size="small" :disabled="taskRunning" @change="onSrcDbChange">
              <el-option v-for="d in srcDbs" :key="d" :label="d" :value="d" />
            </el-select>
            <el-select v-if="srcNeedSchema" v-model="src.schema" size="small" :disabled="taskRunning" @change="loadSrcObjects">
              <el-option v-for="s in srcSchemas" :key="s" :label="s" :value="s" />
            </el-select>
          </div>
        </div>

        <div class="flow-arrow">
          <div class="arrow-ring"><el-icon :size="18"><Right /></el-icon></div>
        </div>

        <div class="side">
          <div class="side-head"><span class="dot tgt"></span>{{ $t('sync.target') }}</div>
          <el-select v-model="tgt.connectionId" size="small" :disabled="taskRunning" popper-class="conn-opt-popper" @change="onTgtConnChange">
            <el-option-group v-for="g in connGroups" :key="g.label" :label="g.label">
              <el-option v-for="c in g.options" :key="c.id" :label="c.name + ' (' + c.type + ')'" :value="c.id" />
            </el-option-group>
          </el-select>
          <div class="pair">
            <el-select v-if="!tgtHideDb" v-model="tgt.database" size="small" :disabled="taskRunning" @change="onTgtDbChange">
              <el-option v-for="d in tgtDbs" :key="d" :label="d" :value="d" />
            </el-select>
            <el-select v-if="tgtNeedSchema" v-model="tgt.schema" size="small" :disabled="taskRunning">
              <el-option v-for="s in tgtSchemas" :key="s" :label="s" :value="s" />
            </el-select>
          </div>
        </div>
      </div>

      <!-- 连接信息摘要（Navicat 式两列） -->
      <div class="info-grid-wrap">
        <div class="info-col" v-if="srcInfo">
          <div class="info-title">{{ $t('sync.source') }}</div>
          <div class="info-row"><span>{{ $t('sync.infoType') }}</span><b>{{ srcInfo.type }}</b></div>
          <div class="info-row"><span>{{ $t('sync.infoName') }}</span><b>{{ srcInfo.name }}</b></div>
          <div class="info-row"><span>{{ $t('sync.infoHost') }}</span><b>{{ srcInfo.host }}</b></div>
          <div class="info-row"><span>{{ $t('sync.infoPort') }}</span><b>{{ srcInfo.port }}</b></div>
        </div>
        <div class="info-col" v-if="tgtInfo">
          <div class="info-title">{{ $t('sync.target') }}</div>
          <div class="info-row"><span>{{ $t('sync.infoType') }}</span><b>{{ tgtInfo.type }}</b></div>
          <div class="info-row"><span>{{ $t('sync.infoName') }}</span><b>{{ tgtInfo.name }}</b></div>
          <div class="info-row"><span>{{ $t('sync.infoHost') }}</span><b>{{ tgtInfo.host }}</b></div>
          <div class="info-row"><span>{{ $t('sync.infoPort') }}</span><b>{{ tgtInfo.port }}</b></div>
        </div>
      </div>
    </div>

    <!-- ==================== 步骤 2：选择对象 ====================
         卡片式分组下拉：每一类对象是一张可展开的卡片（勾选框 + 图标 + 类型名 + 已选/总数 + 箭头），
         展开后才列出该类对象。分组按**源库与目标库的能力**决定要不要画（见 visibleGroups）——
         源库没有这个概念（PG 没有 EVENT）、或目标库建不出来（目标不支持该对象 DDL）就不摆空架子。 -->
    <div class="panel obj-panel" v-if="step === 2 && src.connectionId && src.database" v-loading="objsLoading">
      <div class="panel-head">
        <span class="panel-title"><el-icon :size="14"><FolderOpened /></el-icon>{{ $t('sync.pickObjects') }}</span>
        <span class="panel-count">{{ $t('sync.selectedPrefix') }}<b>{{ totalSelected }}</b> / {{ $t('sync.selectedSuffix', { total: totalAvailable }) }}</span>
      </div>

      <div class="obj-toolbar">
        <el-input v-model="keyword" size="small" clearable :prefix-icon="Search"
                  :placeholder="$t('sync.searchObj')" :disabled="taskRunning" />
        <el-button size="small" text :disabled="taskRunning || !availableCount" @click="applyAll(true)">{{ $t('sync.selectAll') }}</el-button>
        <el-button size="small" text :disabled="taskRunning || !totalSelected" @click="applyAll(false)">{{ $t('sync.unselectAll') }}</el-button>
      </div>

      <div v-if="!availableCount" class="obj-empty-all">{{ $t('sync.noObjects') }}</div>
      <template v-else>
        <!-- 一类对象一个页签（原先是手风琴卡片）。徽标是「已选/总数」；
             页签前的勾选框 = 本类全选/取消（顶部两个按钮才是全部类别）。
             点勾选框不切页签，所以单独 stop。 -->
        <div class="obj-tabs" role="tablist">
          <button v-for="g in visibleGroups" :key="g.key" type="button" role="tab"
                  class="obj-tab"
                  :class="{ on: activeGroup && activeGroup.key === g.key, empty: !objState[g.key].list.length }"
                  :aria-selected="activeGroup && activeGroup.key === g.key"
                  @click="activeTab = g.key">
            <el-checkbox class="ot-check" :model-value="groupChecked(g.key)"
                         :indeterminate="groupIndeterminate(g.key)"
                         :disabled="taskRunning || !objState[g.key].list.length"
                         @click.stop @change="v => setGroup(g.key, v)" />
            <span class="ot-ic" :class="'ty-' + g.key"><el-icon :size="12"><component :is="TYPE_ICON[g.key]" /></el-icon></span>
            <span class="ot-name">{{ typeLabel(g.key) }}</span>
            <span class="ot-count">{{ objState[g.key].selected.length }}/{{ objState[g.key].list.length }}</span>
          </button>
        </div>

        <div class="obj-list" v-if="activeGroup">
          <div v-if="!groupItems(activeGroup.key).length" class="obj-none">
            <span class="nc-ic"><el-icon :size="22"><FolderOpened /></el-icon></span>
            <span class="nc-txt">{{ keyword ? $t('sync.noMatch') : $t('sync.emptyGroup') }}</span>
          </div>
          <div v-for="name in groupItems(activeGroup.key)" :key="name" class="og-item"
               :class="{ on: isSelected({ type: activeGroup.key, name }) }"
               @click="toggleObj({ type: activeGroup.key, name }, !isSelected({ type: activeGroup.key, name }))">
            <el-checkbox :model-value="isSelected({ type: activeGroup.key, name })" :disabled="taskRunning"
                         @click.stop @change="v => toggleObj({ type: activeGroup.key, name }, v)" />
            <span class="og-item-name" :title="name">{{ name }}</span>
          </div>
        </div>
      </template>
    </div>

    <!-- ==================== 步骤 3：传输选项 ====================
         一类设置一个页签，页签名就是那一类设置的名字：
         同步设置 / 表名映射 / 前后置 SQL / 行过滤。
         以前「过滤与脚本」里还套着三块可折叠卡片，翻找成本高、展开还占高度；
         拆成独立页签后每个页签只做一件事，也不再有「一列长一列空」的参差。
         当前页签存在 `optionTab`，来回翻步骤不会被打回第一个页签。 -->
    <div class="settings-card" v-if="step === 3">
      <el-tabs v-model="optionTab" class="opt-tabs">
        <!-- ① 同步设置：怎么同步（方式卡片）+ 搬什么 + 传输参数
             这三块是同一件事的三个层次（目标已有对象怎么办 → 搬哪些东西 → 用什么节奏搬），
             拆成两个页签反而要来回点：改完模式还得切页签确认「同步表数据」勾没勾。
             并成一页，用两条细分隔线分段。 -->
        <el-tab-pane name="strategy" :label="$t('sync.tabSettings')">
        <div class="opt-grid one">
          <div class="settings-col col-policy">
            <div class="settings-label"><el-icon :size="13"><Files /></el-icon>{{ $t('sync.strategyTitle') }}</div>
            <!-- 「目标已存在对象」与「数据模式」原来各占一栏，但两者并不独立：
                 「删除重建」时目标表是刚建出来的空表，「按主键更新」永远命中不了任何已存在的行
                 （后端也按仅插入兜底）—— 于是右侧必须跟着左侧置灰，两组说明还会互相打架。
                 现在合并成一个「同步方式」：每种组合就是一个选项，互斥关系自然消失。
                 样式与下面的勾选行一致（普通单选行，不画卡片面板、不带长说明）。 -->
            <el-radio-group v-model="syncMode" :disabled="taskRunning" class="mode-list">
              <el-radio v-for="m in modeOptions" :key="m.value" :value="m.value" class="mode-row">
                <span class="mode-title">{{ m.title }}</span>
              </el-radio>
            </el-radio-group>
            <!-- 没勾「同步表数据」时只剩结构怎么处理：说明一句，免得用户以为少了两项 -->
            <p class="settings-hint" v-if="!opts.syncData">{{ $t('sync.smStructOnly') }}</p>
            <div class="settings-sep"></div>
            <div class="settings-label"><el-icon :size="13"><Operation /></el-icon>{{ $t('sync.execWhat') }}</div>
            <div class="settings-checks checks-2col">
            <!-- 前四项是「结构/数据」的开关，顺序也按依赖排：先有库、再有表、表建好才谈索引；
                 「建表时一并创建索引」离开「自动建表」没有意义，所以联动置灰。 -->
            <el-checkbox v-model="opts.autoCreateDb" :disabled="taskRunning">{{ $t('sync.autoCreateDb') }}</el-checkbox>
            <el-checkbox v-model="opts.syncStructure" :disabled="taskRunning">{{ $t('sync.autoCreate') }}</el-checkbox>
            <el-checkbox v-model="opts.includeIndexes" :disabled="taskRunning || !opts.syncStructure">
              {{ $t('sync.includeIndexes') }}
            </el-checkbox>
            <el-checkbox v-model="opts.syncData" :disabled="taskRunning">{{ $t('sync.syncData') }}</el-checkbox>
            <span class="checks-sep"></span>
            <!-- 「遇错停止」是任务级行为，和上面四条的「同步内容」不是一类，单独隔开 -->
            <el-checkbox v-model="opts.stopOnError" :disabled="taskRunning">{{ $t('sync.stopOnError') }}</el-checkbox>
          </div>
          <div class="settings-sep"></div>
          <div class="settings-label"><el-icon :size="13"><Clock /></el-icon>{{ $t('sync.paramTitle') }}</div>
          <!-- 参数两列（按列流排，见 .param-grid）：左列 = 怎么搬，右列 = 开多少并发 -->
          <div class="param-grid">
          <!-- 每批行数：**按目标库给默认值、用户可改**。0 = 自动（用默认值）。
               默认值差一个数量级（CH 写 part 要 1 万行、MySQL 在丢包链路上 1000 行、
               SQL Server 有多行 VALUES 的 1000 行硬限制），所以写在 compute 里按类型映射，
               后端的方言默认值与之保持一致（sync::insert_rows_default）。 -->
          <div class="settings-inline">
            <span class="settings-inline-label">
              {{ $t('sync.insertRows') }}
              <el-tooltip :content="$t('sync.insertRowsHint')" placement="top" :show-after="120">
                <el-icon class="hint-q"><QuestionFilled /></el-icon>
              </el-tooltip>
            </span>
            <!-- 下拉而不是数字框（和右侧并发三项一致，用户口径「样式统一」）：
                 第一项就是「自动」= 后端按目标库给默认值；后面的档位是显式指定。 -->
            <el-select :model-value="opts.insertRows" @update:model-value="onInsertRowsInput"
                       size="small" :disabled="taskRunning" class="settings-num">
              <el-option v-for="n in batchRowsOptions" :key="n" :value="n"
                         :label="n === 0 ? $t('sync.auto') : String(n)" />
            </el-select>
          </div>
          <!-- 超时时间：对齐 DBX 的「操作超时（秒），丢弃连接」。默认「继承连接」= 用数据源上
               自己的 socketTimeout；选具体秒数就是单次数据库操作的硬上限（探活通过仍会继续等）。 -->
          <div class="settings-inline">
            <span class="settings-inline-label">
              {{ $t('sync.opTimeout') }}
              <el-tooltip :content="$t('sync.opTimeoutHint')" placement="top" :show-after="120">
                <el-icon class="hint-q"><QuestionFilled /></el-icon>
              </el-tooltip>
            </span>
            <el-select v-model="opts.timeout" size="small" :disabled="taskRunning" class="settings-num">
              <el-option :value="0" :label="$t('sync.opTimeoutInherit')" />
              <el-option :value="-1" :label="$t('sync.opTimeoutUnlimited')" />
              <el-option v-for="n in [30, 60, 120, 300, 600]" :key="n" :value="n"
                         :label="$t('sync.opTimeoutSec', { n })" />
            </el-select>
          </div>
          <!-- 限速（对齐 DataX 的 channel.speed）：两条曲线分开填，0 = 不限。
               行/秒适合「小表多、单行窄」；MB/s 适合宽表 —— 同样是 1 万行，带大字段的
               宽表可能是窄表的几百倍字节，按行限速对它毫无意义。 -->
          <div class="settings-inline">
            <span class="settings-inline-label">
              {{ $t('sync.rateRows') }}
              <el-tooltip :content="$t('sync.rateRowsHint')" placement="top" :show-after="120">
                <el-icon class="hint-q"><QuestionFilled /></el-icon>
              </el-tooltip>
            </span>
            <!-- 下拉（和并发三项统一）：0 的标签写「不限制」，不把字面 0 摆给用户看 -->
            <el-select :model-value="opts.rateRows" @update:model-value="onRateRowsInput"
                       size="small" :disabled="taskRunning" class="settings-num">
              <el-option v-for="n in rateRowsOptions" :key="n" :value="n"
                         :label="n === 0 ? $t('sync.opTimeoutUnlimited') : String(n)" />
            </el-select>
          </div>
          <div class="settings-inline">
            <span class="settings-inline-label">
              {{ $t('sync.rateMb') }}
              <el-tooltip :content="$t('sync.rateMbHint')" placement="top" :show-after="120">
                <el-icon class="hint-q"><QuestionFilled /></el-icon>
              </el-tooltip>
            </span>
            <el-select :model-value="opts.rateMb" @update:model-value="onRateMbInput"
                       size="small" :disabled="taskRunning" class="settings-num">
              <el-option v-for="n in rateMbOptions" :key="n" :value="n"
                         :label="n === 0 ? $t('sync.opTimeoutUnlimited') : String(n)" />
            </el-select>
          </div>
          <!-- 并发三项（自动 / 1..16）：放在**第二列**（`.param-grid` 按列流排，
               前四个参数填满左列后，这三项自然落到右侧）。
               为什么用下拉而不是数字框：数字框的 min=1 会把「清空」夹回 1，
               用户永远回不到「自动」（真机反馈）；下拉里「自动」是一个明确选项。
               显式选定后后端以用户为准，**尾部弹性也不会超过这个值**（填 1 就始终 1 条连接）。 -->
          <div class="settings-inline">
            <span class="settings-inline-label">
              {{ $t('sync.objConcurrency') }}
              <el-tooltip :content="$t('sync.objConcurrencyHint')" placement="top" :show-after="120">
                <el-icon class="hint-q"><QuestionFilled /></el-icon>
              </el-tooltip>
            </span>
            <el-select :model-value="opts.objConcurrency"
                       @update:model-value="v => onConcurrency('objConcurrency', v)"
                       size="small" :disabled="taskRunning" class="settings-num">
              <el-option v-for="n in CONCURRENCY_OPTIONS" :key="n" :value="n"
                         :label="n === 0 ? $t('sync.auto') : String(n)" />
            </el-select>
          </div>
          <div class="settings-inline">
            <span class="settings-inline-label">
              {{ $t('sync.srcSlices') }}
              <el-tooltip :content="$t('sync.srcSlicesHint')" placement="top" :show-after="120">
                <el-icon class="hint-q"><QuestionFilled /></el-icon>
              </el-tooltip>
            </span>
            <el-select :model-value="opts.srcSlices"
                       @update:model-value="v => onConcurrency('srcSlices', v)"
                       size="small" :disabled="taskRunning" class="settings-num">
              <el-option v-for="n in CONCURRENCY_OPTIONS" :key="n" :value="n"
                         :label="n === 0 ? $t('sync.auto') : String(n)" />
            </el-select>
          </div>
          <div class="settings-inline">
            <span class="settings-inline-label">
              {{ $t('sync.tgtWorkers') }}
              <el-tooltip :content="$t('sync.tgtWorkersHint')" placement="top" :show-after="120">
                <el-icon class="hint-q"><QuestionFilled /></el-icon>
              </el-tooltip>
            </span>
            <el-select :model-value="opts.tgtWorkers"
                       @update:model-value="v => onConcurrency('tgtWorkers', v)"
                       size="small" :disabled="taskRunning" class="settings-num">
              <el-option v-for="n in CONCURRENCY_OPTIONS" :key="n" :value="n"
                         :label="n === 0 ? $t('sync.auto') : String(n)" />
            </el-select>
          </div>
          <!-- 「行数上限」已删（用户口径：同步就是不限，抽样不是同步的事）；
               原先那个全局 WHERE 输入框挪到「行过滤」页签**按表设置** —— 一个条件
               拼到几十张表上语义说不清 -->
          </div>
        </div>
          </div>
        </el-tab-pane>

        <!-- ③ 表名映射（按表设置）：对齐 DBX 的「源表 → 目标表」一对一映射，留空 = 同名。
             典型场景：源库是 `t_order` / 目标库规范叫 `ORDER_HEAD`，不用先改名再搬。 -->
        <el-tab-pane name="rename">
          <template #label>
            <span class="opt-tab">{{ $t('sync.tabRename') }}
              <span v-if="renameCount" class="opt-tab-badge">{{ renameCount }}</span>
            </span>
          </template>
          <div class="opt-pane">
            <div class="pane-head">
              <el-icon :size="13"><EditPen /></el-icon>
              <span class="pane-title">{{ $t('sync.renamePerTable') }}</span>
              <el-input v-if="selTables.length > 5" v-model="renameSearch" size="small" clearable
                        :placeholder="$t('sync.whereSearch')" class="sw-search" />
            </div>
            <p class="pane-hint">{{ $t('sync.renamePerTableHint') }}</p>
            <div v-if="selTables.length" class="sw-list">
              <div v-for="name in renameTableList" :key="'rn-' + name" class="sw-item"
                   :class="{ on: !!(renames[name] || '').trim() }">
                <span class="sw-name" :title="name">{{ name }}</span>
                <el-input v-model="renames[name]" size="small" :disabled="taskRunning"
                          :placeholder="name" maxlength="128" />
              </div>
              <div v-if="!renameTableList.length" class="sw-none">{{ $t('sync.whereNoMatch') }}</div>
            </div>
            <div v-else class="sw-none">{{ $t('sync.whereNoTable') }}</div>
          </div>
        </el-tab-pane>

        <!-- ④ 前置 / 后置 SQL（DataX preSql / postSql）
             两份脚本都是**按对象执行**（整库同步时每个对象各跑一遍），而不是整趟任务只跑一次：
             典型用法是「搬之前禁外键/清临时表」「搬之后建索引/刷物化视图」——按对象才说得通。 -->
        <el-tab-pane name="sql">
          <template #label>
            <span class="opt-tab">{{ $t('sync.tabSql') }}
              <span v-if="sqlCount" class="opt-tab-badge">{{ sqlCount }}</span>
            </span>
          </template>
          <div class="opt-pane">
            <div class="pane-head">
              <el-icon :size="13"><Document /></el-icon>
              <span class="pane-title">{{ $t('sync.sqlScripts') }}</span>
            </div>
            <p class="pane-hint">{{ $t('sync.sqlScriptsHint') }}</p>
            <!-- 两个输入框撑满剩余高度（不写死 rows）：窗口/弹窗拉高就跟着变高，
               否则下面永远空一大片，写长脚本还得自己滚 -->
            <div class="sql-grid grow">
              <div class="sql-col">
                <div class="sql-label">{{ $t('sync.sqlPre') }}</div>
                <el-input v-model="opts.preSql" type="textarea" :disabled="taskRunning"
                          resize="none" :placeholder="$t('sync.sqlPrePh')" />
              </div>
              <div class="sql-col">
                <div class="sql-label">{{ $t('sync.sqlPost') }}</div>
                <el-input v-model="opts.postSql" type="textarea" :disabled="taskRunning"
                          resize="none" :placeholder="$t('sync.sqlPostPh')" />
              </div>
            </div>
          </div>
        </el-tab-pane>

        <!-- ⑤ 行过滤（按表设置）
             一份全局 WHERE 在多表同步里没有意义：`amount > 100` 拼到没有 amount 列的表上，
             整张表直接报错。所以改成「表 → 条件」，留空的表全量同步。 -->
        <el-tab-pane name="where">
          <template #label>
            <span class="opt-tab">{{ $t('sync.tabWhere') }}
              <span v-if="whereCount" class="opt-tab-badge">{{ whereCount }}</span>
            </span>
          </template>
          <div class="opt-pane">
            <div class="pane-head">
              <el-icon :size="13"><Filter /></el-icon>
              <span class="pane-title">{{ $t('sync.wherePerTable') }}</span>
              <el-input v-if="selTables.length > 5" v-model="whereSearch" size="small" clearable
                        :placeholder="$t('sync.whereSearch')" class="sw-search" />
            </div>
            <p class="pane-hint">{{ $t('sync.wherePerTableHint') }}</p>
            <div v-if="selTables.length" class="sw-list">
              <div v-for="name in whereTableList" :key="name" class="sw-item"
                   :class="{ on: !!(whereByTable[name] || '').trim() }">
                <span class="sw-name" :title="name">{{ name }}</span>
                <el-input v-model="whereByTable[name]" size="small" :disabled="taskRunning"
                          :placeholder="$t('sync.wherePh')" maxlength="500" />
              </div>
              <div v-if="!whereTableList.length" class="sw-none">{{ $t('sync.whereNoMatch') }}</div>
            </div>
            <div v-else class="sw-none">{{ $t('sync.whereNoTable') }}</div>
          </div>
        </el-tab-pane>
      </el-tabs>
    </div>

    <!-- ==================== 步骤 4：摘要 ==================== -->
    <div class="summary-card" v-if="step === 4">
      <div class="summary-title">{{ $t('sync.stepSummary') }}</div>
      <el-table :data="summaryRows" size="small" border class="data-table summary-table">
        <!-- 行过滤按表展开，标签是「WHERE · 表名」，表名长了要能悬停看全 -->
        <el-table-column prop="label" width="180" show-overflow-tooltip />
        <el-table-column prop="value" show-overflow-tooltip />
      </el-table>
      <el-alert v-if="opts.objectPolicy === 'drop'" :title="$t('sync.dropConfirm')" type="warning" :closable="false" style="margin-top: 10px;" />
    </div>

    <!-- ==================== 步骤 5：执行 ==================== -->
    <!-- has-result：结果表格出来了吗。跑的过程中还没有结果表，日志要独占剩余高度，
         不能只占一半把下面留空（用户口径「结果表格没出来之前这个也高度自适应」）。 -->
    <div v-if="step === 5" class="run-view" :class="{ 'has-result': !!taskResult }">
      <div class="pg-summary">
        <!-- 状态色：失败=红；取消=黄（不是"完成"，但也不该报成错误）；完成=绿 -->
        <!-- 百分比挪到标题行右侧（大号等宽数字），进度条本身只当"刻度尺"：
             `text-inside` 在细条里会挤成一团，10% 时更是被压在左端看不清 -->
        <div class="pg-top">
          <span class="pg-title">
            <el-icon :size="14"><Loading v-if="taskRunning" /><CircleCheck v-else /></el-icon>
            {{ $t('sync.progress') }}
          </span>
          <span class="pg-pct" :class="{ done: isTaskDone, err: taskStatus === 'error', warn: taskStatus === 'canceled' }">
            {{ progressPct }}%
          </span>
        </div>
        <el-progress :percentage="progressPct" :stroke-width="8" :show-text="false"
                     :status="taskStatus === 'error' ? 'exception'
                              : (isTaskDone ? 'success' : (taskStatus === 'canceled' ? 'warning' : ''))" />
        <!-- 「正在同步：表 xxx」提示行已按需求去掉 —— 表级阶段全在下面的日志里 -->
      </div>

      <div class="run-stats">
        <!-- Navicat 式：对象数 + 三个按行实时跳动的计数（读取/传输/失败）+ 耗时。
             读取/传输运行中就有（后端按行累加、轮询带回），不等终态 summary ——
             终态后用 summary 的最终值兜底（含 upsert 的 updated 行）。 -->
        <div class="stat"><span class="num">{{ fmtNum(totalShown) }}</span><span class="label">{{ $t('sync.statTotal') }}</span></div>
        <div class="stat"><span class="num">{{ fmtNum(readShown) }}</span><span class="label">{{ $t('sync.statRead') }}</span></div>
        <div class="stat ok"><span class="num">{{ fmtNum(writtenShown) }}</span><span class="label">{{ $t('sync.statTransferred') }}</span></div>
        <div class="stat danger"><span class="num">{{ fmtNum(failedShown) }}</span><span class="label">{{ $t('sync.statFailCount') }}</span></div>
        <div class="stat"><span class="num">{{ elapsedText }}</span><span class="label">{{ $t('sync.statElapsed') }}</span></div>
        <!-- 吞吐与剩余时间（DataX 的统计报告 / DBX 的进度页）：跑大表时「还要多久」
             比「已经搬了多少」更常被问到，而「行/秒 + MB/s」两个一起给才看得出是
             源端慢还是链路慢（行快字节少 = 窄表）。 -->
        <div class="stat stat-wide"><span class="num">{{ speedText }}</span><span class="label">{{ $t('sync.statSpeed') }}</span></div>
        <div class="stat stat-wide"><span class="num">{{ etaText }}</span><span class="label">{{ $t('sync.statEta') }}</span></div>
      </div>

      <div class="log-card pg-log" :class="{ collapsed: !logsOpen }">
        <!-- 日志头可点：**收起/展开**。跑完之后用户主要看结果表，日志占比太大；
             收起时标题与条数仍在，随时能再打开。 -->
        <div class="log-head fold-head" @click="logsOpen = !logsOpen">
          <div class="log-head-left">
            <el-icon :size="14"><Document /></el-icon>
            <span>{{ $t('sync.logs') }}</span>
          </div>
          <div class="log-head-right">
            <span class="log-count">{{ $t('mon.nRows', { n: syncLogs.length }) }}</span>
            <span class="fold-arrow" :class="{ closed: !logsOpen }"></span>
          </div>
        </div>
        <div ref="logScroll" class="log-body" v-show="logsOpen">
          <div v-if="!syncLogs.length" class="log-empty">
            <el-icon :size="28"><Document /></el-icon>
            <span>{{ $t('sync.waitStart') }}</span>
            <p>{{ $t('sync.waitTip') }}</p>
          </div>
          <!-- 每行带时间戳（后端生成），看得出每一步发生在什么时候 -->
          <div v-for="(line, i) in syncLogs" :key="i" class="log-line" :class="logClass(line)">
            <span class="log-time">{{ line.time }}</span>
            <span class="log-text">{{ line.text }}</span>
          </div>
        </div>
      </div>

      <!-- 同步结果（底部汇总条已按需求去掉 —— 结果表格里的状态列就是账本） -->
      <div v-if="taskResult" class="pg-result">
          <!-- **按类型小计**：视图/存储过程/函数/触发器排在所有表之后（后端就是这个执行顺序），
               400 个表的时候结果表里根本滚不到它们，用户只会看到「全是表，其他对象没输出」。
               这里给一排可点的类型徽标（表 400/400 · 视图 12/12…），点一下只看这一类。 -->
          <div class="res-types">
            <button v-for="t in resultTypes" :key="t.type" type="button"
                    class="res-type" :class="{ on: resFilter === t.type }"
                    @click="resFilter = resFilter === t.type ? '' : t.type">
              <span class="rt-name">{{ typeLabel(t.type) }}</span>
              <b>{{ t.ok }}</b>/{{ t.total }}
              <span v-if="t.failed" class="rt-bad">{{ $t('sync.statFailed') }} {{ t.failed }}</span>
            </button>
            <!-- 失败对象一键重跑：只把失败（含跳过的）对象凑成一份**参数完全一致**的新任务，
                 不必在几百行结果里手工挑出那几张表再重新勾一遍。 -->
            <el-button v-if="failedObjects.length && !taskRunning" class="res-rerun"
                       type="primary" plain size="small" @click="rerunFailed">
              {{ $t('sync.rerunFailed', { n: failedObjects.length }) }}
            </el-button>
          </div>
          <!-- 高度**跟着弹窗走**（不再写死 max-height）：弹窗拉高 / 全屏后，
               结果表把多出来的空间全部吃掉；日志收起时省下的高度也全归它。
               撑满的实现见下方 .pg-result 那组 flex 规则。 -->
          <el-table :data="resultRows" size="small" border class="data-table">
            <el-table-column type="index" label="#" width="45" />
            <el-table-column prop="type" :label="$t('udv.fType')" width="90">
              <template #default="{ row }">{{ typeLabel(row.type) }}</template>
            </el-table-column>
            <el-table-column prop="name" :label="$t('sync.objName')" min-width="150" show-overflow-tooltip />
            <el-table-column prop="status" :label="$t('odv.fStatus')" width="80">
              <template #default="{ row }">
                <el-tag :type="row.status === 'ok' ? 'success' : (row.status === 'skipped' ? 'info' : 'danger')" size="small" effect="dark">
                  {{ row.status === 'ok' ? $t('sync.statOk') : (row.status === 'skipped' ? $t('sync.statSkipped') : $t('sync.statFailed')) }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="$t('sync.colInsertedUpdated')" width="110">
              <template #default="{ row }">
                <span v-if="row.inserted != null" style="color: var(--dc-success);">+{{ row.inserted }}</span>
                <span v-if="row.updated" style="color: var(--dc-warning); margin-left: 4px;">~{{ row.updated }}</span>
              </template>
            </el-table-column>
            <el-table-column prop="message" :label="$t('udv.fComment')" min-width="220" show-overflow-tooltip />
          </el-table>
      </div>
    </div>

    <template #footer>
      <div class="foot-actions">
        <!-- 步骤 1-4：向导导航 -->
        <template v-if="step <= 4">
          <el-button @click="$emit('update:modelValue', false)">{{ $t('common.cancel') }}</el-button>
          <el-button v-if="step > 1" @click="step--">{{ $t('sync.prevStep') }}</el-button>
          <el-button v-if="step < 4" type="primary" @click="nextStep">{{ $t('sync.nextStep') }}</el-button>
          <!-- 必须写成 run()：run 的第一个参数是「现成 payload」（重跑失败对象用），
               直接写 @click="run" 会把 MouseEvent 当成 payload 发出去，
               后端于是报「缺少 sourceConnectionId 参数」（真机踩过） -->
          <el-button v-if="step === 4" type="success" :icon="Promotion" @click="run()">{{ $t('sync.start') }}</el-button>
        </template>
        <!-- 步骤 5：执行中/完成。运行中可「后台运行」：收起对话框，任务照跑，顶栏任务中心随时找回 -->
        <template v-else>
          <el-button v-if="taskRunning" :icon="Clock" @click="runInBackground">{{ $t('sync.runBg') }}</el-button>
          <el-button v-if="taskRunning" type="danger" plain :icon="VideoPause" :loading="stopping"
                     :disabled="stopping" @click="stopRun">{{ stopping ? $t('cmp.stopping') : $t('sync.stop') }}</el-button>
          <el-button v-else @click="$emit('update:modelValue', false)">{{ $t('common.close') }}</el-button>
        </template>
      </div>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, reactive, computed, nextTick, onMounted, onBeforeUnmount, watch } from 'vue'
import { ElMessage } from 'element-plus'
import {
  Promotion, Right, VideoPause, Search, FolderOpened, Files, Operation, Document,
  Grid, View, FirstAidKit, SetUp, Lightning, Calendar, Clock, Filter, QuestionFilled,
  Loading, CircleCheck, EditPen
} from '@element-plus/icons-vue'
import { listConnections, listDatabases, listSchemas, listTables, listProcedures, listTriggers, listEvents, getFeatures, syncDb, syncTaskStatus, syncCancel } from '../../api'
// 「库」这一层要不要画（Oracle / DM 没有库层：它就是连接里填的服务名）
import { hideDbLevelOf } from '../../types'
// 函数 / 存储过程的判据必须与左侧对象树同源（见 treeNodes.js 的 isFunctionRoutine），
// 否则会出现"树上算函数、同步时算存储过程"这种两边对不上的分叉
import { isFunctionRoutine } from '../data/treeNodes'
import { t } from '../../utils/i18n'
import { guardConnWrite, loadProtectProd } from '../../utils/connGuard'
import { groupConnections } from '../../utils/connGroups'
import { addBgTask, bgTasks } from './backgroundTasks'

const props = defineProps({ modelValue: Boolean, conn: Object, database: String, resumeTaskId: String })
const emit = defineEmits(['update:modelValue'])

// 向导步骤：1 源/目标 → 2 选项 → 3 对象 → 4 摘要 → 5 执行（Navicat 数据传输式分步）
const step = ref(1)
const connections = ref([])
/** 连接下拉按分组列出（分组名 = 左侧对象树的目录名，见 utils/connGroups） */
const connGroups = computed(() => groupConnections(connections.value))
const src = ref({ connectionId: '', database: '', schema: '' })
const tgt = ref({ connectionId: '', database: '', schema: '' })
const srcDbs = ref([])
const srcSchemas = ref([])
const tgtDbs = ref([])
const tgtSchemas = ref([])
const keyword = ref('')
/** 对象清单加载中（339 张表要 4 个接口，加载期间给个转圈，别让人以为列表是空的） */
const objsLoading = ref(false)

// ==================== 连接信息摘要 ====================
const connInfoOf = (id) => {
  const c = connections.value.find(x => String(x.id) === String(id))
  if (!c) return null
  return { type: c.type, name: c.name, host: c.host || '--', port: c.port || '--' }
}
const srcInfo = computed(() => connInfoOf(src.value.connectionId))
const tgtInfo = computed(() => connInfoOf(tgt.value.connectionId))

// 每批行数的**按库默认值**：必须与后端 `sync::insert_rows_default` 一致（改一边要改两边）。
// 判据不是「谁能塞最多」，而是「这条报文丢了要重传多久」+「驱动的硬限制」：
//   ClickHouse 写 part，批量大才省；SQL Server 多行 VALUES 硬限 1000 行；
//   Doris 的 FE packet 只有 1MB；MySQL 在丢包链路上 1000 行最稳；本地库没有往返成本。
const BATCH_ROWS_DEFAULT = {
  CLICKHOUSE: 10000,
  DORIS: 2000,
  POSTGRESQL: 2000,
  SQLSERVER: 1000,
  ORACLE: 1000,
  DM: 1000,
  DB2: 1000,
  SQLITE: 5000,
  H2: 5000,
  DERBY: 5000,
}
const tgtKindCode = computed(() => String(tgtInfo.value?.type || '').toUpperCase())
const srcKindCode = computed(() => String(srcInfo.value?.type || '').toUpperCase())
const batchRowsDefault = computed(() => BATCH_ROWS_DEFAULT[tgtKindCode.value] || 1000)

// 并发三项（表数 / 每表读 / 每表写）的可选值：0 = 自动。
// 用下拉而不是数字框：数字框的 min=1 会把「清空」夹回 1，用户改不回「自动」（真机反馈）；
// 下拉里「自动」是一个明确的选项，选回去就是 0。上限 16 已经远超方言声明的 6。
const CONCURRENCY_OPTIONS = [0, 1, 2, 3, 4, 5, 6, 8, 10, 12, 16]
const concurrencyValue = (v) => {
  const n = Number(v)
  return Number.isFinite(n) && n > 0 ? Math.round(n) : 0
}
/** 三个下拉共用一个写入口：0 = 自动 */
const onConcurrency = (key, value) => {
  opts.value[key] = concurrencyValue(value)
}

/** 摘要里显示的每批条数：没改过（0）就是目标库默认值 —— 「显示出来」而不是藏成 0 */
const batchRowsShown = computed(() => (opts.value.insertRows > 0 ? opts.value.insertRows : batchRowsDefault.value))

// ===== 每批条数 / 两条限速也改用下拉（与右侧并发三项统一样式）=====
// 首项是「自动（目标库默认）」/「不限制」= 0；后面是常用档位。
// 为什么不能继续用数字框：清空后会被 min 夹回、回不到「自动/不限制」（用户反馈）。
const BATCH_ROWS_OPTIONS = [0, 500, 1000, 2000, 5000, 10000, 20000, 50000]
const RATE_ROWS_OPTIONS = [0, 1000, 2000, 5000, 10000, 20000, 50000, 100000]
const RATE_MB_OPTIONS = [0, 0.5, 1, 2, 5, 10, 20, 50, 100]
/** 把「当前值」并进档位：老配置里可能是 333 / 17.5 这种自定义值，不能因为下拉里没有就丢掉 */
const withCurrent = (options, current) => {
  const set = new Set(options.map(Number))
  const n = Number(current)
  if (Number.isFinite(n) && n > 0) set.add(n)
  return [...set].sort((a, b) => a - b)
}
const batchRowsOptions = computed(() => withCurrent(BATCH_ROWS_OPTIONS, opts.value.insertRows))
const rateRowsOptions = computed(() => withCurrent(RATE_ROWS_OPTIONS, opts.value.rateRows))
const rateMbOptions = computed(() => withCurrent(RATE_MB_OPTIONS, opts.value.rateMb))
const onInsertRowsInput = (value) => {
  // 0 = 自动（后端按目标库方言默认处理）
  const n = Number(value)
  opts.value.insertRows = Number.isFinite(n) && n > 0 ? Math.round(n) : 0
}
const onRateRowsInput = (value) => {
  const n = Number(value)
  opts.value.rateRows = Number.isFinite(n) && n > 0 ? Math.round(n) : 0
}
const onRateMbInput = (value) => {
  const n = Number(value)
  opts.value.rateMb = Number.isFinite(n) && n > 0 ? Math.round(n * 10) / 10 : 0
}

const srcConnName = computed(() => srcInfo.value ? srcInfo.value.name : '--')
const tgtConnName = computed(() => tgtInfo.value ? tgtInfo.value.name : '--')

// ==================== 向导导航 ====================
// 步骤顺序与顶部步骤条一致：1 源/目标 → 2 对象 → 3 选项 → 4 摘要（曾经面板与标题错位过一格）
const nextStep = async () => {
  if (step.value === 1) {
    if (!src.value.connectionId || !src.value.database) return ElMessage.warning(t('sync.pickSourceDb'))
    if (!tgt.value.connectionId || !tgt.value.database) return ElMessage.warning(t('sync.pickTargetDb'))
    if (src.value.connectionId === tgt.value.connectionId && src.value.database === tgt.value.database) {
      return ElMessage.warning(t('sync.sameDb'))
    }
    // 进入对象页前确保清单已加载（换连接/换库时 onXxxChange 已自动加载，这里兜底）
    if (!totalAvailable.value) await loadSrcObjects()
  }
  // 对象页：一个都没选就拦下 —— 全选是默认值，空选基本是用户自己清空了
  if (step.value === 2 && !totalSelected.value) return ElMessage.warning(t('sync.pickObjectsWarn'))
  step.value++
}

// 摘要行（步骤 4 的表格数据）
// 前面配过的**全部**设置都列进来（开关 → 传输参数 → 按表设置）：
// 摘要页是点「开始同步」前最后一眼，只写源/目标/对象数的话，用户还得翻回上一步
// 确认「每批条数改没改、限速填了多少」。原来横排成一行小字的开关也一并在表里。
const yesNo = (v) => (v ? t('sync.yes') : t('sync.no'))
const summaryRows = computed(() => {
  const o = opts.value
  const timeoutText = o.timeout === 0
    ? t('sync.opTimeoutInherit')
    : (o.timeout < 0 ? t('sync.opTimeoutUnlimited') : t('sync.opTimeoutSec', { n: o.timeout }))
  // 0 = 不限（行/秒、MB/s 两条曲线各自独立）
  const unlimited = t('sync.unlimited')
  const rate = (v) => (v > 0 ? String(v) : unlimited)
  // 并发三项：0 = 自动，摘要里直接写「自动」而不是 0
  const autoText = (v) => (v > 0 ? String(v) : t('sync.auto'))
  return [
    { label: t('sync.source'), value: (srcConnName.value || '--') + ' ▸ ' + src.value.database + (src.value.schema ? '.' + src.value.schema : '') },
    { label: t('sync.target'), value: (tgtConnName.value || '--') + ' ▸ ' + tgt.value.database + (tgt.value.schema ? '.' + tgt.value.schema : '') },
    { label: t('sync.pickObjects'), value: String(totalSelected.value) },
    { label: t('sync.strategyTitle'), value: syncModeLabel.value },
    // ① 搬什么（结构 / 数据开关）
    { label: t('sync.sumAutoCreateDb'), value: yesNo(o.autoCreateDb) },
    { label: t('sync.sumAutoCreate'), value: yesNo(o.syncStructure) },
    // 「建表包含索引」只在要建表时才有意义，没勾自动建表就不占一行
    ...(o.syncStructure ? [{ label: t('sync.sumIndexes'), value: yesNo(o.includeIndexes) }] : []),
    { label: t('sync.sumSyncData'), value: yesNo(o.syncData) },
    { label: t('sync.sumStopOnError'), value: yesNo(o.stopOnError) },
    // ② 传输参数（显示的是**实际会用的值**：每批条数没改过就是目标库默认值）
    // 并发三项：0 = 自动（后端按类型声明与并发表数切预算）
    { label: t('sync.objConcurrency'), value: autoText(o.objConcurrency) },
    { label: t('sync.srcSlices'), value: autoText(o.srcSlices) },
    { label: t('sync.tgtWorkers'), value: autoText(o.tgtWorkers) },
    { label: t('sync.insertRows'), value: String(batchRowsShown.value) },
    { label: t('sync.opTimeout'), value: timeoutText },
    // 两条限速都没填时合成一行「限速：不限」—— 分开写就是两个「不限」，白占一行
    ...(o.rateRows > 0 || o.rateMb > 0
      ? [
        { label: t('sync.rateRows'), value: rate(o.rateRows) },
        { label: t('sync.rateMb'), value: rate(o.rateMb) },
      ]
      : [{ label: t('sync.rateLimit'), value: unlimited }]),
    // ③ 按表设置：只列真正改过的（没配的表全量/同名，不占行）
    ...selTables.value
      .filter(name => (renames[name] || '').trim())
      .map(name => ({ label: 'MAP · ' + name, value: '→ ' + renames[name].trim() })),
    ...selTables.value
      .filter(name => (whereByTable[name] || '').trim())
      .map(name => ({ label: 'WHERE · ' + name, value: whereByTable[name].trim() })),
    ...(sqlCount.value ? [{ label: t('sync.sqlScripts'), value: t('sync.sqlCount', { n: sqlCount.value }) }] : []),
  ]
})

// ==================== 执行统计（Navicat 式：对象数 / 已处理 / 已传输 / 错误 / 耗时） ====================
const runStartedAt = ref(0)
// 耗时显示的心跳：轮询时更新，保证 elapsed 在长时间无进度变化时也会走秒
const tickNow = ref(0)
/** 统计卡数字千分位：大数字（几十万行）没有分隔符根本读不出量级 */
const fmtNum = (n) => (typeof n === 'number' ? n.toLocaleString() : n)
const elapsedText = computed(() => {
  if (!runStartedAt.value) return '--'
  void tickNow.value
  const s = Math.floor((Date.now() - runStartedAt.value) / 1000)
  const h = String(Math.floor(s / 3600)).padStart(2, '0')
  const m = String(Math.floor((s % 3600) / 60)).padStart(2, '0')
  const sec = String(s % 60).padStart(2, '0')
  return `${h}:${m}:${sec}`
})

/** 后端累计的字节数（`estimate_row_bytes` 口径）：MB/s 由它算 */
const bytesRead = ref(0)
/** 字节速率的 EMA（与行速率同款平滑，见 onRowsFetched） */
const bytesRate = ref(0)

/**
 * 吞吐：`行/秒 · 字节/秒`（字节单位按量级自适应 MB/s / KB/s）。
 *
 * 为什么要两个一起给（DataX 的统计报告口径）：只看行/秒分不清「源端慢」还是「链路慢」——
 * 窄表 3 万行/秒可能只有 2MB/s（源端游标慢），宽表 3 千行/秒可能有 40MB/s（链路吃满）。
 * 用户调「限速 / 每批条数」时看的就是这两个数的关系。
 */
const speedText = computed(() => {
  void runningTick.value
  if (!taskRunning.value) return '--'
  const rows = Math.max(rates.value.read, rates.value.written)
  const bytes = bytesRate.value
  if (!rows && !bytes) return '--'
  // 单位自适应，且**永远不写 0**：固定用 MB/s 保留一位小数时，0.3 MB/s 会被舍成
  // 「0.0 MB/s」—— 和「后端没记字节」那个 bug 长得一模一样，用户会以为又坏了。
  // 小于 1 MB/s 走 KB/s，小于 1 KB/s 明说「<1 KB/s」，实在没有字节速率就只给行速。
  const mbs = bytes / 1048576
  const byteText =
    mbs >= 1 ? `${mbs.toFixed(1)} MB/s`
      : bytes >= 1024 ? `${(bytes / 1024).toFixed(1)} KB/s`
        : bytes > 0 ? '<1 KB/s'
          : ''
  const parts = []
  // 行速率同理：速度为 0 时宁可整段不显示，也不摆一个「0/s」
  // 一律给**真实数字 + 千分位**（`12,345/s`）：用户要拿它和源表行数、目标表 count 对账，
  // 「1.3 万/s」这种取整值没法核对（用户口径：显示真实数字）。
  if (rows > 0) {
    parts.push(`${Math.round(rows).toLocaleString()}/s`)
  }
  if (byteText) parts.push(byteText)
  return parts.length ? parts.join(' · ') : '--'
})

/** 毫秒 → 「1 小时 05 分」/「12 分 30 秒」/「45 秒」（文案走词典，中英一致） */
const fmtDuration = (ms) => {
  const secs = Math.max(0, Math.round(ms / 1000))
  if (secs >= 3600) {
    return t('sync.durHour', {
      h: Math.floor(secs / 3600),
      m: String(Math.floor((secs % 3600) / 60)).padStart(2, '0'),
    })
  }
  if (secs >= 60) {
    return t('sync.durMin', { m: Math.floor(secs / 60), s: String(secs % 60).padStart(2, '0') })
  }
  return t('sync.durSec', { s: secs })
}

/**
 * 剩余时间：按**对象数**的完成节奏外推。
 *
 * 不按行数外推的原因：后端只知道「已搬多少行」，不知道「总共多少行」（全表 count 太贵，
 * 而且无键表越往后越慢，按行均速外推反而越算越乐观）。对象数是有确切分母的，且整库
 * 同步里对象大小差别不会太离谱 —— 尾部剩一两张大表时它会偏乐观，这是可接受的取舍。
 */
const etaText = computed(() => {
  if (!taskRunning.value) return '--'
  void tickNow.value
  const total = totalShown.value
  const done = taskDone.value
  if (!total || done <= 0 || done >= total || !runStartedAt.value) return '--'
  const perObject = (Date.now() - runStartedAt.value) / done
  return fmtDuration(perObject * (total - done))
})

const srcFeatures = ref({})
const tgtFeatures = ref({})
// ⚠ 后端 features 的键是 **supportsSchemas**（复数）——写成单数永远 undefined，
// SQL Server / PG 的模式下拉就从不出现（真机踩过：SQL Server 选不了模式）
const srcNeedSchema = computed(() => srcFeatures.value.supportsSchemas === true)
const tgtNeedSchema = computed(() => tgtFeatures.value.supportsSchemas === true)
// 没有「库」这一层的类型（Oracle / DM）：库里填的就是连接自身的服务名，永远一条 ——
// 下拉不画（画出来就是把服务名摆给用户看），请求里照旧带它（见 loadSrcDbs / loadTgtDbs）
const srcHideDb = computed(() => hideDbLevelOf(srcInfo.value?.type))
const tgtHideDb = computed(() => hideDbLevelOf(tgtInfo.value?.type))

const TYPE_ICON = {
  table: Grid, view: View, function: FirstAidKit,
  procedure: SetUp, trigger: Lightning, event: Calendar
}
// 用函数而不是模块级常量：常量只在加载时求值一次，切换语言后对象类型名不会跟着变
const typeMeta = () => ({
  table:     { label: t('mv.catTable'), abbr: t('sync.abbrTable') },
  view:      { label: t('mv.catView'), abbr: t('sync.abbrView') },
  function:  { label: t('mv.catFunc'), abbr: t('sync.abbrFunction') },
  procedure: { label: t('mv.catProc'), abbr: t('sync.abbrProcedure') },
  trigger:   { label: t('mv.catTrigger'), abbr: t('sync.abbrTrigger') },
  event:     { label: t('mv.catEvent'), abbr: t('sync.abbrEvent') },
})
// 页签顺序就是这里的顺序；「有没有内容」只看 objState 的 list
const objTypes = reactive([
  { key: 'table' },
  { key: 'view' },
  { key: 'function' },
  { key: 'procedure' },
  { key: 'trigger' },
  { key: 'event' },
])
/** 当前页签（对象列表只画这一类） */
const activeTab = ref('table')
const objState = reactive({})
for (const t of objTypes) objState[t.key] = { list: [], selected: [] }

// rowLimit / whereClause 已移除：同步就是不限行数；行过滤改按表设置（见 whereByTable）
// 并发表数不在选项里：后端固定「同时 1 张表」（界面上也没有对应的行）

// ==================== 面板收起 ====================
// 选项页的五个页签：同步方式 / 执行选项 / 表名映射 / 前后置 SQL / 行过滤。
// 一类设置一个页签，页签名就是那一类设置的名字（不再有「过滤与脚本」下面
// 再套三块可折叠卡片那种两跳的翻找）。
const optionTab = ref('strategy')
const logsOpen = ref(true)

// 结果按类型筛选：后端执行顺序是「先所有表，再视图/存储过程/函数/触发器」，
// 400 个表时视图排在第 401 行起，表格只有十来行高 —— 不筛选就永远看不到它们。
const resFilter = ref('')
const resultTypes = computed(() => {
  const rows = taskResult.value?.results || []
  const map = new Map()
  for (const r of rows) {
    const t = r.type || 'table'
    if (!map.has(t)) map.set(t, { type: t, total: 0, ok: 0, failed: 0 })
    const g = map.get(t)
    g.total += 1
    if (r.status === 'ok') g.ok += 1
    else if (r.status !== 'skipped') g.failed += 1
  }
  // 表放最前（数量最多、用户最关心），其余按类型名排序，位置稳定不跳动
  return [...map.values()].sort((a, b) =>
    a.type === 'table' ? -1 : (b.type === 'table' ? 1 : a.type.localeCompare(b.type)))
})
const resultRows = computed(() => {
  const rows = taskResult.value?.results || []
  return resFilter.value ? rows.filter(r => (r.type || 'table') === resFilter.value) : rows
})
/** 失败的对象（「重跑失败对象」用）：只认 `error` —— `skipped` 多半是「本来就不该搬」 */
const failedObjects = computed(() => (taskResult.value?.results || []).filter(r => r.status === 'error'))
/**
 * 这条任务**发起时的原始请求体**（后端放在任务快照的 `context.request` 里）。
 *
 * 为什么需要：任务中心「查看」重开界面走的是 resumeTaskId —— 跳过向导直接看进度，
 * 界面上的 src/tgt/选项全是空的。此时点「重跑失败对象」，界面参数一律不可用，
 * 只能拿这份原始请求重发（真机：报「请选择源库」）。
 */
const taskRequest = ref(null)
/** 旧任务重跑时「待自动勾选」的对象（{type,name}）：拿不到原始参数就退回第一步重选 */
const pendingRerun = ref([])
/**
 * 重跑失败对象：**沿用当前参数**，只把失败的那些对象重新提交一次。
 *
 * 整库同步里几十张表失败一两张是常态（锁等待、超长字段、临时断连），以前只能整趟重跑
 * （几十分钟）或者手工在几百行里挑表重新勾选 —— 后者还容易把参数改错（例如把「清空后
 * 导入」误点成「仅插入」）。这里参数原样复用，只换对象清单。
 */
const rerunFailed = async () => {
  const failed = failedObjects.value.filter(r => r.name)
  if (!failed.length) return
  // 按类型分组：结果里的 `type` 与 payload 的字段是一一对应的（table → tables …）
  const fieldOf = {
    table: 'tables', view: 'views', function: 'functions',
    procedure: 'procedures', trigger: 'triggers', event: 'events',
  }
  const lists = { tables: [], views: [], functions: [], procedures: [], triggers: [], events: [] }
  for (const r of failed) lists[fieldOf[r.type || 'table'] || 'tables'].push(r.name)
  // **参数原样复用提交时的那份请求**（后端挂在任务上下文里）：
  // 从任务中心「查看」进来的界面跳过了向导，连接/库/选项/行过滤/表名映射/前后置 SQL
  // 全是空的，只有这份请求能保证「参数完全一致」。
  if (taskRequest.value) return run({ ...taskRequest.value, ...lists })
  // 同一次打开里的重跑：界面参数是活的，直接按界面组装
  if (src.value.connectionId && src.value.database && tgt.value.connectionId && tgt.value.database) {
    return run(buildPayload(new Set(failed.map(r => r.name))))
  }
  // 剩下的情况：任务由**旧版本内核**创建（没存原始请求）+ 界面又是从任务中心恢复的空壳。
  // 这时无论调什么都发不出去，直接告诉用户怎么办，并把这批失败对象带到第一步去 ——
  // 否则就只剩一句看不懂的「请选择源库」（真机反复踩）。
  pendingRerun.value = failed.map(r => ({ type: r.type || 'table', name: r.name }))
  resumedView.value = false
  taskId.value = ''
  step.value = 1
  ElMessage.warning(t('sync.rerunNoParams', { n: failed.length }))
}
const opts = ref({
  // 默认「删除重建」：重建出来的表是空的，dataMode 只能是 insert（见下面的 MODE_DEFS）
  objectPolicy: 'drop', dataMode: 'insert', syncStructure: true, syncData: true,
  includeIndexes: true, autoCreateDb: true, stopOnError: false, insertRows: 0, timeout: 0,
  // 并发三项（0 = 自动，由后端按类型声明与并发表数切预算）：
  // objConcurrency = 同时搬几张表；srcSlices = 每表读几片；tgtWorkers = 每表写几路
  objConcurrency: 0, srcSlices: 0, tgtWorkers: 0,
  // 限速（0 = 不限）：行/秒 与 MB/s 各一条曲线，见后端 transfer::RateLimit
  rateRows: 0, rateMb: 0,
  // 前置 / 后置 SQL（整段脚本，分号或换行分隔；每个对象各执行一遍）
  preSql: '', postSql: '',
})

// ==================== 同步方式（把 objectPolicy + dataMode 合并成一个选择） ====================
/**
 * 一种「同步方式」= 一个 (objectPolicy, dataMode) 组合，界面只让用户选一个。
 *
 * 为什么合并：这两件事本来就不独立 —— 「删除重建」时目标表是刚按源结构建出来的空表，
 * 「按主键更新」永远命中不了任何已存在的行（后端也按仅插入兜底）。分工两组单选时，
 * 右边必须跟着左边置灰，两块说明还互相矛盾；合并后互斥关系天然消失。
 *
 * 提交的 payload 字段（objectPolicy / dataMode）原样保留，后端契约不变。
 */
const MODE_DEFS = [
  { value: 'drop', policy: 'drop', data: 'insert' },
  { value: 'truncate_insert', policy: 'merge', data: 'truncate_insert' },
  { value: 'upsert', policy: 'merge', data: 'upsert' },
  { value: 'insert', policy: 'merge', data: 'insert' },
]
/** 当前选中的同步方式：读写直接落到 opts 的两个字段上 */
const syncMode = computed({
  get: () => {
    if (opts.value.objectPolicy === 'drop') return 'drop'
    // 不同步数据时几种写入方式没有区别，统一显示成「保留已有对象」
    return opts.value.syncData ? opts.value.dataMode : 'merge'
  },
  set: (v) => {
    const def = MODE_DEFS.find(m => m.value === v)
    if (!def) {
      // 「保留已有对象」（只在不同步数据时出现）：dataMode 保持原样，
      // 重新勾上「同步表数据」后能接回上次选的写入方式
      opts.value.objectPolicy = 'merge'
      return
    }
    opts.value.objectPolicy = def.policy
    opts.value.dataMode = def.data
  },
})
/** 选项列表：关了「同步表数据」就只剩结构怎么处理这两项 */
const modeOptions = computed(() => (opts.value.syncData
  ? [
    { value: 'drop', title: t('sync.modeDrop') },
    { value: 'truncate_insert', title: t('sync.modeTruncate') },
    { value: 'upsert', title: t('sync.modeUpsert') },
    { value: 'insert', title: t('sync.modeInsert') },
  ]
  : [
    { value: 'drop', title: t('sync.modeDrop') },
    { value: 'merge', title: t('sync.modeKeepStructure') },
  ]))
/** 摘要页显示的名字 */
const syncModeLabel = computed(() => {
  const hit = modeOptions.value.find(m => m.value === syncMode.value)
  return hit ? hit.title : '--'
})

// ==================== 行过滤（按表设置） ====================
// 为什么不是全局一个 WHERE：一次同步几十上百张表时，`amount > 100` 拼到没有 amount 列的
// 表上就是整张表报错。这里用「表名 → 条件」的映射，只有填了的表才带 where，其余全量。
const whereByTable = reactive({})
/** 本次勾选的表（行过滤只对表有意义：视图/例程/触发器同步的是 DDL） */
const selTables = computed(() => (objState.table ? objState.table.selected : []))
/** 表很多时给个搜索框，省得从上拉到下找那一张 */
const whereSearch = ref('')
const whereTableList = computed(() => {
  const kw = whereSearch.value.trim().toLowerCase()
  if (!kw) return selTables.value
  return selTables.value.filter(name => name.toLowerCase().includes(kw))
})
/** 已设置条件的表数（标题上的徽标） */
const whereCount = computed(() => selTables.value.filter(name => (whereByTable[name] || '').trim()).length)
// ==================== 目标表名映射（按表设置） ====================
// 与行过滤同形：一张表一个「目标名」，留空 = 同名。改名与过滤必须一起清空 ——
// 换了源库以后，旧的映射键指向的是另一批表。
const renames = reactive({})
const renameSearch = ref('')
/** 已改名的表数（标题上的徽标） */
const renameCount = computed(() => selTables.value.filter(name => (renames[name] || '').trim()).length)
const renameTableList = computed(() => {
  const kw = renameSearch.value.trim().toLowerCase()
  if (!kw) return selTables.value
  return selTables.value.filter(name => name.toLowerCase().includes(kw))
})

// ==================== 前置 / 后置 SQL ====================
// 两份脚本按**对象**执行（每个对象各跑一遍），不是整趟任务只跑一次：
// 「搬之前禁外键、搬之后建索引」这类用法只有按对象才说得通。
/** 已填写的脚本条数（分号/换行分隔）：页签上的徽标，切走以后也知道里面有东西 */
const sqlCount = computed(() => {
  const count = (text) => String(text || '').split(/[;\n]/).filter(x => x.trim()).length
  return count(opts.value.preSql) + count(opts.value.postSql)
})
// 换了源连接 / 源库，表清单就是另一批了：旧条件按表名留着只会在**同名表**上误伤
// （上一次给 orders 设了 `amount > 100`，这次换个库里的 orders 没有 amount 列 —— 一开跑就报错）
watch([() => src.value.connectionId, () => src.value.database], () => {
  for (const name of Object.keys(whereByTable)) delete whereByTable[name]
  for (const name of Object.keys(renames)) delete renames[name]
  whereSearch.value = ''
  renameSearch.value = ''
})

// 任务状态
let pollTimer = null
let tickTimer = null
const taskId = ref('')
const taskRunning = ref(false)

/**
 * 右上角 X 常驻显示；主弹窗与进度弹窗都在任务运行中时不直接关 ——
 * 关掉只是「看不见」，任务照旧在跑，用户会误以为已经停了。
 */
const onBeforeClose = (done) => {
  if (taskRunning.value) {
    ElMessage.warning(t('sync.busyStop'))
    return
  }
  done()
}
const onProgressBeforeClose = (done) => {
  if (taskRunning.value) {
    ElMessage.warning(t('sync.busyClose'))
    return
  }
  done()
}
const taskStatus = ref('')
const taskDone = ref(0)
const taskTotal = ref(0)
// 后端在「对象还没数完」时把 total 置为 -1（约定：总量未知）。界面不能把 -1 直接显示成
// 「对象总数 -1」（真机截图反馈），负数一律按 0 显示 —— 数完后自然跳到真实值。
const totalShown = computed(() => Math.max(0, Number(taskTotal.value) || 0))
// 行级实时计数（Navicat 式三行数）：后端按行累加、0.8 秒轮询一次 ——
// 界面上数字是平滑跳动递增的，不再「一万一的跳」
const rowsRead = ref(0)
const rowsWritten = ref(0)
const rowsFailed = ref(0)
const taskCurrent = ref('')
// 统计卡显示值：运行中用实时行计数；
// 终态后 task 快照的计数定格，summary 与它一致，直接用实时值即可（兜 0）。
// summary 的 inserted+updated 与 rowsWritten 本就同源，不需要再拼。
//
// **平滑递增**：轮询约 1 秒一次，每次 +1 万的跳变看着像卡死突然蹦一格。
// 这里对三个计数做补间（ease-out，~450ms 滚到新值）—— 轮询间隔内的跳变
// 变成连续滚动，大表传输时数字一直在动，体感是「在跑」而不是「卡了」。
const tweenTo = (shown, target, allowDown = true) => {
  // **运行中永不倒退**：外插是按上次轮询的速率猜的，真实计数有抖动 ——
  // 目标低于当前显示值时说明猜超前了，停在原地等真实值追上来，而不是往回滚
  //（真机踩过：数字越跳越少）。终态（rates 归零）才允许落到最终值。
  const from = shown.value
  const to = allowDown ? target : Math.max(target, from)
  if (to === from) return
  const t0 = performance.now()
  const dur = 450
  const step = (ts) => {
    const p = Math.min(1, (ts - t0) / dur)
    shown.value = Math.round(from + (to - from) * (1 - Math.pow(1 - p, 3)))
    if (p < 1) shown._raf = requestAnimationFrame(step)
  }
  if (shown._raf) cancelAnimationFrame(shown._raf)
  shown._raf = requestAnimationFrame(step)
}
const readShown = ref(0)
const writtenShown = ref(0)
const failedShown = ref(0)
// **速率外插**：轮询 800ms 一次，靠 tween 追的话数字会"顿一下蹦一下"。
// 把最近几次轮询的行速率（EMA 平滑）记下来，两次轮询之间按速率**匀速外插**目标值
//—— 数字每一帧都在走，肉眼看到的是连续增长的流水，而不是间歇跳变。
// 终态速率归零，数字落到快照值定格。
const fetchAt = ref(0)
const prevRows = ref({ read: 0, written: 0 })
const prevBytes = ref(0)
const rates = ref({ read: 0, written: 0 })
const onRowsFetched = (read, written, bytes, running) => {
  const now = Date.now()
  if (fetchAt.value && running) {
    const dt = (now - fetchAt.value) / 1000
    if (dt > 0.2) {
      const rr = Math.max(0, (read - prevRows.value.read) / dt)
      const wr = Math.max(0, (written - prevRows.value.written) / dt)
      // 字节速率同口径平滑：宽表的 MB/s 波动比行速更剧烈（大字段一页顶几十页）
      const br = Math.max(0, (bytes - prevBytes.value) / dt)
      // EMA 平滑：单次抖动不会让数字忽快忽慢
      rates.value = { read: rates.value.read * 0.6 + rr * 0.4, written: rates.value.written * 0.6 + wr * 0.4 }
      bytesRate.value = bytesRate.value * 0.6 + br * 0.4
    }
  }
  if (!running) {
    rates.value = { read: 0, written: 0 }
    bytesRate.value = 0
  }
  prevRows.value = { read, written }
  prevBytes.value = bytes
  fetchAt.value = now
}
const runningTick = ref(0)
// 句柄必须存下来：这个 100ms 心跳跟着组件活，不清理的话每次打开「数据传输」都留一个孤儿定时器
tickTimer = setInterval(() => { if (taskRunning.value) runningTick.value++ }, 100)
const extrapolate = (base, rate) => {
  runningTick.value
  if (!taskRunning.value || !fetchAt.value) return base
  // 外插打 6 折，且**时间窗钳制在 0.8s**：页面节流/轮询卡顿时 dt 会失控变大，
  // 不钳制的话「速率 × 时间」能冲到远超真实值（真机：已读取 20.5 万显示成 22 万），
  // Math.max 防倒退保护还会让它卡在虚高处不回落。
  // 打折的理由同样是「不许回退」：外插只是让数字在两个轮询之间继续走，猜超的部分
  // 只能等真实值追上来（显示值不回退），猜得越保守，停住等的时间越短。
  const dt = Math.min(Math.max(0, (Date.now() - fetchAt.value) / 1000), 0.8)
  return base + rate * 0.6 * dt
}
const targetRead = computed(() => Math.floor(extrapolate(rowsRead.value, rates.value.read)))
// **传输数恒 ≤ 读取数**（写入行 ⊆ 读取行，语义不变式）—— 两个数各自外插时
// 领先量不同步，会出现「传输数比读取数还多」的荒诞画面（真机踩过）
const targetWritten = computed(() => Math.min(
  Math.floor(extrapolate(rowsWritten.value, rates.value.written)),
  targetRead.value,
))
const targetFailed = computed(() => failedBase.value)
const taskMessage = ref('')
const taskResult = ref(null)
const syncLogs = ref([])
const logScroll = ref(null)
const showProgressDlg = ref(false)
// 失败卡的兜底：对象级失败（建表失败 / identity 开关失败等，还没读到行就死了）不计入
// 行失败计数 —— 终态用结果表里的失败对象数托底，失败卡至少亮出来，别再「结果表 3 个
// 失败、失败卡却是 0」的困惑（真机踩过）。
// ⚠ 必须放在 taskResult 声明**之后**：computed 首次求值就访问它，放前面是 TDZ 报错
//（「Cannot access before initialization」，点数据传输整个弹窗打不开 —— 真机踩过）
const failedBase = computed(() => {
  const objFails = (taskResult.value?.results || []).filter(r => r.status === 'error').length
  return Math.max(rowsFailed.value, objFails)
})
watch([targetRead, targetWritten, targetFailed], () => {
// 运行中禁止倒退（外插猜超了就等真实值追上）；终态 rates 归零，允许落定。
// **运行中不逐次 tween**：轮询每 800ms 一次、每次都重启 450ms 补间，速度曲线
// 锯齿化，体感是「一顿一顿蹦」。运行中改由 raf 循环逐帧外插（见下方 liveLoop），
// 数字每一帧都在走；终态才用 tween 平滑落定。
if (taskRunning.value) return
tweenTo(readShown, targetRead.value, true)
tweenTo(writtenShown, targetWritten.value, true)
tweenTo(failedShown, targetFailed.value, true)
})
// **运行中的连续计数**：raf 每帧把显示值推到「真实值 + 速率 × 时间」的外插点上 ——
// 两个轮询之间数字也在匀速增长，用户看到的是一条连续上涨的流水，而不是间歇跳变。
// Math.max 保护：外插打 9 折仍可能瞬时猜超，绝不让数字往回滚（真机踩过）。
let liveRaf = 0
const liveLoop = () => {
if (!taskRunning.value) { liveRaf = 0; return }
readShown.value = Math.max(readShown.value, Math.floor(extrapolate(rowsRead.value, rates.value.read)))
writtenShown.value = Math.max(writtenShown.value, Math.floor(extrapolate(rowsWritten.value, rates.value.written)))
failedShown.value = Math.max(failedShown.value, targetFailed.value)
liveRaf = requestAnimationFrame(liveLoop)
}
watch(taskRunning, (running) => {
if (running) {
if (!liveRaf) liveRaf = requestAnimationFrame(liveLoop)
return
}
// 落定：外插停掉，tween 平滑滚到最终真实值定格
if (liveRaf) { cancelAnimationFrame(liveRaf); liveRaf = 0 }
tweenTo(readShown, targetRead.value, true)
tweenTo(writtenShown, targetWritten.value, true)
tweenTo(failedShown, targetFailed.value, true)
}, { immediate: true })
/** 「停止」请求已发出（按钮据此变成「正在停止…」并禁用，避免连点） */
const stopping = ref(false)
/** 兜底定时器：停止请求发出后若迟迟不达终态，15 秒后放开关闭（见 stopRun） */
let stopGuard = null

const onProgressClosed = () => {
  if (!taskRunning.value) {
    syncLogs.value = []
    taskResult.value = null
  }
}

const logClass = (line) => {
  const t = line.text
  if (t.includes('失败') || t.includes('错误')) return 'err'
  if (t.includes('跳过') || t.includes('已取消')) return 'warn'
  if (t.includes('成功')) return 'ok'
  return ''
}

// ==================== 派生数据 ====================
const totalSelected = computed(() => objTypes.reduce((s, t) => s + objState[t.key].selected.length, 0))
// 分母固定为**已加载的全部对象**（不随搜索/展开变）：跟着变会让「已选 5 / 2」这种不可能的数字出现
const totalAvailable = computed(() => objTypes.reduce((s, t) => s + objState[t.key].list.length, 0))
const availableCount = computed(() => objTypes.reduce((s, t) => s + (objState[t.key].list.length ? 1 : 0), 0))
/** 某一类对象在当前搜索词下要显示哪些（搜索只过滤展示，不影响已选） */
const groupItems = (key) => {
  const kw = keyword.value.trim().toLowerCase()
  const list = objState[key].list
  return kw ? list.filter(n => n.toLowerCase().includes(kw)) : list
}
/** 搜索时自动切到**第一个有命中的页签**，免得用户以为「搜不到」 */
watch(keyword, (kw) => {
  if (!kw.trim()) return
  const hit = objTypes.find(g => groupItems(g.key).length)
  if (hit) activeTab.value = hit.key
})

const isTaskDone = computed(() => taskStatus.value === 'success')

/**
 * 任务终态有四种，**取消叫 `canceled`**（见后端 `tasks.rs` 的 FinishGuard）：
 * `success` 完成 / `canceled` 已取消 / `error` 失败 / `notfound` 任务过期（只保留 30 分钟）。
 *
 * 这里踩过一次坑：以前只认 success 与 error，于是点了「停止同步」之后，
 * 后端其实几秒内就收尾了（实测取消后 t+3s 就是 canceled），界面却永远停在"同步中" ——
 * `taskRunning` 不落回 false，进度对话框就一直被"请先点停止再关闭"挡着，**关不掉**。
 * 凡是终态都必须放行 —— 这是"关得掉"的唯一保证。
 */
const TERMINAL_STATUS = ['success', 'canceled', 'error', 'notfound']
/** 任务已经不再运行（成功 / 已取消 / 失败 / 过期）—— 界面据此放行「关闭」 */
const isTaskSettled = computed(() => TERMINAL_STATUS.includes(taskStatus.value))

/** 进度条百分比：完成后直接给 100（各对象粒度不同，停在 done/total 的原始值上会像"卡住"） */
const progressPct = computed(() => {
  if (isTaskDone.value) return 100
  // 取消时停在"实际写到哪"，不假装 100%
  if (!taskTotal.value || taskTotal.value < 0) return 0
  return Math.min(100, Math.round(taskDone.value / taskTotal.value * 100))
})

/**
 * 进度文案。前缀「正在同步：」与计数「（已完成/总数）」**只在界面这一处拼**。
 *
 * 后端那条 `phase`（接口里以 `current` 返回）曾经自己又写了一遍「同步 X（1/5）」，
 * 于是界面渲染成「正在同步：同步 X（1/5）（1/5）」—— 动词和计数各重复一次。
 * 现在后端只说对象名，这里统一负责措辞。
 *
 * `total < 0` 是内核约定的「总量未知」，不能拼成「（0/-1）」。
 */
const progressText = computed(() => {
  if (taskStatus.value === 'error') return taskMessage.value
  if (isTaskDone.value) return t('sync.done')
  // 取消：说清楚"停在哪"，也说明已写入的不回滚（否则很像白跑了一趟）
  if (taskStatus.value === 'canceled') {
    const done = taskTotal.value > 0 ? t('sync.doneCount', { done: taskDone.value, total: taskTotal.value }) : ''
    return t('sync.stopped') + done
  }
  if (taskStatus.value === 'notfound') return taskMessage.value || t('sync.taskExpired')
  const current = taskCurrent.value
  const counts = taskTotal.value > 0 ? t('sync.progressParen', { done: taskDone.value, total: taskTotal.value }) : ''
  return (current ? t('sync.runningWith', { current }) : t('sync.runningShort')) + counts
})

const typeLabel = (t) => (typeMeta()[t]?.label) || t

// ==================== 分组 / 对象操作 ====================
/**
 * 分组要不要显示：**由源库与目标库的能力决定**（用户要求「根据源数据库和目标数据库的特性展示」）。
 *
 * 判据优先用后端 features 的 `objectDdlKinds`（源库真能取出 DDL 的对象种类，
 * 例如 PG 是 view/procedure/function/trigger，没有 event），拿不到再退回
 * supportsProcedures / supportsTriggers / supportsEvents 这几个布尔。
 * 两边都支持才画（目标建不出同类对象时，勾了也只会得到一排「跳过」）；
 * 但**源库里确实有对象**的分组照旧显示 —— 宁可让用户看见再决定，也不能悄悄藏掉。
 */
const KIND_FLAG = {
  view: 'supportsDdl', procedure: 'supportsProcedures', function: 'supportsProcedures',
  trigger: 'supportsTriggers', event: 'supportsEvents',
}
const kindSupported = (feats, key) => {
  // 能力没取到（连接刚选好/接口失败）时一律按支持处理，不能因为拿不到就少摆一栏
  if (!feats || !Object.keys(feats).length) return true
  if (key === 'table') return true
  const kinds = feats.objectDdlKinds
  if (Array.isArray(kinds) && kinds.length) return kinds.includes(key)
  const flag = KIND_FLAG[key]
  return flag ? feats[flag] !== false : true
}
const visibleGroups = computed(() => objTypes.filter(g =>
  objState[g.key].list.length > 0
  || (kindSupported(srcFeatures.value, g.key) && kindSupported(tgtFeatures.value, g.key))
))
/** 当前页签对应的那一类（页签被能力过滤/切库重建时兜底到第一个可见页签） */
const activeGroup = computed(() =>
  visibleGroups.value.find(g => g.key === activeTab.value) || visibleGroups.value[0] || null
)
/** 组头勾选框三态：按**当前搜索过滤后的可见项**判断（搜「ods_」时勾一下就是勾这些） */
const groupChecked = (key) => {
  const items = groupItems(key)
  return items.length > 0 && items.every(n => objState[key].selected.includes(n))
}
const groupIndeterminate = (key) => {
  const items = groupItems(key)
  const n = items.filter(x => objState[key].selected.includes(x)).length
  return n > 0 && n < items.length
}
const setGroup = (key, val) => {
  const items = groupItems(key)
  const sel = new Set(objState[key].selected)
  for (const n of items) { if (val) sel.add(n); else sel.delete(n) }
  objState[key].selected = [...sel]
}
const isSelected = (o) => objState[o.type].selected.includes(o.name)
const toggleObj = (o, v) => {
  const sel = objState[o.type].selected
  const i = sel.indexOf(o.name)
  if (v && i < 0) sel.push(o.name)
  if (!v && i >= 0) sel.splice(i, 1)
}
/** 全部选择 / 取消全选：只作用于**显示出来的分组**（被能力过滤掉的不碰） */
const applyAll = (val) => {
  for (const g of visibleGroups.value) setGroup(g.key, val)
}

// ==================== 对象加载 ====================
const loadConnections = async () => {
  // 只展示真正有传输能力的连接。NoSQL 的连接模型不是二维表，后端没有
  // 对应 Reader/Writer；让它们出现在这里只会把“暂不支持”伪装成配置错误。
  connections.value = (await listConnections()).filter(c => c.dataTransfer !== false)
  // 只有从外部带着明确上下文进来（右键某连接传 props.conn）才预填；
  // 顶栏直接打开时**连连接也不预选**（用户要求）——库、连接全部「请选择」，
  // 让用户从头自己挑，避免默认就是上次随手选的东西
  if (props.conn) {
    src.value.connectionId = props.conn.id
    tgt.value.connectionId = props.conn.id
  }
  if (src.value.connectionId) await loadSrcDbs()
  if (tgt.value.connectionId) await loadTgtDbs()
  // 预填库必须在清单里才保留（清单是 internal 裸名；树曾给过 `internal.ods`
  // 这类全限定名 —— 不在清单就丢掉，别让无效值带进后续请求）
  if (props.conn && srcDbs.value.includes(props.database || '')) {
    src.value.database = props.database
    await onSrcDbChange()
  }
}

const onSrcConnChange = async () => {
  src.value.database = ''
  src.value.schema = ''
  srcDbs.value = []
  srcSchemas.value = []
  srcFeatures.value = {}
  await loadSrcDbs()
}
const loadSrcDbs = async () => {
  if (!src.value.connectionId) { srcDbs.value = []; return }
  if (srcHideDb.value) {
    // 没有库层：库值取连接自身的服务名（下拉不画），直接进「模式」那一步
    srcDbs.value = []
    src.value.database = String(srcInfo.value?.database || '')
    try { srcFeatures.value = await getFeatures(src.value.connectionId) } catch (e) { srcFeatures.value = {} }
    await onSrcDbChange()
    return
  }
  srcDbs.value = await listDatabases(src.value.connectionId)
  // **默认不预选**（用户要求）：库留空由用户自己选 —— 系统库（INFORMATION_SCHEMA）
  // 排第一时自动选中它，反而掩盖了真实业务库
  try { srcFeatures.value = await getFeatures(src.value.connectionId) } catch (e) { srcFeatures.value = {} }
  await onSrcDbChange()
}
const onSrcDbChange = async () => {
  src.value.schema = ''
  srcSchemas.value = []
  if (srcNeedSchema.value) {
    try { srcSchemas.value = await listSchemas(src.value.connectionId, src.value.database) } catch (e) { srcSchemas.value = [] }
    if (srcSchemas.value.length) src.value.schema = srcSchemas.value[0]
  }
  await loadSrcObjects()
}

const onTgtConnChange = async () => {
  tgt.value.database = ''
  tgt.value.schema = ''
  tgtDbs.value = []
  tgtSchemas.value = []
  tgtFeatures.value = {}
  await loadTgtDbs()
}
const loadTgtDbs = async () => {
  if (!tgt.value.connectionId) { tgtDbs.value = []; return }
  if (tgtHideDb.value) {
    // 同源：没有库层就取连接自身的服务名，下拉不画
    tgtDbs.value = []
    tgt.value.database = String(tgtInfo.value?.database || '')
    try { tgtFeatures.value = await getFeatures(tgt.value.connectionId) } catch (e) { tgtFeatures.value = {} }
    await onTgtDbChange()
    return
  }
  tgtDbs.value = await listDatabases(tgt.value.connectionId)
  // 同源：默认不预选
  try { tgtFeatures.value = await getFeatures(tgt.value.connectionId) } catch (e) { tgtFeatures.value = {} }
  await onTgtDbChange()
}
const onTgtDbChange = async () => {
  tgt.value.schema = ''
  tgtSchemas.value = []
  if (tgtNeedSchema.value) {
    try { tgtSchemas.value = await listSchemas(tgt.value.connectionId, tgt.value.database) } catch (e) { tgtSchemas.value = [] }
    if (tgtSchemas.value.length) tgt.value.schema = tgtSchemas.value[0]
  }
}

const loadSrcObjects = async () => {
  const connId = src.value.connectionId
  // 支持 schema 的数据库（SQL Server / PG 等）把选中的模式拼成 "库.模式"，后端据此按模式加载对象
  const refDb = srcNeedSchema.value && src.value.schema
    ? src.value.database + '.' + src.value.schema
    : src.value.database
  const db = refDb
  for (const t of objTypes) {
    objState[t.key].list = []
    objState[t.key].selected = []
  }
  keyword.value = ''
  if (!connId || !db) return
  objsLoading.value = true
  try {
    try {
      const tables = await listTables(connId, db)
      objState.table.list = tables.filter(t => (t.type || 'TABLE') === 'TABLE').map(t => t.name)
      objState.view.list = tables.filter(t => t.type === 'VIEW').map(t => t.name)
    } catch (e) { /* ignore */ }
    try {
      const procs = await listProcedures(connId, db)
      // 判据用 isFunctionRoutine（唯一的定义处，与树上分类、数量同源）。
      //
      // 原来这里判的是 `p.comment || p.type` 里有没有 "FUNCTION" —— 而这条接口
      // **根本不返回**这两个字段（后端给的是 routineType），所以条件恒为假：
      // 结果是**所有函数都被塞进了「存储过程」一栏**，函数那一栏永远是空的。
      // 与树上"函数混在 Procedures 里"是同一个病，一起治。
      const funcs = procs.filter(isFunctionRoutine)
      const procsOnly = procs.filter(p => !isFunctionRoutine(p))
      objState.function.list = funcs.map(p => p.name)
      objState.procedure.list = procsOnly.map(p => p.name)
    } catch (e) { /* ignore */ }
    try {
      objState.trigger.list = (await listTriggers(connId, db)).map(p => p.name)
    } catch (e) { /* ignore */ }
    try {
      objState.event.list = (await listEvents(connId, db)).map(p => p.name)
    } catch (e) { /* ignore */ }
  } finally {
    objsLoading.value = false
  }
  // **默认全选**（用户口径「有的都显示出来也都支持同步」）：进来就能直接下一步，
  // 不想同步的某一类在组头一次取消即可 —— 以前是全都不选 + 类型徽标当筛选器，
  // 界面停在「已选 0 / 339、列表空白」，看着像坏了。
  for (const t of objTypes) {
    objState[t.key].selected = [...objState[t.key].list]
  }
  // 默认停在第一个有内容的页签（表通常就是第一个；只有视图的库也不会停在空页签）
  const first = objTypes.find(t => objState[t.key].list.length > 0)
  if (first) activeTab.value = first.key
  // 「重跑失败对象」在旧任务上拿不到原始参数时会被退回第一步（见 rerunFailed）：
  // 清单默认全选，这里再收窄成上次失败的那几个 —— 用户只需重新确认源/目标。
  if (pendingRerun.value.length) {
    const want = pendingRerun.value
    pendingRerun.value = []
    for (const t of objTypes) objState[t.key].selected = []
    for (const r of want) {
      const key = objState[r.type] ? r.type : 'table'
      if (objState[key].list.includes(r.name)) objState[key].selected.push(r.name)
    }
    if (!totalSelected.value) ElMessage.warning(t('sync.rerunObjectsGone'))
  }
}

// ==================== 同步执行 ====================
/**
 * 组装 `/api/sync/db` 的请求体。
 *
 * `only`（可选）= 只提交这批对象名（「重跑失败对象」用）：其它参数（模式 / 映射 /
 * 限速 / 前后置 SQL）与上一次完全一致，用户不必重新勾一遍，也不会出现「重跑时手滑
 * 改了写入模式，把目标表清空了」这种事故。
 */
const buildPayload = (only = null) => {
  const pick = (key) => {
    const selected = objState[key].selected
    return only ? selected.filter(name => only.has(name)) : selected
  }
  return {
    sourceConnectionId: src.value.connectionId,
    sourceDatabase: src.value.database,
    sourceSchema: src.value.schema,
    targetConnectionId: tgt.value.connectionId,
    targetDatabase: tgt.value.database,
    targetSchema: tgt.value.schema,
    objectPolicy: opts.value.objectPolicy,
    dataMode: opts.value.dataMode,
    syncStructure: opts.value.syncStructure,
    syncData: opts.value.syncData,
    includeIndexes: opts.value.includeIndexes,
    autoCreateDb: opts.value.autoCreateDb,
    stopOnError: opts.value.stopOnError,
    // 并发三项：0 = 自动（后端按类型声明与并发表数切预算）；>0 以界面为准，
    // 且尾部弹性（别的表退出后扩片）不会超过这里填的值
    maxConcurrency: opts.value.objConcurrency || 0,
    srcSlices: opts.value.srcSlices || 0,
    tgtWorkers: opts.value.tgtWorkers || 0,
    // 0 = 按生成方（目标库）默认；>0 以界面为准 —— 后端再和字节上限、方言硬限制取小
    insertRows: opts.value.insertRows || 0,
    // 0 = 继承连接（用数据源上的 socketTimeout）；-1 = 不限制；>0 = 秒
    timeoutSeconds: opts.value.timeout,
    // 行过滤按表提交：只带真正填了条件的表（没勾选的表就算残留了旧条件也不传）
    whereClauses: Object.fromEntries(selTables.value
      .map(name => [name, (whereByTable[name] || '').trim()])
      .filter(([, clause]) => clause)),
    // 目标表名映射：只带真正改过名的（留空 = 同名，不提交）
    tableRenames: Object.fromEntries(selTables.value
      .map(name => [name, (renames[name] || '').trim()])
      .filter(([, to]) => to)),
    // 限速（0 = 不限）：MB/s → 字节/秒；后端两条曲线取更慢的那条
    rateLimitRows: opts.value.rateRows || 0,
    rateLimitBytes: opts.value.rateMb > 0 ? Math.round(opts.value.rateMb * 1048576) : 0,
    // 前后置 SQL：整段脚本原样提交，后端按「引号之外的分号 / 换行」切成多条语句
    preSql: (opts.value.preSql || '').trim(),
    postSql: (opts.value.postSql || '').trim(),
    tables: pick('table'),
    views: pick('view'),
    functions: pick('function'),
    procedures: pick('procedure'),
    triggers: pick('trigger'),
    events: pick('event'),
  }
}

const run = async (rawOverride = null) => {
  // 只认 buildPayload 产出的现成 payload（重跑失败对象用）。
  // 直接绑在 @click 上时传进来的是 MouseEvent —— 它不是 payload，若当成 payload 用，
  // axios 会把事件对象序列化发出去，后端必然报「缺少 sourceConnectionId 参数」。
  // 模板已改成 run()，这里再兜一层，避免以后又有人写成 @click="run"。
  const payloadOverride = rawOverride && typeof rawOverride === 'object' && 'sourceConnectionId' in rawOverride
    ? rawOverride
    : null
  const payload = payloadOverride || buildPayload()
  // 校验一律看**真正要提交的 payload**，不看界面上的 src/tgt：
  // 从任务中心「查看」进来时（resumeTaskId）界面跳过向导、src/tgt 是空的，
  // 而「重跑失败对象」用的是后端存的原始请求 —— 按界面校验就会误报「请选择源库」。
  if (!payload.sourceConnectionId || !payload.sourceDatabase) return ElMessage.warning(t('sync.pickSourceDb'))
  if (!payload.targetConnectionId || !payload.targetDatabase) return ElMessage.warning(t('sync.pickTargetDb'))
  // 目标库是要被写入的一方：只读 / 生产保护连接在点击执行那一刻就拦下（全站统一闸门）
  await loadProtectProd()
  const targetConn = connections.value.find(x => String(x.id) === String(payload.targetConnectionId))
  if (!guardConnWrite(targetConn || {}, t('nav.sync'))) return
  // 重跑失败对象时传的是现成 payload（对象清单已收窄），不再校验「勾选数」
  if (!payloadOverride && totalSelected.value === 0) return ElMessage.warning(t('sync.pickObjectsWarn'))
  if (payload.sourceConnectionId === payload.targetConnectionId
      && payload.sourceDatabase === payload.targetDatabase) {
    return ElMessage.warning(t('sync.sameDb'))
  }

  taskResult.value = null
  taskMessage.value = ''
  syncLogs.value = []
  // 新任务是自己提交、用户正盯着进度页 —— 终态提示照常弹（区别于任务中心「查看」恢复）
  resumedView.value = false
  // 进入执行步骤（向导第 5 步，不再弹二级进度框）
  step.value = 5
  runStartedAt.value = Date.now()
  try {
    const { taskId: tid } = await syncDb(payload)
    taskId.value = tid
    // **提交即登记**任务中心：不点「后台运行」的任务也要出现在执行记录里
    //（用户口径：执行成功的历史记录要在任务中心能看到），之后的状态刷新/快照留底统一走任务中心
    addBgTask({ id: tid, kind: 'sync', title: bgTitle.value })
    taskRunning.value = true
    taskStatus.value = 'running'
    taskDone.value = 0
    // 对象总数按**实际提交的清单**算：重跑失败对象时那份清单比当前勾选少
    taskTotal.value = ['tables', 'views', 'functions', 'procedures', 'triggers', 'events']
      .reduce((sum, key) => sum + (payload[key] || []).length, 0)
    rowsRead.value = 0
    rowsWritten.value = 0
    rowsFailed.value = 0
    bytesRead.value = 0
    bytesRate.value = 0
    prevBytes.value = 0
    rates.value = { read: 0, written: 0 }
    fetchAt.value = 0
    taskCurrent.value = t('cmp.preparing')
    pollTimer = setInterval(pollTask, 800)
    } catch (e) {
      ElMessage.error(t('sync.startFailed', { detail: e.message }))
      step.value = 4
    }
    }

    // ===== 后台运行 / 恢复 =====
    // 「后台运行」：任务在后端继续跑（后端任务本来独立于对话框），这里只是收起窗口
    // 并把 taskId 登记到任务中心；用户随时从顶栏任务中心点回来继续看进度。
    const runInBackground = () => {
    if (!taskId.value) return
    // kind 必须显式给：靠 addBgTask 里 prev?.kind 兜底时，同 id 复用/旧记录
    // 可能被错路由到 CompareDialog（审查发现）
    addBgTask({ id: taskId.value, kind: 'sync', title: bgTitle.value })
    // 与 CompareDialog 口径一致：后台运行即停轮询，状态刷新交给任务中心 ——
    // 不停的话对话框关了轮询还在 800ms 打接口，终态还会在主界面弹 toast
    stopPolling()
    // 「停止」的 15 秒兜底守卫一并解除：到期会把还在跑的任务当卡死强行收摊
    if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
    stopping.value = false
    ElMessage.success(t('sync.bgAdded'))
    emit('update:modelValue', false)
    }
    // 恢复标题（任务中心与后台登记共用）：「源连接.源库 → 目标连接.目标库」。
    // 只写库名时，同样叫 xingwei_kechuan 的两个不同实例分不清是哪一趟（真机反馈：
    // 任务中心那一列只有一个库名，看不出连的是哪台机器）。
    // 连接名与库名之间用**点**而不是斜杠（用户口径：`conn.db` 像一个对象路径，斜杠看着像 URL 路径）。
    const bgSide = (conn, db) => (db ? `${conn}.${db}` : conn)
    const bgTitle = computed(() => {
      const srcConn = srcConnName.value
      const tgtConn = tgtConnName.value
      // 连接名拿不到（从任务中心「查看」恢复时界面上的源/目标是空的）：**不给标题**。
      // 用 `--` 拼出来的「-- → --」也含可读字符，会通过 addBgTask 的无效标题检查，
      // 把记录里原本好好的标题冲成垃圾（同 id 再点一次「后台运行」就会发生）
      if (!srcConn || srcConn === '--' || !tgtConn || tgtConn === '--') return ''
      const srcDb = src.value.database || ''
      const tgtDb = tgt.value.database || ''
      // 两端库名相同（跨实例同步同名库，最常见）时，真正的差异点是**连接名**：
      // 「连接.库 → 连接.库」两边都长，任务中心/顶栏一截断，两个不同的任务看着一模一样
      // （用户真机反馈「2 个任务名字一样了」）。这种写法库名只出现一次，两个连接名都在最前面
      if (srcDb && srcDb === tgtDb) {
        return t('sync.bgTitleSameDb', { src: srcConn, tgt: tgtConn, db: srcDb })
      }
      return t('sync.bgTitle', { src: bgSide(srcConn, srcDb), tgt: bgSide(tgtConn, tgtDb) })
    })
    // 任务中心点「查看」→ 父组件带 resumeTaskId 打开对话框：跳过向导直接进进度页接着轮询。
    // ⚠ 下面那个「打开即重置向导」的 watch（step=1）与本 watch 同帧触发，注册序在前先跑 ——
    // 这里必须等 nextTick 之后再跳到 5，否则刚设置的进度页被重置回第一步（真机踩过）。
    watch([() => props.resumeTaskId, () => props.modelValue], async ([id, open]) => {
      if (!id || !open) return
      await nextTick()
      if (id !== props.resumeTaskId) return
      taskId.value = id
      step.value = 5
      taskRunning.value = true
      taskStatus.value = 'running'
      taskResult.value = null
      syncLogs.value = []
      // **从任务中心「查看」进来**：用户就是冲着结果界面来的 ——
      // 终态 toast（「同步完成…」）不再弹，结果已经在眼前（真机反馈）
      resumedView.value = true
      // 恢复时复位停止态与统计卡计数：上一次「停止」残留的 stopping 会让停止按钮
      // 假禁用（一直「正在停止…」）；不清 shown 计数则恢复瞬间先闪现上次的旧数字
      if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
      stopping.value = false
      readShown.value = 0
      writtenShown.value = 0
      failedShown.value = 0
      // 字节吞吐同样复位：恢复瞬间不该先闪出上一次任务的速度
      bytesRead.value = 0
      bytesRate.value = 0
      prevBytes.value = 0
      // 耗时卡先给个占位 —— pollTask 拿到任务快照的 elapsedMs 后按真实开始时间续算，
      // 从任务中心/悬浮恢复进来不能从 00:00 重新数（真机反馈）
      runStartedAt.value = Date.now()
      await pollTask()
      if (taskRunning.value) pollTimer = setInterval(pollTask, 800)
    })
    // 任务终态**不再从任务中心摘掉**：执行记录由用户自己删（用户口径：不删就保留），
    // 摘掉会让「本地留底」失效 —— 删除入口在任务中心每一行上
    /** 终态提示去重：同一任务同一次运行只弹一次完成/失败提示 ——
     *  恢复（resume）与收起前后的多次轮询都可能到达终态，不挡会反复弹（真机踩过） */
    const notifiedKey = ref('')
    /** **恢复查看标记**：任务中心「查看」进来的轮询不弹终态 toast ——
     *  用户就是冲着结果界面来的；新提交的任务（run 里复位）照常弹 */
    const resumedView = ref(false)
    watch(taskStatus, (s) => {
      if (!TERMINAL_STATUS.includes(s)) return
      const key = taskId.value + ':' + s
      if (notifiedKey.value === key) return
      notifiedKey.value = key
    })

const nowTime = () => {
  // **带日期时间（含年份）**：执行记录要能跨年回溯
  const d = new Date()
  const p2 = (n) => String(n).padStart(2, '0')
  const hm = `${p2(d.getHours())}:${p2(d.getMinutes())}:${p2(d.getSeconds())}`
  return `${d.getFullYear()}-${p2(d.getMonth() + 1)}-${p2(d.getDate())} ${hm}`
}

const mergeLogs = (serverLogs) => {
  if (!Array.isArray(serverLogs)) return
  const existing = new Set(syncLogs.value.map(l => l.text))
  let added = 0
  for (const text of serverLogs) {
    if (!existing.has(text)) {
      syncLogs.value.push({ time: nowTime(), text })
      added++
    }
  }
  const scrollEl = logScroll.value
  if (added && scrollEl) {
    setTimeout(() => { scrollEl.scrollTop = scrollEl.scrollHeight }, 50)
  }
}

// **轮询必须串行**：间隔只有 800ms，而 `/api/sync/task/{id}` 会把日志一并带回 ——
// 任务跑久了日志上千行、弱链路上单个请求几百毫秒到数秒都有。setInterval 不管上一个
// 请求有没有回来，于是同时挂着好几个，而它们**返回顺序无法保证**：先发出的旧快照
// 后到，就会把已经涨上去的「读取数/传输数」写回旧值 —— 界面表现就是数字越蹦越小
//（真机反馈，跑得越久日志越多、越容易叠）。这里同一时刻只允许一个请求在途。
let polling = false
let pollingTask = ''
const pollTask = async () => {
  // 同一任务只允许一个在途请求（见上）；换任务（任务中心恢复 / 重新开跑）时立刻放行，
  // 否则上一个任务的在途请求会把新任务的恢复首刷也挡掉（界面停在「准备中」）。
  if (polling && pollingTask === taskId.value) return
  const myTask = taskId.value
  polling = true
  pollingTask = myTask
  tickNow.value = Date.now()
  try {
    // 局部变量改名：本函数后面要用 t() 翻译，同名会把它遮蔽掉
    const st = await syncTaskStatus(myTask)
    // 请求在途期间任务已经换了（旧任务的迟到响应）：数字/状态一概不能回写
    if (myTask !== taskId.value) return
    taskStatus.value = st.status
    taskDone.value = st.done || 0
    taskTotal.value = st.total || taskTotal.value
    // 任务发起时的原始请求：任务中心「查看」重开界面后「重跑失败对象」靠它复原参数
    //（老内核没有 context，保持 null，重跑走「按界面组装」的兜底）
    if (st.context?.request) taskRequest.value = st.context.request
    // **运行中这两个计数只增不减**。后端本来就是累计工作量（重跑读/写过的行不再扣回），
    // 再夹一道 max 兜住「快照乱序 / 迟到」这种前端时序问题 —— 任何情况下都不让已经
    // 显示过的数字往回退（真机反馈：两个数都有越蹦越小的情况）。
    // 终态（含取消）用快照的真实值落定，收尾数字与结果表一致。
    const nextRead = Math.max(0, Number(st.rowsRead) || 0)
    const nextWritten = Math.max(0, Number(st.rowsWritten) || 0)
    if (taskRunning.value) {
      rowsRead.value = Math.max(rowsRead.value, nextRead)
      rowsWritten.value = Math.max(rowsWritten.value, nextWritten)
    } else {
      rowsRead.value = nextRead
      rowsWritten.value = nextWritten
    }
    rowsFailed.value = st.rowsFailed || 0
    // 字节吞吐（后端累计估算值）：宽表窄表都能给出同一尺度的速度
    const nextBytes = Math.max(0, Number(st.bytesRead) || 0)
    bytesRead.value = taskRunning.value ? Math.max(bytesRead.value, nextBytes) : nextBytes
    // 采样行/字节速率（供数字外插与「速度」卡，见 rates/onRowsFetched）；用真实值采样
    onRowsFetched(nextRead, nextWritten, nextBytes, taskRunning.value)
    // 接口把阶段文案放在 `current`（= 任务的 phase）；`phase` 兜底，免得字段名一变界面就空白
    taskCurrent.value = st.current || st.phase || ''
    // **耗时续算**：任务快照带 elapsedMs（后端任务的真实运行时长，终态后后端已冻结该值）——
    // 从任务中心/悬浮恢复进来时（包括已完成的任务），耗时卡按真实开始时间显示，不归零重数
    if (st.elapsedMs > 0) runStartedAt.value = Date.now() - st.elapsedMs
    mergeLogs(st.logs)
    // 只有**终态**才收摊。这里必须包含 canceled：取消后后端给的就是 canceled，
    // 漏掉它界面就永远停在"同步中"，对话框再也关不掉（见 TERMINAL_STATUS 的说明）。
    if (isTaskSettled.value) {
      stopPolling()
      if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
      taskRunning.value = false
      stopping.value = false
      // 终态提示**只弹一次**：恢复 + 多次轮询都可能撞上终态，弹过就不再弹
      const nkey = taskId.value + ':' + st.status
      const alreadyNotified = notifiedKey.value === nkey
      notifiedKey.value = nkey
      // 取消也把已写入的逐对象结果展示出来 —— 哪些表插了多少行是有用的信息，不该因为"取消"就丢掉
      taskResult.value = st.result || taskResult.value
      if (st.status === 'success') {
        // 恢复查看（任务中心进来）不弹：结果已经在界面上，toast 纯属打扰
        if (!alreadyNotified && !resumedView.value) ElMessage.success(st.result?.message || t('sync.done'))
      } else if (st.status === 'canceled') {
        if (!alreadyNotified) ElMessage.info(st.result?.message || st.message || t('sync.stoppedKeep'))
      } else if (st.status === 'notfound') {
        // 后端任务不在了（服务重启 / 超过内存保留期）：**本地快照兜底** ——
        // 任务中心的记录里存过终态结果就照常展示，别让用户看到一句"只保留 30 分钟"
        //（用户口径：记录只要不删就保留）；本地也没有才提示已中断
        const rec = bgTasks.find(x => x.id === taskId.value)
        const snap = rec && rec.snapshot
        if (snap && snap.status && snap.status !== 'running') {
          rowsRead.value = snap.rowsRead || 0
          rowsWritten.value = snap.rowsWritten || 0
          rowsFailed.value = snap.rowsFailed || 0
          taskResult.value = snap.result || taskResult.value
          taskMessage.value = snap.message || ''
          taskStatus.value = snap.status
          ElMessage.info(t('sync.bgRestoredLocal'))
        } else {
          taskMessage.value = t('sync.bgInterrupted')
          ElMessage.warning(taskMessage.value)
        }
      } else {
        taskMessage.value = st.message || t('sync.failed')
        ElMessage.error(taskMessage.value)
      }
    }
  } catch (e) {
    // 旧任务的请求失败：界面已经切到别的任务了，不能拿它去停轮询/弹错
    if (myTask !== taskId.value) return
    stopPolling()
    if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
    taskRunning.value = false
    stopping.value = false
    ElMessage.error(t('cmp.statusFailed', { detail: e.message }))
  } finally {
    // 只清自己那一份：换任务后新请求可能已经把标志指向新的 taskId
    if (pollingTask === myTask) { polling = false; pollingTask = '' }
  }
}

/**
 * 请求停止。
 *
 * 两点：
 * 1. **立刻置 `stopping`**：按钮当场变成「正在停止…」并禁用 —— 否则连点几下会发好几个取消请求，
 *    界面上却毫无反应（"点了没动静"说的就是这个）。
 * 2. **加一道兜底**：取消是协作式的（后端写完当前那一**页**才停），正常几秒内就会到达 canceled；
 *    万一某个长事务迟迟不回来，15 秒后也放开关闭 —— 否则就是一个"关不掉"的死窗口。
 */
const stopRun = async () => {
  if (stopping.value) return
  stopping.value = true
  try {
    await syncCancel(taskId.value)
    ElMessage.info(t('sync.stopRequested'))
    if (stopGuard) clearTimeout(stopGuard)
    stopGuard = setTimeout(() => {
      if (taskRunning.value) {
        stopPolling()
        taskRunning.value = false
        stopping.value = false
        ElMessage.warning(t('cmp.closingAllowed'))
      }
    }, 15000)
  } catch (e) {
    stopping.value = false
    ElMessage.error(t('sync.stopFailed', { detail: e.message }))
  }
}

const stopPolling = () => {
  if (pollTimer) { clearInterval(pollTimer); pollTimer = null }
}

onBeforeUnmount(() => {
  // 心跳定时器一并停掉（见它的声明处）
  if (tickTimer) { clearInterval(tickTimer); tickTimer = null }
  stopPolling()
  // 兜底定时器一起清掉：组件都没了，它再去改状态只会报"操作已卸载的组件"
  if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
})
/**
 * **打开即全新**：清掉上一次任务的全部运行痕迹（进度/统计卡/日志/结果/轮询）——
 * 用户口径：点「数据传输」开的就是**新传输**，不带上次跑完的缓存数字；
 * 任务中心「查看」走 resumeTaskId 恢复链路，在本重置之后（nextTick）重新拉起进度页。
 */
const resetRunState = () => {
  stopPolling()
  if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
  if (liveRaf) { cancelAnimationFrame(liveRaf); liveRaf = 0 }
  stopping.value = false
  step.value = 1
  runStartedAt.value = 0
  taskId.value = ''
  taskRunning.value = false
  taskStatus.value = ''
  taskDone.value = 0
  taskTotal.value = 0
  rowsRead.value = 0
  rowsWritten.value = 0
  rowsFailed.value = 0
  taskCurrent.value = ''
  taskMessage.value = ''
  taskResult.value = null
  syncLogs.value = []
  // 原始请求跟着任务走：新任务由自己的 context 重新填，旧任务的不能残留
  taskRequest.value = null
  pendingRerun.value = []
  resumedView.value = false
  showProgressDlg.value = false
  readShown.value = 0
  writtenShown.value = 0
  failedShown.value = 0
  rates.value = { read: 0, written: 0 }
  prevRows.value = { read: 0, written: 0 }
  fetchAt.value = 0
}
watch(() => props.modelValue, v => {
  if (!v) return
  // 每次打开都重置成全新向导（上一次任务的数字/日志/结果一概不留）；
  // 之前「任务还在跑就切回进度页」的分支已删 —— 回看进度走任务中心/时钟图标
  resetRunState()
  loadConnections()
})
onMounted(() => { if (props.modelValue) loadConnections() })
</script>

<style scoped>
/* ==================== 对话框高度：默认给足，内容多时**内部滚动** ====================
   高度定死而不是随内容长：切步骤时底栏不会上下跳。
   默认 min(660px, 90vh)：比最初的 780px 收了一圈（用户反馈默认太大、挡视野），
   `90vh` 是给矮窗口兜底的 —— 高度只给 84vh 时，可视区 700 上下的小窗只剩 590，
   对象页展开一张卡片就挤成一团；嫌小仍可拖右下角放大，
   双击手柄恢复这个默认值 —— 见 utils/dialogResize.js（所有功能性弹窗都有）。

   ⚠ 必须用 `:global()`：弹窗根节点（.el-dialog）是 el-dialog 组件 teleport 出去的，
   身上**没有本组件的 data-v 属性**，`.sync-dialog { … }` 这种写法在这里命中不了任何元素
   （本文件里其余的 `.sync-dialog :deep(...)` 一类的规则同病，先不动）。 */
/* 头 / 身 / 脚三段式：高度定死，**身体弹性**，内容自己滚。
   以前只有身体是 flex column，弹窗根节点仍是块级 —— 拖大以后多出来的高度落不到
   任何元素上（日志卡还是那 320px，下面空一大片）。 */
:global(.el-dialog.sync-dialog) {
  height: min(720px, 90vh);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
/* 执行页（步骤 5）**高度跟内容走**：日志收起时整窗一起缩，不留一大块空白；
   内容长到放不下时仍由 660 / 90vh 封顶，正文自己滚。 */
:global(.el-dialog.sync-dialog.run-mode) {
  /* 定高（原来是 height:auto + max-height）：内容一超（同步完成后结果表顶到 240px）
     就没空间可分，日志卡被压到 33px —— 标题裁一半、正文完全看不到（用户口径
     「同步完成后日志展不开、样式乱了」）；根节点 overflow:hidden 还会把超出的结果表
     断在半行上、连滚动条都没有。定高后正文拿到确定高度，放不下时由正文滚动。 */
  height: min(900px, 92vh);
}
:global(.el-dialog.sync-dialog .el-dialog__header),
:global(.el-dialog.sync-dialog .el-dialog__footer) { flex: none; }
:global(.el-dialog.sync-dialog .el-dialog__body) {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  min-height: 0;
  /* auto 而不是 hidden：日志卡自己滚，但真挤到放不下时（矮窗口 + 结果表）
     身体还能滚一下，不至于把内容裁掉 */
  overflow: auto;
  padding-top: 4px;
}
/* ==================== 字号层级 ====================
   五级阶梯由全局 index.css 的 --dlg-* 变量派生（标题/区块标题/正文/说明/徽标），
   本弹窗只做一处偏置：信息密度高（单选、勾选一行接一行），正文比全局基准小一档。
   控件与表格的字号已在全局统一，这里不用再重复写一遍。 */
:global(.el-dialog.sync-dialog) {
  --dlg-body: calc(var(--dc-dlg-fs, 14px) - 1px);
  --sd-fs-title: var(--dlg-h1);
  --sd-fs-h2: var(--dlg-h2);
  --sd-fs-body: var(--dlg-body);
  --sd-fs-sm: var(--dlg-sm);
  --sd-fs-xs: var(--dlg-xs);
}

/* 步骤条/底栏不参与弹性分配，剩下的高度全给当前步骤的内容 */
.sync-steps { flex: none; }

/* ==================== 连接信息摘要（步骤 1） ==================== */
.info-grid-wrap { display: flex; gap: 14px; margin-top: 14px; }
.info-col {
  flex: 1; min-width: 0;
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border); border-radius: 10px;
  padding: 12px 14px;
}
.info-title { font-size: 15px; font-weight: 700; color: var(--dc-primary); margin-bottom: 8px; }
/* 标签比取值小一档：一眼先看值（连接名、地址），标签是提示 */
.info-row { display: flex; justify-content: space-between; padding: 3px 0; font-size: 13px; gap: 12px; }
.info-row span { font-size: 12px; color: var(--dc-text-dim); flex: none; }
.info-row b { color: var(--dc-text); font-weight: 500; text-align: right; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

/* ==================== 步骤 4 摘要 ==================== */
/* 卡片撑满弹窗剩余高度，表格再撑满卡片 —— 否则摘要只有六七行，下面空一大片，
   看着像内容没加载完（与「同步设置」页同样的毛病）。行多时表体内部滚动。 */
.summary-card {
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border); border-radius: 10px;
  padding: 14px;
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.summary-title { flex: none; font-size: 14px; font-weight: 700; color: var(--dc-primary); margin-bottom: 10px; }
/* 开关项都进表格了，表格下面只剩可选的警告条（自己带 margin-top） */
.summary-table { flex: 1 1 auto; min-height: 120px; margin-bottom: 0; }
/* 摘要行比较密（十几行），单元格 padding 收到 4px：一行 28px 刚好一屏放完，
   否则最后几行躲在表体滚动条后面，用户会以为配置没生效 */
.summary-table :deep(.el-table td.el-table__cell),
.summary-table :deep(.el-table th.el-table__cell) { padding: 4px 0; }
/* el-table 默认高度跟内容走；这里把表头固定、表体吃掉剩余高度，行多时表体内滚
   （.el-table__inner-wrapper / __body-wrapper 是 Element 的内部结构，只能 :deep 进去） */
.summary-table :deep(.el-table__inner-wrapper) { height: 100%; display: flex; flex-direction: column; }
.summary-table :deep(.el-table__header-wrapper) { flex: none; }
.summary-table :deep(.el-table__body-wrapper) { flex: 1 1 auto; min-height: 0; overflow-y: auto; }

/* ==================== 步骤 5 执行 ==================== */
/* 执行页是「进度 + 统计 + 日志」三段，只有日志吃弹性高度：
   拖大弹窗 ⇒ 日志变长；拖小 ⇒ 日志先缩（保底 96px）再让结果表出去。 */
.run-view {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.run-view .pg-summary,
.run-view .run-stats { flex: none; }
/* 结果表吃弹性高度：弹窗拉高 / 全屏后多出来的空间归它（原先 max-height 写死 200/300，
   拉大弹窗只有日志变长，结果表永远那么几行 —— 用户口径「没自适应」）。
   日志收起时它独占剩余高度。 */
.run-view .pg-result {
  flex: 1 1 auto;
  min-height: 150px;
  display: flex;
  flex-direction: column;
}
/* 表格再吃掉 .pg-result 的剩余高度：表头固定、表体自己滚（行多时不会把弹窗撑破）。
   el-table 的 __inner-wrapper / __body-wrapper 是 Element 的内部结构，只能 :deep 进去。 */
.run-view .pg-result .res-types { flex: none; }
.run-view .pg-result :deep(.el-table) { flex: 1 1 auto; min-height: 0; }
.run-view .pg-result :deep(.el-table__inner-wrapper) {
  height: 100%;
  display: flex;
  flex-direction: column;
}
.run-view .pg-result :deep(.el-table__header-wrapper) { flex: none; }
.run-view .pg-result :deep(.el-table__body-wrapper) {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
}
.run-view .pg-log {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  margin-bottom: 0;
}
/* 结果表出来之后才限高（两者平分剩余高度）；还没出来时日志独占，
   否则跑到一半整个下半屏是空的（用户口径「结果表格没出来之前这个也高度自适应」）。 */
.run-view.has-result .pg-log { max-height: 50%; }
/* 展开时保底 160px（标题 51 + 正文 109）：结果表变长后日志卡会被压成 0 高的缝，
   正文整个消失（用户口径「同步完成后日志展不开」）。放不下时由弹窗正文滚动。 */
.run-view .pg-log:not(.collapsed) { min-height: 160px; }
.run-view .pg-log .log-head { flex: none; }
/* 正文吃掉卡片剩余高度（不再被基类的 max-height:200px 封顶） */
.run-view .pg-log .log-body { flex: 1 1 auto; min-height: 0; max-height: none; }
/* 日志收起后**卡片本身也收起**（只留标题那一行），省下的高度给结果表；
   以前这里仍是 flex:1，收起来就留一大块空白（用户截图）。 */
.run-view .pg-log.collapsed { flex: none; }

.run-stats {
  display: flex; gap: 10px; margin: 12px 0;
  /* 拖窄以后 5 张卡会挤成一条缝，允许换行 */
  flex-wrap: wrap;
}
.run-stats .stat {
  flex: 1 1 108px; min-width: 96px; text-align: center;
  background: linear-gradient(180deg, var(--dc-bg-card), var(--dc-bg-soft));
  border: 1px solid var(--dc-border); border-radius: 10px;
  padding: 10px 8px;
  transition: border-color .15s, box-shadow .15s;
}
.run-stats .stat:hover { border-color: var(--dc-border-hover); box-shadow: var(--dc-shadow-sm); }
.run-stats .num {
  display: block; font-size: 19px; font-weight: 700; color: var(--dc-text);
  /* 等宽数字：计数每 2 秒跳一次，比例数字宽窄不一会让整排左右抖 */
  font-variant-numeric: tabular-nums;
  letter-spacing: .2px;
}
.run-stats .label { display: block; margin-top: 2px; font-size: 12px; color: var(--dc-text-dim); }
.run-stats .stat.ok { background: linear-gradient(180deg, var(--dc-success-wash), var(--dc-bg-soft)); }
.run-stats .stat.ok .num { color: var(--dc-success); }
.run-stats .stat.danger { background: linear-gradient(180deg, var(--dc-danger-wash), var(--dc-bg-soft)); }
.run-stats .stat.danger .num { color: var(--dc-danger); }
/* 「1.2万/s · 3.4 MB/s」「12 分 30 秒」比纯数字长：给更宽的基数、字号降一档，
   否则 19px 在 108px 的卡里会被挤成两行（高度跟着变，整排就参差不齐） */
.run-stats .stat-wide { flex: 1 1 170px; }
.run-stats .stat-wide .num { font-size: 15px; letter-spacing: 0; }

/* 弹窗标题是整窗的「一级标题」：吃全局 --dlg-h1（默认 18），不跟着主题漂移 */
.dlg-title { display: flex; align-items: center; gap: 8px; font-size: var(--sd-fs-title); font-weight: 700; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}
/* 弹窗标题 17px（:global 里统一设了），后面的副标题故意小两档，做对比 */
.dlg-title-sub { font-size: 12px; font-weight: 400; color: var(--dc-text-dim); margin-left: 4px; }

/* ==================== 源 / 目标 流 ==================== */
/* 步骤条（向导式交互，与数据对比一致）—— 仅向导阶段显示，执行页（步骤 5）隐藏 */
.sync-steps { margin-bottom: 16px; }
/* 步骤条：未走到的步骤 12px 常规色，当前/已完成 13px 加粗 —— 让"进行到哪了"一眼可见 */
.sync-steps :deep(.el-step__title) { font-size: 12px; }
.sync-steps :deep(.el-step__title.is-process),
.sync-steps :deep(.el-step__title.is-success) { font-size: 13px; font-weight: 700; }
/* 步骤 1 撑满正文高度：全屏 / 拉高弹窗后，源/目标卡片跟着变高，
   而不是缩在上半部分、下面留一大片空白（用户口径「没自适应」）。 */
.step-pane { flex: 1 1 auto; min-height: 0; }
.step-source { display: flex; flex-direction: column; }
.step-source .flow { flex: 1 1 auto; min-height: 120px; }
.flow { display: flex; align-items: stretch; gap: 12px; margin-bottom: 14px; }
.side {
  flex: 1; min-width: 0;
  display: flex; flex-direction: column; gap: 8px;
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border);
  border-radius: 12px;
  padding: 12px 14px;
}
.side-head {
  display: flex; align-items: center; gap: 7px;
  font-size: 13px; font-weight: 600; color: var(--dc-text);
  letter-spacing: .3px;
}
.dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; }
.dot.src { background: linear-gradient(135deg, var(--dc-primary-light), var(--dc-primary-deep)); box-shadow: 0 0 8px var(--dc-primary-glow); }
.dot.tgt { background: linear-gradient(135deg, var(--dc-accent), var(--dc-primary-light)); box-shadow: 0 0 8px rgba(61,220,151,.4); }
.pair { display: flex; gap: 8px; }
.pair .el-select { flex: 1; min-width: 0; }
.flow-arrow { display: flex; align-items: center; }
.arrow-ring {
  width: 40px; height: 40px; border-radius: 50%;
  display: flex; align-items: center; justify-content: center;
  color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-accent), var(--dc-primary-light));
  box-shadow: 0 0 0 4px rgba(61, 220, 151, .12), 0 4px 14px rgba(61, 220, 151, .25);
}

/* ==================== 通用面板 ==================== */
.panel, .opts, .progress-card, .result-card {
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border);
  border-radius: 12px;
  padding: 12px 14px;
  margin-bottom: 14px;
}
.panel-head {
  display: flex; align-items: center; justify-content: space-between;
  margin-bottom: 10px;
}
.panel-title {
  display: inline-flex; align-items: center; gap: 6px;
  font-size: 15px; font-weight: 700; color: var(--dc-text);
}
.panel-title .el-icon { color: var(--dc-primary); }
.panel-count { font-size: 13px; color: var(--dc-text-dim); }
.panel-count b { color: var(--dc-primary); font-weight: 700; }

/* ==================== 对象选择：卡片式分组下拉 ====================
   每一类对象是一张卡片（组头一行 + 可展开的对象网格），视觉上与「卡片」一致：
   圆角、独立描边、悬停微亮、展开时组头高亮。 */
.obj-toolbar {
  display: flex; align-items: center; gap: 8px;
  margin-bottom: 10px;
}
.obj-toolbar .el-input { flex: 1; min-width: 0; }
.obj-empty-all {
  padding: 26px 0; text-align: center;
  font-size: 13px; color: var(--dc-text-dim);
}
/* 对象页 = 一行页签 + 占满剩余高度的列表。
   原先是手风琴（一类一张卡、展开的那张吃剩余高度）：窗口一矮，折叠的卡片会被 flex
   压成 2px（`overflow:hidden` 让 `min-height:auto` 解析成 0），看着像样式炸了。
   改成页签后 6 类固定只占一行（约 32px），列表独占剩余高度，任何窗口高度都稳。 */
.obj-panel { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; }
.obj-tabs {
  flex: none;
  display: flex; align-items: flex-end; gap: 2px;
  /* 页签名与正文字号同级（13），激活时加粗 + 主色做区分 */
  font-size: 13px;
  border-bottom: 1px solid var(--dc-border);
  margin-bottom: 8px;
  overflow-x: auto;
  scrollbar-width: none;
}
.obj-tabs::-webkit-scrollbar { height: 0; }
.obj-tab {
  flex: none;
  display: inline-flex; align-items: center; gap: 7px;
  /* 显式定高：不让 el-checkbox 的 32px 行盒把页签顶高（否则一行页签要 41px） */
  height: 30px;
  padding: 0 8px;
  border: none; background: transparent;
  border-bottom: 2px solid transparent;
  border-radius: 7px 7px 0 0;
  font: inherit;
  color: var(--dc-text-dim);
  cursor: pointer;
  white-space: nowrap;
  transition: background .15s, color .15s, border-color .15s;
}
.obj-tab:hover { background: var(--dc-bg-hover); color: var(--dc-text); }
.obj-tab.on {
  color: var(--dc-primary);
  font-weight: 600;
  background: var(--dc-primary-wash);
  border-bottom-color: var(--dc-primary);
}
.obj-tab.empty { opacity: .62; }
/* 页签里的小勾选框（= 本类全选）：默认 32px 高会把页签顶高一圈，压到 16px */
/* ⚠ 只压**行高**，不要动 `.el-checkbox__inner` / `::after` 的尺寸：
   对号是 `::after` 上用 3×7 的边框画的，跟着框缩小就对不齐（真机反馈「勾上了看不见对号」）。
   框本身默认就是 14×14，够小；把容器 32px 的行高收掉即可。 */
.ot-check { margin-right: -2px; display: inline-flex; }
.ot-check :deep(.el-checkbox) { height: 18px; margin-right: 0; }
.ot-check :deep(.el-checkbox__label) { display: none; }
.ot-ic {
  width: 18px; height: 16px;
  display: inline-flex; align-items: center; justify-content: center;
  border-radius: 4px;
  color: #0f1522;
  flex: none;
  margin-left: 1px;
}
.ot-name { font-size: 13px; }
.ot-count {
  font-size: 11px; line-height: 15px;
  padding: 0 5px; border-radius: 8px;
  background: var(--dc-bg-soft); color: var(--dc-text-dim);
  font-variant-numeric: tabular-nums;
}
.obj-tab.on .ot-count { background: var(--dc-primary); color: #fff; }
/* 列表：多列铺排，对象多了在这一层滚动（高度 = 面板剩余空间）
   列宽用 minmax 的固定份数而不是 auto-fill：否则最后一份会被滚动条挤出行外 */
.obj-list {
  flex: 1 1 auto; min-height: 0;
  overflow-y: auto; overflow-x: hidden;
  padding-right: 2px;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(100%, 210px), 1fr));
  gap: 0 10px;
  align-content: start;
}
/* 空页签 / 无匹配：一个柔和的小空状态（虚线圈 + 图标 + 一行说明），居中在列表区中间，
   而不是左上角一句小灰字。min-height 让它落在视觉中心（列表区高约 280~320px）。 */
.obj-none {
  grid-column: 1 / -1;
  min-height: 200px;
  display: flex; flex-direction: column; align-items: center; justify-content: center;
  gap: 10px;
  padding: 32px 12px;
  color: var(--dc-text-dim);
  font-size: 13px;
  text-align: center;
}
.obj-none .nc-ic {
  width: 48px; height: 48px; border-radius: 15px;
  display: inline-flex; align-items: center; justify-content: center;
  background: var(--dc-bg-soft);
  border: 1px dashed var(--dc-border);
  color: var(--dc-text-dim);
  opacity: .85;
}
.obj-none .nc-txt { line-height: 1.6; }
/* 列表项里的勾选框只收行高（32→20px，一行从 38px 降到 22px）；
   **不动 inner/::after** —— 对号是 ::after 上用 3×7 边框画的，缩框就对不齐、看不见勾。 */
.obj-list :deep(.el-checkbox) { height: 20px; margin-right: 2px; }
.og-item {
  display: flex; align-items: center; gap: 6px;
  padding: 2px 6px;
  border-radius: 6px;
  cursor: pointer;
  min-width: 0;
  transition: background .15s;
}
.og-item:hover { background: var(--dc-bg-hover); }
.og-item.on { background: var(--dc-primary-wash); }
.og-item-name {
  flex: 1; min-width: 0;
  font-size: 13px; color: var(--dc-text);
  font-family: "Cascadia Code", Consolas, "JetBrains Mono", monospace;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
/* 分组图标底色（沿用原对象类型配色，一眼能分清是哪一类） */
.ty-table { background: var(--dc-accent); }
.ty-view { background: #7cabff; }
.ty-function { background: #c4b0ff; }
.ty-procedure { background: #f8c878; }
.ty-trigger { background: #ff9d9d; }
.ty-event { background: #7fe3f5; }

/* ==================== 同步设置 ==================== */
.settings-card {
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border);
  border-radius: 14px;
  padding: 0;
  margin-bottom: 14px;
  box-shadow: var(--dc-shadow-sm);
  overflow: hidden;
  /* 卡片吃掉弹窗剩余高度（内容不够高时不留一大块空白；内容过高时由页签内容区自己滚），
     页签内容区是唯一滚动容器 —— 窗口矮也不会把「行过滤」列表拦腰裁断。 */
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
/* ==================== 选项页的三个页签 ====================
   el-tabs 默认是亮色主题的一套外边距与下划线，这里全部压到本项目的深色变量上：
   - 页签栏贴卡片顶边（没标题栏了，它就是卡片头）；
   - 下划线用主色，未选中项用弱化文字色；
   - 内容区去掉 tabs 自带的 15px 下边距，交给 .opt-grid / .opt-pane 自己控制。 */
.opt-tabs :deep(.el-tabs__header) {
  margin: 0;
  padding: 0 10px;
  border-bottom: 1px solid var(--dc-border);
  background: linear-gradient(90deg, var(--dc-primary-wash), var(--dc-success-wash));
}
.opt-tabs { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; }
.opt-tabs :deep(.el-tabs__nav-wrap::after) { display: none; }
/* 四个页签（同步设置 / 表名映射 / 前后置 SQL / 行过滤）要在弹窗宽度内
   排满一行：左右内边距收到 12px，别让最后一项被挤到滚动箭头里 */
.opt-tabs :deep(.el-tabs__item) {
  height: 42px; line-height: 42px;
  padding: 0 12px;
  font-size: 14px; font-weight: 600;
  color: var(--dc-text-dim);
}
.opt-tabs :deep(.el-tabs__item.is-active) { font-weight: 700; }
.opt-tabs :deep(.el-tabs__item.is-active) { color: var(--dc-primary); }
.opt-tabs :deep(.el-tabs__item:hover) { color: var(--dc-text); }
.opt-tabs :deep(.el-tabs__active-bar) { background-color: var(--dc-primary); height: 2px; }
/* 内容区是**唯一的滚动容器**（卡片自己 overflow: hidden）：
   内容矮时它吃掉剩余高度，内容高时它自己滚，弹窗不会因此变高 */
.opt-tabs :deep(.el-tabs__content) { flex: 1 1 auto; min-height: 0; padding: 0; overflow: auto; }
.opt-tab { display: inline-flex; align-items: center; gap: 6px; }
.opt-tab .el-icon { color: var(--dc-accent); }
.opt-tabs :deep(.el-tabs__item.is-active) .opt-tab .el-icon { color: var(--dc-primary); }
/* 页签上的计数徽标（这个页签里配了几项）：切走以后也知道里面填过东西 */
.opt-tab-badge {
  font-size: 11px; font-weight: 700; line-height: 1;
  padding: 2px 6px; border-radius: 9px;
  color: var(--dc-primary); background: var(--dc-primary-wash);
  border: 1px solid var(--dc-border);
}
/* 页签内容撑满内容区：卡片既然吃满了高度，两栏小卡也跟着拉高（左色条一竖到底），
   否则高卡片下面空一片，看着像没加载完。
   注意要 `height: 100%` 而不是 `min-height: 100%`：min-height 只给下限、高度仍按内容撑，
   长列表页签（表名映射 / 行过滤）的 .sw-list 按 max-height 400px 撑满后
   会把内容区顶出 27px，于是外层 .el-tabs__content 多出一条滚动条、和列表自己的滚动条并存。 */
.opt-tabs :deep(.el-tab-pane) {
  display: flex; flex-direction: column;
  flex: 1 1 auto; min-height: 0; height: 100%;
}
/* 页签内容：两栏并排（内容量相近的才并排，「过滤与脚本」是一整列长列表） */
.opt-grid {
  flex: 1 1 auto;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
  padding: 14px 16px;
}
/* 「同步设置」只有一块内容，铺满整行，高度**跟着内容走**：
   - align-items: start ⇒ 卡片不被拉伸到行高，矮内容时不会下面空一大片；
   - flex: 0 0 auto ⇒ 容器不进弹性分配（否则内容比剩余高度高时，卡片会被压矮、
     然后被自己的 overflow:hidden 裁掉 —— 真机截图里「传输参数」就是这么消失的）。
   内容真超过一屏时由弹窗身体滚动，而不是把卡片裁一半。 */
.opt-grid.one {
  flex: 0 0 auto;
  grid-template-columns: minmax(0, 1fr);
  align-content: start;
  align-items: start;
}
/* 一张卡里塞了三段（同步方式 / 搬什么 / 传输参数）。原来为了「一屏放得下」把间距
   压到 2~4px，真机反馈「贴在一起，看着累」—— 每行之间留一口气（卡片下方仍有空余）。 */
.opt-grid.one .settings-col { gap: 8px; }
.opt-grid.one .settings-checks { gap: 8px; }
.opt-grid.one .settings-inline { margin-top: 0; }
/* 「搬什么」两列：五项单列占满一屏的大半，两列后这一页才装得下传输参数。
   分隔线（.checks-sep）横跨两列，「遇错停止」排在它后面自成一行 */
.opt-grid.one .checks-2col {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px 14px;
}
.opt-grid.one .checks-2col .checks-sep { grid-column: 1 / -1; }
@media (max-width: 720px) {
  .opt-grid.one .checks-2col { grid-template-columns: minmax(0, 1fr); }
}

/* ==================== 单列表页签（表名映射 / 前后置 SQL / 行过滤） ====================
   标题行 + 一句说明 + 内容。内容里的长列表（一表一行）吃掉剩余高度、
   自己内部滚动 —— 几百张表也不会把弹窗顶高。 */
.opt-pane {
  flex: 1 1 auto;
  min-height: 0;
  display: flex; flex-direction: column; gap: 8px;
  padding: 14px 18px 16px;
}
.pane-head { display: flex; align-items: center; gap: 8px; }
.pane-head .el-icon { color: var(--dc-accent); flex: none; }
.pane-title { font-size: 15px; font-weight: 700; color: var(--dc-text); }
/* 搜索框靠右停靠（表少的时候不出现） */
.pane-head .sw-search { margin-left: auto; }
.pane-hint { font-size: 12px; line-height: 1.5; color: var(--dc-text-weak); margin: 0; }
/* ==================== 可收起的卡片头 ====================
   整条标题可点（cursor:pointer + 右侧三角）。三角用纯 CSS 边框画：
   展开朝下（rotate 90deg）、收起朝右（0deg）—— 不依赖某个图标是否被注册。 */
.fold-head { cursor: pointer; user-select: none; }
.fold-head:hover { background: var(--dc-bg-soft); }
.fold-arrow {
  flex: none;
  width: 0; height: 0;
  border-left: 5px solid currentColor;
  border-top: 4px solid transparent;
  border-bottom: 4px solid transparent;
  opacity: .6;
  transform: rotate(90deg);
  transition: transform .15s ease;
}
.fold-arrow.closed { transform: rotate(0deg); }
.sw-title .fold-arrow { margin-left: auto; }
.log-head-right { display: inline-flex; align-items: center; gap: 10px; }
.settings-col {
  min-width: 0;
  display: flex; flex-direction: column; gap: 8px;
  padding: 14px 16px;
  border-radius: 12px;
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border);
  position: relative;
  overflow: hidden;
}
.settings-col::before {
  content: '';
  position: absolute; left: 0; top: 0; bottom: 0; width: 4px;
}
.col-policy::before { background: linear-gradient(180deg, var(--dc-primary), var(--dc-primary-deep)); }
.col-exec::before   { background: linear-gradient(180deg, var(--dc-warning), var(--dc-danger)); }
.settings-label {
  display: inline-flex; align-items: center; gap: 6px;
  font-size: 15px; font-weight: 700;
  color: var(--dc-text);
  margin-bottom: 2px;
}
.settings-label .el-icon { color: var(--dc-accent); }
.settings-col :deep(.el-radio-group),
.settings-col :deep(.el-checkbox-group) { display: flex; flex-direction: column; align-items: flex-start; gap: 6px; }
.settings-col :deep(.el-radio),
.settings-col :deep(.el-checkbox) { margin-right: 0; height: auto; }
/* 勾选项一律**允许换行**：这里的中文标签很长（「建表包含索引（把源表的索引一起建到目标）」），
   Element 默认 `white-space: nowrap` 会把它硬裁掉半句（真机截图：执行选项三行全被切） */
.settings-col :deep(.el-checkbox) { white-space: normal; align-items: flex-start; }
.settings-col :deep(.el-checkbox__input) { margin-top: 1px; }
.settings-col :deep(.el-radio__label),
.settings-col :deep(.el-checkbox__label) {
  font-size: 13px; color: var(--dc-text);
  white-space: normal; line-height: 1.45;
}
.settings-hint {
  font-size: 12px; line-height: 1.55;
  color: var(--dc-text-weak);
  margin: 0;
}
/* ==================== 同步方式（普通单选行，不画面板） ====================
   一行 = 一个 (objectPolicy, dataMode) 组合，只写选项名（不带长说明：
   四个名称已经把差别说清，再挂四句注解反而把这一页撑满、还要逐行读）。
   之前用 2×2 卡片面板，和下面「同步内容」的勾选列表是两套视觉语言，
   看起来像两个互不相干的设置区。 */
/* 必须带上 .el-radio-group 才压得住上面那条「单选组竖排」的通用规则 */
.settings-col :deep(.el-radio-group.mode-list) {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 6px;
  width: 100%;
}
.settings-col :deep(.el-radio.mode-row) {
  display: flex; align-items: center;
  margin: 0;
  height: auto;
  min-height: 22px;
  padding: 2px 0;
}
/* 方框与文字**同一行高**对齐：Element 的 radio 默认高度 32px、label 行高 1，
   13px 中文在 22px 行盒里会看着偏上（真机反馈「单选框没和字体对齐」）。 */
.settings-col :deep(.el-radio.mode-row .el-radio__input) {
  margin-top: 0;
  height: 20px;
  display: flex; align-items: center;
}
.settings-col :deep(.el-radio.mode-row .el-radio__label) { padding-left: 8px; line-height: 20px; }
.mode-title {
  font-size: 13px; font-weight: 600; line-height: 20px;
  color: var(--dc-text);
}
/* 单选也画「对号」：和下面「同步内容」的勾选行统一视觉语言。
   Element 的 radio 默认是圆圈 + 圆点；这里把 inner 换成圆角方块，
   选中态的对号沿用 checkbox 的画法（右/下两条边框 + 45° 旋转）。 */
.settings-col :deep(.el-radio.mode-row .el-radio__inner) {
  width: 14px; height: 14px;
  border-radius: 4px;
}
.settings-col :deep(.el-radio.mode-row .el-radio__inner::after) {
  width: 3px; height: 7px;
  border-radius: 0;
  background: transparent;
  border: 1.5px solid var(--dc-on-accent, #fff);
  border-left: 0; border-top: 0;
  transform: translate(-50%, -58%) rotate(45deg) scale(0);
  transition: transform .12s ease-out;
}
.settings-col :deep(.el-radio.mode-row .el-radio__input.is-checked .el-radio__inner::after) {
  transform: translate(-50%, -58%) rotate(45deg) scale(1);
}
/* 选中项的名称用主色，替掉原来的卡片高亮 —— 一行文字里唯一的强调点 */
.settings-col :deep(.el-radio.mode-row.is-checked) .mode-title { color: var(--dc-primary); }

.settings-checks { display: flex; flex-direction: column; gap: 6px; }
/* 勾选框与文字同高：Element 默认 32px 行盒，13px 文字在里面会看着偏下/偏上。
   方框本身也要**显式**居中（inline-block 默认按 baseline 排，实测会高出 2px）。 */
.settings-checks :deep(.el-checkbox) { height: 20px; margin-right: 0; }
.settings-checks :deep(.el-checkbox__input) { display: flex; align-items: center; height: 20px; }
.settings-checks :deep(.el-checkbox__label) { line-height: 20px; padding-left: 6px; }
/* 设置行：左侧标签（+ 问号悬浮说明），右侧控件。
   左列：每批条数 / 超时时间 / 两条限速；右列：并发三项（见 .param-grid 的按列流排） */
.settings-inline { display: flex; align-items: center; gap: 8px; margin-top: 4px; }
.settings-inline-label { font-size: 13px; color: var(--dc-text); line-height: 24px; flex: none; }
.hint-q { font-size: 12px; color: var(--dc-text-weak); cursor: help; vertical-align: -1px; margin-left: 2px; }
.hint-q:hover { color: var(--el-color-primary); }
.settings-num { width: 96px; flex: none; }
/* 执行选项里「搬什么」（结构/数据）与「怎么应对错误」之间的一条细线 */
.checks-sep { height: 1px; background: var(--dc-border); margin: 4px 0; }
/* 「搬什么」与「传输参数」之间的分隔（合并成一栏后靠这条线分段） */
.settings-sep { height: 1px; background: var(--dc-border); margin: 6px 0 3px; }
/* 传输参数：两列（每格一行「标签 + 控件」），按**列**填：左列怎么搬、右列并发。
   标签列**定宽**（min-width: 10em ≈ 130px）：标签长短不一（「每批条数」52px vs
   「限速（MB/秒）」89px），不定宽时同一列上下两个控件会左右错开 30 多像素。
   10em 按最长标签（含末尾的问号图标、含英文 "Rate limit (rows/s)"）取的，先按最长留够。 */
.param-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  /* **按列**填充（不是按行）：左列 4 行放「怎么搬」（每批条数 / 超时 / 两条限速），
     右列放并发三项。用户口径：并发要集中出现在右侧，扫一眼就知道在调什么。
     行数固定 4 是为左列服务的 —— 右列只有 3 项，第四行留空不影响对齐。 */
  grid-auto-flow: column;
  grid-template-rows: repeat(4, auto);
  gap: 6px 18px;
}
.param-grid .settings-inline-label { min-width: 10em; }
@media (max-width: 720px) {
  .param-grid {
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: none;
    grid-auto-flow: row;
  }
}

/* ==================== 按表设置的长列表（表名映射 / 行过滤） ====================
   一行一张表：左侧表名定宽、右侧输入框吃掉剩余宽度。
   列表自己滚动并**封顶**：整库几百张表时，若让列表自由长高，会把卡片顶出弹窗
   （弹窗高度定死 + overflow:hidden，多出来的部分直接被裁掉，反而滚不到）。 */
.sw-search { width: 180px; flex: none; }
.sw-list {
  flex: 1 1 auto;
  min-height: 90px;
  max-height: min(46vh, 400px);
  overflow-y: auto;
  display: flex; flex-direction: column; gap: 6px;
  padding-right: 4px;
}
.sw-list::-webkit-scrollbar { width: 6px; }
.sw-list::-webkit-scrollbar-thumb { background: var(--dc-border); border-radius: 3px; }
.sw-item { display: flex; align-items: center; gap: 8px; }
.sw-item.on .sw-name { color: var(--dc-primary); font-weight: 600; }
.sw-name {
  flex: none; width: 190px;
  font-size: 12px; color: var(--dc-text-dim);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.sw-item .el-input { flex: 1; }
.sw-none { font-size: 13px; color: var(--dc-text-weak); padding: 4px 0; }

/* 前置 / 后置 SQL：两栏并排（一栏一段脚本），等宽字体才对得上粘贴进来的脚本缩进 */
.sql-grid { display: flex; gap: 12px; padding-bottom: 2px; }
.sql-col { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
.sql-label { font-size: 12px; color: var(--dc-text-dim); }
.sql-col :deep(.el-textarea__inner) {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 12px; line-height: 1.5;
}
/* 高度自适应：整块吃掉页签里的剩余高度，两个 textarea 各占一半。
   120px 是底线（矮窗口下不至于只剩一行）；再矮由弹窗身体滚动。 */
.sql-grid.grow { flex: 1 1 auto; min-height: 120px; }
.sql-grid.grow .sql-col { min-height: 0; }
.sql-grid.grow :deep(.el-textarea) { flex: 1 1 auto; min-height: 120px; display: flex; }
.sql-grid.grow :deep(.el-textarea__inner) { flex: 1 1 auto; height: auto; }
/* 结果区右上角「重跑失败对象」：靠右停靠，不挤占类型徽标 */
.res-rerun { margin-left: auto; }

/* ==================== 实时日志 ==================== */
.log-card {
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border);
  border-radius: 14px;
  padding: 0;
  margin-bottom: 14px;
  box-shadow: var(--dc-shadow-sm);
  overflow: hidden;
}
.log-head {
  display: flex; align-items: center; justify-content: space-between;
  padding: 12px 18px;
  font-size: 14px; font-weight: 700; color: var(--dc-text);
  border-bottom: 1px solid var(--dc-border);
  background: linear-gradient(90deg, var(--dc-primary-wash), var(--dc-success-wash));
}
.log-head-left {
  display: inline-flex; align-items: center; gap: 8px;
}
.log-head-left .el-icon {
  color: var(--dc-primary);
  filter: drop-shadow(0 0 4px var(--dc-primary-glow));
}
.log-count {
  font-size: 12px; font-weight: 600;
  color: var(--dc-text-dim);
  background: var(--dc-bg-soft);
  padding: 3px 10px; border-radius: 999px;
  border: 1px solid var(--dc-border);
}
.log-body {
  max-height: 200px; overflow-y: auto;
  padding: 10px 18px;
  background: var(--dc-bg-code);
}
.log-empty {
  display: flex; flex-direction: column; align-items: center; gap: 8px;
  padding: 28px 0;
  color: var(--dc-text-weak);
}
.log-empty .el-icon { color: var(--dc-text-weak); }
.log-empty span { font-size: 14px; font-weight: 600; color: var(--dc-text-weak); }
.log-empty p { font-size: 11.5px; color: var(--dc-text-weak); margin: 0; }
.log-line {
  display: flex; align-items: flex-start; gap: 10px;
  padding: 2px 8px;
  border-radius: 6px;
  font-size: 13px; line-height: 1.6;
  color: var(--dc-code-text);
  font-family: "Cascadia Code", Consolas, "JetBrains Mono", monospace;
}
/* 长日志一屏几十行，hover 高亮帮眼睛「不串行」；出问题的那几行整行带底色更醒目 */
.log-line:hover { background: var(--dc-bg-hover); }
.log-line.err { color: var(--dc-danger); background: var(--dc-danger-wash); }
.log-line.warn { color: var(--dc-warning); }
.log-line.ok { color: var(--dc-success); }
.log-time {
  flex: none;
  color: var(--dc-text-weak);
  font-size: 12px;
  /* 定宽让所有行的正文左边缘对齐（时间戳等长，不定宽会被等宽字体带歪） */
  font-variant-numeric: tabular-nums;
}
/* 正文占满剩余宽度：长句子换行后仍与第一行左对齐，超长 token（URL、SQL 片段）也不顶破卡片 */
.log-text {
  flex: 1 1 auto;
  min-width: 0;
  overflow-wrap: anywhere;
}

/* ==================== 进度 / 结果 ==================== */
.progress-card { padding: 14px 16px; }
.progress-text { font-size: 13px; color: var(--dc-text-dim); margin-top: 8px; text-align: center; }
.result-card { padding: 14px 16px; }

/* ==================== 进度弹窗 ==================== */
.progress-dialog :deep(.el-dialog__body) { padding: 16px 20px 10px; max-height: 74vh; overflow: auto; }
.pg-summary { margin-bottom: 14px; }
.pg-text { font-size: 13px; color: var(--dc-text-dim); margin-top: 8px; text-align: center; }

/* 进度标题行：左边「同步进度」+ 运行态图标，右边大号百分比（替代挤在细条里的 text-inside） */
.pg-top {
  display: flex; align-items: center; justify-content: space-between;
  margin-bottom: 8px;
}
.pg-title {
  display: inline-flex; align-items: center; gap: 6px;
  font-size: 15px; font-weight: 700; color: var(--dc-text);
}
.pg-title .el-icon { color: var(--dc-primary); }
.pg-pct {
  font-size: 16px; font-weight: 700; line-height: 1;
  color: var(--dc-primary);
  font-variant-numeric: tabular-nums;
}
.pg-pct.done { color: var(--dc-success); }
.pg-pct.warn { color: var(--dc-warning); }
.pg-pct.err { color: var(--dc-danger); }
/* 进度条只当刻度尺：柔和轨道 + 圆角 + 宽度变化有过渡（百分比本身在上面） */
.run-view :deep(.el-progress-bar__outer) { background: var(--dc-border); border-radius: 999px; }
.run-view :deep(.el-progress-bar__inner) { border-radius: 999px; transition: width .45s ease; }
/* 日志吃掉剩下的高度：拖动弹窗时跟着变（旧的 200/320px 固定值会让拉高后底下空一片） */
.pg-log { margin-bottom: 0; }
.pg-log .log-body { max-height: none; min-height: 96px; }
.pg-result { margin-top: 14px; }
/* 结果类型小计（表/视图/存储过程/函数/触发器）：点一下只看这一类 */
.res-types { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; margin-bottom: 8px; }
.res-type {
  display: inline-flex; align-items: center; gap: 6px;
  padding: 4px 11px; border-radius: 999px;
  border: 1px solid var(--dc-border); background: var(--dc-bg-soft);
  color: var(--dc-text-dim); font-size: 12px; cursor: pointer;
}
.res-type:hover { border-color: var(--dc-primary); }
.res-type b { color: var(--dc-success); font-weight: 700; }
.res-type.on { border-color: var(--dc-primary); background: var(--dc-primary-wash); color: var(--dc-text); }
.rt-bad { color: var(--dc-danger); font-weight: 600; }
.stat-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(96px, 1fr));
  gap: 8px;
  margin-bottom: 12px;
}
.stat {
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border);
  border-radius: 10px;
  padding: 10px;
  text-align: center;
}
.stat .num { display: block; font-size: 20px; font-weight: 700; color: var(--dc-primary); }
.stat .label { font-size: 12px; color: var(--dc-text-dim); }
.stat.ok .num { color: var(--dc-success); }
.stat.danger .num { color: var(--dc-danger); }

.foot-actions {
  display: flex; align-items: center; justify-content: flex-end;
  gap: 10px; width: 100%;
}
.data-table { width: 100%; }

/* ==================== Element Plus 适配 ==================== */
.sync-dialog :deep(.el-dialog__body) { padding-top: 4px; }
.sync-dialog :deep(.el-dialog__footer) { display: flex; align-items: center; justify-content: flex-end; border-top: 1px solid var(--dc-border); padding-top: 14px; padding-bottom: 4px; gap: 12px; }
.sync-dialog :deep(.el-dialog__footer) > div { display: flex; align-items: center; gap: 10px; }
.sync-dialog :deep(.el-radio__label),
.sync-dialog :deep(.el-checkbox__label) { font-size: 13px; white-space: nowrap; }
/* 上面这条是全局兜底（nowrap 让短标签不换行），但它和 `.settings-col :deep(.el-checkbox__label)`
   的 `white-space: normal` 特异性相同、位置更靠后 —— 会把「允许换行」压回 nowrap，
   长中文标签又被硬裁半句。所以必须在这里按区块再覆盖一次（多一个 .sync-dialog，
   特异性更高，不依赖书写顺序）。 */
.sync-dialog .settings-col :deep(.el-checkbox__label),
.sync-dialog .settings-col :deep(.el-radio__label) { white-space: normal; }
</style>
