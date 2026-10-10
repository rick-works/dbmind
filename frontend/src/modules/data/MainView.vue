<template>
  <div class="main-view">
    <!-- 顶栏：桌面壳里系统标题栏已隐藏，这一条同时充当「标题栏 + 窗口按钮」。
         拖动只走自己的三段式（见 onTopbarMouseDown / Move / Up）：按下记起点，
         移动超过阈值才真正开始拖 —— 这样双击最大化不会被拖动抢走。
         两条"捷径"都刻意不用：data-tauri-drag-region 要靠壳注入的脚本兜底（行为不在自己手里），
         -webkit-app-region 是 Chromium 私有拖动特性、会和上面那套互相打架。 -->
    <header class="topbar" :class="{ 'is-desktop': isDesktop, 'is-mac': isMacShell }"
            @mousedown="onTopbarMouseDown"
            @mousemove="onTopbarMouseMove"
            @mouseup="onTopbarMouseUp"
            @mouseleave="onTopbarMouseUp"
            @dblclick="onTopbarDblClick">
      <!-- 品牌 logo：顶栏最左角，只放图形不带文字（顶栏本是导航区，加标题会把它挤窄）。
           它落在顶栏自带的三段式拖动区内：桌面端按住它拖窗口 = 系统标题栏的行为。 -->
      <img class="topbar-logo" :src="logoSmUrl" alt="DBmind" draggable="false" />
      <nav class="top-nav">
        <!-- 左侧：数据 → 治理/运维 → 动作（文字 + 图标）；右侧：知识库 / AI / 设置（只留图标 + 悬停提示） -->
        <!-- 只在侧栏折叠时出现的「展开」入口：展开态由侧栏工具条里那枚钮负责收起，
             而收起态侧栏整块消失，才需要这里接住。只留图标（对齐右侧那排图标钮），
             图标沿用原来的 Expand。 -->
        <el-tooltip v-if="sidebarHidden" :content="$t('nav.expandSidebar')" placement="bottom">
          <span class="top-nav-item top-icon-btn" @click="sidebarHidden = false"><el-icon><Expand /></el-icon></span>
        </el-tooltip>

        <!-- 顶栏菜单项**可配置**：显示哪些、顺序如何，在「设置 → 通用 → 顶栏菜单」里
             设置并持久化（dbmind.db）。默认与历史版本一致 -->
        <span v-for="mi in topMenuVisible" :key="mi.id" class="top-nav-item"
              :class="{ 'top-nav-action': mi.id === 'newScript' }"
              @click="topMenuRun(mi.id)">
          <el-icon><component :is="mi.icon" /></el-icon>{{ mi.label() }}
        </span>
        <span class="top-nav-right">
          <el-tooltip :content="$t('nav.knowledge')" placement="bottom"><span class="top-nav-item top-icon-btn" :class="{ 'dc-top-active': knowledgeOpen }" @click="openKnowledge"><el-icon><Reading /></el-icon></span></el-tooltip>
          <el-tooltip :content="$t('nav.ai')" placement="bottom"><span class="top-nav-item top-icon-btn" :class="{ 'dc-top-active': aiOpen }" @click="toggleAi"><el-icon><MagicStick /></el-icon></span></el-tooltip>
          <!-- 右侧时钟图标：**正在执行**的任务 —— 鼠标悬浮直接下拉展示（纯 CSS hover，
               不用 popover/弹窗 —— 定位受顶栏布局影响会飘）。点任务项回到进度窗。 -->
          <span class="bg-hover" @mouseenter="refreshBgStatus()">
            <span class="top-nav-item top-icon-btn bg-task-btn">
              <el-icon><Clock /></el-icon>
              <span v-if="bgRunningCount" class="bg-count">{{ bgRunningCount }}</span>
            </span>
            <div class="bg-running-panel">
              <div class="bg-running-head">{{ $t('sync.bgRunningTitle') }}</div>
              <div v-if="!bgRunningList.length" class="bg-task-empty">{{ $t('sync.bgRunningEmpty') }}</div>
              <div v-for="t in bgRunningList" :key="t.id" class="bg-task-item" @click="resumeBgTask(t)">
                <span class="bg-dot" :class="bgStatusOf(t.id)"></span>
                <span class="bg-kind" :class="t.kind === 'compare' ? 'is-cmp' : ''">{{ t.kind === 'compare' ? $t('nav.compare') : $t('nav.sync') }}</span>
                <span class="bg-title">{{ t.title }}</span>
                <span class="bg-state">{{ bgStateText(t.id) }}</span>
                <el-button size="small" text type="danger" @click.stop="stopBgTask(t)">{{ $t('common.stop') }}</el-button>
              </div>
            </div>
          </span>
          <el-tooltip :content="themeTip" placement="bottom"><span class="top-nav-item top-icon-btn" :class="{ 'dc-top-active': themeMode !== 'system' }" @click="cycleTheme"><el-icon><component :is="themeIcon" /></el-icon></span></el-tooltip>
          <el-tooltip :content="langTip" placement="bottom"><span class="top-nav-item top-icon-btn top-lang-btn" @click="toggleLocale">{{ localeShort }}</span></el-tooltip>

          <!-- 检查更新：比对 GitHub 最新 Release，有新版可在线下载安装。
               图标用 Refresh（循环箭头 = 检查/更新），不用 Upload（上传）——后者语义不对 -->
          <el-tooltip :content="dlStatus === 'running' ? t('update.dlShowProgress') : $t('nav.update')" placement="bottom"><span class="top-nav-item top-icon-btn"
                     data-act="check-update"
                     :class="{ 'top-updating': updateChecking || dlStatus === 'running' }" @click="checkUpdate"><el-icon><Refresh /></el-icon></span></el-tooltip>

          <el-tooltip :content="$t('nav.settings')" placement="bottom"><span class="top-nav-item top-icon-btn" @click="settingsOpen = true"><el-icon><Setting /></el-icon></span></el-tooltip>
        </span>
      </nav>

      <!-- 窗口按钮：仅桌面端自绘（macOS 保留系统红绿灯，不重复渲染）。
           图形用内联 SVG，描边跟随主题色，无需图标组件 -->
      <div v-if="isDesktop && !isMacShell" class="win-acts">
        <button class="win-act" type="button" :title="$t('win.minimize')" @click="winMinimize">
          <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M2.5 6h7" /></svg>
        </button>
        <button class="win-act" type="button" :title="winMaximized ? $t('win.restore') : $t('win.maximize')" @click="winToggleMax">
          <svg v-if="winMaximized" viewBox="0 0 12 12" aria-hidden="true">
            <path d="M4 4V2.5h5.5V8H8" />
            <rect x="2" y="4.5" width="5.5" height="5.5" rx="0.5" />
          </svg>
          <svg v-else viewBox="0 0 12 12" aria-hidden="true"><rect x="2.5" y="2.5" width="7" height="7" rx="0.5" /></svg>
        </button>
        <button class="win-act is-close" type="button" :title="$t('win.close')" @click="winClose">
          <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M2.8 2.8l6.4 6.4M9.2 2.8l-6.4 6.4" /></svg>
        </button>
      </div>
    </header>

    <div class="body">
      <!-- 左侧树：连接 → 数据库 → 表/视图/索引/过程/触发器/事件（一棵统一树） -->
      <aside v-show="!sidebarHidden" class="sidebar" :style="{ width: treeWidth + 'px' }">
        <div class="tree-area" @contextmenu.prevent="onTreeAreaContextMenu">
          <!-- 工具栏：原「对象浏览器」标题与「新建脚本」已移除 —— 前者在这里被按钮挤成竖排，
               后者挪到顶栏（那里才有位置）。这 5 个动作仍然贴着树，用起来不离手。 -->
          <!-- 工具条（重做）：只留三个动作 —— 新建分组 / 新建连接 / 刷新。
               等宽、带文字：只剩三个时，"图标 + 文字"比五个纯图标更能撑住这一栏，也更好认。
               展开/收缩两个函数仍在（expandTreeOneLevel / collapseTreeAll），想加回来只需再放两个按钮。 -->
          <!-- 一行搞定：三个动作 + 搜索框。300px 宽里三个带文字的按钮 + 搜索框放不下，
               所以按钮回到"纯图标 + 悬停提示"（标题写在 el-tooltip 里），把宽度让给搜索。 -->
          <div class="tree-tools">
            <el-input v-model="filterText" clearable :prefix-icon="Search" class="tree-search" />
            <el-tooltip :content="$t('tree.newFolder')" placement="bottom"><button class="tree-tool" @click="treeCtxNewFolder('')"><el-icon><FolderAdd /></el-icon></button></el-tooltip>
            <el-tooltip :content="$t('tree.newConnection')" placement="bottom"><button class="tree-tool" @click="openNewConn('')"><el-icon><Plus /></el-icon></button></el-tooltip>
            <el-tooltip :content="$t('tree.refresh')" placement="bottom"><button class="tree-tool" @click="refreshTree"><el-icon><Refresh /></el-icon></button></el-tooltip>
          <el-tooltip :content="$t('tree.collapseTree')" placement="bottom"><button class="tree-tool" @click="sidebarHidden = true"><el-icon><Fold /></el-icon></button></el-tooltip>
          </div>
          <!-- 多选浮条：Ctrl / ⌘ 点选节点后出现，汇总这一批能做的动作。
               只做「打开 / 导出 / 删除连接」三件；删除**只认连接**（表 / 库的删除是 DDL，
               各走各的入口），且确认框只弹一次 —— 逐条弹一次就不叫批量了。 -->
          <div v-if="multiSel.length" class="multi-bar">
            <span class="multi-count">{{ $t('tree.multiSelected', { n: multiSel.length }) }}</span>
            <button class="multi-act" :disabled="batchBusy" @click="batchOpen">{{ $t('tree.multiOpen') }}</button>
            <button class="multi-act" :disabled="batchBusy" @click="batchExport">{{ $t('tree.multiExport') }}</button>
            <button class="multi-act danger" :disabled="batchBusy" @click="batchDelete">{{ $t('tree.multiDelete') }}</button>
            <button class="multi-act" :disabled="batchBusy" @click="clearMultiSelect">{{ $t('tree.multiCancel') }}</button>
          </div>
          <div v-loading="treeLoading" class="tree-loading">
          <el-tree
            ref="treeRef"
            :data="treeData"
            :props="{ label: 'label', children: 'children' }"
            node-key="id"
            :expand-on-click-node="false"
            :default-expanded-keys="defaultExpanded"
            :filter-node-method="filterNode"
            lazy
            :load="lazyLoad"
            highlight-current
            draggable
            :allow-drag="allowDragNode"
            :allow-drop="allowDropNode"
            @node-drag-start="onTreeDragStart"
            @node-drag-end="onTreeDragEnd"
            @node-drop="onTreeNodeDrop"
            @node-click="onNodeClick"
            @node-contextmenu="onTreeNodeContextMenu"
            @dblclick="onTreeDoubleClick"
            class="dc-tree"
          >
            <template #default="{ node, data }">
              <span class="tree-node" :class="{ 'multi-sel': isMultiSelected(data) }">
                <!-- 展开箭头：只有"能展开"的节点才显示，未展开=右向、
                     展开=下向、懒加载中=转圈。点箭头**只**切换展开，不触发"打开对象"；
                     点行则由 onNodeClick 决定语义（打开连接 / 打开库 / 预览）。
                     自绘它的三个理由：箭头单独可点、加载态有地方显示、
                     不可展开的节点不留一个空箭头位。 -->
                <el-icon
                  v-if="canExpandNode(data)"
                  class="tree-arrow"
                  :class="{ 'tree-arrow-expanded': node.expanded }"
                  @click.stop="onArrowClick(data, node)"
                >
                  <Loading v-if="isNodeLoading(node, data)" class="tree-arrow-spin" />
                  <ArrowRight v-else />
                </el-icon>
                <span v-else class="tree-arrow-spacer"></span>
                <!-- 环境文件夹 -->
                <el-icon v-if="data.kind === 'env-folder'" class="tree-icon" :color="envColor(data.env)">
                  <FolderOpened v-if="node.expanded" /><Folder v-else />
                </el-icon>
                <!-- 数据源节点：DB Logo + 连接名 + 环境角标 -->
                <span v-else-if="data.kind === 'conn'" class="conn-row">
                  <span class="logo-wrap">
                    <DbLogo :type="data.connType" :size="14" :connected="isConnOpen(data.connId) && !connErrorSet.has(String(data.connId))" class="tree-logo" />
                  </span>
                  <!-- hover 显示备注：库里看不出"这条连接是干嘛的"，备注能一句话说清 -->
                  <span class="node-label conn-label"
                        :title="data.note ? (data.name + '：' + data.note) : data.name">{{ data.label || data.name || node.label }}</span>
                  <!-- 只读徽标：这个状态决定写操作会不会被拦，得随时看得见（藏在编辑弹窗里没人记得住） -->
                  <span v-if="data.readOnly" class="ro-badge" :title="$t('cd.readOnlyTip')">{{ $t('cd.readOnlyShort') }}</span>
                  <span class="env-corner" :class="'env-corner-' + (data.env || 'none')" :title="envTitle(data.env)">{{ envShort(data.env) }}</span>
                </span>
                <!-- 库图标：**未打开=变暗**。
                     原先用"当前库=蓝、其它=黄"来区分 —— 那是拿**颜色**说**状态**，
                     而同一行的颜色还兼着"类型"的含义，用户没法一眼分清谁在说状态。
                     现在：颜色恒为琥珀（类型），明暗说状态，行尾绿点再说一次"它开着"。 -->
                <el-icon v-else-if="data.kind === 'db'" class="tree-icon"
                         :class="{ 'tree-icon-dim': !isDbOpened(data) }" color="#f5b34d"><Coin /></el-icon>
                <el-icon v-else-if="data.kind === 'schema'" class="tree-icon" color="#f5b34d">
                  <FolderOpened v-if="node.expanded" /><Folder v-else />
                </el-icon>
                <!-- catalog（Doris 的 internal / 外部 catalog）：比"库"更高一层的命名空间。
                     颜色沿用"容器"族的琥珀，只换形状 —— 与「库=Coin / schema=Folder」同一套规则
                     （颜色说类型族、形状说层级）。不给它图标的话会落到末尾的兜底分支，
                     和「库」长得一模一样，树上分不出来。 -->
                <el-icon v-else-if="data.kind === 'catalog'" class="tree-icon"
                         color="#f5b34d"><Collection /></el-icon>
                <el-icon v-else-if="data.kind === 'table'" class="tree-icon" color="#4f8cff"><Grid /></el-icon>
                <el-icon v-else-if="data.kind === 'view'" class="tree-icon" color="#3ddc97"><View /></el-icon>
                <el-icon v-else-if="data.kind === 'category' && data.cat === 'tables'" class="tree-icon" color="#4f8cff"><Grid /></el-icon>
                <el-icon v-else-if="data.kind === 'category' && data.cat === 'views'" class="tree-icon" color="#3ddc97"><View /></el-icon>
                <el-icon v-else-if="data.kind === 'category' && data.cat === 'scripts'" class="tree-icon" color="#22d3ee"><Document /></el-icon>
                <el-icon v-else-if="data.kind === 'script'" class="tree-icon" color="#22d3ee"><Document /></el-icon>
                <el-icon v-else-if="data.kind === 'category' && data.cat === 'procs'" class="tree-icon" color="#f59e0b"><SetUp /></el-icon>
                <!-- Functions 分类：图标与颜色跟下面的函数节点保持一致（Cpu / 蓝），
                     这样"分类"和"它里面的对象"一眼能对上，不用想"这两个是不是一类"。 -->
                <el-icon v-else-if="data.kind === 'category' && data.cat === 'functions'" class="tree-icon" color="#3b82f6"><Cpu /></el-icon>
                <el-icon v-else-if="data.kind === 'category' && data.cat === 'triggers'" class="tree-icon" color="#ef4444"><BellFilled /></el-icon>
                <el-icon v-else-if="data.kind === 'category' && data.cat === 'events'" class="tree-icon" color="#06b6d4"><Timer /></el-icon>
                <el-icon v-else-if="data.kind === 'collection'" class="tree-icon" color="#06b6d4"><Coin /></el-icon>
                <el-icon v-else-if="data.kind === 'function'" class="tree-icon" color="#3b82f6"><Cpu /></el-icon>
                <el-icon v-else-if="data.kind === 'procedure'" class="tree-icon" color="#f59e0b"><Setting /></el-icon>
                <el-icon v-else-if="data.kind === 'trigger'" class="tree-icon" color="#ef4444"><BellFilled /></el-icon>
                <el-icon v-else-if="data.kind === 'event'" class="tree-icon" color="#06b6d4"><Timer /></el-icon>
                <el-icon v-else-if="data.kind === 'category' && data.cat === 'users'" class="tree-icon" color="#a855f7"><User /></el-icon>
                <el-icon v-else-if="data.kind === 'user'" class="tree-icon" color="#a855f7"><User /></el-icon>
                <el-icon v-else-if="data.kind === 'placeholder' && data.label === $t('tree.loading')" class="tree-icon placeholder-spin" color="#4f8cff"><Loading /></el-icon>
                <el-icon v-else-if="data.kind === 'placeholder'" class="tree-icon" color="#555a68"><DocumentRemove /></el-icon>
                <el-icon v-else class="tree-icon" color="#f5b34d"><Coin /></el-icon>
                <!-- 「加载中…」占位只出转圈图标、不出文字；但**说明性**占位（如"未取到库清单"）
                     必须把文字显示出来 —— 否则用户只看到一个没有任何解释的图标。 -->
                <!-- 树内直接重命名：分组节点进入改名态时，标签**就地**换成输入框 ——
                     回车 / 失焦提交，Esc 取消。不再弹一个"请输入分组名称"的对话框。 -->
                <el-input v-if="data.kind === 'env-folder' && renamingKey === data.env"
                          v-model="renameDraft" size="small" class="tree-rename-input"
                          @click.stop @keyup.enter="commitRenameGroup()" @keyup.esc="cancelRenameGroup()"
                          @blur="commitRenameGroup()" />
                <span v-else-if="data.kind !== 'conn' && !(data.kind === 'placeholder' && data.label === $t('tree.loading'))"
                      class="node-label"
                      :class="{ 'env-folder-label': data.kind === 'env-folder', 'db-label-current': data.kind === 'db' && isDbOpened(data), 'hint-label': data.kind === 'placeholder' }">{{ data.i18nKey ? $t(data.i18nKey) : (data.label || node.label) }}</span>
                <!-- 行数未知时整个不显示（不是显示 0，也不是留一个空药丸）：
                     「不知道」和「这张表是空的」必须能一眼区分开 -->
                <el-tag v-if="data.kind === 'table' && (tableRows[data.id] != null || data.rows != null)" size="small" effect="dark" class="row-tag">{{ tableRows[data.id] != null ? tableRows[data.id] : data.rows }}</el-tag>
                <!-- 数量直接取 children.length：**别在模板里 filter** —— 模板表达式每次重渲染
                     都会对所有分类节点重算一遍（树大时等于 O(n²)）；而"排除 placeholder"这件事
                     在连接节点上早已不存在（那行"加载中…"占位已删除）。 -->
                <!-- 计数只在**该分类真的加载过**之后才显示：分类是懒加载的，
                     未加载时显示 0 会被读成"这一类没有对象"（和"还没查"完全是两回事）。 -->
                <!-- 数量：预取的**响应式表**优先（不必点开就有 ✓），否则回落到 children.length；
                     两者都没有就不显示 —— "还没取到" 与 "这一类是 0" 必须能分清。 -->
                <span v-if="data.kind === 'category' && catCounts[data.id] != null" class="cat-count">{{ catCounts[data.id] }}</span>
                <span v-if="data.kind === 'env-folder'" class="env-count">{{ (data.children || []).length }}</span>
              </span>
            </template>
            <template #empty>
              <div class="tree-empty">{{ $t('tree.empty') }}</div>
            </template>
          </el-tree>
          </div>
        </div>
      </aside>
      <!-- 侧栏拖拽手柄 -->
      <div v-show="!sidebarHidden" class="tree-resizer" :title="$t('tree.resizerTitle')"
           @mousedown.prevent="startResize" />


      <!-- 右侧内容。
           拖拽页签时"整条页签栏都是落点"的监听挂在这里，但闸门是「指针必须在页签栏那条带内」——
           指针在下面数据区时一概不插手，免得误显示落点。 -->
      <main class="content"
            @dragover="onTabsStripDragOver"
            @drop="onTabsStripDrop"
            @dragleave="onTabsStripDragLeave">
        <div v-if="!conn && allConnections.length === 0" class="conn-empty">
          <el-empty :description="$t('tree.noConnection')" :image-size="120" />
        </div>
        <!-- 无 tab 时直接展示引导页 -->
        <div v-else-if="tabs.length === 0" class="empty-hint">
          <!-- 空态 = 首页仪表盘：KPI → 快速开始 / 常用操作 → 最近查询 / MCP → 趋势 / 失败查询。
               KPI（总连接数 / 已连接数 / 连接类型）曾按「只留有操作价值的块」砍掉，
               但「有多少家底、活了几条」是第一眼问题，已按要求加回顶部；
               外层 .empty-stack 用 margin:auto 垂直居中：内容不满一屏时上下留白均分，
               超出一屏时 margin 自动归零 + 容器本身可滚，顶部不会被裁（比
               justify-content:center 安全）。 -->
          <div class="empty-stack">
          <!-- KPI：总连接数 / 已连接数 / 连接类型 —— 先给「家底」，
               扫一眼就知道有多少数据源、活了几条、都什么类型 -->
          <div class="empty-kpis">
            <div class="ek">
              <span class="ek-k">{{ $t('empty.kpiTotal') }}</span>
              <span class="ek-v">{{ allConnections.length }}</span>
            </div>
            <div class="ek">
              <span class="ek-k">{{ $t('empty.kpiConnected') }}</span>
              <span class="ek-v" :class="{ ok: connectedCount > 0 }">{{ connectedCount }}</span>
            </div>
            <div class="ek">
              <span class="ek-k">{{ $t('empty.kpiTypes') }}</span>
              <span class="ek-v" :title="connTypeSummary">{{ connTypeCount }}</span>
            </div>
          </div>
          <div class="empty-grid">
            <!-- 左：快速开始（点一行即打开该连接） -->
            <div class="empty-block">
              <div class="empty-dss-head">
                <span>{{ $t('empty.quickStart') }}</span>
                <span class="empty-dss-count">{{ allConnections.length }}</span>
              </div>
              <div v-if="allConnections.length" class="empty-dss-list">
                <button v-for="c in allConnections.slice(0, 8)" :key="c.id" class="empty-ds"
                        :title="c.host ? $t('tree.openWithHost', { name: c.name, host: c.host }) : $t('tree.open', { name: c.name })" @click="selectConn(c.id)">
                  <DbLogo :type="c.type" :size="18"
                          :connected="isConnOpen(c.id) && !connErrorSet.has(String(c.id))" />
                  <span class="ds-main">
                    <span class="ds-name">{{ c.name }}</span>
                    <span class="ds-sub"><span class="ds-type">{{ typeLabel(c.type) }}</span></span>
                  </span>
                  <el-icon class="ds-go"><ArrowRight /></el-icon>
                </button>
              </div>
              <div v-else class="empty-hs-none">{{ $t('empty.noDatasource') }}</div>
            </div>

            <!-- 右：常用操作 -->
            <div class="empty-block">
              <div class="empty-dss-head"><span>{{ $t('empty.commonActions') }}</span></div>
              <div class="empty-acts">
                <button class="empty-act" @click="treeCtxNewFolder('')">
                  <el-icon><FolderAdd /></el-icon><span>{{ $t('tree.menu.newFolder') }}</span>
                </button>
                <button class="empty-act" @click="openNewConn('')">
                  <el-icon><Plus /></el-icon><span>{{ $t('tree.newConnection') }}</span>
                </button>
                <button class="empty-act" @click="newQueryTab()">
                  <el-icon><Document /></el-icon><span>{{ $t('empty.newQuery') }}</span>
                </button>
                <button class="empty-act" @click="toggleAi">
                  <el-icon><MagicStick /></el-icon><span>{{ $t('empty.aiAssistant') }}</span>
                </button>
              </div>
            </div>
          </div>

          <!-- 第二行两栏：[最近查询 | MCP 集成]
               历史**不拉高**去填满（拉高后卡里一大片空白，比空着更难看）；
               宽度改由两栏并排用掉，屏幕窄了自动折成一栏。 -->
          <div class="empty-grid">
            <div class="empty-block">
              <div class="empty-dss-head">
                <span>{{ $t('empty.recentQueries') }}</span>
                <span class="empty-dss-count">{{ recentHistory.length }}</span>
                <!-- 清空历史：后端一直有 DELETE /api/dbmind/history，界面却没有入口。
                     只在**有记录**时出现 —— 空卡片上摆一个点了也没用的按钮是噪音。 -->
                <el-tooltip v-if="recentHistory.length" :content="$t('empty.clearHistory')" placement="top">
                  <el-button class="empty-head-btn" text size="small" :icon="Delete"
                             @click="onClearHistory" />
                </el-tooltip>
              </div>
              <div v-if="recentHistory.length" class="empty-hist-list">
                <button v-for="h in recentHistory" :key="h.id" class="empty-hi"
                        :title="h.sql" @click="openHistoryItem(h)">
                  <span class="hi-dot" :class="h.status === 'ok' ? 'ok' : 'bad'"></span>
                  <code class="hi-sql">{{ h.sql }}</code>
                  <span class="hi-meta">{{ h.connectionName }} · {{ clockText(h.createdAt) }}</span>
                </button>
              </div>
              <div v-else class="empty-hs-none">
                  <!-- 过滤掉程序自动发的查询后，可能出现"历史非空但这里为空"——文案必须说清，
                       否则用户会以为记录丢了（原来那句"还没有执行过查询"就是这种误导）。 -->
                  <template v-if="recentHistory.length">{{ $t('empty.autoQueriesOnly') }}</template>
                  <template v-else>{{ $t('empty.noQueries') }}</template>
                </div>
            </div>

            <!-- MCP 集成（`crates/dbmind-mcp` 是真实存在的 stdio 服务） -->
            <div class="empty-block">
              <div class="empty-dss-head">
                <span>{{ $t('empty.mcpTitle') }}</span>
                <span class="mcp-inline">{{ $t('empty.mcpHint') }}</span>
                <button class="mcp-go" @click="openSettings('mcp')">{{ $t('empty.mcpGoSettings') }} →</button>
              </div>
              <div class="mcp-code">
                <code>{{ mcpSnippet }}</code>
                <el-button text size="small" class="mcp-copy" @click="copyMcp">{{ $t('empty.copy') }}</el-button>
              </div>
            </div>
          </div>
<!-- 第三行：[近 7 天趋势 | 失败查询] —— 欢迎页底部的留白补真内容：
     左边是近一周的执行量柱状图（成功/失败两段堆叠），右边把最近失败的查询列出来，
     点一条就填进新脚本，改一改重跑 —— 比「记在脑子里再去历史里翻」省事。 -->
<div class="empty-grid empty-extra">
  <div class="empty-block trend-block">
    <div class="empty-dss-head">
      <span>{{ $t('empty.trendTitle') }}</span>
      <span class="trend-legend">
        <i class="lg ok"></i>{{ $t('empty.legendOk') }}
        <i class="lg bad"></i>{{ $t('empty.legendFail') }}
      </span>
    </div>
    <div class="empty-trend">
      <div v-for="d in trendDays" :key="d.key" class="trend-col"
           :title="$t('empty.trendDayTip', { day: d.label, n: d.count })">
        <div class="trend-bars">
          <div v-if="d.fails > 0" class="trend-bar bad" :style="{ height: barHeight(d.fails) }"></div>
          <div v-if="d.count - d.fails > 0" class="trend-bar ok" :style="{ height: barHeight(d.count - d.fails) }"></div>
        </div>
        <span class="trend-n">{{ d.count || '' }}</span>
        <span class="trend-label">{{ d.label }}</span>
      </div>
    </div>
  </div>
  <div class="empty-block">
    <div class="empty-dss-head"><span>{{ $t('empty.dataTools') }}</span></div>
    <div class="empty-acts">
      <button class="empty-act" @click="openCompare">
        <el-icon><Operation /></el-icon><span>{{ $t('empty.compare') }}</span>
      </button>
      <button class="empty-act" @click="openSync">
        <el-icon><Switch /></el-icon><span>{{ $t('empty.sync') }}</span>
      </button>
      <button class="empty-act" @click="openGovernance('quality')">
        <el-icon><Collection /></el-icon><span>{{ $t('qa.rulesTitle') }}</span>
      </button>
      <button class="empty-act" @click="openGovernance('analysis')">
        <el-icon><DataAnalysis /></el-icon><span>{{ $t('qa.title') }}</span>
      </button>
    </div>
  </div>
</div>
          </div>
        </div>
        <el-tabs v-else v-model="activeTab" type="border-card" closable class="dc-tabs"
                 @tab-remove="closeTab" @tab-change="onMainTabChange">
          <el-tab-pane v-for="tab in tabs" :key="tab.id" :name="tab.id" :closable="true" lazy>
            <template #label>
              <!-- 拖拽排序：el-tabs 自己不管排序，重排交给下面几个 handler 改 tabs 数组。
                   tab-drop-before / tab-drop-after 决定插入指示条画在左边还是右边
                   （靠由 dragover 的 clientX 与页签中线比出来）。 -->
              <span class="tab-label"
                    :class="{
                      'tab-dirty': tab.dirty,
                      'tab-dragging': dragTabId === tab.id,
                      'tab-drop-target': dragOverTabId === tab.id && dragTabId !== tab.id,
                      'tab-drop-before': dragOverTabId === tab.id && dragTabId !== tab.id && dragOverSide === 'before',
                      'tab-drop-after': dragOverTabId === tab.id && dragTabId !== tab.id && dragOverSide === 'after'
                    }"
                    draggable="true"
                    @dragstart="onTabDragStart($event, tab)"
                    @dragover="onTabDragOver($event, tab)"
                    @drop="onTabDrop($event, tab)"
                    @dragend="onTabDragEnd"
                    @contextmenu.prevent="showTabCtxMenu($event, tab)">
                <!-- 连接环境外显：PROD 连接的页签带红色 P 标（防误连） -->
                <span v-if="isProdTab(tab)" class="tab-prod-dot" :title="$t('nav.prodTabTip')">P</span>
                {{ tab.dirty ? '* ' : '' }}{{ tabLabel(tab) }}
              </span>
            </template>
            <TableDataView v-if="tab.type === 'table'" :conn="connFor(tab.connId)" :database="tab.database || currentDb" :table="tab.table" :readOnly="tab.readOnly" @update-table-rows="onUpdateTableRows" @open-query="onOpenQueryFromTable" />
            <SqlQueryView v-else-if="tab.type === 'sql'" :ref="el => setSqlRef(tab.id, el)" :conn="conn" :database="currentDb" :script-name="tab.scriptName" :tab-conn-id="tab.connId" :tab-database="tab.database" :initial-sql="tab.initialSql" :auto-run="tab.autoRun" @save="onScriptSave(tab.id, $event)" @dirty-change="onScriptDirty(tab.id, $event)" @sql-change="onScriptSqlChange(tab.id, $event)" @conn-change="onEditorConnChange" @db-change="onEditorDbChange" />
            <ErDiagramView v-else-if="tab.type === 'er'" :conn="connFor(tab.connId)" :database="tab.database || currentDb" :conn-id="tab.connId" />

            <TableDetailView v-else-if="tab.type === 'detail'" :conn="connFor(tab.connId)" :database="tab.database || currentDb" :table="tab.table" @deleted="onDetailDeleted" />
            <NoSqlDataView v-else-if="tab.type === 'nosql'" :conn="connFor(tab.connId)" :database="tab.database"
                           :collection="tab.collection" :kind="tab.kind" />
            <ObjectDetailView v-else-if="tab.type === 'object'" :conn="connFor(tab.connId)" :database="tab.database || currentDb"
                              :type="tab.objectType" :name="tab.objectName" @edit="onObjDetailEdit" />
            <UserDetailView v-else-if="tab.type === 'user'" :conn="connFor(tab.connId)" :database="tab.database || currentDb"
                            :name="tab.userName" @edit="onUserDetailEdit" />
            <UserForm v-else-if="tab.type === 'user-form'" :conn="connFor(tab.connId)" :database="tab.database || currentDb"
                      :edit-mode="tab.mode === 'edit'" :edit-data="tab.editData"
                      @saved="onUserFormSaved" @close="closeTab(tab.id)" />
            <ObjectFormTab v-else-if="tab.type === 'form'" :conn="connFor(tab.connId)" :database="tab.database || currentDb"
                           :cat="tab.cat" :mode="tab.mode" :object="tab.object" @saved="onObjFormSaved"
                           @kind="tab.formKind = $event" @close="closeTab(tab.id)" />
            <!-- AI 助手：中央工作区形态（左侧能力导航 + 大内容区），与表数据/SQL 并列为一个 Tab -->
            <AiPanel v-else-if="tab.type === 'ai'" mode="studio" :conn="conn" :database="currentDb"
                     :pending="pendingAi"
                     @insert-sql="insertSqlToActive" @panel="onAiPanelCommand" @run-plan="onRunPlan"
                     @open-knowledge="openKnowledge" @close="closeTab(tab.id)" />
            <!-- 知识库工作台：左栏库列表 + 右栏「文档 / 召回测试 / 分段设置」 -->
            <KnowledgeStudio v-else-if="tab.type === 'knowledge'"
                             :focus-kb-id="pendingKbFocus" @focus-done="pendingKbFocus = ''" />
            <!-- 监控工作台：关键指标卡片 + 锁等待/会话/慢查询/表空间/关键参数，支持终止会话 -->
            <MonitorPanel v-else-if="tab.type === 'monitor'" :conn-id="tab.connId" :database="tab.database" />
          </el-tab-pane>
        </el-tabs>
      </main>

      <!-- 把表结构存进知识库：内容由「数据字典」生成，这里只负责选目标库 -->
      <SaveToKbDialog v-model="saveKbOpen" :content="saveKbContent"
                      :default-title="saveKbTitle" :source="saveKbSource" />

      <!-- AI 面板 -->
      <AiPanel ref="aiPanelRef" v-show="aiOpen" :conn="conn" :database="currentDb" class="ai-panel"
               @insert-sql="insertSqlToActive" @close="aiOpen = false"
               @open-knowledge="openKnowledge"
               @panel="onAiPanelCommand" @run-plan="onRunPlan" />
    </div>

    <!-- 数据对比 / 同步 -->
    <CompareDialog :model-value="compareDialogOpen" :conn="conn" :database="currentDb"
    :tables="allTables" :resume-id="compareResumeId" @update:model-value="onCompareDialogVisible" />
    <SyncDialog :model-value="syncDialogOpen" :conn="conn" :database="currentDb"
                :resume-task-id="bgResumeId" @update:model-value="onSyncDialogVisible" />

    <!-- 任务中心两个视图共用一个弹窗：history = 全部执行记录（**表格 + 分页**，字段：
         类型/任务/状态/开始/结束/耗时/操作）；running = 正在执行（右侧时钟悬浮的轻列表）。
         状态在打开时实时刷新，终态写回记录（持久化），刷新页面后历史仍显示完成/失败。 -->
    <el-dialog v-model="bgCenterOpen" :title="bgCenterMode === 'running' ? $t('sync.bgRunningTitle') : $t('sync.bgCenter')" width="1000px" append-to-body>
    <template v-if="bgCenterMode === 'history'">
    <div v-if="bgTasks.length" class="bg-toolbar">
      <el-button size="small" type="danger" plain :icon="ElDelete" @click="confirmClearBg">{{ $t('sync.bgClear') }}</el-button>
    </div>
    <el-table :data="bgPageRows" size="small" border class="data-table" @row-click="resumeBgTask" :row-class-name="() => 'bg-row-click'">
    <el-table-column :label="$t('ts.kind')" width="80" align="center" header-align="center">
      <template #default="{ row }"><span class="bg-kind" :class="row.kind === 'compare' ? 'is-cmp' : ''">{{ row.kind === 'compare' ? $t('nav.compare') : $t('nav.sync') }}</span></template>
    </el-table-column>
    <el-table-column prop="title" :label="$t('ts.task')" min-width="170" show-overflow-tooltip align="center" header-align="center" />
    <el-table-column :label="$t('ts.status')" width="90" show-overflow-tooltip align="center" header-align="center">
      <template #default="{ row }"><span class="bg-dot bg-dot-inline" :class="bgStatusOf(row.id)"></span>{{ bgStateShort(row.id) }}</template>
    </el-table-column>
    <el-table-column :label="$t('ts.start')" align="center" header-align="center" width="150">
      <template #default="{ row }">{{ bgTimeText(row, 'start') }}</template>
    </el-table-column>
    <el-table-column :label="$t('ts.end')" align="center" header-align="center" width="150">
      <template #default="{ row }">{{ row.finishedAt ? bgTimeText(row) : '—' }}</template>
    </el-table-column>
    <el-table-column :label="$t('ts.duration')" align="center" header-align="center" width="80">
      <template #default="{ row }">{{ bgDurationText(row) }}</template>
    </el-table-column>
    <el-table-column :label="$t('ts.actions')" align="center" header-align="center" width="130">
      <template #default="{ row }">
        <el-button text size="small" type="primary" @click.stop="resumeBgTask(row)">{{ $t('ts.view') }}</el-button>
        <!-- 运行中：停止（卡住任务的出口）；已结束：删除 -->
        <el-button v-if="bgStatusOf(row.id) === 'running'" text size="small" type="danger" @click.stop="stopBgTask(row)">{{ $t('common.stop') }}</el-button>
        <el-button v-else text size="small" type="danger" @click.stop="removeBgTask(row.id)">{{ $t('common.delete') }}</el-button>
      </template>
    </el-table-column>
    </el-table>
    <el-pagination v-model:current-page="bgPage" :page-size="bgPageSize" :total="bgTasks.length"
                   layout="total, prev, pager, next" size="small" class="bg-pager" />
    </template>
    <div v-else class="bg-task-list">
    <div v-if="!bgRunningList.length" class="bg-task-empty">{{ $t('sync.bgRunningEmpty') }}</div>
    <div v-for="t in bgRunningList" :key="t.id" class="bg-task-item" @click="resumeBgTask(t)">
    <span class="bg-dot" :class="bgStatusOf(t.id)"></span>
    <span class="bg-kind" :class="t.kind === 'compare' ? 'is-cmp' : ''">{{ t.kind === 'compare' ? $t('nav.compare') : $t('nav.sync') }}</span>
    <span class="bg-title">{{ t.title }}</span>
    <span class="bg-state">{{ bgStateText(t.id) }}</span>
    <el-button size="small" text type="danger" @click.stop="stopBgTask(t)">{{ $t('common.stop') }}</el-button>
    </div>
    </div>
    
    </el-dialog>

    <!-- 表右键菜单 -->
    <ul v-if="ctxMenu.show" class="ctx-menu" :style="{ left: ctxMenu.x + 'px', top: ctxMenu.y + 'px' }"
        @click.stop @contextmenu.prevent>
      <!-- ===== 表 ===== -->
      <template v-if="ctxMenu.data?.objectKind === 'table'">
        <!-- 能力位门控：`supportsDDLExec`= 该类型能执行 DDL（SQL 类恒 true、NoSQL false），
             `supportsDdl` = 能**取回**建表语句（Derby/DB2 取不到，但 DROP TABLE 照样能跑）——
             两者别混用：拿 supportsDdl 门控删除操作会把 Derby / DB2 的删除表整个藏掉。 -->
        <li @click="ctxOpenData"><el-icon><Grid /></el-icon><span>{{ $t('common.viewData') }}</span></li>
        <li v-if="featureOf(ctxMenu.data?.connId, 'supportsDDLExec')" @click="ctxOpenDetail"><el-icon><Document /></el-icon><span>{{ $t('menu.editStructure') }}</span></li>
        <li v-if="featureOf(ctxMenu.data?.connId, 'supportsDDLExec')" @click="ctxRenameTable"><el-icon><Edit /></el-icon><span>{{ $t('common.rename') }}</span></li>
        <li v-if="featureOf(ctxMenu.data?.connId, 'supportsDdl')" @click="ctxShowDdl"><el-icon><DocumentCopy /></el-icon><span>{{ $t('menu.viewDdl') }}</span></li>
        <li @click="ctxShowIndexes"><el-icon><Coin /></el-icon><span>{{ $t('menu.viewIndexes') }}</span></li>
        <li class="divider" />
        <li @click="ctxCopyName"><el-icon><CopyDocument /></el-icon><span>{{ $t('menu.copyTableName') }}</span></li>
        <li class="divider" />
        <li class="has-sub" @mouseenter="ctxSub('sql-tpl')" @mouseleave="ctxSub(null)">
          <el-icon><EditPen /></el-icon><span>{{ $t('menu.genSql') }}</span><el-icon class="arrow"><ArrowRight /></el-icon>
          <ul v-show="ctxMenu.subKey === 'sql-tpl'" class="ctx-submenu">
            <li @click.stop="ctxGenSql('select')">{{ $t('menu.sqlSelect') }}</li>
            <li @click.stop="ctxGenSql('insert')">{{ $t('menu.sqlInsert') }}</li>
            <li @click.stop="ctxGenSql('update')">{{ $t('menu.sqlUpdate') }}</li>
            <li @click.stop="ctxGenSql('delete')">{{ $t('menu.sqlDelete') }}</li>
          </ul>
        </li>
        <li v-if="featureOf(ctxMenu.data?.connId, 'supportsExport')" class="has-sub" @mouseenter="ctxSub('export')" @mouseleave="ctxSub(null)">
          <el-icon><Download /></el-icon><span>{{ $t('menu.exportData') }}</span><el-icon class="arrow"><ArrowRight /></el-icon>
          <ul v-show="ctxMenu.subKey === 'export'" class="ctx-submenu">
            <li @click.stop="ctxExport('csv')">{{ $t('menu.exportCsv') }}</li>
            <li @click.stop="ctxExport('excel')">{{ $t('menu.exportExcel') }}</li>
            <li @click.stop="ctxExport('json')">{{ $t('menu.exportJson') }}</li>
          </ul>
        </li>
        <li v-if="featureOf(ctxMenu.data?.connId, 'supportsImport')" @click="ctxOpenImport"><el-icon><Upload /></el-icon><span>{{ $t('menu.importData') }}</span></li>
        <li v-if="featureOf(ctxMenu.data?.connId, 'supportsTableData')" @click="ctxOpenDataGen"><el-icon><MagicStick /></el-icon><span>{{ $t('menu.dataGen') }}</span></li>
        <!-- 「发送到 AI」放**一级菜单**：放在 AI 助手子菜单里的话，右键后要悬停才展开，
             第一眼会以为"表上根本没有这个入口"（用户实测反馈）。子菜单里只留
             「数据洞察 / 把表结构存入知识库」这两个更重的动作。 -->
        <li @click="sendNodeToAi(ctxMenu.data)"><el-icon><Cpu /></el-icon><span>{{ $t('menu.sendToAi') }}</span></li>
        <!-- AI 入口铺到工作流里：不用先打开 AI 面板再复述一遍表名 -->
        <li class="has-sub" @mouseenter="ctxSub('ai')" @mouseleave="ctxSub(null)">
          <el-icon class="ai-ic"><MagicStick /></el-icon><span>{{ $t('menu.aiAssistant') }}</span><el-icon class="arrow"><ArrowRight /></el-icon>
          <ul v-show="ctxMenu.subKey === 'ai'" class="ctx-submenu">
            <!-- 带上下文提问：把这张表（含连接 / 库）写进问题并开中央 AI 页签 -->
            <li @click.stop="ctxAiInsightTable">{{ $t('menu.aiInsight') }}</li>
            <li @click.stop="ctxSaveTableToKb">{{ $t('menu.saveTableToKb') }}</li>
          </ul>
        </li>
        <li class="divider" />
        <li v-if="featureOf(ctxMenu.data?.connId, 'supportsDDLExec')" class="danger" @click="ctxClearTable"><el-icon><Delete /></el-icon><span>{{ $t('menu.clearTable') }}</span></li>
        <li v-if="featureOf(ctxMenu.data?.connId, 'supportsDDLExec')" class="danger" @click="ctxTruncateTable"><el-icon><Delete /></el-icon><span>{{ $t('menu.truncateTable') }}</span></li>
        <li v-if="featureOf(ctxMenu.data?.connId, 'supportsDDLExec')" class="danger" @click="ctxDropTable"><el-icon><Delete /></el-icon><span>{{ $t('menu.dropTable') }}</span></li>
        <li class="divider" />
        <li @click="ctxRefreshRows"><el-icon><Refresh /></el-icon><span>{{ $t('common.refresh') }}</span></li>
      </template>
      <!-- ===== 视图 ===== -->
      <template v-else-if="ctxMenu.data?.objectKind === 'view'">
        <li @click="ctxOpenData"><el-icon><View /></el-icon><span>{{ $t('common.view') }}</span></li>
        <li v-if="objectDdlAllowed(ctxMenu.data?.connId, 'view')" @click="objEditScript"><el-icon><EditPen /></el-icon><span>{{ $t('common.edit') }}</span></li>
        <li @click="ctxCopyObjName"><el-icon><CopyDocument /></el-icon><span>{{ $t('common.copy') }}</span></li>
        <li v-if="objectDdlAllowed(ctxMenu.data?.connId, ctxMenu.data?.objectKind)" @click="ctxRenameObject"><el-icon><Edit /></el-icon><span>{{ $t('common.rename') }}</span></li>
        <li class="divider" />
        <li @click="ctxAiExplainView" class="ai-item"><el-icon><MagicStick /></el-icon><span>{{ $t('menu.explainView') }}</span></li>
        <li class="divider" />
        <li v-if="featureOf(ctxMenu.data?.connId, 'supportsDDLExec')" class="danger" @click="objDrop"><el-icon><Delete /></el-icon><span>{{ $t('common.delete') }}</span></li>
        <li @click="objRefresh"><el-icon><Refresh /></el-icon><span>{{ $t('common.refresh') }}</span></li>
      </template>
      <!-- ===== 用户 ===== -->
      <template v-else-if="ctxMenu.data?.objectKind === 'user'">
        <!-- 用户管理**只读**：只留「查看」（详情 + 权限展示）。编辑/新建/删除入口
             全部下掉 —— 用户要求先只做查看（编辑链路保留在代码里，随时可恢复） -->
        <li v-if="featureOf(ctxMenu.data?.connId, 'supportsUserDetail')" @click="openUserDetail"><el-icon><View /></el-icon><span>{{ $t('common.view') }}</span></li>
        <li class="divider" />
        <li @click="ctxCopyName"><el-icon><CopyDocument /></el-icon><span>{{ $t('common.copy') }}</span></li>
        <li class="divider" />
        <li @click="objRefresh"><el-icon><Refresh /></el-icon><span>{{ $t('common.refresh') }}</span></li>
      </template>
      <!-- ===== NoSQL collection / key / index ===== -->
      <template v-else-if="ctxMenu.data?.objectKind === 'collection'">
        <li @click="ctxOpenNoSqlData"><el-icon><Grid /></el-icon><span>{{ $t('common.viewData') }}</span></li>
        <li class="divider" />
        <li @click="ctxCopyName"><el-icon><CopyDocument /></el-icon><span>{{ $t('menu.copyName') }}</span></li>
        <li class="divider" />
        <li class="danger" @click="ctxDropCollection"><el-icon><Delete /></el-icon><span>{{ $t('common.delete') }}</span></li>
        <li class="divider" />
        <li @click="ctxRefreshCollection"><el-icon><Refresh /></el-icon><span>{{ $t('common.refresh') }}</span></li>
      </template>
      <!-- ===== 存储过程 / 函数 / 触发器 / 事件 ===== -->
      <template v-else-if="['procedure', 'function', 'trigger', 'event'].includes(ctxMenu.data?.objectKind)">
        <li v-if="objectDdlAllowed(ctxMenu.data?.connId, ctxMenu.data?.objectKind)" @click="objViewDetail"><el-icon><Document /></el-icon><span>{{ $t('common.view') }}</span></li>
        <li v-if="objectDdlAllowed(ctxMenu.data?.connId, ctxMenu.data?.objectKind)" @click="objEditScript"><el-icon><EditPen /></el-icon><span>{{ $t('common.edit') }}</span></li>
        <li @click="ctxCopyObjName"><el-icon><CopyDocument /></el-icon><span>{{ $t('common.copy') }}</span></li>
        <li v-if="objectDdlAllowed(ctxMenu.data?.connId, ctxMenu.data?.objectKind)" @click="ctxRenameObject"><el-icon><Edit /></el-icon><span>{{ $t('common.rename') }}</span></li>
        <li class="divider" />
        <li v-if="featureOf(ctxMenu.data?.connId, 'supportsDDLExec')" class="danger" @click="objDrop"><el-icon><Delete /></el-icon><span>{{ $t('common.delete') }}</span></li>
        <li @click="objRefresh"><el-icon><Refresh /></el-icon><span>{{ $t('common.refresh') }}</span></li>
      </template>
      <!-- ===== 索引 ===== -->
      <!--
        这里原来是 `v-else`（兜底），于是它成了一段**永远不可达**的代码：
        树的右键只会落在 table / view / procedure / function / trigger / event / user / collection 上，
        从来不会产生 index 节点（索引在「编辑结构 → 索引」页签里管）。
        写成明确的 `v-else-if`：既去掉「兜底却是死代码」的误导，也避免将来某天真的多了
        一种节点时，菜单静默落到一组不相干的动作上。
      -->
      <template v-else-if="ctxMenu.data?.objectKind === 'index'">
        <li @click="objViewDetail"><el-icon><Document /></el-icon><span>{{ $t('common.view') }}</span></li>
        <li @click="objEditDdl"><el-icon><EditPen /></el-icon><span>{{ $t('common.edit') }}</span></li>
        <li class="danger" @click="objDrop"><el-icon><Delete /></el-icon><span>{{ $t('common.delete') }}</span></li>
        <li @click="objRefresh"><el-icon><Refresh /></el-icon><span>{{ $t('common.refresh') }}</span></li>
      </template>
    </ul>

    <!-- DDL 弹窗 -->
    <el-dialog v-model="ctxDdlVisible" width="720px" append-to-body class="main-dialog">
      <template #header>
        <div class="dlg-title">
          <span class="dlg-title-ic"><el-icon :size="16"><Document /></el-icon></span>
          <span>{{ $t('menu.ddlTitle') }}</span>
          <el-button class="dlg-copy-btn" size="small" plain :icon="CopyDocument" @click="copyCtxDdl">{{ $t('common.copy') }}</el-button>
        </div>
      </template>
      <pre class="ctx-pre">{{ ctxDdl }}</pre>
    </el-dialog>

    <!-- 索引元数据弹窗：只读看一眼"这张表有哪些索引"（列名跟着各库返回走） -->
    <el-dialog v-model="ctxIndexVisible" width="760px" append-to-body class="main-dialog">
      <template #header>
        <div class="dlg-title">
          <span class="dlg-title-ic"><el-icon :size="16"><Coin /></el-icon></span>
          <span>{{ $t('menu.indexTitle', { table: ctxIndexTable }) }}</span>
          <el-button class="dlg-copy-btn" size="small" plain :icon="CopyDocument"
                     :disabled="!ctxIndexRows.length" @click="copyCtxIndexes">{{ $t('common.copy') }}</el-button>
        </div>
      </template>
      <div v-loading="ctxIndexLoading" class="ctx-index-wrap">
        <el-table v-if="ctxIndexRows.length" :data="ctxIndexRows" size="small" border style="width:100%">
          <el-table-column v-for="c in ctxIndexCols" :key="c" :prop="c" :label="c" show-overflow-tooltip />
        </el-table>
        <div v-else-if="!ctxIndexLoading" class="ctx-index-empty">
          {{ $t('menu.indexEmpty') }}
        </div>
      </div>
    </el-dialog>

    <!-- 导入数据弹窗（逻辑在 ImportDataDialog.vue，打开时快照目标表，成功后回调刷新树） -->
    <ImportDataDialog v-model="ctxImportVisible" :conn="conn" :target="ctxImportTarget" @done="onImportDone" />

    <!-- 数据生成弹窗（逻辑在 DataGenDialog.vue，按真实字段结构推导规则，可预览后批量写入） -->
    <DataGenDialog v-model="ctxDataGenVisible" :conn="connFor(ctxDataGenTarget?.connId)" :target="ctxDataGenTarget" @done="onDataGenDone" />

    <!-- AI 解释视图弹窗 -->
    <el-dialog v-model="aiExplainVisible" width="640px" :close-on-click-modal="true" append-to-body class="main-dialog ai-explain-dialog">
      <template #header>
        <div class="dlg-title">
          <el-icon><MagicStick /></el-icon>
          <span>{{ aiExplainTitle }}</span>
        </div>
      </template>
      <div class="ai-explain-body" v-html="aiExplainContent"></div>
    </el-dialog>

    <!-- 新建数据源：类型选择器（顶部还有「导入/导出连接配置」入口；
         导入完成后走 onConnSaved 同一条刷新链路：重载连接 + 重建左侧树） -->
    <DataSourcePicker v-model="dsPickerVisible" @selected="onDsSelected" @imported="onConnSaved"
                      @open-conn="onRecentConn" />

    <!-- 新建/编辑连接 -->
    <ConnectionDialog v-model="connDialogVisible" :conn="editingConn" :custom-folders="customFolders" :initial-group="connDialogInitialEnv" :initial-type="connDialogInitialType" @saved="onConnSaved" @folder-added="refreshFolders" />

    <!-- 数据库 DDL 弹窗 -->
    <el-dialog v-model="dbDdlVisible" width="720px" append-to-body class="main-dialog">
      <template #header>
        <div class="dlg-title">
          <span class="dlg-title-ic"><el-icon :size="16"><Coin /></el-icon></span>
          <span>{{ $t('menu.dbDdlTitle') }}</span>
          <el-button class="dlg-copy-btn" size="small" plain :icon="CopyDocument" @click="copyDbDdl">{{ $t('common.copy') }}</el-button>
        </div>
      </template>
      <pre class="ctx-pre">{{ dbDdlText }}</pre>
    </el-dialog>

    <!-- 数据库转储弹窗（逻辑在 DbDumpDialog.vue，打开时自动拉取对象列表） -->
    <DbDumpDialog v-model="dbDumpVisible" :conn="conn" :database="dbCtxTarget" />

    <!-- 在数据库中查找弹窗（逻辑在 DbSearchDialog.vue，打开时自动清空上次关键字与结果） -->
    <DbSearchDialog v-model="dbSearchVisible" :conn="conn" :database="dbCtxTarget" @opentable="openTable" />

    <!-- 运行 SQL 文件弹窗（逻辑在 RunSqlFileDialog.vue，执行成功后回调刷新库节点） -->
    <RunSqlFileDialog v-model="dbRunSqlVisible" :conn="conn" :database="dbCtxTarget" @done="onRunSqlFileDone" />

    <!-- 树右键菜单（folder/conn/blank/db/category） -->
    <ul v-if="treeCtxMenu.show"
        class="tree-ctx-menu"
        :style="{ left: treeCtxMenu.x + 'px', top: treeCtxMenu.y + 'px' }"
        @click.stop="closeTreeCtxMenu">
      <!-- 文件夹上 -->
      <template v-if="treeCtxMenu.kind === 'env-folder'">
        <li class="ctx-item" @click="treeCtxNewConn(treeCtxMenu.env)">
          <el-icon><Plus /></el-icon>{{ $t('tree.menu.newConnection') }}
        </li>
        <li class="ctx-item" @click="treeCtxNewFolder(treeCtxMenu.env)">
          <el-icon><FolderAdd /></el-icon>{{ $t('tree.menu.newFolder') }}
        </li>
        <li class="ctx-item" @click="startRenameGroup(treeCtxMenu.env)">
          <el-icon><Edit /></el-icon>{{ $t('tree.menu.renameFolder') }}
        </li>
        <li class="ctx-item danger" @click="treeCtxDeleteFolder(treeCtxMenu.env)">
          <el-icon><Delete /></el-icon>{{ $t('tree.menu.deleteFolder') }}
        </li>
      </template>
      <!-- conn 节点上 -->
      <template v-else-if="treeCtxMenu.kind === 'conn'">
        <li v-if="!isConnOpen(treeCtxMenu.data.connId)" class="ctx-item" @click="treeCtxOpenConn(treeCtxMenu.data.connId)">
          <el-icon><Connection /></el-icon>{{ $t('tree.menu.openConn') }}
        </li>
        <li v-else class="ctx-item" @click="treeCtxCloseConn(treeCtxMenu.data.connId)">
          <el-icon><CircleClose /></el-icon>{{ $t('tree.menu.closeConn') }}
        </li>
        <li class="ctx-item" @click="treeCtxNewConn(treeCtxMenu.env)">
          <el-icon><Plus /></el-icon>{{ $t('tree.menu.newConnection') }}
        </li>
        <li class="divider" />
        <li class="ctx-item" @click="treeCtxEditConn(treeCtxMenu.data.connId)">
          <el-icon><Edit /></el-icon>{{ $t('tree.menu.editConn') }}
        </li>
        <li class="ctx-item" @click="treeCtxCopyConn(treeCtxMenu.data.connId)">
          <el-icon><CopyDocument /></el-icon>{{ $t('tree.menu.copyConn') }}
        </li>
        <!-- `treeCtxTestConn` 早就写好了，只是**从来没挂到菜单上** —— 于是「只想验一下
             连接是否还通」只能靠「打开连接」（顺带展开整棵树）。这里补上入口。 -->
        <li class="ctx-item" @click="treeCtxTestConn(treeCtxMenu.data.connId)">
          <el-icon><Connection /></el-icon>{{ $t('tree.menu.testConn') }}
        </li>
        <li class="divider" />
        <li class="ctx-item" @click="treeCtxNewQueryConn(treeCtxMenu.data.connId)">
          <el-icon><Document /></el-icon>{{ $t('tree.menu.newQuery') }}
        </li>
        <li v-if="!isNoSqlConn(treeCtxMenu.data.connId)" class="ctx-item" @click="treeCtxNewDb(treeCtxMenu.data.connId)">
          <el-icon><FolderAdd /></el-icon>{{ $t('tree.menu.newDb') }}
        </li>
        <li v-if="!isNoSqlConn(treeCtxMenu.data.connId)" class="ctx-item" @click="openBackupRestore('backup', treeCtxMenu.data.connId, '')">
          <el-icon><Download /></el-icon>{{ $t('tree.menu.backupDb') }}
        </li>
        <li v-if="!isNoSqlConn(treeCtxMenu.data.connId)" class="ctx-item" @click="openBackupRestore('restore', treeCtxMenu.data.connId, '')">
          <el-icon><Upload /></el-icon>{{ $t('tree.menu.restoreDb') }}
        </li>
        <!-- 发送到 AI：排在「备份 / 还原」之后、危险操作之前 ——
             它是"把这个对象交给 AI 分析"的入口，与上面那串工作流动作同类。 -->
        <li class="ctx-item" @click="sendNodeToAi(treeCtxMenu.data)">
          <el-icon><Cpu /></el-icon>{{ $t('menu.sendToAi') }}
        </li>
        <li class="divider" />
        <li class="ctx-item danger" @click="treeCtxDeleteConn(treeCtxMenu.data.connId)">
          <el-icon><Delete /></el-icon>{{ $t('tree.menu.deleteConn') }}
        </li>
        <li class="ctx-item" @click="treeCtxRefreshConn(treeCtxMenu.data.connId)">
          <el-icon><Refresh /></el-icon>{{ $t('common.refresh') }}
        </li>
      </template>
      <!-- 空白处 -->
      <template v-else-if="treeCtxMenu.kind === 'blank'">
        <li class="ctx-item" @click="treeCtxNewFolder('')">
          <el-icon><FolderAdd /></el-icon>{{ $t('tree.menu.newFolder') }}
        </li>
        <li class="ctx-item" @click="treeCtxNewConn('')">
          <el-icon><Plus /></el-icon>{{ $t('tree.menu.newConnection') }}
        </li>
        <li class="divider" />
        <li class="ctx-item" @click="importConns('')">
          <el-icon><Upload /></el-icon>{{ $t('tree.menu.importConn') }}
        </li>
        <li class="ctx-item" @click="exportConns('')">
          <el-icon><Download /></el-icon>{{ $t('tree.menu.exportConn') }}
        </li>
      </template>
      <!-- 数据库 / schema 节点 -->
      <template v-else-if="treeCtxMenu.kind === 'db' || treeCtxMenu.kind === 'schema'">
        <li class="ctx-item" @click="dbCtxToggleOpen()">
          <el-icon><FolderOpened /></el-icon>{{ isDbCurrentOpen ? $t('tree.menu.closeDb') : $t('tree.menu.openDb') }}
        </li>
        <li class="ctx-item" @click="dbCtxNewQuery()">
          <el-icon><Document /></el-icon>{{ $t('tree.menu.newQuery') }}
        </li>
        <li class="divider" />
        <template v-if="!isNoSqlConn(treeCtxMenu.data?.connId)">
          <li class="ctx-item" @click="dbCtxShowDdl()">
            <el-icon><DocumentCopy /></el-icon>{{ $t('tree.menu.showDbDdl') }}
          </li>
          <li class="ctx-item" @click="dbCtxRunSqlFile()">
            <el-icon><Upload /></el-icon>{{ $t('tree.menu.runSqlFile') }}
          </li>
          <li class="ctx-item" @click="dbCtxDumpSql()">
            <el-icon><Download /></el-icon>{{ $t('tree.menu.dumpSql') }}
          </li>
          <li class="ctx-item" @click="openBackupRestore('backup', treeCtxMenu.data?.connId, treeCtxMenu.data?.db || treeCtxMenu.data?.label)">
            <el-icon><Download /></el-icon>{{ $t('tree.menu.backupDb') }}
          </li>
          <li class="ctx-item" @click="openBackupRestore('restore', treeCtxMenu.data?.connId, treeCtxMenu.data?.db || treeCtxMenu.data?.label)">
            <el-icon><Upload /></el-icon>{{ $t('tree.menu.restoreDb') }}
          </li>
          <!-- 与「连接」菜单保持同一位置（备份 / 还原之后），两个菜单不打架 -->
          <li class="ctx-item" @click="sendNodeToAi(treeCtxMenu.data)">
            <el-icon><Cpu /></el-icon>{{ $t('menu.sendToAi') }}
          </li>
          <li class="divider" />
          <li class="ctx-item" @click="dbCtxSearch()">
            <el-icon><Search /></el-icon>{{ $t('tree.menu.searchInDb') }}
          </li>
          <li class="ctx-item" @click="dbCtxEr()">
            <el-icon><Connection /></el-icon>{{ $t('tree.menu.erDiagram') }}
          </li>
          <li class="divider" />
          <!-- 删除数据库：NoSQL 节点也走这套菜单，之前它写在这个 template **之外**，
               Redis 的库节点上同样会冒出这一项（点了必然失败）。移进来一起门控，
               再叠一层 supportsDDLExec：不能执行 DDL 的源不摆。 -->
          <li v-if="featureOf(treeCtxMenu.data?.connId, 'supportsDDLExec')" class="ctx-item danger" @click="dbCtxDrop()">
            <el-icon><Delete /></el-icon>{{ $t('tree.menu.dropDb') }}
          </li>
          <li class="divider" />
        </template>
        <li class="ctx-item" @click="dbCtxRefresh()">
          <el-icon><Refresh /></el-icon>{{ $t('common.refresh') }}
        </li>
      </template>
      <!-- catalog 节点（Doris）：只留 catalog 真正能做的事。
           ⚠️ 动作名用 `catalogCtx*` 前缀，别叫 `catRefresh` —— 那个名字已经被
           **分类目录（category）**那套占了（`cat` 指 category，不是 catalog），
           曾经因此直接编译失败：Identifier 'catRefresh' has already been declared。 -->
      <template v-else-if="treeCtxMenu.kind === 'catalog'">
        <li class="ctx-item" @click="catalogCtxNewQuery()">
          <el-icon><Document /></el-icon>{{ $t('tree.menu.newQuery') }}
        </li>
        <li class="divider" />
        <li class="ctx-item" @click="catalogCtxRefresh()">
          <el-icon><Refresh /></el-icon>{{ $t('common.refresh') }}
        </li>
        <!-- 内置 catalog（internal）装着用户真正的库与表，Doris 不允许删 —— 门控掉；
             外部 catalog（如 jdbc 映射的 mysql_216）删的只是「映射」，外部数据不受影响 -->
        <template v-if="String(treeCtxMenu.data?.catalog || '').toLowerCase() !== 'internal'">
          <li class="divider" />
          <li class="ctx-item danger" @click="catalogCtxDrop()">
            <el-icon><Delete /></el-icon>{{ $t('tree.menu.dropCatalog') }}
          </li>
        </template>
      </template>
      <!-- 分类目录（Tables/Views/Procedures/...） -->
      <template v-else-if="treeCtxMenu.kind === 'category'">
        <!-- 视图 / 存储过程·函数 / 触发器 / 事件 不再提供「新建」入口（保留表、脚本、用户） -->
        <li v-if="treeCtxMenu.cat === 'tables'" class="ctx-item" @click="catCreate('tables')">
          <el-icon><Plus /></el-icon>{{ $t('tree.menu.create') }}
        </li>
        <li v-else-if="treeCtxMenu.cat === 'scripts'" class="ctx-item" @click="catCreate('scripts')">
          <el-icon><Plus /></el-icon>{{ $t('tree.menu.create') }}
        </li>
        <!-- 用户「新建」入口已随「用户管理只读」一并下掉 -->
        <li class="divider" />
        <li class="ctx-item" @click="catRefresh(treeCtxMenu.db)">
          <el-icon><Refresh /></el-icon>{{ $t('common.refresh') }}
        </li>
      </template>
    </ul>
  </div>

  <!-- Tab 右键菜单 -->
  <ul v-if="tabCtxMenu.show" class="tab-ctx-menu"
      :style="{ left: tabCtxMenu.x + 'px', top: tabCtxMenu.y + 'px' }"
      @click.stop @contextmenu.prevent>
    <li class="ctx-item" @click="tabCtxClose('self')">
      <el-icon><Close /></el-icon><span>{{ $t('tab.closeSelf') }}</span>
    </li>
    <li class="ctx-item" @click="tabCtxClose('others')">
      <el-icon><CircleClose /></el-icon><span>{{ $t('tab.closeOthers') }}</span>
    </li>
    <li class="ctx-item" @click="tabCtxClose('left')">
      <el-icon><ArrowLeft /></el-icon><span>{{ $t('tab.closeLeft') }}</span>
    </li>
    <li class="ctx-item" @click="tabCtxClose('right')">
      <el-icon><ArrowRight /></el-icon><span>{{ $t('tab.closeRight') }}</span>
    </li>
    <li class="divider" />
    <li class="ctx-item" @click="tabCtxClose('all')">
      <el-icon><Delete /></el-icon><span>{{ $t('tab.closeAll') }}</span>
    </li>
  </ul>

  <!-- 可视化新建/编辑对象对话框（按数据库方言渲染，覆盖表/视图/索引/存储过程/触发器/事件） -->
  <ObjectFormDialog
    v-if="conn"
    v-model="objForm.visible"
    :conn="conn"
    :database="objFormDb || currentDb"
    :cat="objForm.cat"
    :mode="objForm.mode"
    :object="objForm.object"
    @saved="onObjFormSaved"
  />

  <!-- 树 CRUD 弹窗：新建库 / 新建分组 / 重命名分组 / 重命名表（逻辑在各子组件中，父只保留开关 / 上下文 / 刷新回调） -->
  <NewDbDialog v-model="newDbVisible" :conn-id="newDbCtx.connId" :conn-type="newDbCtx.connType" @created="onNewDbCreated" />
  <FolderDialogs v-model:new-open="folderCreateVisible" v-model:rename-open="folderRenameVisible"
                 :env="folderEnv" :connections="allConnections"
                 @folder-created="onFolderCreated" @folder-renamed="onFolderRenamed" />
  <RenameTableDialog v-model="renameTableVisible" :ctx="renameTableCtx" @renamed="onTableRenamed" />
  <RenameObjectDialog v-model="renameObjectVisible" :ctx="renameObjectCtx" @renamed="onObjectRenamed" />

  <BackupRestoreDialog
    v-if="backupRestore.connectionId"
    :key="backupRestore.connectionId + '|' + backupRestore.mode + '|' + backupRestore.database"
    v-model="backupRestore.visible"
    :mode="backupRestore.mode"
    :connection-id="backupRestore.connectionId"
    :database="backupRestore.database"
  />

  <SettingsView v-model="settingsOpen" :initial-tab="settingsTab" />

  <!-- 检查更新：Release 说明按 Markdown 渲染（下载表格 / 代码块不再是原始文本） -->
  <!-- 检查更新：Release 说明按 Markdown 渲染（下载表格 / 代码块不再是原始文本）。
       高度**自适应内容、但不超过 76vh**：内容少就矮着，内容多在 body 内部滚动，
       不会把弹窗撑成整屏。高度/滚动用内联样式（el-dialog 是 teleport 渲染的，
       scoped 里的 :deep 规则不一定命中，内联最稳） -->
  <el-dialog
    v-model="updateVisible"
    class="upd-dialog"
    width="560px"
    align-center
    :title="`${t('update.title')} · v${updateInfo && updateInfo.latest}`"
    :close-on-click-modal="false"
    :style="{ maxHeight: '76vh', display: 'flex', flexDirection: 'column' }"
    :body-style="{ flex: '1 1 auto', minHeight: '0', overflowY: 'auto', padding: '4px 20px 10px' }"
    @close="closeUpdateDialog(false)"
  >
    <div v-if="updateInfo" class="upd-body">
      <p class="upd-lead">{{ t('update.found', { v: updateInfo.latest, cur: updateInfo.current }) }}</p>
      <!-- eslint-disable-next-line vue/no-v-html -- 说明来自自家 Release，渲染器已转义 -->
      <div class="upd-notes markdown-body" v-html="updateInfo.notesHtml"></div>
      <a v-if="updateInfo.truncated" class="upd-more" :href="updateInfo.releaseUrl"
         target="_blank" rel="noopener">{{ t('update.moreNotes') }}</a>
    </div>
    <template #footer>
      <div class="upd-footer">
        <!-- 「不再提示此版本」只在自动弹出时出现；手动点图标检查时不显示。
             右上角 × / ESC 只是关闭（不记 skip，下次有新版照常提示） -->
        <el-button v-if="updateInfo && updateInfo.isAuto" size="small" @click="closeUpdateDialog(true)">{{ t('update.neverAgain') }}</el-button>
        <el-button size="small" @click="gotoDownload">{{ t('update.goto') }}</el-button>
        <el-button size="small" type="primary" @click="doApplyUpdate">{{ t('update.apply') }}</el-button>
      </div>
    </template>
  </el-dialog>

  <!-- 下载进度：**只在后台下载真的在跑时才展示**（关掉窗口不会中断下载） -->
  <el-dialog
    v-model="dlVisible"
    class="upd-dl"
    width="420px"
    :title="t('update.downloading')"
    :show-close="dlStatus === 'running'"
    :close-on-click-modal="false"
  >
    <div class="upd-dl-body">
      <el-progress
        :percentage="dlTotal > 0 ? Math.min(100, Math.round((dlReceived / dlTotal) * 100)) : 0"
        :status="dlStatus === 'failed' ? 'exception' : dlStatus === 'done' ? 'success' : undefined"
        :stroke-width="10"
      />
      <div class="upd-dl-meta">
        <span v-if="dlStatus === 'running'">{{ t('update.dlProgress', { done: fmtBytes(dlReceived), total: fmtBytes(dlTotal), speed: fmtBytes(dlSpeed) }) }}</span>
        <span v-else-if="dlStatus === 'done'">{{ t('update.dlDone') }}</span>
        <span v-else class="upd-dl-err">{{ dlError }}</span>
      </div>
      <!-- 让"闷"变透明：从哪个源下、走没走代理、还要多久（只给速度，用户只能干等） -->
      <div v-if="dlStatus === 'running'" class="upd-dl-meta">{{ dlSourceText }}</div>
      <!-- 过程说明（换源 / 续传 / 第几次重试）：只在下载进行中才有意义 -->
      <div v-if="dlStatus === 'running' && dlNote" class="upd-dl-tip">{{ dlNote }}</div>
      <div v-if="dlStatus === 'done'" class="upd-dl-tip">{{ t('update.applyDone') }}</div>
      <div v-if="dlStatus === 'done' && dlVerified" class="upd-dl-tip">{{ t('update.dlVerified') }}</div>
      <div v-if="dlStatus === 'done' && dlUnverified" class="upd-dl-tip">{{ t('update.dlUnverified') }}</div>
    </div>
    <template #footer>
      <!-- 失败必须能**原地重试**：以前失败后只能重启软件，点更新图标永远弹回同一个报错页 -->
      <template v-if="dlStatus === 'failed'">
        <el-button size="small" @click="closeDownloadDialog">{{ t('update.dlClose') }}</el-button>
        <el-button v-if="dlUrl" size="small" @click="copyText(dlUrl, t('common.copied'))">{{ t('update.dlCopyLink') }}</el-button>
        <el-button size="small" @click="openDownloadDir">{{ t('update.dlOpenDir') }}</el-button>
        <el-button size="small" type="primary" @click="retryDownload">{{ t('update.dlRetry') }}</el-button>
      </template>
      <template v-else-if="dlStatus !== 'running'">
        <el-button size="small" @click="openDownloadDir">{{ t('update.dlOpenDir') }}</el-button>
        <el-button size="small" type="primary" @click="closeDownloadDialog">{{ t('common.confirm') }}</el-button>
      </template>
      <template v-else>
        <!-- 代理慢于镜像时（实测：本机代理 ~143KB/s、镜像 ~220KB/s），给用户一条主动换源的路 -->
        <el-button size="small" :loading="dlSwitching" @click="switchSource">{{ t('update.dlSwitchSource') }}</el-button>
        <el-button size="small" type="primary" @click="closeDownloadDialog">{{ t('update.dlBackground') }}</el-button>
      </template>
    </template>
  </el-dialog>

  <!-- 数据治理（敏感数据 / 质量 / 关系 / 容量 / 变更影响 / 索引建议） -->
  <AiGovernanceDialog v-model="governanceOpen" :conn="conn" :database="currentDb"
                      :initial-tab="governanceTab" :auto-run="governanceAutoRun" />

  <!-- 命令面板（Ctrl/⌘ + K）：自然语言 → 操作计划 -->
  <CommandPalette v-model="paletteOpen" :conn="conn" :database="currentDb" @run-plan="onRunPlan" />

  <!-- 危险操作进度：删除 / 清空 / 截断 / 批量删除。
       秒级操作不会看到它（延迟 400ms 才弹）；真慢的时候才有计时 + 逐条日志。
       `cancellable` 只对「批量删除连接」为真 —— 那个循环在前端，取消是真的停；
       单个 DDL 已经交给数据库执行了，客户端停不下来，索性不给取消按钮。 -->
  <TaskProgressDialog
    v-model:visible="dangerVisible"
    :title="dangerTitle"
    :target-name="dangerTargetName"
    :status="dangerStatus"
    :total="-1"
    :message="dangerMessage"
    :logs="dangerLogs"
    :show-logs="true"
    :cancellable="dangerCancellable"
    :canceling="false"
    @cancel="dangerCancel"
    @close="dangerVisible = false"
  />

  <!-- 导出任务进度弹窗：显示真实进度、实时日志、取消、完成下载 -->
  <TaskProgressDialog
    v-model:visible="exportProgressVisible"
    task-kind="export"
    :target-name="exportTargetName || $t('tree.dataName')"
    :status="exportStatus"
    :done="exportDone"
    :total="exportTotal"
    :phase="exportPhase"
    :message="exportMessage"
    :logs="exportLogs"
    :canceling="exportCanceling"
    @cancel="cancelExportTask"
    @close="onExportProgressClose"
  />
</template>

<script setup>
import { ref, computed, onMounted, onBeforeUnmount, watch, nextTick, defineAsyncComponent } from 'vue'

import { applyTaskSnapshot, finishExport } from '../../utils/useExportTask'
import { useRoute, useRouter } from 'vue-router'
import { ElMessage, ElMessageBox, ElLoading } from 'element-plus'
import { renderMarkdown } from '../../utils/markdown'
import { errMsg } from '../../utils/errMsg'
import { PREDEF_ENVS, ENV_ORDER_BASE, envLabel, envColor, envShort, envTitle } from '../../utils/envs'
import { desktopShell } from '../../utils/desktopShell'
import { readSchemaCache, writeSchemaCache, removeSchemaCache, invalidateSchemaCache } from '../../utils/schemaCache'
import { parseMySqlGrants, parseSqlServerPerms } from '../../utils/grants'
import { getEditorSettings } from '../../utils/settings'
import { topMenuVisible } from '../../utils/topMenu'
import { Sunny, Moon, Clock, Reading, Delete as ElDelete } from '@element-plus/icons-vue'
import { getThemeSettings, saveThemeSettings, applyTheme, getResolvedTheme, onResolvedThemeChange } from '../../utils/theme'
import { locale, setLocale } from '../../utils/i18n'
import { formatSql as smartFormatSql, connDialectOf } from '../../utils/sqlFormat'
import { buildCatChildren, buildObjectCategories, isFunctionRoutine } from './treeNodes'
import { t } from '../../utils/i18n'
import { bgTasks, removeBgTask, clearBgTasks, setBgTaskStatus } from '../sync/backgroundTasks'
import { Coin, Refresh, Setting, SetUp, MagicStick, Plus, Folder, FolderAdd, Grid, View, Mouse, Switch, Promotion, Search, Operation, BellFilled, Timer, DocumentRemove, Document, DocumentCopy, Download, Upload, ArrowRight, ArrowLeft, ArrowUp, Expand, Fold, Edit, Delete, Connection, FolderOpened, CopyDocument, EditPen, Cpu, CircleClose, Close, SwitchButton, Loading, User, CollectionTag, Collection, ArrowDown, DataAnalysis, Monitor } from '@element-plus/icons-vue'
import { listConnections, listCatalogs, listDatabases, listSchemas, listTables, listColumns, listIndexes, getTableCounts, disconnectSessions, disconnectDatabase, noSqlDatabases, syncTaskStatus, noSqlCollections, noSqlDeleteCollection, listProcedures, listTriggers, listEvents, listUsers, getUserInfo, userAction, getTableDdl, getObjectInfo, exportData, exportStart, exportTask, exportCancel, exportDownload, testConnectionById, deleteConnection, tableAction, getFeatures, alterTable, executeSql, copyConnection, saveConnection, aiNl2sql, aiExplain, aiInsight, aiDataDict, listHistory, clearHistory } from '../../api'
import TaskProgressDialog from '../../common/TaskProgressDialog.vue'
import { compareTaskStatus, compareCancel, syncCancel } from '../../api'
import { recordRecentConnection } from '../../utils/recentConnections'
import { pureFolders, loadPureFolders, addPureFolder, renamePureFolder, removePureFolder, savePureFolders } from '../../utils/folders'
import { buildConnectionBundle, parseConnectionBundle, importPayloadOf, downloadJson } from '../../utils/connTransfer'
import { isNoSql as isNoSqlType, labelOf, quoteStyleOf, schemaLevelOf, byType } from '../../types'
import AiPanel from '../ai/AiPanel.vue'
import logoSmUrl from '../../assets/logo-sm.png'
import SaveToKbDialog from '../knowledge/SaveToKbDialog.vue'
import DbLogo from '../../common/DbLogo.vue'

// ===== 按需加载的视图与弹窗 =====
// 下面这些组件只有「打开某个 tab」或「点开某个弹窗」之后才会用到，静态 import 会把它们全部打进首屏 index chunk，
// 启动时就要多下载、多解析、多编译几百 KB。改成 defineAsyncComponent 后由 Vite 各自拆成独立 chunk，用到时才拉取
// （本地服务，首次点击的加载几乎无感）。
// 例外：AiPanel 用 v-show 常驻（要保留内部对话状态）、DbLogo 只有几 KB，这两类保持静态导入。
// 注意：新增的 tab/弹窗组件请沿用这种写法，避免首屏 chunk 再次膨胀。
const TableDataView = defineAsyncComponent(() => import('./TableDataView.vue'))
const ErDiagramView = defineAsyncComponent(() => import('./ErDiagramView.vue'))
const SqlQueryView = defineAsyncComponent(() => import('./SqlQueryView.vue'))
const TableDetailView = defineAsyncComponent(() => import('./TableDetailView.vue'))
const NoSqlDataView = defineAsyncComponent(() => import('./NoSqlDataView.vue'))
const ObjectDetailView = defineAsyncComponent(() => import('./ObjectDetailView.vue'))
const UserDetailView = defineAsyncComponent(() => import('./UserDetailView.vue'))
const UserForm = defineAsyncComponent(() => import('./UserForm.vue'))
const ObjectFormTab = defineAsyncComponent(() => import('./ObjectFormTab.vue'))
const DbDumpDialog = defineAsyncComponent(() => import('./DbDumpDialog.vue'))
const DbSearchDialog = defineAsyncComponent(() => import('./DbSearchDialog.vue'))
const RunSqlFileDialog = defineAsyncComponent(() => import('./RunSqlFileDialog.vue'))
const ImportDataDialog = defineAsyncComponent(() => import('./ImportDataDialog.vue'))
const DataGenDialog = defineAsyncComponent(() => import('./DataGenDialog.vue'))
const NewDbDialog = defineAsyncComponent(() => import('./NewDbDialog.vue'))
const FolderDialogs = defineAsyncComponent(() => import('./FolderDialogs.vue'))
const RenameTableDialog = defineAsyncComponent(() => import('./RenameTableDialog.vue'))
const RenameObjectDialog = defineAsyncComponent(() => import('./RenameObjectDialog.vue'))
const BackupRestoreDialog = defineAsyncComponent(() => import('./BackupRestoreDialog.vue'))
const CompareDialog = defineAsyncComponent(() => import('../compare/CompareDialog.vue'))
const SyncDialog = defineAsyncComponent(() => import('../sync/SyncDialog.vue'))
const ConnectionDialog = defineAsyncComponent(() => import('../connection/ConnectionDialog.vue'))
const DataSourcePicker = defineAsyncComponent(() => import('../../common/DataSourcePicker.vue'))
const ObjectFormDialog = defineAsyncComponent(() => import('../../common/objectforms/ObjectFormDialog.vue'))
const SettingsView = defineAsyncComponent(() => import('../settings/SettingsView.vue'))
import {
  checkUpdate as checkUpdateApi, applyUpdate as applyUpdateApi, updateProgress as updateProgressApi,
  cancelUpdate as cancelUpdateApi, dismissUpdate as dismissUpdateApi, openLocalDir as openLocalDirApi,
  updateDirs as updateDirsApi, pickUpdateDir as pickUpdateDirApi
} from '../../api'

// ===== 检查更新：与 GitHub 最新 Release 比对；有新版可在线下载安装 =====
// 说明正文按 Markdown 渲染（Release body 里带下载表格），不再显示原始文本
const updateChecking = ref(false)
const updateInfo = ref(null)   // { latest, current, notesHtml, releaseUrl }
const updateVisible = ref(false)
// 下载保存位置：点「在线更新」后弹窗让用户选，选完才开始下载。
// 目录里不做自动清理 —— 文件留在用户自己找得到的地方，留不留他说了算。
const DL_DIR_LS = 'dbmind_update_dir'
const dlDir = ref(localStorage.getItem(DL_DIR_LS) || '')
const openDownloadDir = async () => {
  try {
    await openLocalDirApi(dlDir.value || null)
  } catch {
    ElMessage.warning(t('update.dlDirOpenFail'))
  }
}
// 下载进度（后台任务在跑，前端只轮询展示）
const fmtBytes = (n) => {
  const v = Number(n) || 0
  if (v < 1024) return `${v} B`
  if (v < 1024 * 1024) return `${(v / 1024).toFixed(0)} KB`
  return `${(v / 1024 / 1024).toFixed(1)} MB`
}
const dlVisible = ref(false)
// 初值必须是 idle：以前写成 'running'，于是**还没开始下载**时顶栏图标的提示就已经是
// 「查看下载进度」，点开却是个空进度 —— 用户一眼就看出不对。
const dlStatus = ref('idle') // idle | running | done | failed
const dlReceived = ref(0)
const dlTotal = ref(0)
const dlSpeed = ref(0)
const dlError = ref('')
// 过程说明（换源 / 续传 / 第几次重试），来自后端 note —— 不是错误，画在进度条下方
const dlNote = ref('')
const dlUrl = ref('')
// 下载完成后：是否用 GitHub 官方 sha256 校验过；走了镜像又没官方值时可自证性为 0，界面必须说清
const dlVerified = ref(false)
const dlUnverified = ref(false)
// 当前是不是走代理（后端探测结果）+ 换源请求进行中
const dlProxy = ref('')
const dlSwitching = ref(false)

// 剩余时间：有总大小与速度才算得出来，否则留空（不要瞎猜）
const dlEtaText = computed(() => {
  const rest = (Number(dlTotal.value) || 0) - (Number(dlReceived.value) || 0)
  const sp = Number(dlSpeed.value) || 0
  if (rest <= 0 || sp <= 0) return ''
  const s = Math.round(rest / sp)
  return Math.floor(s / 60) + ':' + String(s % 60).padStart(2, '0')
})
// 一行讲清"从哪下、走不走代理、还要多久"
const dlSourceText = computed(() => {
  let host = ''
  try { host = new URL(dlUrl.value).host } catch { host = '' }
  const via = dlProxy.value
    ? t('update.dlViaProxy', { proxy: dlProxy.value })
    : t('update.dlDirect')
  const eta = dlEtaText.value ? ' · ' + t('update.dlEta', { time: dlEtaText.value }) : ''
  return (host ? host + ' · ' : '') + via + eta
})

// 弹窗只放「更新摘要」：Release 全文动辄几千字，整篇塞进弹窗会把弹窗撑满屏、
// 还要滚动才看得到按钮。这里截取开头一小段 + 给「查看完整更新说明」链接。
const UPDATE_NOTES_LIMIT = 320
const showUpdateDialog = (r, isAuto) => {
  const raw = String(r.notes || '')
  let cut = raw.slice(0, UPDATE_NOTES_LIMIT)
  // 切在段落边界（空行），避免半句话 / 半截表格
  const br = cut.lastIndexOf('\n\n')
  if (br > 120) cut = cut.slice(0, br)
  const notes = cut.trim()
  updateInfo.value = {
    latest: r.latest,
    current: r.current,
    notesHtml: renderMarkdown(notes),
    truncated: raw.length > notes.length,
    // 只有「自动弹出」才给「不再提示此版本」；手动点图标检查的用户本来就是主动想看
    isAuto: !!isAuto,
    releaseUrl: r.releaseUrl || `https://github.com/rick-works/dbmind/releases/tag/v${r.latest}`
  }

  updateVisible.value = true
}

// 关闭弹窗：neverAgain = true 时记住「不再提示此版本」
const closeUpdateDialog = (neverAgain) => {
  if (neverAgain && updateInfo.value) {
    try { localStorage.setItem(UPDATE_SKIP_LS, updateInfo.value.latest) } catch { }
  }
  updateVisible.value = false
}

// 点「在线更新」：先让用户选保存位置，选完才开始下载
const doApplyUpdate = async () => {
  updateVisible.value = false
  let picked
  try {
    picked = await pickUpdateDirApi(t('update.dlPickTitle'))
  } catch (e) {
    ElMessage.error((e && e.message) || t('update.dlDirOpenFail'))
    return
  }
  if (!picked || !picked.success) {
    ElMessage.error((picked && picked.message) || t('update.dlDirOpenFail'))
    return
  }
  if (!picked.dir) return   // 用户在系统对话框里点了取消，不作处理
  const dir = String(picked.dir).trim()
  try { localStorage.setItem(DL_DIR_LS, dir) } catch { /* 隐私模式，忽略 */ }
  await startDownload(dir)
}

// 失败后原地重试：沿用上次选的目录（没选过就交给后端用系统临时目录）
const retryDownload = () => { startDownload(dlDir.value || '') }

// 换个更快的源：先取消当前下载（已下载的 .part 保留），再让后端镜像优先重下 —— 会接着下
const switchSource = async () => {
  if (dlSwitching.value) return
  dlSwitching.value = true
  dlNote.value = t('update.dlSwitching')
  try { await cancelUpdateApi() } catch { /* 取消失败也让后面的重试照常进行 */ }
  // 等后台真的停下来（最多 20 秒），否则后端会以"任务已在进行中"拒绝新的下载
  for (let i = 0; i < 40; i++) {
    await new Promise((r) => setTimeout(r, 500))
    try {
      const p = await updateProgressApi()
      if (!p || p.status !== 'running') break
    } catch { break }
  }
  stopProgressPolling()
  await startDownload(dlDir.value || '', true)
}

// 真正开始下载（目录已定）
const startDownload = async (dir, preferMirror = false) => {
  dlVisible.value = true
  dlStatus.value = 'running'
  dlReceived.value = 0
  dlTotal.value = 0
  dlSpeed.value = 0
  dlError.value = ''
  dlNote.value = ''
  dlUrl.value = ''
  dlVerified.value = false
  dlUnverified.value = false
  dlSwitching.value = false
  startProgressPolling()
  try {
    const res = await applyUpdateApi(dir, preferMirror)
    if (!res || res.success === false) {
      dlStatus.value = 'failed'
      dlError.value = (res && res.message) || t('update.applyFail')
    }
  } catch (e) {
    dlStatus.value = 'failed'
    dlError.value = (e && e.message) || t('update.applyFail')
  }
}

// 轮询下载进度：弹窗打开时500ms（进度条要顺），关掉后3s —— 只为让顶栏图标
// 保持「有任务在跑」的状态，随时能再调出来
let dlTimer = null
const startProgressPolling = (fast = true) => {
  stopProgressPolling()
  const tick = async () => {
    try {
      const p = await updateProgressApi()
      dlReceived.value = p.received || 0
      dlTotal.value = p.total || 0
      dlSpeed.value = p.speed || 0
      dlNote.value = p.note || ''
      dlUrl.value = p.url || ''
      dlVerified.value = !!p.verified
      dlUnverified.value = !!p.unverified
      dlProxy.value = p.proxy || ''
      if (p.status === 'done' || p.status === 'failed') {
        dlStatus.value = p.status
        if (p.status === 'failed') dlError.value = p.error || t('update.applyFail')
        dlNote.value = ''
        stopProgressPolling()
      } else {
        dlStatus.value = p.status || 'running'
      }
    } catch {
      // 单次轮询失败不打断（网络抖动），下一拍继续
    }
  }
  dlTimer = setInterval(tick, fast ? 500 : 3000)
}
const stopProgressPolling = () => {
  if (dlTimer) { clearInterval(dlTimer); dlTimer = null }
}
const closeDownloadDialog = () => {
  // 只是隐藏进度窗，不取消后台下载：继续低频轮询，顶栏图标保持可点（随时调出来）
  dlVisible.value = false
  if (dlStatus.value === 'running') {
    startProgressPolling(false)
    ElMessage.info(t('update.dlBackgroundHint'))
    return
  }
  // 已结束（成功/失败）就把后端那份终态也清掉：留着的话下次点图标会被它拦住，
  // 又变回「只能重启软件」（真机踩过）
  try { dismissUpdateApi() } catch { /* 忽略 */ }
}


const gotoDownload = () => {
  const url = updateInfo.value && updateInfo.value.releaseUrl
  updateVisible.value = false
  if (url) window.open(url, '_blank')
}

const checkUpdate = async () => {
  if (updateChecking.value) return
  updateChecking.value = true
  try {
    // 后台已有下载任务在跑：直接把进度窗调出来（这就是「上次关掉了，从哪再找到」）
    try {
      const p = await updateProgressApi()
      if (p && p.status === 'running') {
        dlVisible.value = true
        dlStatus.value = 'running'
        dlReceived.value = p.received || 0
        dlTotal.value = p.total || 0
        dlSpeed.value = p.speed || 0
        dlNote.value = p.note || ''
        dlUrl.value = p.url || ''
        dlVerified.value = !!p.verified
        dlUnverified.value = !!p.unverified
        dlProxy.value = p.proxy || ''
        startProgressPolling()
        return
      }
      // 已结束的任务（成功/失败）**不该拦住下一次检查**：以前失败后点图标永远弹回同一个
      // 报错页，得重启软件才行。这里先把终态清掉，再走正常的检查流程。
      if (p && (p.status === 'done' || p.status === 'failed')) {
        try { await dismissUpdateApi() } catch { /* 清不掉也不挡检查 */ }
      }
    } catch { /* 进度接口不可用就走正常检查 */ }

    const r = await checkUpdateApi()
    if (!r || r.success === false) { ElMessage.error((r && r.message) || t('update.checkFail')); return }
    if (!r.hasNew) { ElMessage.success(t('update.upToDate', { v: r.current })); return }
    showUpdateDialog(r, false)   // 手动检查：不显示「不再提示此版本」
  } catch {
    ElMessage.error(t('update.checkFail'))
  } finally {
    updateChecking.value = false
  }
}

// ===== 启动自动检测新版本：延迟后静默检查；有新版才弹窗 =====
// 三态按钮：立即更新 / 暂不更新 / 不再提示此版本（右上角 ×）
const UPDATE_SKIP_LS = 'dbmind_update_skip'
// 服务可能比页面晚就绪（首次启动要初始化数据库/驱动），失败后重试一次
onMounted(() => {
  setTimeout(autoCheckUpdate, 3000)
  // 启动时也看一眼有没有正在跑的下载任务：有就让顶栏图标进入「查看进度」状态
  setTimeout(async () => {
    try {
      const p = await updateProgressApi()
      if (p && p.status === 'running') {
        dlStatus.value = 'running'
        dlReceived.value = p.received || 0
        dlTotal.value = p.total || 0
        startProgressPolling(false)
      }
    } catch { }
  }, 1500)
})
const autoCheckUpdate = async () => {
  try {
    const r = await checkUpdateApi()
    if (!r || r.success === false) throw new Error('check failed')
    if (!r.hasNew) return
    // 用户对同一版本点过「不再提示」就静默
    let skipped = ''
    try { skipped = localStorage.getItem(UPDATE_SKIP_LS) || '' } catch { }
    if (skipped === r.latest) return
    showUpdateDialog(r, true)    // 自动弹出：给「不再提示此版本」
  } catch {
    // 首次启动时后端可能还没就绪 —— 8 秒后再试一次，仍失败才静默
    setTimeout(autoCheckUpdate, 8000)
  }
}
const AiGovernanceDialog = defineAsyncComponent(() => import('../ai/AiGovernanceDialog.vue'))
const MonitorPanel = defineAsyncComponent(() => import('./MonitorStudio.vue'))  // 监控工作台（tab 全屏版）
const KnowledgeStudio = defineAsyncComponent(() => import('../knowledge/KnowledgeStudio.vue'))
const CommandPalette = defineAsyncComponent(() => import('../ai/CommandPalette.vue'))

const route = useRoute()
const router = useRouter()
const conn = ref(null)
const allConnections = ref([])
// 按 tab 归属的连接 ID 解析连接对象；找不到时回退到当前选中连接。
// 用于跨数据源打开表结构/数据 tab：右键所属连接可能不是当前 conn，直接拿 conn 会导致串台或 null 报错
/** 页签对应连接是否生产环境（PROD）——页签上挂红 P 标防误连 */
const isProdTab = (tab) => String((connFor(tab.connId) || {}).env || '').toUpperCase() === 'PROD'
const connFor = (connId) => {
  if (connId == null) return conn.value
  const sid = String(connId)
  const c = allConnections.value.find(x => String(x.id) === sid)
  return c || conn.value
}
const connDialogVisible = ref(false)
const editingConn = ref(null)
const testing = ref(false)
const dbs = ref([])
// 新建数据源：先弹选择器
const dsPickerVisible = ref(false)
const dsPickerEnv = ref('')
// 当前选中的连接 ID（与 conn.value.id 保持一致，独立出来便于树构建）
const currentConnId = ref(null)
const currentDb = ref('')
const treeData = ref([])
const treeRef = ref(null)
const treeLoading = ref(false)
const filterText = ref('')
const defaultExpanded = ref([])
const connErrorSet = ref(new Set()) // 记录已报过错的连接，避免重复弹窗
const connLoadingSet = ref(new Set()) // 记录正在连接中的连接，防止重复点击与错误状态
const tabs = ref([])
const activeTab = ref('')
// 打开或激活 tab：不存在则追加，已存在则仅激活（id 里带连接，跨数据源同名对象互不干扰；
// 跨库同名表的结构/数据页有特殊更新逻辑，保留在 openTable/openDetail 内）
const openTab = (tab) => {
  // 页签自带连接：没显式给就跟上「打开时的当前连接」，
  // 这样之后切到别的连接，这个页签依旧指向自己的数据源
  if (tab.connId == null && currentConnId.value) tab.connId = String(currentConnId.value)
  if (!tabs.value.find(t => t.id === tab.id)) tabs.value.push(tab)
  activeTab.value = tab.id
  // 常驻页签（AI 助手 / 知识库）：记住「处于打开状态」与位置
  if (isGlobalTab(tab)) {
    globalTabState.value = {
      ...globalTabState.value,
      [tab.id]: { open: true, index: tabs.value.findIndex(t => t.id === tab.id) }
    }
  }
}

// ===== 知识库工作台（顶部导航入口）=====
// 团队资料是全局资源、与「当前连接」无关：点入口打开页签，已经开着就直接切过去，
// 不重复开第二个（两个知识库页签会各自维护状态，容易让人分不清在改哪个）。
const KNOWLEDGE_TAB_ID = 'knowledge:studio'
const knowledgeOpen = computed(() => {
  const t = tabs.value.find(x => x.id === activeTab.value)
  return !!t && t.type === 'knowledge'
})
/** 待聚焦的知识库：从别处跳过来时带到工作台，让它选中对应那个库（用掉后清空） */
const pendingKbFocus = ref('')
const openKnowledge = (kbId) => {
  if (kbId) pendingKbFocus.value = String(kbId)
  if (tabs.value.some(t => t.id === KNOWLEDGE_TAB_ID)) {
    activeTab.value = KNOWLEDGE_TAB_ID
    return
  }
  openTab({ id: KNOWLEDGE_TAB_ID, type: 'knowledge', label: t('mv.knowledge') })
}
// ===== 桌面壳：系统标题栏已由壳关闭，顶部这条顶栏同时充当标题栏 =====
// 桥的具体形态在 utils/desktopShell.js：它同时认 Tauri（现在的壳）与上游 Electron（历史壳）。
// 浏览器里两者都不存在 → available=false：不渲染窗口按钮，也不做任何拖动处理。
const winApi = desktopShell
const isDesktop = winApi.available
/** macOS 保留系统红绿灯按钮，不再自绘一组，避免出现两套窗口按钮 */
const isMacShell = isDesktop && winApi.platform === 'darwin'
const winMaximized = ref(false)
let offWinMax = null
const winMinimize = () => winApi && winApi.minimize()
const winToggleMax = () => winApi && winApi.toggleMaximize()
/**
 * 关闭窗口（右上角 ×）：有未保存的脚本页签时先问一遍。
 *
 * 口径与「关闭页签」（closeTab）**完全一致** —— 保存 / 不保存 / 取消 三态、同一套按钮文案，
 * 只是这里把**所有**未保存的页签一次性列出来问，而不是逐个弹（关窗口时弹 N 次没人受得了）。
 *
 * 局限（已知，写在注释里免得以后误以为万能）：只覆盖**脚本页签**（它们通过 dirty-change
 * 上报了 tab.dirty）；表格页里的数据改动（TableDataView 的"有未保存的修改"）目前没上报，
 * 检测不到。另外 Alt+F4 / 任务栏关闭会绕过这里（那是系统级的关闭请求）。
 */
const winClose = async () => {
  if (!winApi || !winApi.available) return
  // 后台任务检查（先于未保存脚本）：有同步任务在后台跑，直接关窗它们**不会中断**
  //（任务在后端进程里），但用户多半以为关窗=停止 —— 说清楚再走。
  await refreshBgStatus()
  const runningBg = bgTasks.filter(t => (bgStatusMap.value[t.id] || 'running') === 'running')
  if (runningBg.length) {
    const names = runningBg.map(bt => t('common.quoted', { name: bt.title })).join(t('common.listSep'))
    try {
      await ElMessageBox.confirm(t('sync.bgCloseAsk', { n: runningBg.length, names }), t('sync.bgCenter'), {
        type: 'warning',
        confirmButtonText: t('sync.bgCloseOk'),
        cancelButtonText: t('common.cancel'),
        closeOnClickModal: false
      })
    } catch {
      return // 取消关闭 → 窗口留着，任务继续
    }
  }
  const dirtyTabs = tabs.value.filter(t => t.type === 'sql' && t.dirty)
  if (dirtyTabs.length) {
    const names = dirtyTabs.map(tb => t('common.quoted', { name: tabLabel(tb) })).join(t('common.listSep'))
    const head = dirtyTabs.length === 1
      ? t('mv.oneDirty', { names })
      : t('mv.manyDirty', { n: dirtyTabs.length, names })
    let action = 'cancel'
    try {
      await ElMessageBox.confirm(t('mv.closeAsk', { head }), t('mv.unsavedTitle'), {
        type: 'warning',
        confirmButtonText: t('mv.saveAndClose'),
        cancelButtonText: t('mv.dontSave'),
        distinguishCancelAndClose: true,
        showClose: true,
        closeOnClickModal: false
      })
      action = 'save'
    } catch (e) {
      // cancel = 点「不保存」→ 弃用改动；close = 点 X / ESC → 取消关闭
      action = e === 'cancel' ? 'discard' : 'cancel'
    }
    if (action === 'cancel') return
    if (action === 'save') {
      // 逐个走页签那套保存（saveForClose 内部管命名与覆盖）；任何一个没保存成就别关，
      // 并把该页签切到前台 —— 否则用户只会看到"窗口没关"，却不知道是哪个脚本卡住了
      for (const t of dirtyTabs) {
        let ok = true
        try { ok = sqlViewRefs[t.id] ? await sqlViewRefs[t.id].saveForClose() : true } catch { ok = false }
        if (ok === false) {
          activeTab.value = t.id
          return
        }
      }
    }
  }
  // 事务未提交的页签：关窗 = 自动回滚（组件卸载钩子会兜底发 rollback）—— 列出来说清楚再走
  const txTabs = tabs.value.filter(t => t.type === 'sql' && sqlViewRefs[t.id]?.txOpenDirty?.())
  if (txTabs.length) {
    const names = txTabs.map(tb => t('common.quoted', { name: tabLabel(tb) })).join(t('common.listSep'))
    try {
      await ElMessageBox.confirm(t('mv.txCloseAsk', { names }), t('sqlq.txCloseTitle'), {
        type: 'warning',
        confirmButtonText: t('sqlq.txCloseRollback'),
        cancelButtonText: t('common.cancel'),
        closeOnClickModal: false
      })
    } catch { return }
  }
  winApi.close()
}
/** 双击顶栏空白处 = 最大化 / 还原（无边框窗口系统不再代劳；落在导航/按钮上时不动窗口） */
const onTopbarDblClick = (e) => {
  if (!isDesktop || isMacShell) return
  // 只排除**真正可点的那些**（导航项、窗口按钮）。
  // 曾经写成 `.top-nav, .win-acts` —— 自检显示顶栏中心点命中的正是 `.top-nav` 本身
  // （它比想象中宽得多），于是整条中间区域都被当成"可点项"跳过，双击与拖动双双失效。
  if (e.target instanceof Element && e.target.closest('.top-nav-item, .win-act')) return
  winToggleMax()
}

/**
 * 拖动顶栏的三段式：按下只记起点 → 移动超过阈值才真正开始拖 → 松手复位。
 *
 * 为什么不能"一按下就 startDragging"：那样系统会立刻接管鼠标，浏览器**再也合成不出
 * `dblclick`**，双击最大化就没了（实测如此）。原生标题栏也是"按下并移动"才算拖动，
 * 单击与双击因此都不受影响 —— 这里照同一个口径来。
 * 4px 阈值足够过滤手抖，又快到不会让人觉得"拖不动"。
 */
const topbarDrag = { x: 0, y: 0, armed: false, active: false }
const onTopbarMouseDown = (e) => {
  if (!isDesktop || isMacShell) return
  if (e.button !== 0) return          // 只认左键；右键留给上下文菜单
  if (e.target instanceof Element && e.target.closest('.top-nav-item, .win-act')) return
  topbarDrag.x = e.clientX
  topbarDrag.y = e.clientY
  topbarDrag.armed = true
  topbarDrag.active = false
}
const onTopbarMouseMove = (e) => {
  if (!topbarDrag.armed || topbarDrag.active) return
  if (Math.abs(e.clientX - topbarDrag.x) < 4 && Math.abs(e.clientY - topbarDrag.y) < 4) return
  topbarDrag.active = true
  winApi.startDragging()
}
const onTopbarMouseUp = () => { topbarDrag.armed = false }
/**
 * 按住顶栏空白处 = 拖动窗口。
 *
 * 为什么不用 Tauri 的 `data-tauri-drag-region` 单打独斗：那套靠壳注入的脚本判断
 * `e.target` 上有没有该属性 —— 我们的页面是**远程 URL**，注入脚本的行为不在自己手里。
 * 这里直接调壳的 `startDragging()`：与窗口按钮同一条通路（已验证可用），行不行一眼可知。
 *
 * ⚠️ 这个函数曾经"凭空消失"过一次（模板还在引用它，script 里却没有），
 * 表现就是**拖动完全没反应**、且不报任何可见错误。改动这一带时请顺手确认它还在。
 */
onMounted(() => {
  if (winApi && !isMacShell) offWinMax = winApi.onMaximizeChange((v) => { winMaximized.value = v })
  // 欢迎页的「最近查询」：首次进来就取一次（失败无所谓，那块会显示一句说明）
  loadHistory()
  // 纯分组从后端设置加载（并顺带完成 localStorage 旧数据的一次性迁移）
  loadPureFolders()
})
onBeforeUnmount(() => { if (offWinMax) offWinMax() })

const aiOpen = ref(false)
/**
 * 交给**中央 AI 工作区页签**的「待填入问题」载荷（`{ seq, text }`）。
 * 对象树右键「发送到 AI」用它把上下文送进面板 —— 走页签而不是右侧窄面板：
 * 「分析这张表」是要一块大地方看结果的场景，右侧那条窄栏留给 Ctrl/⌘+K 的"边看数据边问"。
 * 用自增 seq 而非文本做触发条件：同一张表连点两次也要每次都响应。
 */
const pendingAi = ref(null)
/** AI 助手组件引用：供快捷键直接切到某个能力页签 */
const aiPanelRef = ref(null)

/**
 * 一键展开：只展开**已经加载过**的节点。
 *
 * lazy 模式下展开未加载节点会触发一次远程加载 —— 那等于"点一下就把所有连接都连一遍"，
 * 不是"展开"该有的代价。所以这里只逐层展开已加载的：环境分组立刻展开，连接/库要用户自己点
 * （点一次再按一次，可以继续往里展开一层）。
 */
const expandTreeOneLevel = () => {
  const store = treeRef.value?.store
  if (!store || !store.nodesMap) return
  Object.keys(store.nodesMap).forEach(key => {
    const node = store.nodesMap[key]
    if (node && !node.expanded && node.loaded && node.childNodes && node.childNodes.length) node.expand()
  })
}
/** 一键收缩：全部收起。收起**不会触发任何加载**，所以可以放心一次收干净。 */
const collapseTreeAll = () => {
  const store = treeRef.value?.store
  if (!store || !store.nodesMap) return
  Object.keys(store.nodesMap).forEach(key => {
    const node = store.nodesMap[key]
    if (node && node.expanded) node.collapse()
  })
}
const treeWidth = ref(360)
const sidebarHidden = ref(false)

// ===== 顶栏菜单：显示哪些 + 顺序，在「设置 → 通用」里配置（utils/topMenu.js 共享状态）=====
const topMenuRun = (id) => {
  if (id === 'compare') openCompare()
  else if (id === 'sync') openSync()
  else if (id === 'governance') governanceOpen.value = true
  // 驱动管理：直接打开设置的驱动页签（openSettings 的第二个参数就是直达页签）
  else if (id === 'drivers') openSettings('driver')
  else if (id === 'monitor') openMonitor()
  else if (id === 'bgCenter') { bgCenterMode.value = 'history'; bgCenterOpen.value = true; refreshBgStatus() }
  else if (id === 'newScript') newQueryTab()
}
const compareDialogOpen = ref(false)
const syncDialogOpen = ref(false)
// ===== 后台任务中心 =====
// 数据传输「后台运行」的任务在这里登记（见 backgroundTasks.js），点开顶栏图标看列表，
// 点某一项重新弹出进度窗（SyncDialog 的 resumeTaskId 负责跳回进度页接着轮询）。
const bgResumeId = ref('')
const bgCenterOpen = ref(false)
/** 任务中心视图：history = 全部执行记录（顶栏菜单项）；running = 正在执行（弹窗复用） */
const bgCenterMode = ref('history')
const bgTasksVisible = computed(() =>
  bgCenterMode.value === 'running' ? bgRunningList.value : bgTasks
)
// **执行记录分页**：任务多时表格一次全渲染又长又卡，按 10 条一页翻
const bgPage = ref(1)
const bgPageSize = 10
// 删除/清空记录后当前页可能超出范围 → 空表格假死，钳回有效页
watch(() => bgTasks.length, (n) => {
  const max = Math.max(1, Math.ceil(n / bgPageSize))
  if (bgPage.value > max) bgPage.value = max
})
const bgPageRows = computed(() => {
  const start = (bgPage.value - 1) * bgPageSize
  return bgTasks.slice(start, start + bgPageSize)
})
/** 耗时：**已结束用快照里的后端运行时长**（addedAt 是登记时刻，晚于任务真正开始，
 *  用它会虚高）；运行中 = 后端时长 + 距上次刷新的本地推进，**1 秒心跳**刷新显示
 *  （bgTick 只触发重渲染，不发请求）；已中断但没记下结束时间的旧记录显示 — */
const bgTick = ref(0)
setInterval(() => { bgTick.value++ }, 1000)
const bgDurationText = (t) => {
  bgTick.value
  if (bgStatusOf(t.id) !== 'running' && !t.finishedAt) return '—'
  // 已结束：**后端墙钟差**（权威）；运行中：后端时长 + 距上次取数的本地推进
  const s = bgStatusOf(t.id) === 'running'
    ? Math.floor(((t.elapsedMs || 0) + (Date.now() - (t.fetchedAt || Date.now()))) / 1000)
    : (t.finishedAt && t.startedAt)
      ? Math.max(0, Math.floor((t.finishedAt - t.startedAt) / 1000))
      : t.snapshot?.elapsedMs > 0
        ? Math.floor(t.snapshot.elapsedMs / 1000)
        : Math.max(0, Math.floor((t.finishedAt - t.addedAt) / 1000))
  if (s < 0) return '0s'
  if (s < 60) return s + 's'
  const m = Math.floor(s / 60)
  if (m < 60) return m + 'm ' + (s % 60) + 's'
  return Math.floor(m / 60) + 'h ' + (m % 60) + 'm'
}
/** 正在执行的任务（右侧时钟图标的悬浮下拉用，与弹窗模式无关） */
const bgRunningList = computed(() =>
  bgTasks.filter(t => (bgStatusMap.value[t.id] || t.status || 'running') === 'running')
)
/** 数据对比的后台任务恢复标记（对比点「后台运行」后从任务中心点开走这里） */
const compareResumeId = ref('')
/**
 * 顶栏「数据对比」入口：对话框已开着（残留打开态）时**先关再下帧重开** ——
 * 否则赋值 true 没有变化，子组件的打开 watch 不触发，上一次的查看/隐藏态
 * 清不掉，表现就是「点了没反应」（间歇性，真机踩过）。
 */
/** 顶栏「数据对比」：开新对比（防重 = 已开着时先关重开，保证是全新向导） */
const openCompare = () => {
  // 用户口径：点「数据对比」就是开新对比 —— 即使有任务在跑也不切回（要回看进度走任务中心）
  if (compareDialogOpen.value) {
    compareDialogOpen.value = false
    nextTick(() => { compareDialogOpen.value = true })
    return
  }
  compareDialogOpen.value = true
}
const openSync = () => {
  // 同上：点「数据传输」开新传输，不切回进行中的任务（回看进度走任务中心/时钟图标）
  if (syncDialogOpen.value) {
    syncDialogOpen.value = false
    nextTick(() => { syncDialogOpen.value = true })
    return
  }
  syncDialogOpen.value = true
}
const onCompareDialogVisible = (v) => {
  compareDialogOpen.value = v
  if (!v) compareResumeId.value = ''
}
/** 对话框关闭时清掉恢复标记 —— 下次从任务中心点开才是「新的一次恢复」 */
const onSyncDialogVisible = (v) => {
  syncDialogOpen.value = v
  if (!v) bgResumeId.value = ''
}
const bgStatusMap = ref({})
const refreshBgStatus = async () => {
// 刷一次每个后台任务的实时状态（徽标、列表、关闭检查共用）
// **按任务类型**查对应的状态接口：数据传输 → syncTaskStatus，数据对比 → compareTaskStatus
// 终态**写回记录**（持久化）—— 执行记录在页面刷新后仍显示完成/失败，而不是误标运行中
for (const t of bgTasks) {
try {
const st = t.kind === 'compare'
? await compareTaskStatus(t.id)
: await syncTaskStatus(t.id)
// **后端说任务不存在（notfound）≠ 本地记录作废**：服务重启后内存任务丢了，
// 但本地留底过的终态（成功/失败/取消）必须保留 —— 不能把「成功」覆盖成「已中断」（真机踩过）。
// 优先用记录状态，记录被旧版本污染过就用快照里的终态救回
if (st.status === 'notfound') {
const local = (t.status && t.status !== 'running') ? t.status
: (t.snapshot?.status && t.snapshot.status !== 'running' ? t.snapshot.status : null)
if (local) { bgStatusMap.value[t.id] = local; t.status = local; continue }
}
bgStatusMap.value[t.id] = st.status || 'running'
setBgTaskStatus(t.id, st.status || 'running', st)
// 运行时长 + 取数时刻留在记录上：耗时列对运行中的任务按
// 「后端时长 + 距本次取数的本地推进」显示（1 秒心跳让它连续走）
t.elapsedMs = st.elapsedMs || 0
t.fetchedAt = Date.now()
// **后端的墙钟起止是开始/结束时间的权威值**：任务真实提交/结束的时刻由后端记录，
// 前端登记时刻可能晚于真实开始（恢复/补登记场景），推算链路任何偏差都会显示错
if (st.startedAtWall > 0) t.startedAt = st.startedAtWall
if (st.finishedAtWall > 0) t.finishedAt = st.finishedAtWall
} catch {
// 查询失败（典型：应用重启后任务已不在内存，接口 404）→ **视为已过期**，
// 不再兜成 running —— 否则时钟图标的角标永远挂着「1」（真机踩过）。
// 本地已有终态留底的（含快照救回）同样沿用，别降级成中断
const local = (t.status && t.status !== 'running') ? t.status
: (t.snapshot?.status && t.snapshot.status !== 'running' ? t.snapshot.status : null)
if (local) { bgStatusMap.value[t.id] = local; t.status = local; continue }
bgStatusMap.value[t.id] = 'notfound'
setBgTaskStatus(t.id, 'notfound')
}
}
}
/** 停止一个运行中的后台任务（任务中心/悬浮下拉都有入口）——
    任务卡住时用户需要出口，停掉后状态落到 canceled，角标随之消失 */
const stopBgTask = async (t) => {
  try {
    if (t.kind === 'compare') await compareCancel(t.id)
    else await syncCancel(t.id)
    ElMessage.success(t('cmp.stopRequested'))
  } catch (e) {
    ElMessage.error(e?.message || t('common.unknownError'))
  }
  await refreshBgStatus()
}
// 记录时间：**年月日 时分秒**（用户指定格式）。
// mode='start' 显示开始（优先后端墙钟），否则显示结束（无结束时刻回退开始，不再是同刻假象）
const bgTimeText = (t, mode) => {
  const raw = mode === 'start' ? (t.startedAt || t.addedAt) : (t.finishedAt || t.startedAt || t.addedAt)
  const d = new Date(raw)
  const p = (n) => String(n).padStart(2, '0')
  return d.getFullYear() + '-' + p(d.getMonth() + 1) + '-' + p(d.getDate()) + ' ' + p(d.getHours()) + ':' + p(d.getMinutes()) + ':' + p(d.getSeconds())
}
/** 清空全部执行记录：确认后一次删光 */
const confirmClearBg = () => {
  ElMessageBox.confirm(
    t('sync.bgClearConfirm'),
    t('sync.bgCenter'),
    { type: 'warning', confirmButtonText: t('common.delete'), cancelButtonText: t('common.cancel') }
  ).then(() => { clearBgTasks(); bgPage.value = 1; ElMessage.success(t('sync.bgCleared')) }).catch(() => {})
}
/** 删除单条后当前页可能越界（最后一页只剩这一条）—— 收回来 */
watch(() => bgTasks.length, (n) => {
  const maxPage = Math.max(1, Math.ceil(n / bgPageSize))
  if (bgPage.value > maxPage) bgPage.value = maxPage
})
// 挂载后拉一次 + 每 5 秒刷新（任务少，几个请求的事），徽标才「活」
onMounted(() => { refreshBgStatus() })
setInterval(refreshBgStatus, 5000)
const bgRunningCount = computed(() =>
  bgTasks.filter(t => (bgStatusMap.value[t.id] || 'running') === 'running').length
)
const bgStateText = (id) => {
// **notfound = 任务已不存在**（服务重启后内存任务丢了/超过保留期）——
// 必须显示「已中断」而不是落进 else 的「运行中」（真机踩过：重启后任务中心全是假"运行中"）
const s = bgStatusMap.value[id] || 'running'
return s === 'success' ? t('sync.statOk') : s === 'canceled' ? t('sync.statSkipped')
: s === 'error' ? t('sync.statFailed') : s === 'notfound' ? t('sync.bgInterrupted') : t('sync.running')
}
/** 表格里的**短**状态文案：中断原因放悬浮（列窄，长文案会挤爆布局） */
const bgStateShort = (id) => {
  const s = bgStatusMap.value[id] || 'running'
  return s === 'notfound' ? t('sync.bgInterruptedShort') : bgStateText(id)
}
const bgStatusOf = (id) => (bgStatusMap.value[id] || 'running')
const resumeBgTask = (t) => {
  bgCenterOpen.value = false
  // 按**任务类型**路由回对应对话框的进度窗：数据传输 → SyncDialog；
  // 数据对比 → CompareDialog（各自带恢复标记，进度窗直接接着轮询）
  if (t.kind === 'compare') {
    compareResumeId.value = t.id
    compareDialogOpen.value = true
    return
  }
  bgResumeId.value = t.id
  syncDialogOpen.value = true
}
const settingsOpen = ref(false)
/** 要直达的设置页签（'' = 上次停留的页签）。首页 MCP 卡「前往设置」用 */
const settingsTab = ref('')
const openSettings = (tab = '') => { settingsTab.value = tab; settingsOpen.value = true }
// AI 助手等子组件通过全局事件请求打开设置（如「AI 服务未配置」提示里的直达链接）
const onOpenSettingsEvent = (e) => openSettings(e.detail?.tab || '')
window.addEventListener('dc-open-settings', onOpenSettingsEvent)
onBeforeUnmount(() => window.removeEventListener('dc-open-settings', onOpenSettingsEvent))

// ===== 顶栏右侧：主题 / 语言 快速切换（原来只有知识库 / AI / 设置三枚）=====
/** 主题模式：system / light / dark，点一次按这个顺序循环 */
const themeMode = ref(getThemeSettings().mode)
/** 当前「实际生效」的外观（system 模式下由系统决定），用来选图标 */
const resolvedTheme = ref(getResolvedTheme())
onResolvedThemeChange((v) => { resolvedTheme.value = v })
const themeIcon = computed(() => (resolvedTheme.value === 'dark' ? Moon : Sunny))
const themeTip = computed(() => t(themeMode.value === 'dark' ? 'nav.themeDark' : themeMode.value === 'light' ? 'nav.themeLight' : 'nav.themeSystem'))
const cycleTheme = () => {
  const order = ['system', 'light', 'dark']
  const next = order[(order.indexOf(themeMode.value) + 1) % order.length]
  themeMode.value = saveThemeSettings({ mode: next }).mode
  applyTheme(themeMode.value)
}
/** 语言钮用「中 / EN」而不是图标：这两个字比任何语言图标都直白 */
const localeShort = computed(() => (locale.value === 'en-US' ? 'EN' : '中'))
const langTip = computed(() => t('nav.switchLang'))
const toggleLocale = () => setLocale(locale.value === 'en-US' ? 'zh-CN' : 'en-US')
const governanceOpen = ref(false)
/** 治理弹窗要定位到的页签：sensitive / capacity / quality / analysis（AI 助手的面板命令会改它） */
const governanceTab = ref('sensitive')
/** 打开治理弹窗时是否直接开跑（敏感数据 / 数据容量这类「点开即出结果」的扫描） */
const governanceAutoRun = ref(false)
/** 治理弹窗的四个页签键：AI 助手面板命令直达用 */
const GOVERNANCE_TABS = ['sensitive', 'capacity', 'quality', 'analysis', 'patrol']
/** 「点开即出结果」的页签：从 AI 助手跳过去时自动执行 */
const GOVERNANCE_AUTORUN_TABS = ['sensitive', 'capacity']
const paletteOpen = ref(false)
/** 顶部导航「监控」入口：打开为工作区 tab（连接/库在打开时固化，后续切连接不影响本页数据源） */
const MONITOR_TAB_PREFIX = 'monitor@'
const openMonitor = () => {
  if (!currentConnId.value) { ElMessage.warning(t('mv.needConn')); return }
  const id = MONITOR_TAB_PREFIX + currentConnId.value
  const exist = tabs.value.find(t => t.id === id)
  if (exist) { exist.database = currentDb.value; activeTab.value = id; return }
  openTab({ id, type: 'monitor', label: t('mv.monitorTab'), connId: String(currentConnId.value), database: currentDb.value })
}
const aiExplainVisible = ref(false)
const aiExplainTitle = ref('')
const aiExplainContent = ref('')
// ===== 树 CRUD 弹窗（新建库 / 目录、重命名分组 / 表：逻辑已抽到 NewDbDialog.vue / FolderDialogs.vue / RenameTableDialog.vue，这里仅保留开关与上下文快照） =====
const newDbVisible = ref(false)
const newDbCtx = ref({ connId: null, connType: '' })
const folderCreateVisible = ref(false)
const folderRenameVisible = ref(false)
const folderEnv = ref('')
const renameTableVisible = ref(false)
const renameTableCtx = ref({ connId: '', db: '', oldName: '' })
const renameObjectVisible = ref(false)
const renameObjectCtx = ref({ connId: '', db: '', type: '', oldName: '' })
const allTables = ref([])
const isNoSql = computed(() => isNoSqlType(conn.value?.type))
// 判断指定连接是否为 NoSQL（右键菜单中隐藏"新建数据库"等 SQL 专属操作）
const isNoSqlConn = (connId) => {
  const c = allConnections.value.find(x => String(x.id) === String(connId))
  return c ? isNoSqlType(c.type) : false
}

// ====== 多连接并开：连接各自维护「当前库 / 库列表」缓存 ======
// 页签是**全局**的：所有数据源打开的表 / SQL / 结构都留在同一条页签栏里。
// 切连接、切库、双击别的连接下的表，都不再把页签栏整条换掉
// （以前按连接分开存，打开 B 的表会把 A 的页签收走，看着像页签被关了）。
const dbsByConn = ref({})        // connId -> 数据库列表（展开时缓存，避免重复请求）
// connId -> catalog 列表（catalog 层级的类型才有内容；其余类型是空数组，等价于"没有这一层"）
const catalogsByConn = ref({})
const currentDbByConn = ref({})  // connId -> 上次选中的当前库
// 与「当前连接」无关的全局工作区：页签只认自己的 ×，切连接、删连接都不会消失。
const GLOBAL_TABS = [
  { id: 'ai:studio', type: 'ai', label: t('sqlq.aiAssistant') },
  { id: 'knowledge:studio', type: 'knowledge', label: t('mv.knowledge') },
]
const isGlobalTab = (t) => !!t && GLOBAL_TABS.some(g => g.type === t.type)
/** 常驻页签的开关与位置（全局共享，key = 页签 id） */
const globalTabState = ref({})

/**
 * 页签 id 带上所属连接：页签栏全局之后，不同数据源很可能有同名表 / 同名对象
 * （同一套库的 dev / test / prod 环境），不带连接就会互相顶掉，看着又像「页签被关了」。
 */
const tabKey = (base, connId) => (connId ? base + '@' + connId : base)

// ===== 会话持久化：只服务「刷新界面」=====
// 页签 + 当前连接 + 各连接的当前库，活着时只在内存里，一刷新就没了，所以要落盘。
// 但它不是跨次启动的配置存储 —— 新开软件应当是一张白纸（树收起、不自动连库、没有页签）：
//   1) 存 sessionStorage：同一标签页内刷新还在，关掉标签页/窗口就没了；
//   2) 再叠加「只有刷新这种加载才恢复」：桌面版（Electron）关掉进程再打开时，
//      sessionStorage 可能仍留在磁盘上，靠这一层保证它一定被清掉。
const SESSION_KEY = 'dbmind.session'
let sessionTimer = null

/** 本次页面加载是不是「刷新」/前进后退 —— 只有这种才恢复会话快照 */
const isReloadLoad = (() => {
  try {
    const nav = performance.getEntriesByType && performance.getEntriesByType('navigation')[0]
    if (nav && nav.type) return nav.type === 'reload' || nav.type === 'back_forward'
    return !!(performance.navigation && performance.navigation.type === 1) // 旧接口：1 = RELOAD
  } catch { return false }
})()

/** 不是刷新：清掉上次会话留下的东西（页签快照 + 记忆的连接），下次从空开始 */
const dropStaleSession = () => {
  try {
    localStorage.removeItem(SESSION_KEY)           // 旧版本把快照存在这里，一并清掉
    sessionStorage.removeItem(SESSION_KEY)
    sessionStorage.removeItem('dbmind.currentConnId')  // 不记忆连接 → 新开时不自动连库、不展开树
  } catch { /* ignore */ }
}

const saveSession = () => {
  try {
    // 把编辑器里的实时内容写回页签快照 —— 页签里的 initialSql 是「打开这份页签时」的内容，
    // 不刷新它，保存/编辑过的 SQL 在刷新后会退回旧文本
    for (const t of tabs.value) {
      if (t.type !== 'sql') continue
      const live = sqlViewRefs[t.id]?.getSql?.()
      if (typeof live === 'string') t.initialSql = live
    }
    saveConnState()   // 顺手把当前连接的库列表 / 当前库一起存了
    // 页签是全局一条列表（每个页签自带 connId / database），不再按连接分开存
    //
    // `dbsByConn`（每个连接的数据库列表）**刻意不写进快照**：它是**服务端状态**，
    // 存进会话就变成「一次存错，永远错」—— 新建的库看不见、后端后来学会了列库也看不见，
    // 而且刷新页面还会把它读回来，用户没有任何办法自愈（实测踩过：MySQL 连接上永远只显示
    // 一个 `(default)`，而后端其实返回了 10 个库）。会话内的内存缓存照旧，
    // 只是不再跨页面加载复用；重新加载后展开连接会重新拉一次真实列表。
    sessionStorage.setItem(SESSION_KEY, JSON.stringify({
      currentConnId: currentConnId.value,
      tabs: tabs.value,
      activeTab: activeTab.value,
      globalTabState: globalTabState.value,
      currentDbByConn: currentDbByConn.value
    }))
  } catch { /* 配额溢出等异常不该影响正常使用 */ }
}

/** 防抖：连续开关页签会频繁触发，没必要每次都序列化一遍 */
const scheduleSaveSession = () => {
  clearTimeout(sessionTimer)
  sessionTimer = setTimeout(saveSession, 400)
}

/** 旧版快照（页签按连接分开存）→ 摊平成全局一条列表，并补上连接后缀 */
const tabsFromLegacySnapshot = (d) => {
  const byConn = d.tabsByConn || {}
  const cur = byConn[d.currentConnId] || []
  const rest = Object.keys(byConn).filter(k => k !== d.currentConnId).flatMap(k => byConn[k] || [])
  const seen = new Set()
  const out = []
  for (const t of [...cur, ...rest]) {
    if (!t || !t.id) continue
    const id = tabKey(t.id, t.connId)
    if (seen.has(id)) continue
    seen.add(id)
    out.push({ ...t, id })
  }
  return out
}

/** 恢复上次会话：把快照灌回内存，页签是全局的，直接灌回去就行 */
const restoreSession = () => {
  // 新开软件 / 新开窗口（不是刷新）→ 丢弃上次会话，一切从空开始
  if (!isReloadLoad) { dropStaleSession(); return }
  try {
    const raw = sessionStorage.getItem(SESSION_KEY)
    if (!raw) return
    const d = JSON.parse(raw)
    if (d.globalTabState) globalTabState.value = d.globalTabState
    // 旧快照里可能还带着 `dbsByConn` —— **故意不恢复**：那是服务端状态，
    // 跨页面加载复用会让一份过期的库列表永远生效（见 saveConnState 的注释）。
    // 展开连接时按需重新拉取，拿到的一定是现状。
    if (d.currentDbByConn) currentDbByConn.value = d.currentDbByConn
    // 页签：全局一条列表；旧版本的快照按连接分开存，摊平后照旧能用
    if (Array.isArray(d.tabs)) {
      tabs.value = d.tabs
      activeTab.value = d.tabs.some(t => t.id === d.activeTab) ? d.activeTab : (d.tabs[0]?.id || '')
    } else if (d.tabsByConn) {
      tabs.value = tabsFromLegacySnapshot(d)
      activeTab.value = tabs.value[0]?.id || ''
    }
    // 「编辑用户」页签**故意不恢复**：它的权限数据是打开那一刻的快照，恢复后界面显示旧状态、
    // 而保存又是「以表单为准」的 —— 中间在库里新增的权限会被静默撤掉。
    // （实测踩到：root@% 的 GRANT OPTION 刷新后显示成未勾，实际它有。）
    // 「新建用户」没有这个风险（权限区本来就是空的），照旧恢复。
    tabs.value = tabs.value.filter(t => !(t && t.type === 'user-form' && t.mode === 'edit'))
    if (!tabs.value.some(t => t.id === activeTab.value)) activeTab.value = tabs.value[0]?.id || ''
    // 桌面版 / 新开窗口时 sessionStorage 的 dbmind.currentConnId 是空的 ——
    // 不补一个的话，页签恢复进内存了却没有连接去承载它，界面上依旧什么都没有。
    let cid = d.currentConnId
    if (!cid) cid = tabs.value.find(t => t.connId)?.connId || ''
    if (cid && !sessionStorage.getItem('dbmind.currentConnId')) {
      sessionStorage.setItem('dbmind.currentConnId', String(cid))
    }
  } catch { /* 存档损坏就当作全新会话 */ }
}
/** 当前处于打开状态的常驻页签 */
const openGlobalTabs = () => GLOBAL_TABS.filter(g => globalTabState.value[g.id]?.open)

// 保存当前连接的状态快照（页签是全局的，不在这里存）
const saveConnState = () => {
  if (!currentConnId.value) return
  // 常驻页签：记录开关状态；已关掉的要显式记成 false，否则下次还会被插回来
  const state = { ...globalTabState.value }
  tabs.value.forEach((t, i) => {
    if (isGlobalTab(t)) state[t.id] = { open: true, index: i }
  })
  GLOBAL_TABS.forEach(g => {
    if (!tabs.value.some(t => t.id === g.id)) state[g.id] = { open: false, index: state[g.id]?.index || 0 }
  })
  globalTabState.value = state
  // **空列表不写进缓存**：`dbs.value` 在"还没加载 / 加载失败"时就是 `[]`，写进去等于
  // 告诉后面的展开"这个连接没有库" —— 命中缓存后直接 resolve([])，而 el-tree 一旦收到
  // 空数组就把节点标成 `isLeaf`，**这个标记是粘住的**：之后服务器上明明有库，
  // 展开也永远空白（用户报的"显示展开了却没东西，其实有东西"就是这个）。
  if (dbs.value.length) dbsByConn.value[currentConnId.value] = dbs.value
  currentDbByConn.value[currentConnId.value] = currentDb.value
}
/** 连接被删除：只关掉属于它的页签（常驻页签和别的连接的页签都留着） */
const closeTabsOfConn = (sid) => {
  tabs.value = tabs.value.filter(t => isGlobalTab(t) || String(t.connId || '') !== String(sid))
  if (activeTab.value && !tabs.value.some(t => t.id === activeTab.value)) {
    activeTab.value = tabs.value[0]?.id || ''
  }
}
// 设置当前数据库（同步写入该连接的状态缓存，切换连接后仍能记住）
const setCurrentDb = (db) => {
  currentDb.value = db
  if (currentConnId.value) currentDbByConn.value[currentConnId.value] = db
}
// 判断某个树节点属于「当前库」。
//
// schema 型数据库（SQL Server / PostgreSQL / KingbaseES）的树里，库下面还有一层 schema：
// schema 节点的 db 是「库.schema」（如 succbi.dbo），而 currentDb 只存库名（succbi）。
// 于是 `db === currentDb.value` **永远不成立** —— 它守着两件事：行数的精确回填、
// 以及「表名下拉」的数据源。表现为 SQL Server 上「树上每张表恒为 0 / 表名补全为空」，
// 而在非 schema 型库上一切正常（所以这个 bug 能一直藏着）。
const inCurrentDb = (db) => {
  const target = String(currentDb.value || '')
  const value = String(db || '')
  return value === target || (!!target && value.startsWith(target + '.'))
}
// 判断某连接是否已打开（展开加载过数据库，或开着它的页签；正在连接中的不算）
const isConnOpen = (connId) => {
  const sid = String(connId)
  if (connLoadingSet.value.has(sid)) return false
  return sid in dbsByConn.value || tabs.value.some(t => String(t.connId || '') === sid)
}

// 表右键菜单状态（subKey: 'sql-tpl' | 'export' | null，控制二级子菜单）
const ctxMenu = ref({ show: false, x: 0, y: 0, subKey: null, data: null })
const ctxDdl = ref('')
const ctxDdlVisible = ref(false)
const ctxImportVisible = ref(false)
// 导入数据的目标表快照（打开时捕获 ctxMenu.data，避免弹窗期间菜单状态被后续右键覆盖）
const ctxImportTarget = ref(null)
// 数据生成弹窗（按字段结构造数写入目标表）
const ctxDataGenVisible = ref(false)
const ctxDataGenTarget = ref(null)
// 导出任务进度弹窗状态
const exportProgressVisible = ref(false)
const exportTaskId = ref('')
const exportStatus = ref('running')
const exportDone = ref(0)
const exportTotal = ref(-1)
const exportPhase = ref('')
const exportMessage = ref('')
const exportLogs = ref([])
const exportCanceling = ref(false)
const exportTargetName = ref('')
let exportPollTimer = null

// 树右键菜单（folder/conn/blank/db/category）
const treeCtxMenu = ref({ show: false, x: 0, y: 0, kind: '', env: '', cat: '', db: '', data: null })
const closeTreeCtxMenu = () => { treeCtxMenu.value.show = false }
// 数据库右键相关弹窗状态
const dbDdlVisible = ref(false)
const dbDdlText = ref('')
// ===== 转储 / 查找 / 运行 SQL 弹窗（逻辑已抽到 DbDumpDialog.vue / DbSearchDialog.vue / RunSqlFileDialog.vue，这里仅保留开关） =====
const dbDumpVisible = ref(false)
const dbSearchVisible = ref(false)
const dbRunSqlVisible = ref(false)
const backupRestore = ref({ visible: false, mode: 'backup', connectionId: '', database: '' })
const openBackupRestore = (mode, connId, db) => {
  backupRestore.value = { visible: true, mode, connectionId: String(connId || ''), database: db || '' }
}
// 点击外部自动关闭
window.addEventListener('click', closeTreeCtxMenu)
window.addEventListener('scroll', closeTreeCtxMenu, true)

// 自定义目录 = 所有 connection.group 中非预置、非空的值 ∪ 后端「纯目录」
// （还没有任何连接的分组，唯一真相源在后端 app_settings 的 ui.folders，见 utils/folders.js）
const folderRefresh = ref(0)
const customFolders = computed(() => {
  // eslint-disable-next-line no-unused-expressions
  folderRefresh.value
  const set = new Set()
  for (const c of allConnections.value) {
    const env = c.group || ''
    if (env && !PREDEF_ENVS.includes(env)) set.add(env)
  }
  for (const f of pureFolders.value) if (f && !PREDEF_ENVS.includes(f)) set.add(f)
  return Array.from(set)
})

const refreshFolders = () => folderRefresh.value++

let tabSeq = 0

// 加载所有连接（用于顶部下拉切换）
const loadAllConnections = async () => {
  treeLoading.value = true
  try {
    allConnections.value = await listConnections()
  } catch (e) {
    allConnections.value = []
    ElMessage.error(t('mv.loadConnsFailed', { detail: (e?.message || e) }))
  } finally {
    treeLoading.value = false
  }
}

/**
 * MCP 配置片段。`dbmind-mcp` 是仓库里**真实存在**的 stdio MCP 服务（`crates/dbmind-mcp`），
 * 默认只读。这里给的是可直接复制的最小示例 —— 路径换成自己的安装位置即可
 * （与 dbmind-web 同一个目录）。
 */
const mcpSnippet = computed(() => [
  '{',
  '  "mcpServers": {',
  '    "dbmind": {',
  t('mv.mcpCommandLine'),
  '    }',
  '  }',
  '}',
].join('\n'))
const copyMcp = async () => {
  try {
    await navigator.clipboard.writeText(mcpSnippet.value)
    ElMessage.success(t('mv.mcpCopied'))
  } catch {
    ElMessage.warning(t('mv.copyManually'))
  }
}

// ====== 欢迎页的「最近查询」======
// 数据来自**内核原生接口** `/api/dbmind/history`（上游那层没有这个端点）。
// 只在欢迎页用：一行一条，点一下把那句 SQL 开成新脚本 —— 比让用户自己去翻历史省一步。
const recentHistory = ref([])
// ====== 启动后静默预热：把"第一次建连"的几秒提前到用户还没点的时候 ======
//
// 为什么值得做（实测数据）：同一接口连测三次是 **5089ms → 86ms → 91ms**、
// **10074ms → 215ms → 210ms** —— 那几秒全是 **Java 宿主冷启动 + 首次建连** 的
// 一次性成本，跟 SQL 本身无关。
//
// 为什么一次就够：JDBC 宿主是**一个进程服务所有关系型连接**。实测把 4 个不同库
// 逐个点一遍：`265 / 255 / 271 / 231 ms`，而 java 进程数**始终是 1** ——
// 也就是说点着它一次，MySQL / Doris / ClickHouse / SQL Server / PG / Oracle… 就全热了。
//
// 所以这里只发**一个**请求，不再逐个连接预热（那是白花钱：4 个宿主各要一个 JVM）。
// 目标取「上一次用过的连接」，覆盖绝大多数场景 —— 用户下一步几乎总是点它。
// 失败一律忽略：预热只是让第一次变快，不该影响任何功能。
const warmUpBeforeFirstClick = async () => {
  try {
    const remembered = sessionStorage.getItem('dbmind.currentConnId') || ''
    const target = allConnections.value.find(c => String(c.id) === remembered)
      || allConnections.value[0]
    if (!target) return
    if (isNoSqlType(target.type)) {
      await noSqlDatabases(target.id)
      return
    }
    // 用**本次返回**的库列表挑目标，而不是 dbsByConn：那是 lazyLoad 才会写的缓存，
    // 非刷新的新启动里它还是空的 —— 拿它取库名会挑到空字符串，会话就白预热了（踩过）。
    const dbs = await listDatabases(target.id)
    const dbsArr = Array.isArray(dbs) ? dbs : []
    // 再往前一步：把「上次展开过的那个库」的会话也建好。
    //
    // 为什么值得多发这一个请求：实测把宿主预热起来之后，开一个**新库**仍要 4959ms，
    // 而本机 SQL Server 只要 100~600ms —— 宿主已热、Java 侧也没有连接池、会话也不回收，
    // 所以那 5 秒只能出在**新建一条 TCP 连接**上（最像服务端反向 DNS 解析超时）。
    // 客户端改不了服务端，但可以**提前付掉**；而会话建好后不会再被回收（宿主无闲置回收），
    // 于是用户点开那一刻就是现成的。
    //
    // 顺手把表清单写进缓存：这一个请求本来就要跑，不填缓存就白跑了。
    // 跳过系统库：`information_schema` / `mysql` / `sys` 这些用户根本不会点，
    // 而且查它们本身很慢（实测 `information_schema` 的 /tables 要 10 秒）——
    // 拿它当预热目标等于把预算花在最没用的地方。
    const SYSTEM_DBS = ['information_schema', 'performance_schema', 'mysql', 'sys', '__internal_schema', '(default)']
    const usable = dbsArr.filter(d => d && !SYSTEM_DBS.includes(String(d).toLowerCase()))
    const rememberedDb = currentDbByConn.value[target.id]
    const pick = (rememberedDb && usable.includes(rememberedDb)) ? rememberedDb : (usable[0] || '')
    if (pick) {
      try {
        const tables = await listTables(target.id, pick)
        if (Array.isArray(tables)) writeSchemaCache('tables:' + target.id + ':' + pick, tables)
      } catch { /* 预热失败无所谓：用户点开时照常按真实路径加载 */ }
    }
  } catch { /* 预热失败：用户点开时照常按真实路径加载 */ }
}

/** 欢迎页的「最近查询」：首次进来就取一次（失败无所谓，那块会显示一句说明） */
const loadHistory = async () => {
  try {
    // 底部「近 7 天趋势 / 失败查询」需要更多样本（最近 8 条撑不起一周的图），多取一份
    const [rows, many] = await Promise.all([listHistory(8), listHistory(200)])
    // 4 条：加了第四行卡片后竖向空间更紧 —— 首页宁可少两条也别出现滚动条
    recentHistory.value = (Array.isArray(rows) ? rows : []).slice(0, 4)
    trendHistory.value = Array.isArray(many) ? many : []
  } catch {
    // 拿不到历史不影响欢迎页的其它内容（新库本来就没有历史）
    recentHistory.value = []
    trendHistory.value = []
  }
}

/** 近 7 天趋势 / 失败查询的数据源（比「最近查询」的 6 条多） */
const trendHistory = ref([])

/** 近 7 天每天的执行量：按自然日分桶，没有记录的天也占位（柱高 0），图才是连续的一周 */
const trendDays = computed(() => {
  const list = Array.isArray(trendHistory.value) ? trendHistory.value : []
  const now = new Date()
  const days = []
  for (let i = 6; i >= 0; i--) {
    const d = new Date(now.getFullYear(), now.getMonth(), now.getDate() - i)
    days.push({ key: d.toDateString(), label: (d.getMonth() + 1) + '/' + d.getDate(), count: 0, fails: 0 })
  }
  const byDay = new Map(days.map((d, i) => [d.key, i]))
  for (const h of list) {
    const t = h.createdAt ? new Date(h.createdAt) : null
    if (!t || isNaN(t.getTime())) continue
    const i = byDay.get(t.toDateString())
    if (i === undefined) continue
    days[i].count++
    if (h.status !== 'ok') days[i].fails++
  }
  return days
})

const trendMax = computed(() => Math.max(1, ...trendDays.value.map(d => d.count)))
/** 柱高：按当日量归一到 0-56px；0 条也给 2px 的基线，表示「这天没查」而不是「缺数据」 */
/** 柱高：按当日量归一到 0-72px。0 不画（v-if 拦在模板层）—— 灰槽本身就表示「没查」，
 *  之前给 0 也画 2px 基线，结果没查询的天也冒出蓝红各一小条，自相矛盾（真机踩过）。 */
const barHeight = (n) => Math.max(4, Math.round((n / trendMax.value) * 72)) + 'px'

/**
 * 清空查询历史。
 *
 * 为什么是**整体清空**而不是按条删：后端只有一个清空接口，且历史表里没有「来源」字段 ——
 * 早期（没有 `internal` 标记时）写进去的那些元数据查询，事后无法可靠地识别出来。
 * 既然如此就把话说在前面：确认框里明确写"全部 N 条、不能只删一条"。
 */
const onClearHistory = async () => {
  const total = recentHistory.value.length
  if (!total) return
  try {
    await ElMessageBox.confirm(
      t('empty.clearHistoryBody', { n: total }),
      t('empty.clearHistoryTitle'),
      {
        type: 'warning',
        confirmButtonText: t('common.delete'),
        cancelButtonText: t('common.cancel'),
        closeOnClickModal: false,
        closeOnPressEscape: false
      }
    )
  } catch {
    return // 用户取消
  }
  try {
    await clearHistory()
    recentHistory.value = []
    ElMessage.success(t('empty.clearHistoryDone'))
  } catch (e) {
    ElMessage.error(t('empty.clearHistoryFailed', { detail: errMsg(e, t('common.unknownError')) }))
  }
}
/**
 * 把一条历史记录解析成**当前真实存在**的连接。
 *
 * 历史里的 connectionId 可能是过期的：连接被删掉再重建就会换一个 id。
 * （实测 history 里全是 conn_18d6785349655d340046，而当前连接是 conn_18d6784ecfbc68280041 ——
 * 前缀一模一样、后缀不同，就是同一条连接重建后的新 id。）
 * 于是点一下「最近查询」就报「连接不存在或类型未知」，连接下拉还会显示一串原始 id
 * （el-select 找不到匹配选项时就会把原始值当文本显示）。
 * 好在历史里带着 connectionName，形如「192.168.2.216 ▸ test」（**连接名 ▸ 库名**），
 * 按名字再兜一次底，就能找回真正的那条连接。
 */
const resolveHistoryConn = (h) => {
  const list = allConnections.value || []
  const byId = list.find(c => String(c.id) === String(h?.connectionId || ''))
  if (byId) return byId
  const name = String(h?.connectionName || '').split('▸')[0].trim()
  if (!name) return null
  return list.find(c => String(c.name || '').trim() === name) || null
}
/** 历史里的库名：connectionName 的「▸」后半段（可能为空） */
const historyDatabase = (h) => String(h?.connectionName || '').split('▸')[1]?.trim() || ''

/** 点一条历史：先切到它所属的连接，再把 SQL 开成新脚本（这样库上下文才对得上） */
const openHistoryItem = async (h) => {
  if (!h?.sql) return
  // 连接列表可能还没到手（首次进首页就点历史）：先补一次，否则会把"还没加载"误判成"连接不存在"
  if (!allConnections.value.length) await loadAllConnections()
  const c = resolveHistoryConn(h)
  if (!c) {
    // 实在找不到就别把失效 id 塞进页签 —— 那会一路报「连接不存在或类型未知」。
    // 用当前连接打开（没有当前连接就留空，由用户在脚本页自行选择），并说明原因。
    ElMessage.warning(t('mv.historyConnGone'))
    newQueryTab({ initialSql: h.sql })
    return
  }
  const connId = String(c.id)
  try {
    if (connId !== String(currentConnId.value || '')) await selectConn(connId)
  } catch {
    // 连接打不开也照样把 SQL 摆出来：用户至少能看到那句是什么、错在哪
  }
  newQueryTab({ connId, database: historyDatabase(h) || undefined, initialSql: h.sql })
}
/** 历史时间 → 今天显示 HH:MM，更早显示 MM-DD（一串 ISO 时间戳没人愿意读） */
const clockText = (iso) => {
  if (!iso) return ''
  const d = new Date(iso)
  if (isNaN(d.getTime())) return ''
  const p = (n) => String(n).padStart(2, '0')
  return d.toDateString() === new Date().toDateString()
    ? `${p(d.getHours())}:${p(d.getMinutes())}`
    : `${p(d.getMonth() + 1)}-${p(d.getDate())}`
}
// 回到欢迎页时刷新一次：刚跑过的 SQL 应该立刻出现在「最近查询」里
watch(() => tabs.value.length, (n) => { if (n === 0) loadHistory() })

// 打开新建/编辑连接弹窗（initialEnv/initialType 仅在新建时生效）
const openConnDialog = (c, initialEnv, initialType) => {
  editingConn.value = c || null
  connDialogInitialEnv.value = initialEnv || ''
  connDialogInitialType.value = initialType || ''
  connDialogVisible.value = true
}
const connDialogInitialEnv = ref('')
const connDialogInitialType = ref('')

// 新建连接：先弹数据源选择器，选择后再进入连接表单
const openNewConn = (env) => {
  dsPickerEnv.value = env || ''
  dsPickerVisible.value = true
}
const onDsSelected = (type) => {
  dsPickerVisible.value = false
  openConnDialog(null, dsPickerEnv.value, type)
}
// 选择器顶部「最近使用」点一条：直接打开它的编辑表单（省得去树里翻）
const onRecentConn = async (id) => {
  dsPickerVisible.value = false
  const target = allConnections.value.find(c => String(c.id) === String(id))
  if (!target) {
    ElMessage.warning(t('mv.connGone'))
    await loadAllConnections()
    return
  }
  openConnDialog(target, dsPickerEnv.value, target.type)
}

// ====== 树右键菜单操作 ======
const treeCtxNewConn = (env) => { closeTreeCtxMenu(); openNewConn(env || customFolders.value[0] || '') }

// ====== 连接配置导入 / 导出（与「新建数据源」弹窗底部那组按钮同一套口径，共用 utils/connTransfer）======
const exportConns = async (env) => {
  closeTreeCtxMenu()
  try {
    const all = await listConnections()
    const picked = env ? all.filter(c => (c.env || c.group) === env) : all
    if (!picked.length) {
      ElMessage.warning(env ? t('mv.groupNoConns', { env }) : t('mv.nothingToExport'))
      return
    }
    downloadJson('dbmind-connections-' + new Date().toISOString().slice(0, 10) + '.json',
      buildConnectionBundle(picked))
    ElMessage.success(t('mv.exportedConns', { n: picked.length }))
  } catch (e) {
    ElMessage.error(t('mv.exportFailed', { detail: errMsg(e, t('common.unknownError')) }))
  }
}
const importConns = (env) => {
  closeTreeCtxMenu()
  // 动态建 input：比在模板里挂一个隐藏 input 更省事，也不会在多处复用时打架
  const input = document.createElement('input')
  input.type = 'file'
  input.accept = '.json,application/json'
  input.onchange = async () => {
    const file = input.files && input.files[0]
    if (!file) return
    const parsed = parseConnectionBundle(await file.text())
    if (parsed.error) {
      ElMessage.error(parsed.error)
      return
    }
    const list = parsed.list
    try {
      // 名单与分隔符都随语言：中文用顿号、英文用逗号，句末标点也各按各的
      const names = list.slice(0, 5).map(c => c.name).join(t('common.listSep'))
      const more = list.length > 5 ? t('mv.importMore') : ''
      await ElMessageBox.confirm(
        t('mv.importConfirm', { n: list.length, names, more })
          + (env ? t('mv.importIntoGroup', { env }) : '')
          + t('mv.importPasswordNote'),
        t('mv.importTitle'),
        { type: 'warning', confirmButtonText: t('mv.importStart'), cancelButtonText: t('common.cancel') }
      )
    } catch (e) {
      return
    }
    let ok = 0
    const failed = []
    for (const item of list) {
      try {
        await saveConnection(importPayloadOf(item, env))
        ok++
      } catch (e) {
        failed.push(t('mv.failItem', { name: item.name, detail: errMsg(e, t('mv.failed')) }))
      }
    }
    await loadAllConnections()
    await buildTree()
    if (failed.length) {
      ElMessage.warning(t('mv.importDone', { ok, failed: failed.length }))
      console.warn('导入失败的连接：', failed)
    } else {
      ElMessage.success(t('mv.importedConns', { n: ok }))
    }
  }
  input.click()
}
// ====== 树内直接重命名分组（就地改，不弹窗）======
// 有两处存储要一起改：
// 1) 该分组下所有连接的 group（连接是靠它归属分组的）
// 2) 后端「纯分组」（还没有任何连接的分组，存 app_settings 的 ui.folders）
// 少改任何一处，都会出现「名字改了、连接没跟过去」或「旧名字又冒出来」。
const renamingKey = ref('')
const renameDraft = ref('')
const startRenameGroup = async (env) => {
  closeTreeCtxMenu()
  renamingKey.value = env
  // 多级目录：env 是全路径（`公司资源/研发`），输入框只放**尾段** ——
  // 用户改的是这一级的名字，父级路径保持不动
  renameDraft.value = env.includes('/') ? env.slice(env.lastIndexOf('/') + 1) : env
  await nextTick()
  // el-input 在树节点的 v-for 里，组件 ref 不好定位；直接拿 DOM 兜底，聚焦并全选原名
  const input = document.querySelector('.tree-rename-input input')
  if (input) {
    input.focus()
    input.select()
  }
}
const cancelRenameGroup = () => {
  renamingKey.value = ''
  renameDraft.value = ''
}
const commitRenameGroup = async () => {
  const from = renamingKey.value
  if (!from) return
  const to = String(renameDraft.value || '').trim()
  // 先退出改名态：@blur 会再触发一次，靠这个提前 return 掉，避免提交两遍
  renamingKey.value = ''
  if (!to || to === from) return
  if (to.includes('/')) return ElMessage.warning(t('mv.groupNameTooLong', { n: 20 }))
  if (to.length > 20) return ElMessage.warning(t('mv.groupNameTooLong', { n: 20 }))
  // 多级目录：renameDraft 是**尾段**新名，拼回父路径得到完整新路径
  const idx = from.lastIndexOf('/')
  const parent = idx > 0 ? from.slice(0, idx) : ''
  const newPath = parent ? parent + '/' + to : to
  if (newPath !== from && (customFolders.value.includes(newPath) || PREDEF_ENVS.includes(newPath))) {
    return ElMessage.warning(t('mv.groupNameExists', { name: newPath }))
  }
  try {
    // **级联改名**：不仅这个目录本身，它的**所有子目录**（group 以 `from/` 开头）都要跟上，
    // 否则改父名后子目录的路径前缀就断了，树会散架
    const targets = allConnections.value.filter(c => {
      const g = c.group || ''
      return g === from || g.startsWith(from + '/')
    })
    for (const c of targets) {
      const g = c.group || ''
      await saveConnection({ ...c, group: newPath + g.slice(from.length) })
    }
    for (let i = 0; i < pureFolders.value.length; i++) {
      const f = pureFolders.value[i] || ''
      if (f === from || f.startsWith(from + '/')) pureFolders.value[i] = newPath + f.slice(from.length)
    }
    await savePureFolders()
    folderRefresh.value++
    await loadAllConnections()
    await buildTree()
    ElMessage.success(t('mv.renamedGroups', { name: newPath })
    + (targets.length ? t('mv.renamedGroupsConns', { n: targets.length }) : ''))
  } catch (e) {
    ElMessage.error(t('mv.renameFailed', { detail: (e?.message || e?.toString?.() || t('common.unknownError')) }))
  }
}

// 兼容 FolderDialogs 的创建回调（那个弹窗现在只是"想手输名字"时的备用入口）：
// 主路径已改为「直接建默认名 + 就地改名」，不再走弹窗 —— 但回调保留，免得挂空。
const onFolderCreated = async (name) => {
  folderRefresh.value++
  await buildTree()
  if (name) ElMessage.success(t('mv.groupCreated', { name }))
}

// 新建分组：**不再弹窗问名字**，直接建一个默认名并立刻进入改名态（光标落在名字上，直接输入即可）。
// **多级目录**：env 现在可能是路径（`公司资源/研发`）—— 在它下面新建 = 建子目录，
// 新路径 = `父路径/默认名`，树上嵌套渲染（无限级）。
const treeCtxNewFolder = async (env) => {
  closeTreeCtxMenu()
  const base = env ? env + '/' + t('mv.newGroup') : t('mv.newGroup')
  let name = base
  let n = 2
  while (customFolders.value.includes(name) || PREDEF_ENVS.includes(name)) {
    name = base + n
    n++
  }
  try {
    await addPureFolder(name)
    folderRefresh.value++
    await buildTree()
    await startRenameGroup(name)
  } catch (e) {
    ElMessage.error(t('mv.createGroupFailed', { detail: (e?.message || e?.toString?.() || t('common.unknownError')) }))
  }
}

const treeCtxRenameFolder = (env) => {
  closeTreeCtxMenu()
  if (!env) return
  // 改走**树内联改名**（多级路径级联在 commitRenameGroup 里统一处理）；
  // 旧的弹窗改名不认识路径（改了父名子目录会散），不能再用
  startRenameGroup(env)
}
// 重命名分组（连接环境改写 / 目录落盘在 FolderDialogs.vue，成功后刷新连接列表与树）
const onFolderRenamed = async ({ old, new: renamed }) => {
  folderRefresh.value++
  await loadAllConnections()
  await buildTree()
  ElMessage.success(t('mv.groupRenamed', { old: envLabel(old), new: renamed }))
}

const treeCtxDeleteFolder = async (env) => {
  closeTreeCtxMenu()
  if (!env) return
  const name = envLabel(env)
  // 多级目录：删目录连**子目录**一起删（连接移到未分组，不删连接本身）
  const targets = allConnections.value.filter(c => {
    const g = c.group || ''
    return g === env || g.startsWith(env + '/')
  })
  try {
    await ElMessageBox.confirm(
      t('mv.deleteGroupBody', { name, n: targets.length }),
      t('mv.deleteGroupTitle'), { type: 'warning', confirmButtonText: t('common.delete'), cancelButtonText: t('common.cancel') }
    )
  } catch { return }
  try {
    for (const c of targets) {
      await saveConnection({ ...c, group: '' })
    }
    // 同步后端纯目录（子目录一并移除）
    pureFolders.value = pureFolders.value.filter(f => !(f === env || f.startsWith(env + '/')))
    await savePureFolders()
    folderRefresh.value++
    await loadAllConnections()
    await buildTree()
    ElMessage.success(t('mv.groupDeleted', { name }))
  } catch (e) { ElMessage.error(t('mv.deleteFailed', { detail: (e?.message || e?.toString?.() || t('common.unknownError')) })) }
}

const treeCtxEditConn = async (connId) => {
  closeTreeCtxMenu()
  const c = allConnections.value.find(x => x.id === connId)
  if (c) openConnDialog(c)
}
const treeCtxTestConn = async (connId) => {
  closeTreeCtxMenu()
  testing.value = true
  try {
    const res = await testConnectionById(connId)
    if (res.success) ElMessage.success(t('mv.connectOk', { version: (res.serverVersion || '') }))
    else ElMessage.error(t('mv.connectFailed', { detail: res.message }))
    } catch (e) { ElMessage.error(t('mv.testFailed', { detail: errMsg(e) })) }
  finally { testing.value = false }
}
const treeCtxDeleteConn = async (connId) => {
  closeTreeCtxMenu()
  const c = allConnections.value.find(x => x.id === connId)
  if (!c) return
  try {
    await ElMessageBox.confirm(t('mv.deleteConnBody', { name: c.name }), t('mv.deleteConfirmTitle'), { type: 'warning' })
  } catch { return }
  // 单条删除也是"已交给后端"，没法中途取消；秒级的话窗口根本不会出现（延迟 400ms）
  dangerBegin(t('mv.deleteConnPhase', { name: c.name }), c.name, { message: t('mv.deleteSubmitted') })
  try {
    await deleteConnection(connId)
    ElMessage.success(t('mv.deleted'))
    // 清理该连接的状态缓存；页签也跟着关 —— 连接都删了，它的表页签留着也取不到数据
    const sid = String(connId)
    delete dbsByConn.value[sid]
    delete currentDbByConn.value[sid]
    closeTabsOfConn(sid)
    const remaining = allConnections.value.filter(x => x.id !== connId)
    allConnections.value = remaining
    await buildTree()
    if (!remaining.length) {
      conn.value = null
      currentConnId.value = null
      sessionStorage.removeItem('dbmind.currentConnId')
    } else if (String(connId) === currentConnId.value) {
      // 删除的是当前连接：静默切换到第一个剩余连接（不展开树、不测试连接）
      const next = remaining[0]
      const nid = String(next.id)
      conn.value = next
      currentConnId.value = nid
      sessionStorage.setItem('dbmind.currentConnId', nid)
      dbs.value = dbsByConn.value[nid] || []
      currentDb.value = currentDbByConn.value[nid] || ''
    }
    dangerFinish(true, t('mv.connDeleted', { name: c.name }))
  } catch (e) {
    dangerFinish(false, t('mv.deleteFailed', { detail: (e?.message || e) }))
    ElMessage.error(t('mv.deleteFailed', { detail: (e?.message || e) }))
  }
}

const treeCtxOpenConn = async (connId) => {
  closeTreeCtxMenu()
  await selectConn(connId)
}
// 真正关闭连接：清除该连接状态（图标变暗）、折叠树并清空子节点。
// 页签不跟着关 —— 关掉的只是连接，用户开着的表 / SQL 不该凭空消失（页签只认自己的 ×）
const closeConnection = (connId) => {
  const sid = String(connId)
  // 顺带断掉内核连接池里这个类型的缓存会话：数据库侧改过权限/密码后，
  // 旧会话还带着旧的全局权限快照（MySQL 的全局权限变更只对新建连接生效），
  // 断开重连即可拿到新权限 —— 不必重启应用。失败无害（静默，下次会话照样能用）
  disconnectSessions(connId).catch(() => {})
  const wasCurrent = currentConnId.value === sid
  // 删除 dbsByConn 是必须的：否则 isConnOpen 仍为 true，图标不会变暗
  delete dbsByConn.value[sid]
  delete currentDbByConn.value[sid]
  // 清理错误标记，允许下次重新打开时正常提示
  connErrorSet.value.delete(sid)
  // 折叠树并清除子节点数据（下次展开会重新连接加载）
  const node = treeRef.value?.getNode('conn:' + connId)
  if (node) {
    node.expanded = false
    node.loaded = false
    node.childNodes = []
    if (node.data) node.data.children = []
  }
  if (wasCurrent) {
    conn.value = null
    currentConnId.value = null
    currentDb.value = ''
    dbs.value = []
    sessionStorage.removeItem('dbmind.currentConnId')
  }
}
const treeCtxCloseConn = async (connId) => {
  closeTreeCtxMenu()
  const sid = String(connId)
  // 「关不掉」的真相：isConnOpen 认页签 —— 只要有脚本占着这个连接，
  // 树上的图标就永远亮着，看起来就是"关了没反应"。
  // 所以关闭连接前先把占用的页签摆上台面：未保存的走「保存 / 不保存 / 取消」三态
  //（口径、按钮文案与关窗时完全一致），用户选完再关页签 + 关连接。
  const owned = tabs.value.filter(t => !isGlobalTab(t) && String(t.connId || '') === sid)
  if (!owned.length) return closeConnection(sid)
  const dirty = owned.filter(t => t.type === 'sql' && t.dirty)
  if (!dirty.length) {
    // 都已保存：只需确认"有 N 个脚本页签会一并关闭"
    const names = owned.map(tb => t('common.quoted', { name: tabLabel(tb) })).join(t('common.listSep'))
    try {
      await ElMessageBox.confirm(t('mv.closeConnTabs', { n: owned.length, names }), t('mv.closeConnTitle'), {
        type: 'info',
        confirmButtonText: t('mv.closeConnOk'),
        cancelButtonText: t('common.cancel'),
        closeOnClickModal: false
      })
    } catch { return }
    closeTabsOfConn(sid)
    closeConnection(sid)
    return
  }
  const names = dirty.map(tb => t('common.quoted', { name: tabLabel(tb) })).join(t('common.listSep'))
  const head = dirty.length === 1
    ? t('mv.oneDirty', { names })
    : t('mv.manyDirty', { n: dirty.length, names })
  let action
  try {
    await ElMessageBox.confirm(t('mv.closeAsk', { head }), t('mv.unsavedTitle'), {
      type: 'warning',
      confirmButtonText: t('mv.saveAndClose'),
      cancelButtonText: t('mv.dontSave'),
      distinguishCancelAndClose: true,
      showClose: true,
      closeOnClickModal: false
    })
    action = 'save'
  } catch (e) {
    // cancel = 点「不保存」→ 弃用改动继续关；close = 点 X / ESC → 取消，连接保持打开
    action = e === 'cancel' ? 'discard' : 'cancel'
  }
  if (action === 'cancel') return
  if (action === 'save') {
    // 与 winClose 同款：逐个走页签保存，任何一个没保存成就中断（并把它切到前台）
    for (const tb of dirty) {
      let ok = true
      try { ok = sqlViewRefs[tb.id] ? await sqlViewRefs[tb.id].saveForClose() : true } catch { ok = false }
      if (ok === false) {
        activeTab.value = tb.id
        return
      }
    }
  }
  closeTabsOfConn(sid)
  closeConnection(sid)
}
// ====== 发送到 AI（连接 / 库 / 表 三类节点）======
/**
 * 把树里选中的对象**带着上下文**送进 AI 助手：切到对话页签并填好输入，**不自动发送** ——
 * 既不替用户花掉一次调用，也让他把问题补完整（这是 `AiPanel.openWith` 的既定约定）。
 *
 * <p>为什么把上下文写在文本里，而不是只靠面板自己的连接选择器：
 * 面板的默认选择跟的是**左侧树当前选中项**，而右键的节点可能不是当前选中项 ——
 * 只靠选择器会出现"我说的是 A 表、它按 B 库回答"。所以文本里显式带上连接 / 库 / 对象。
 */
const sendNodeToAi = async (data) => {
  // 两个菜单都关：表 / 视图 / 对象走 `.ctx-menu`，连接 / 库 / 分组走 `.tree-ctx-menu`
  closeTreeCtxMenu()
  closeCtxMenu()
  if (!data) return
  const conn = connFor(data.connId)
  const connLabelText = [conn?.name, conn?.type].filter(Boolean).join(' · ')
  const db = String(data.db || (data.kind === 'db' ? data.label : '') || '')
  const name = String(data.table || data.label || '')
  const ctx = []
  if (connLabelText) ctx.push(t('mv.ctxConn', { name: connLabelText }))
  if (db) ctx.push(t('mv.ctxDb', { name: db }))
  if (data.kind === 'table' || data.kind === 'view') ctx.push(t('mv.ctxObject', { name: (db ? db + '.' : '') + name }))
  const head = ctx.length ? ctx.join('，') + '\n' : ''
  const ask = {
    table: t('mv.aiAskTable', { name: (db ? db + '.' : '') + name }),
    view: t('mv.aiAskView', { name: (db ? db + '.' : '') + name }),
    db: t('mv.aiAskDb', { name: db }),
    conn: t('mv.aiAskConn', { name: (conn?.name || name) })
  }[data.kind] || t('mv.aiAskGeneric', { name })
  // ① 把「当前连接 / 当前库」切到**右键那个对象所在的库**，再开面板。
  //    AI 面板的上下文默认跟随"左侧当前选中项"（`:conn` / `:database`）——
  //    不切的话会出现"问题里写着 dify、下面下拉却是 test"这种自相矛盾（实测抓到的一幕）。
  //    口径与单击库节点完全一致（见 onNodeClick 里同一段注释）。
  if (db) {
    const connChanged = !!data.connId && String(data.connId) !== String(currentConnId.value)
    if (connChanged) ensureDbConn(data)
    const dbChanged = db !== currentDb.value
    if (dbChanged) setCurrentDb(db)
    if (connChanged || dbChanged) await refreshAllTables()
  }
  // ② 开的是**中央工作区页签**（`ai:studio`），不是右侧窄面板：
  //    「分析这张表 / 这个库」是要一块大地方看结果的场景；右侧窄栏留给 Ctrl/⌘+K 的"边看数据边问"。
  //    seq 自增 → 同一张表连点两次也会重新填一次（只比文本的话第二次不响应）。
  // `table` 一并带过去：面板的上下文标签会显示到「表」这一级
  // （只到库的话，面板写着 dify、问题里说的却是 dify.orders，等于又缺一层）
  const tableForCtx = (data.kind === 'table' || data.kind === 'view') ? name : ''
  pendingAi.value = { seq: (pendingAi.value?.seq || 0) + 1, text: head + ask, table: tableForCtx }
  toggleAi()
}

// ====== 多选节点批量操作（Ctrl / ⌘ + 单击进入多选）======
// 为什么要有：连接 / 表一多，"连开几条连接""把这几条导出""删掉这几条"都得一条条点。
// 只认 Ctrl/⌘ + 单击，**不改"普通单击 = 打开对象"** 这个已经用熟的手感。
const multiSel = ref([])
const batchBusy = ref(false)
const isMultiSelected = (data) => multiSel.value.some(d => d.id === data?.id)
const toggleMultiSelect = (data) => {
  if (!data?.id) return
  const i = multiSel.value.findIndex(d => d.id === data.id)
  if (i >= 0) multiSel.value.splice(i, 1)
  else multiSel.value.push(data)
}
const clearMultiSelect = () => { multiSel.value = [] }

/** 多选下「打开」的语义 = 双击打开那一套（叶子开预览、连接去连接、库切库） */
const openOneNode = async (data) => {
  if (data.kind === 'conn') await selectConn(data.connId)
  else if (data.kind === 'table' || data.kind === 'view') await openTable(data)
  else if (data.kind === 'collection') openNoSql(data)
  else if (data.kind === 'db' && data.connType === 'REDIS') openRedisDb(data)
  else if (data.kind === 'db' || data.kind === 'schema') {
    // 切库那套（与 onNodeClick 同一口径）：先切连接、再切库、然后重拉表清单
    const connChanged = !!data.connId && String(data.connId) !== String(currentConnId.value)
    if (connChanged) ensureDbConn(data)
    const dbChanged = data.db !== currentDb.value
    if (dbChanged) setCurrentDb(data.db)
    if (connChanged || dbChanged) await refreshAllTables()
  } else if (data.kind === 'script') openScript(data)
  else if (['function', 'procedure', 'trigger', 'event'].includes(data.kind)) openObject(data, data.kind, data.label)
  else if (data.kind === 'user') openUserTab(data.objectName, { connId: data.connId, db: data.db })
}

const batchOpen = async () => {
  const picked = multiSel.value.slice()
  if (!picked.length) return
  batchBusy.value = true
  let ok = 0
  for (const d of picked) {
    try { await openOneNode(d); ok++ } catch { /* 单条失败不打断整批 */ }
  }
  batchBusy.value = false
  clearMultiSelect()
  ElMessage.success(t('mv.openedCount', { ok, total: picked.length }))
}

/** 导出：只导连接 —— 导出格式就是"连接配置包"，表数据不属于它。
 *  混选了别的节点时不假装能导，明确告诉用户只导了哪几条连接、跳过了几个。 */
const batchExport = () => {
  const pickedConns = multiSel.value.filter(d => d.kind === 'conn')
  const connsPicked = pickedConns
    .map(d => allConnections.value.find(c => String(c.id) === String(d.connId)))
    .filter(Boolean)
  if (!connsPicked.length) {
    ElMessage.warning(t('mv.exportOnlyConns'))
    return
  }
  downloadJson('dbmind-connections-' + new Date().toISOString().slice(0, 10) + '.json',
    buildConnectionBundle(connsPicked))
  const skipped = multiSel.value.length - connsPicked.length
  ElMessage.success(t('mv.exportedConns', { n: connsPicked.length }) + (skipped ? t('mv.exportSkipped', { n: skipped }) : ''))
  clearMultiSelect()
}

/** 删除：只删**连接**（表 / 库的删除是 DDL，属于危险操作，各走各的入口）。
 *  确认框**只弹一次** —— 每条都弹一次的话，批量就失去意义了。 */
const batchDelete = async () => {
  const picked = multiSel.value.filter(d => d.kind === 'conn')
  const ids = picked.map(d => String(d.connId))
  if (!ids.length) {
    ElMessage.warning(t('mv.batchDeleteOnlyConns'))
    return
  }
  const names = picked.map(d => d.label || d.name || d.connId).join(t('common.listSep'))
  try {
    await ElMessageBox.confirm(t('mv.batchDeleteBody', { n: ids.length, names }),
      t('mv.batchDeleteTitle'), { type: 'warning' })
  } catch { return }
  batchBusy.value = true
  // 批量：循环在**前端**，所以取消是真的停（这一点和单个 DDL 不同，要给取消按钮）
  dangerBegin(t('mv.batchDeletePhase'), t('mv.itemsUnit', { n: ids.length }), {
    cancellable: true,
    message: t('mv.batchDeleteNote')
  })
  let ok = 0
  for (const id of ids) {
    if (dangerCanceled.value) break
    const one = picked.find(d => String(d.connId) === id)
    const label = one?.label || one?.name || id
    dangerLog(t('mv.deletingItem', { name: label, i: ok + 1, n: ids.length }))
    try {
      await deleteConnection(id)
      delete dbsByConn.value[id]
      delete currentDbByConn.value[id]
      closeTabsOfConn(id)
      ok++
    } catch (e) { dangerLog(t('mv.failItemLong', { name: label, detail: errMsg(e) })) }
  }
  batchBusy.value = false
  clearMultiSelect()
  await loadAllConnections()
  // 当前连接被删掉了：清空右侧状态，别让界面停在一个已经不存在的连接上
  if (!allConnections.value.some(c => String(c.id) === String(currentConnId.value))) {
    conn.value = null
    currentConnId.value = null
    sessionStorage.removeItem('dbmind.currentConnId')
  }
  await buildTree()
  ElMessage.success(t('mv.deletedCount', { ok, total: ids.length }) + (dangerCanceled.value ? t('mv.deletedCountCanceled') : ''))
  dangerFinish(ok === ids.length || dangerCanceled.value,
    t('mv.deletedCount', { ok, total: ids.length }) + (dangerCanceled.value ? t('mv.deletedCountUserCanceled') : ''))
}

const treeCtxCopyConn = async (connId) => {
  closeTreeCtxMenu()
  try {
    const original = allConnections.value.find(x => String(x.id) === String(connId))
    if (!original) return
    // 口令不出服务端：复制动作由后端完成（前端只给一个新名字）
    await copyConnection(connId, t('mv.connCopyName', { name: (original.name || t('mv.conn')) }))
    await loadAllConnections()
    await buildTree()
    ElMessage.success(t('mv.connCopied'))
  } catch (e) {
    ElMessage.error(t('mv.copyConnFailed', { detail: (e?.message || e) }))
  }
}
const treeCtxNewDb = (connId) => {
  closeTreeCtxMenu()
  const c = allConnections.value.find(x => String(x.id) === String(connId))
  newDbCtx.value = { connId, connType: c?.type || '' }
  newDbVisible.value = true
}
// 新建数据库（表单与建库 SQL 在 NewDbDialog.vue，创建成功后刷新该连接节点）
const onNewDbCreated = async (connId) => {
  // 刚建出来的库要能被看到：先丢掉该连接的持久结构缓存（库列表 + 各库表清单），
  // 否则重建节点时 lazyLoad 又会用旧缓存把「没有新库」的那份列表画回来
  invalidateSchemaCache(String(connId))
  const node = treeRef.value?.getNode('conn:' + connId)
  if (node) {
    node.loaded = false
    node.childNodes = []
    if (node.expanded) {
      node.expand(() => {})
    }
  }
  if (currentConnId.value === connId) {
    await refreshTree()
  }
}
const treeCtxNewQueryConn = async (connId) => {
  closeTreeCtxMenu()
  if (currentConnId.value !== connId) {
    await selectConn(connId)
  }
  newQueryTab({ connId: String(connId), database: currentDb.value })
}
const treeCtxRefreshConn = async (connId) => {
  closeTreeCtxMenu()
  // 先清掉这个连接的库列表缓存。不清的话，下面 `node.loaded = false` 触发的重新加载
  // 会**直接命中缓存**、把同一份旧列表原样画回来 —— 「刷新」变成一句空话，
  // 却还提示「已刷新」（实测踩过：MySQL 连接只显示一个 (default)，点刷新毫无变化）。
  delete dbsByConn.value[String(connId)]
  // 持久缓存（localStorage）那份也要清，且要连 `tables:` 一起清：
  // 「刷新连接」的语义是「这个连接下的东西我都要重看」，只清内存里那份等于没清。
  invalidateSchemaCache(String(connId))
  const node = treeRef.value?.getNode('conn:' + connId)
  if (node) {
    node.loaded = false
    node.childNodes = []
    if (node.expanded) {
      node.expand(() => {})
    }
  }
  if (currentConnId.value === connId) {
    await refreshTree()
  }
  ElMessage.success(t('mv.refreshed'))
}

// 打开指定连接：切换上下文并展开树（多连接并开，各自状态互不干扰）
const selectConn = async (id) => {
  if (!id) { conn.value = null; currentConnId.value = null; return }
  const c = allConnections.value.find(x => x.id === id)
  if (!c) return
  const connId = String(c.id)
  // 正在连接中，忽略重复点击
  if (connLoadingSet.value.has(connId)) return
  connLoadingSet.value.add(connId)
  const prevConnId = currentConnId.value
  const prevConn = conn.value
  // **连上之前不展开**：以前是"先展开、在节点里显示加载中、失败再收起"，结果是
  // 连不上时用户先看到一个展开的空节点（里面那行"加载中…"），然后再自己缩回去 ——
  // 看起来像点了两下。现在"正在连"只由**箭头位置的转圈**表达（见 isNodeLoading），
  // 节点保持收起；连上了再展开（见下面成功分支里的 expandNode）。
  saveConnState()
  conn.value = c
  currentConnId.value = connId
  sessionStorage.setItem('dbmind.currentConnId', connId)
  // 页签栏是全局的：换连接只换「当前库 / 库列表 / 树高亮」，页签原封不动
  dbs.value = dbsByConn.value[connId] || []
  // 同上：库里已经有该连接的库列表时先校验一遍缓存值，避免切回来的瞬间先显示一个失效的库名
  // （库列表还没加载过的情况交给 lazyLoad 里的校验处理）
  const cachedDb = currentDbByConn.value[connId] || ''
  currentDb.value = (dbs.value.length && !dbs.value.includes(cachedDb)) ? '' : cachedDb
  const rollback = () => {
    connErrorSet.value.add(connId)
    collapseNode('conn:' + c.id)
    // 只回收「展开缓存」这类派生状态，页签是用户自己开的东西，一律不动
    delete dbsByConn.value[connId]
    delete currentDbByConn.value[connId]
    if (prevConnId && prevConn) {
      conn.value = prevConn
      currentConnId.value = prevConnId
      dbs.value = dbsByConn.value[prevConnId] || []
      currentDb.value = currentDbByConn.value[prevConnId] || ''
      sessionStorage.setItem('dbmind.currentConnId', prevConnId)
    } else {
      conn.value = null
      currentConnId.value = null
      sessionStorage.removeItem('dbmind.currentConnId')
    }
  }
  try {
    // **不再单独"测试连接"**：直接用**列库**这一次真实请求当验证。
    //
    // 为什么：测试连接是一次独立往返，在有些服务器上要等 3~10 秒（本机实测 6.3 秒），
    // 紧接着还要第二次往返去列库 —— 两次等待叠在一起，就是"打开连接慢"的主因。
    // 列库本身就是一次真实的会话操作：成功即证明连接可用；失败就回滚并把错误如实说出来。
    const dbList = await (isNoSqlType(c.type) ? noSqlDatabases(connId) : listDatabases(connId))
    const list = Array.isArray(dbList) ? dbList : []
    // **必须自己识别"软失败"**：后端列不出库时**不报错**，而是 200 返回占位名
    // （`(default)` / 本地文件的 `main` 是真名，不能算失败）。不识别的话会把
    // "连不上"当成"连接成功"，还会写进一个空缓存 —— 那正是之前"展开了却没东西"的老毛病。
    // NoSQL 的 `(default)` 是能用的伪库（key/集合挂在它下面），所以对它不设这道闸。
    const usable = list.filter(db => isNoSqlType(c.type) || String(db) !== NO_DEFAULT_DB)
    if (!usable.length) {
      rollback()
      // 只有失败时才回头问一次"为什么"：正常路径省掉那次往返，失败路径仍要给出准确原因
      let why = ''
      try {
        const probe = await testConnectionById(c.id)
        why = probe?.message || ''
      } catch (e2) {
        why = errMsg(e2, '')
      }
      showConnFail(why || t('mv.noDatabases'))
      return
    }
    // 顺手落进缓存：下面 expandNode 触发的 lazyLoad 会直接命中它，不再重复请求一次
    dbsByConn.value[connId] = list
    // 校验"上次选中的库"是否还在（库可能已被删/改名）：不在就回退到连接配置里的库，
    // 都没有就不选任何库 —— 与 lazyLoad 里同一套口径，避免出现一个不存在的"当前库"。
    const cachedDb = currentDbByConn.value[connId]
    const configuredDb = String(c.database || '').trim()
    if (!cachedDb || !dbsByConn.value[connId].includes(cachedDb)) {
      currentDbByConn.value[connId] =
        (configuredDb && dbsByConn.value[connId].includes(configuredDb)) ? configuredDb : ''
    }
    if (String(currentConnId.value) === connId) {
      dbs.value = dbsByConn.value[connId]
      setCurrentDb(currentDbByConn.value[connId])
    }
    // 连接可用：清除之前的错误标记，展开该节点（列库的子节点走 lazyLoad 的缓存分支）
    connErrorSet.value.delete(connId)
    // 连接成功才记「最近使用」——失败的不记，免得最近列表里全是连不上的
    recordRecentConnection(connId)
    await nextTick()
    expandNode('conn:' + c.id)
  } catch (e) {
    rollback()
    showConnFail(errMsg(e, t('mv.connFailedCheck')))
  } finally {
    connLoadingSet.value.delete(connId)
  }
}

// ConnectionDialog 保存回调
const onConnSaved = async (savedConn) => {
  connDialogVisible.value = false
  await loadAllConnections()
  // 立即重建左侧树，保证新建/编辑后的数据源无需刷新页面即可显示在树中
  await buildTree()
  if (editingConn.value) {
    // 编辑：若修改了当前连接，重新加载
    if (conn.value && editingConn.value.id === conn.value.id) {
      await selectConn(editingConn.value.id)
    }
  } else {
    // 新建：展开新连接所在分组并自动选中，确保用户无需手动刷新/展开即可看到
    const created = savedConn || allConnections.value[allConnections.value.length - 1]
    if (created) {
      const envKey = 'env:' + (created.group || '')
      if (!defaultExpanded.value.includes(envKey)) {
        expandNode(envKey)
      }
      await selectConn(created.id)
      // 若新分组位于视口外，滚动到新连接所在节点
      try {
        const el = treeRef.value?.$el?.querySelector('.el-tree-node[data-key="conn:' + created.id + '"]')
        el?.scrollIntoView?.({ block: 'nearest' })
      } catch { /* 滚动失败不影响保存 */ }
    }
  }
}

// ====== 环境分组 / 树构建 ======
// （PREDEF_ENVS / ENV_ORDER_BASE / envLabel / envColor / envShort / envTitle 已统一收敛到 src/utils/envs.js）

// 读取某连接某库下保存的脚本（localStorage，按连接+库隔离）
const getDbScripts = (connId, db) => {
  try {
    const raw = localStorage.getItem(`dc_scripts:${connId}:${db}`)
    return raw ? JSON.parse(raw) : []
  } catch { return [] }
}

// 连接失败统一弹窗（selectConn 校验失败 / 树连接节点懒加载失败共用，避免 selectConn 已弹过后重复打扰）
// 与「无法加载」保持同一套关闭方式：右上角 X + 「知道了」
const showConnFail = (msg) => {
  ElMessageBox.alert(String(msg || t('mv.connFailedCheck')), t('mv.connFailedTitle'), {
    type: 'error',
    showClose: true,
    showConfirmButton: true,
    confirmButtonText: t('mv.gotIt'),
    closeOnClickModal: true,
    closeOnPressEscape: true,
    customClass: 'dc-conn-error-box'
  }).catch(() => {})
}

// 屏幕居中的错误弹窗（树节点懒加载的权限/连接错误等）
// 必须留出关闭方式：右上角 X + 「知道了」按钮，点遮罩 / 按 ESC 也能关，
// 否则弹出来就只能卡在那里（曾经是 showClose + showConfirmButton 都为 false）
const showErrorModal = (msg) => {
  ElMessageBox.alert(String(msg || t('mv.actionFailed')), t('mv.loadFailedTitle'), {
    type: 'error',
    showClose: true,
    showConfirmButton: true,
    confirmButtonText: t('mv.gotIt'),
    closeOnClickModal: true,
    closeOnPressEscape: true,
    customClass: 'dc-conn-error-box'
  }).catch(() => {})
}

// 每个连接的能力（后端 `/features`）：树按它决定显示哪些分类、以及要不要去调对应接口。
//
// 以前树是**无条件**画 7 个分类、无条件并发调 5 个 list 接口 —— 于是 SQL Server 上
// 永远挂着一个 0 的 Events、SQLite 上挂着一个点开就报「未实现」的 Procedures。
// 取不到能力时按「全支持」处理：一次请求失败不该把树削秃。
const featuresByConn = ref({})
const loadFeatures = async (connId) => {
  const key = String(connId || '')
  if (!key) return {}
  if (!featuresByConn.value[key]) {
    let value = {}
    try {
      value = (await getFeatures(key)) || {}
    } catch (e) {
      value = {}
    }
    featuresByConn.value[key] = value
  }
  return featuresByConn.value[key]
}
// 模板里按能力决定某个菜单项要不要出现（`!== false` 让"没这个字段"也算支持）
const featureOf = (connId, key) => {
  const f = featuresByConn.value[String(connId || currentConnId.value || '')]
  return f ? f[key] !== false : true
}
// 某个对象类型能不能「查看 / 编辑」——靠后端 features 的 objectDdlKinds（= 有 object_source）。
// ClickHouse / DB2 / H2 / Derby 取不到定义，以前右键点下去必然报「未实现」。
// 老后端没有这个字段时按"支持"处理（别把功能藏了）。
const objectDdlAllowed = (connId, kind) => {
  const f = featuresByConn.value[String(connId || currentConnId.value || '')]
  if (!f || !Array.isArray(f.objectDdlKinds)) return true
  return f.objectDdlKinds.includes(kind)
}
const objectCategoriesOf = (features) => ({
  procs: features.supportsProcedures !== false,
  // 函数与存储过程同源（同一条 information_schema.routines），所以**同开同关**：
  // 有例程的类型才画出 Functions。漏掉这一项的话，SQLite 这种"没有例程概念"的库
  // 也会多出一个恒为 0 的 Functions 分类（`!== false` 对 undefined 是成立的）。
  functions: features.supportsProcedures !== false,
  triggers: features.supportsTriggers !== false,
  events: features.supportsEvents !== false,
  users: features.supportsUsers !== false
})

// 懒加载某连接的单个库：构建该库下的表/视图/索引/过程/触发器/事件/脚本节点
// （节点 id 全部带 connId 前缀，保证多连接下 node-key 全局唯一）
const buildDbDetailNodes = async (dbNode) => {
  const { connId, connType, db } = dbNode
  const isNosql = isNoSqlType(connType)
  if (isNosql) {
    // Redis 不在树里展开 key，点击 db 后在右侧列表展示
    if (byType(connType).code === 'REDIS') {
      return []
    }
    const collections = await noSqlCollections(connId, db).catch((e) => {
      const msg = errMsg(e, t('mv.loadCollectionsFailed'))
      showErrorModal(msg)
      return []
    })
    return (Array.isArray(collections) ? collections : []).map(col => ({
      id: 'nosql:' + connId + ':' + db + ':' + col, label: col, kind: 'collection',
      collection: col, database: db, connId, connType
    }))
  }
  // 需要 schema 层级的数据库（SQL Server / PostgreSQL / KingbaseES 等）：库下先展示 schema
  if (schemaLevelOf(connType) === 'schema') {
    try {
      const schemas = await listSchemas(connId, db)
      return (Array.isArray(schemas) ? schemas : []).map(s => ({
        id: 'schema:' + connId + ':' + db + ':' + s,
        label: s,
        kind: 'schema',
        db: db + '.' + s,
        realDb: db,
        schema: s,
        connId,
        connType,
        leaf: false
      }))
    } catch (e) {
      const msg = e.message || t('mv.loadSchemaFailed')
      showErrorModal(msg)
      return []
    }
  }
  // 关系型（非 SQL Server / 非 PG）
  const safeArr = (v, label) => {
    if (!Array.isArray(v)) {
      console.warn(`[buildDbDetailNodes] ${label} 返回非数组:`, v)
      return []
    }
    return v
  }
  // 按连接能力决定哪些接口要调：该类型没有的对象（SQLite 的过程、SQL Server 的事件）
  // 调了只会拿到 501/空，白等一轮
  const can = objectCategoriesOf(await loadFeatures(connId))
  // **只建分类壳，不查任何对象**：这几条查询原来在这里一次性并发发出，
  // 其中 listTables 在有些库上要十几秒（实测 `/tables?database=mysql` = 10.96 秒）——
  // 用户只是想展开库看看有哪些分类，却被最慢的那条挡住。现在某个分类展开时才按需加载
  // （见 lazyLoad 的 category 分支 + loadCategoryItems）。
  const built = buildObjectCategories({ connId, db })
  // 下面原有的"行数回填"仍按旧签名调用：此时没有表可填（真回填已移到 Tables 分类加载时）。
  const tables = []
  // 行数先按 listTables 的估算值渲染，随后异步用真实 COUNT(*) 回填（Doris/ClickHouse 的 TABLE_ROWS 常为 0）
  // 仅对「当前库」回填，避免每展开一个库都对全库逐表 COUNT(*)（后端已按 15s 缓存，但仍应避免无谓抖动）
  if (inCurrentDb(db)) fillRealRowCounts(connId, db, tables)
  // 该类型没有的分类不画（以前 7 个分类无条件画满）
  const nodes = built.nodes.filter(node => can[node.cat] !== false)
  // 壳先返回（不挡展开），清单在后台并行取回 —— 数量不必等用户点开才出现，
  // 且展开那一类时数据已在手上（见 prefetchCategoryItems）
  prefetchCategoryItems(nodes)
  return nodes
}

// 懒加载 schema 节点（SQL Server / PostgreSQL / KingbaseES 等）：拉取 schema 下对象后由 treeNodes.js 组装分类节点
/**
 * 分类计数（节点 id → 个数）。**响应式表**，模板直接读它。
 *
 * 为什么不只写在节点对象上：实测（真浏览器里）预取请求全部 200 且有数据，但计数元素
 * 始终不出现 —— 说明"我 mutate 的节点对象"与"模板渲染用的那一份"并不总是同一个，
 * 依赖对象身份就等于把显示寄托在一个看不见的假设上。这张表由模板直接观察，稳定。
 */
const catCounts = ref({})

/** 表的精确行数（节点 id → 行数）：COUNT(*) 回填后写这里，模板直接观察（同分类计数的道理） */
const tableRows = ref({})

/**
 * 后台并行预取每个分类的清单（展开库时立刻发起，不等用户点开）。
 *
 * 为什么要有它：分类是懒加载的（"展开库只出壳"是有意为之 —— 某些库 `/tables` 要十几秒，
 * 一次性并发会把整个展开卡住），但"懒"的代价是**计数要用户先点开才出现**，
 * 看起来就像"数量都没出来"。这两件事其实可以兼得：
 *
 *  - 壳照旧**立刻**返回（展开依旧是快的，预取在后台跑，失败也只是没有数字）；
 *  - 拿到清单就写 `catCount` 并**存下 items**，于是——
 *    ① 数字自己出现，用户不必逐类点开；
 *    ② 真去点开时 `lazyLoad` 直接用这份缓存，**秒开**，不再等一次远程查询。
 *
 * 不缓存到模块级别的 Map：items 挂在节点上，而节点在刷新时会重建 —— 缓存随之失效，
 * 不必再维护一份需要手动清理的状态。
 */
const prefetchCategoryItems = (nodes) => {  nodes.forEach(node => {
    if (node.catLoaded || node.catItems) return
    const attempt = (left) => {
      loadCategoryItems(node)
        .then(items => {
          // 只有**拿到确定清单**才写数字（`Array.isArray` 是这道门）：
          // - 拿到空数组 ⇒ 写 **0**，这是事实（"这一类就是没有对象"），必须显示出来；
          // - 没拿到（非数组，走的是下面的 catch）⇒ **一个数字都不写**，
          //   界面上就是"还没取到"，与"真的是 0"分得开。
          if (!Array.isArray(items)) return
          node.catItems = items
          node.catCount = items.length
          // 写进**响应式表**：模板直接观察它，不再指望"我 mutate 的那个节点就是渲染用的那一个"
          catCounts.value[node.id] = items.length
        })
        .catch(() => {
          // 失败**不留痕**：catLoaded 没置位 → 不显示数字，点开时照旧重新查
          if (left > 0) setTimeout(() => attempt(left - 1), 2000)
        })
    }
    // 展开库那一瞬间，连接/驱动往往才刚起步：实测第一批并发请求正是这样吃到 500
    //（控制台里留着 `读取表清单失败：No operations allowed after connection closed,
    // SQLState=08003`），于是"数字永远出不来、点开却是好的"。晚一拍发，失败再补两次。
    //
    // 从 1200ms 收到 300ms：当初那 500 的根因是「新会话要等 5 秒的反向 DNS」，
    // 现在（宿主自有 hosts + 首条会话预热）新建会话只要 ~300ms，再压 1.2 秒
    // 只会让用户多盯 1 秒空白。失败仍有两次补偿，不必靠"等得久"来规避。
    setTimeout(() => attempt(2), 300)
  })
}

/**
 * 只加载「被展开的那一个分类」的对象（展开库只出分类壳，展开分类才查）。
 * 失败时返回 `[]` 且**不置** `catLoaded` —— 分类上的计数保持隐藏，
 * 不会把"还没查/查失败"显示成一个扎眼的「0」。
 */
const loadCategoryItems = async (data) => {
  const { cat, connId, db } = data
  try {
    if (cat === 'tables' || cat === 'views') {
      // 持久缓存命中就立刻出清单（点开某库的 Tables 分类同样要付一次建连 5~10 秒，
      // 展开过的库不该再等），随后后台校准；表与视图共用同一份 /tables 结果。
      const cacheKey = 'tables:' + connId + ':' + db
      const hit = readSchemaCache(cacheKey)
      if (hit && Array.isArray(hit.value)) {
        const want0 = cat === 'tables' ? 'TABLE' : 'VIEW'
        const mine0 = hit.value.filter(t => (t.type || 'TABLE') === want0)
        if (cat === 'tables' && inCurrentDb(db)) allTables.value = mine0
        // ⚠️ 精确行数在这条分支里也必须发一次。
        //
        // 缓存命中（15 分钟内重复展开走的就是它）以前是**直接 return** 的，
        // 于是"清单来自缓存"时**永远不回填精确行数** —— 树上只剩估算值 `rows`，
        // 而 MySQL 的 `TABLE_ROWS` 对多数 InnoDB 表是 **NULL 而不是 0**：
        // 结果就是空表与"还没取到"长得一模一样，一个数字都没有。
        //
        // 实测（MySQL · dify）：`/table-count` 明明返回 {"boxoffice":0,…}，
        // 树上却**只有那张估算值非空的表**有数字 —— 精确值压根没被写进去。
        fillRealRowCounts(connId, db, mine0, true)
        data.catCount = mine0.length
        data.catItems = mine0
        data.catLoaded = true
        calibrateCategoryItems(connId, db, data, cat)
        return mine0
      }
      const tables = await listTables(connId, db)
      const list = Array.isArray(tables) ? tables : []
      writeSchemaCache(cacheKey, list)
      // `/tables` 一次返回**表和视图**（dify 实测 7 = 表 2 + 视图 5）。所以"这一类有多少个"
      // 必须按类型分开算：拿 list.length 去当表数，Tables 与 Views 会各显示 7 —— 数字看着
      // 有了却全是错的，比没有数字更坏。分类自己的清单也一并按类型收敛，展开与计数同源。
      const want = cat === 'tables' ? 'TABLE' : 'VIEW'
      const mine = list.filter(t => (t.type || 'TABLE') === want)
      if (cat === 'tables') {
        // 表名下拉 / 数据对比等场景要的就是「仅表（不含视图）」这一份
        if (inCurrentDb(db)) allTables.value = mine
        // 行数先按估算值渲染，随后异步用真实 COUNT(*) 回填（Doris/ClickHouse 的 TABLE_ROWS 常为 0）
        // 行数回填**不限定当前库**：Doris / ClickHouse 的 `TABLE_ROWS` 恒为 0（未 ANALYZE），
        // 限定当前库的结果就是——展开别的库时树上**完全没有数字**，只有点开某张表才有
        // （实测如此）。成本可控：只对展开的 Tables 分类发**一次**批量 COUNT(*)，
        // 后端对同一次统计还有缓存，不会因为多展开几个库就一直扫表。
        fillRealRowCounts(connId, db, mine, true)
      }
      data.catCount = mine.length
      data.catItems = mine
      data.catLoaded = true
      return mine
    }
    if (cat === 'scripts') { data.catLoaded = true; return getDbScripts(connId, db) }
    const can = objectCategoriesOf(await loadFeatures(connId))
    // 存储过程与函数来自**同一个**接口（按 routineType 区分），所以在这里就按分类收敛。
    // 与上面表/视图那一支同理：`catItems` 会直接被拿去算"这一类有多少个"，
    // 若把整份清单返回，Procedures 与 Functions 会显示**同一个总数** ——
    // 数字看着有了，其实两个都是错的，比不显示更糟。
    if ((cat === 'procs' || cat === 'functions') && can.procs) {
      data.catLoaded = true
      const routines = await listProcedures(connId, db)
      const list = Array.isArray(routines) ? routines : []
      return list.filter(o => (cat === 'functions' ? isFunctionRoutine(o) : !isFunctionRoutine(o)))
    }
    if (cat === 'triggers' && can.triggers) { data.catLoaded = true; return await listTriggers(connId, db) }
    if (cat === 'events' && can.events) { data.catLoaded = true; return await listEvents(connId, db) }
    if (cat === 'users' && can.users) { data.catLoaded = true; return await listUsers(connId, db) }
    // 该类型本来就没有这一类对象（SQLite 的过程、SQL Server 的事件…）：直接当"空但已确定"
    data.catLoaded = true
    return []
  } catch (e) {
    console.warn('[loadCategoryItems] 失败', cat, e)
    return []
  }
}

const buildSchemaDetailNodes = async (schemaNode) => {
  const { connId, db } = schemaNode
  // 同 buildDbDetailNodes：按连接能力决定哪些分类要显示（但不在这里查对象，见下）
  const can = objectCategoriesOf(await loadFeatures(connId))
  // **只建分类壳，不查任何对象**：这几条查询以前在这里一次性并发发出，
  // 其中 listTables 在有些库上要十几秒（实测 `/tables?database=mysql` = 10.96 秒）——
  // 用户只是想展开看看有哪些分类，却被最慢的那条挡住。现在展开某个分类时才按需加载
  // （见 lazyLoad 的 category 分支与 loadCategoryItems）。
  // 注意 db 是「库.schema」：分类 id 与后续按需查询都原样用它，不能改名。
  const built = buildObjectCategories({ connId, db })
  const nodes = built.nodes.filter(node => can[node.cat] !== false)
  // ⚠️ 必须和 buildDbDetailNodes 一样发起预取，否则 schema 层级的类型
  // （SQL Server / PostgreSQL / KingbaseES）**分类上永远不显示数量** ——
  // 它们走的是这条路径，而计数只由 prefetchCategoryItems 写进 `catCounts`。
  // 实测：SQL Server 的 Database_1 展开后 6 个分类全是光秃秃的，而 MySQL 有数字。
  prefetchCategoryItems(nodes)
  return nodes
}

// 顶栏「数据」下拉菜单命令分发
const onTopCmd = (cmd) => {
  switch (cmd) {
    case 'compare': openCompare(); break
    case 'sync': syncDialogOpen.value = true; break
  }
}

// 构建完整树：环境分组 → 数据源（连接节点的库/表在展开时懒加载）
// 重建前后记录并恢复展开状态，保证复制/删除/刷新等操作不改变树的展开与收起
const buildTree = async () => {
  treeLoading.value = true
  // Element Plus el-tree 没有 getExpandedKeys，通过 store.nodesMap 遍历获取已展开节点的 key
  const expandedKeys = []
  const store = treeRef.value?.store
  if (store && store.nodesMap) {
    for (const key of Object.keys(store.nodesMap)) {
      const n = store.nodesMap[key]
      if (!n || !n.expanded) continue
      // **已关闭的连接不恢复展开** —— 否则新建/重命名分组等任何树重建都会把它
      // 重新展开并加载库列表，图标跟着变亮：连接被「自动打开」（用户踩过）。
      // 关闭态的口径与 isConnOpen 一致：库缓存没了、也没有页签占用它。
      if (key.startsWith('conn:')) {
        const cid = key.slice(5)
        const open = cid in dbsByConn.value || tabs.value.some(tb => String(tb.connId || '') === cid)
        if (!open) continue
      }
      expandedKeys.push(key)
    }
  }
  try {
    // 按环境分组（即使没有任何连接，也要渲染 localStorage 中的自定义目录）
    const groups = new Map()
    for (const c of allConnections.value) {
      const env = c.group || ''
      if (!groups.has(env)) groups.set(env, [])
      groups.get(env).push(c)
    }
    // 角标环境：优先独立 env 字段；兼容旧数据（environment 为预置环境时沿用）。
    // 什么都没有的连接角标**默认「开发」**—— 用户要求角标永远落在 开发/测试/生产 之一，
    // 不出现「未分组」角标（未分组的**分组节点**保留，但角标按开发显示）
    const envTagOf = (c) => c.env || (ENV_ORDER_BASE.includes(c.group || '') ? c.group : '') || 'DEV'
    // 顺序：自定义目录（按创建顺序）→ 预置 DEV/TEST/PROD → 未分组（''，放最后）。
    // 未分组组保留（用户定稿）：组名就叫「未分组」，连接角标照常显示 开发/测试/生产。
    const envOrder = [...customFolders.value, ...ENV_ORDER_BASE, '']
    const envNodes = []
    // **多级目录**：group/纯目录存的是**路径**（`公司资源/子组`，`/` 分隔）——
    // 树上按段拆开嵌套渲染，目录可以无限级。folderIndex 缓存「路径 → 节点」；
    // 遇到深层路径先递归建父（父可能还没轮到遍历），再把父挂到顶层或祖先的 children。
    const folderIndex = new Map()
    const segLabel = (path) => {
      const t2 = envLabel(path)
      if (t2 !== path) return t2 // 预置环境（DEV 等）用译名
      const segs = path.split('/').filter(Boolean)
      return segs[segs.length - 1] || path
    }
    const ensureFolderNode = (path) => {
      if (folderIndex.has(path)) return folderIndex.get(path)
      const node = { id: 'env:' + path, label: segLabel(path), kind: 'env-folder', env: path, children: [] }
      folderIndex.set(path, node)
      const idx = path.lastIndexOf('/')
      const parentPath = idx > 0 ? path.slice(0, idx) : ''
      if (parentPath) ensureFolderNode(parentPath).children.push(node)
      else envNodes.push(node)
      return node
    }
    for (const env of envOrder) {
      const conns = groups.get(env) || []
      // 跳过没有 conn 的空分组
      if (conns.length === 0 && !customFolders.value.includes(env)) continue
      // 连接节点不带 children：懒加载模式下展开时才会加载数据库列表
      const connNodes = conns.map(c => ({
        id: 'conn:' + c.id, label: c.name, name: c.name, kind: 'conn',
        connId: String(c.id), env: envTagOf(c), connType: c.type,
        // 记录真实所属目录（environment），右键"新建连接"时继承所在目录而非角标环境
        environment: c.group || '',
        // 记录连接配置的数据库名称，展开时据此自动选中（而非默认第一个）
        database: c.database || '',
        // 备注（extra.note）：树上 hover 显示
        note: c.note || '',
        // 只读标记：树上直接摆出来。这个状态藏在编辑弹窗的「高级选项」里，
        // 而它决定的是「写操作会不会被拦」—— 用户需要随时看得见，不能靠记
        readOnly: !!c.readOnly
      }))
      // 带路径的目录 → 嵌套进父节点；顶层目录/预置环境/未分组直接挂根。
      // 全部走 ensureFolderNode 统一索引 —— 否则「父目录先被子目录递归创建、
      // 轮到自己时又新建一份」会出现重复的顶层节点（真机踩过）。
      const node = env === ''
        ? { id: 'env:', label: envLabel(''), kind: 'env-folder', env: '', children: [] }
        : ensureFolderNode(env)
      node.children = [...node.children, ...connNodes]
    }
    treeData.value = envNodes
    // treeData 每次赋值为新数组会触发 el-tree 重建；保留 defaultExpanded
    // 让已加载节点自动展开，未加载的懒加载节点由下面的 expandNode 手动触发
    defaultExpanded.value = expandedKeys
    await nextTick()
    // 恢复之前展开的节点（不存在的 key 会被自动忽略，如已删除的连接）
    // 对懒加载节点，default-expanded-keys 不会自动触发 load，需要手动展开
    for (const key of expandedKeys) {
      expandNode(key)
    }
  } finally {
    treeLoading.value = false
  }
}

// ===== 拖拽：把数据源（连接）拖到另一个目录（文件夹） =====
const treeDragActive = ref(false)
let treeDragEndAt = 0
// 拖拽起止：标记拖拽态，供 onNodeClick 忽略拖拽结束瞬间产生的点击
const onTreeDragStart = (node, ev) => {
  treeDragActive.value = true
  const d = node?.data
  if (!d || d.kind === 'conn') return // 连接仅用于树内重排，不写入编辑器标识符
  const name = objName(d)
  if (name && ev?.dataTransfer) {
    ev.dataTransfer.setData('application/x-上游-sql-ident', name)
    ev.dataTransfer.setData('text/plain', name)
    ev.dataTransfer.effectAllowed = 'copy'
  }
}
const onTreeDragEnd = () => { treeDragActive.value = false; treeDragEndAt = Date.now() }
// 允许拖动：① 连接节点（用于拖到目录重排）；② 对象节点（表/视图/触发器/事件/过程/函数/集合，用于拖入 SQL 编辑器）
const DRAG_OBJECT_KINDS = ['table', 'view', 'procedure', 'function', 'trigger', 'event', 'collection']
const allowDragNode = (node) => {
  const k = node?.data?.kind
  return k === 'conn' || DRAG_OBJECT_KINDS.includes(k)
}
// 只允许放入目录节点内部（inner）；不允许插到目录前后，也不允许放进连接/库/表等节点
const allowDropNode = (draggingNode, dropNode, type) => {
  if (draggingNode?.data?.kind !== 'conn') return false
  if (dropNode?.data?.kind !== 'env-folder') return false
  return type === 'inner'
}
// 放下后：把连接的分组改成目标目录（**全路径**，支持任意层级子目录）并落库，再重建树
//（el-tree 已先行移动了 DOM 节点）
const onTreeNodeDrop = async (draggingNode, dropNode, dropType) => {
  if (dropType !== 'inner') { await buildTree(); return }
  const dragData = draggingNode?.data
  const targetData = dropNode?.data
  if (dragData?.kind !== 'conn' || targetData?.kind !== 'env-folder') { await buildTree(); return }
  const connId = String(dragData.connId)
  const source = allConnections.value.find(c => String(c.id) === connId)
  // 目标目录：env-folder 的 env 是完整路径（预置为 DEV/TEST/PROD，自定义目录可以是
  // `公司资源/研发组` 这种多级路径 —— 树上嵌套到哪层就能拖进哪层）
  const targetEnv = targetData.env || ''
  if (!source || (source.group || '') === targetEnv) { await buildTree(); return }
  try {
    // ⚠ 分组真相源是 **group** 字段（树的归属按它算）。曾经只写 environment ——
    // 树纹丝不动，拖了像没拖一样（用户真机踩过）。environment 同步写一份兜底旧逻辑。
    await saveConnection({ ...source, group: targetEnv, environment: targetEnv })
    ElMessage.success(t('mv.movedTo', { name: source.name, env: envLabel(targetEnv) }))
  } catch (e) {
    ElMessage.error(t('mv.moveFailed', { detail: errMsg(e, t('common.unknownError')) }))
  }
  refreshFolders()
  await loadAllConnections()
  await buildTree()
}

// 加载当前库的表列表（供数据对比/同步弹窗使用）
const refreshAllTables = async () => {
  if (!conn.value || !currentDb.value) { allTables.value = []; return }
  try {
    const tables = isNoSql.value ? [] : await listTables(conn.value.id, currentDb.value)
    allTables.value = tables.filter(t => (t.type || 'TABLE') === 'TABLE')
    // 把这批新数据写回持久缓存（口径与 refreshCatNode 一致）：本函数只在「刷新对象」
    // 时跑，拿到的就是最新的；不写回的话，下次展开该库仍会先渲染旧清单
    if (!isNoSql.value && Array.isArray(tables)) {
      writeSchemaCache('tables:' + conn.value.id + ':' + currentDb.value, tables)
    }
  } catch { allTables.value = [] }
}

// 连接节点的库子节点统一构造（缓存命中 / 后台加载完成两处共用，避免字段漂移）
/**
 * 后端在「列不出库清单」时（连接没配默认库 + 列库查询失败/权限不足）会退回一个**占位名**
 * `(default)`（本地文件型是 `main`，那是真名字，不能动）。它**不是数据库**：
 * 展开它必然报错（实测：MySQL 放了一夜被 wait_timeout 掐掉后点它，
 * 报「No operations allowed after connection closed」）。树里一律不画它。
 *
 * 上限只对**关系型**生效：Redis / Mongo / ES 的 `(default)` 是**能用的伪库**
 * （它们的 key / 集合 / 索引就挂在它下面列出来），过滤掉等于把这个入口删了。
 */
const NO_DEFAULT_DB = '(default)'
/**
 * catalog 层级的类型（目前 Doris）：连接下面先出一层 catalog，进了 catalog 才是库。
 *
 * `catalog` 传空时行为与以前完全一致（两层：连接 → 库）—— 其余 16 种类型走的都是这条路。
 * 传了 catalog 时，库节点的 `db` 属性写成 `<catalog>.<库>`，于是：
 *   · 节点 id 天然带 catalog，不同 catalog 下的同名库不会撞；
 *   · 展开库时 `?database=<catalog>.<库>` 原样传给后端，由后端走
 *     `<catalog>.information_schema` 查询（见 dialect::tables_in_catalog）。
 */
const dbChildNodes = (connId, connType, dbList, catalog) => {
  const raw = Array.isArray(dbList) ? dbList : []
  const list = raw.filter(db => isNoSqlType(connType) || String(db) !== NO_DEFAULT_DB)
  // 过滤后一个库都没有：说明真正发生的是「列库查询没成功」，而不是「这台服务器没有库」。
  // 留一行说明，比留一片空白强 —— 空白会被读成后者（用户会以为库全没了）。
  if (!list.length) {
    // 注意：这里**不能**返回 `[]`。el-tree 收到空数组会把节点标成 `isLeaf`（粘住），
    // 之后就算服务器上有库也永远展开不出东西 —— 一律给一行说明，节点保持可再展开。
    return [{
      id: 'ph:' + connId + ':nodblist', kind: 'placeholder', connId, connType,
      label: t('mv.noDbList')
    }]
  }
  return list.map(db => {
    // catalog 层级：库名带上 catalog 前缀，作为后续所有元数据请求的 `database` 参数
    const qualified = catalog ? catalog + '.' + db : db
    return {
      id: 'db:' + connId + ':' + qualified, label: db, kind: 'db',
      db: qualified, connId, connType,
      leaf: connType === 'REDIS' // Redis key 不进树
    }
  })
  }

  /**
  * 懒加载 resolve 之前的统一收尾：**必须重置 `isLeaf`**。
  *
  * el-tree 在懒加载 resolve 出**空数组**时会把节点标成 `isLeaf = true`，而这个标记是
  * **粘住**的：之后再 resolve 出真实子节点，el-tree 依然认为"这是叶子"，一个子节点都不渲染 ——
  * 表现就是**节点展开着、里面却空的，而服务器上明明有库**（实测节点上就挂着 `is-leaf`）。
  * 所以每次给子节点都顺手把标记改回来：有子节点 = 不是叶子。
  */
  const finishLazyChildren = (data, children) => {
  data.children = children
  const node = treeRef.value?.getNode(data.id)
  if (node) {
    node.isLeaf = children.length === 0
    node.loaded = true
  }
  return children
  }

/**
 * 后台校准库列表（持久缓存命中时调用，**不阻塞展开**）。
 *
 * 拿到真实列表就更新内存与持久缓存；**只有库集合真的变了**才动树节点 ——
 * 否则每次展开都重建一遍子节点，用户刚展开的东西会莫名其妙抖一下。
 */
const calibrateDatabases = async (connId, data) => {
  try {
    const isNosql = isNoSqlType(data.connType)
    const list = await (isNosql ? noSqlDatabases(connId) : listDatabases(connId))
    const prev = dbsByConn.value[connId]
    dbsByConn.value[connId] = list
    writeSchemaCache('dbs:' + connId, list)
    if (connId === currentConnId.value) dbs.value = list
    const same = Array.isArray(prev) && prev.length === list.length && prev.every((v, i) => v === list[i])
    if (same) return
    data.children = dbChildNodes(connId, data.connType, list)
    treeRef.value?.updateKeyChildren(data.id, data.children)
    console.info('[展开] 库列表已按后端校准（' + (prev ? prev.length : 0) + ' → ' + list.length + '）')
  } catch (e) {
    console.warn('[展开] 后台校准库列表失败', e)
  }
}

/**
 * 后台校准某个分类的清单（持久缓存命中时调用，**不阻塞展开**）。
 *
 * 只更新计数，**不动已经展开出来的子节点**：那些节点的形状（id/kind/图标）与这里的原始
 * 对象不同，硬塞进树里会渲染成空白行。最坏情况是"数字变了、点开的清单还是旧的"，
 * 下次重新展开这个库就一致了 —— 比为了让边缘情况正确而引入渲染错乱划算。
 */
const calibrateCategoryItems = async (connId, db, data, cat) => {
  try {
    const tables = await listTables(connId, db)
    const list = Array.isArray(tables) ? tables : []
    writeSchemaCache('tables:' + connId + ':' + db, list)
    const wanted = cat === 'tables' ? 'TABLE' : 'VIEW'
    const mine = list.filter(t => (t.type || 'TABLE') === wanted)
    if (data.cat !== cat) return
    const same = Array.isArray(data.catItems) && data.catItems.length === mine.length
      && data.catItems.every((t, i) => (t.name || '') === (mine[i].name || ''))
    if (same) return
    // 清单变了（别人建了表 / 改了名 / 删了表）——
    data.catCount = mine.length
    catCounts.value[data.id] = mine.length
    if (cat === 'tables' && inCurrentDb(db)) allTables.value = mine
    // 再把这一类的子节点**重建**，让用户当次就看到变化。
    //
    // 原来只改计数（注释里也承认了这个后果）：用户看到的是「数字变了、清单里却
    // 没有那张新表」，得收起再展开一次才对 —— 而"再展开一次"能不能对，又取决于
    // 这次校准有没有把新数据写回持久缓存，等于绕一圈还慢一拍。
    //
    // 用 refreshCatNode 而不是在这里自己拼子节点：它内部走 buildCatChildren，
    // 节点形状与树首次构建完全一致（这正是本函数原先不自己动子节点的原因 ——
    // 硬塞形状不同的节点会渲染成空白行）。它同时会把新清单写回持久缓存。
    refreshCatNode(db, cat, connId)
  } catch (e) {
    console.warn('[展开] 后台校准清单失败', e)
  }
}

// 正在懒加载的节点 id（每层展开都要有 loading 反馈 —— 用户要求）。
// 不依赖 el-tree 的 node.loading：部分入口（expandNode 的手动 loadData、预取缓存）下
// 它可能一闪而过或不可靠，这里用「进函数记账、resolve 撤账」的口径，与 connLoadingSet 同款。
//
// 时序保证（用户反馈迭代出的最终形态）：**点击立即转圈、转圈期间不展开、撤圈与展开同时**。
// 不再需要「最小显示时长」—— 那是为了弥补展开被表清单请求挡住、转圈出现太晚的缺陷；
// 展开先行修掉根因后，快层（预取命中）瞬间完成直接展开就是正确的体验，人为延迟反而拖沓。
const nodeLoadingIds = ref([])
// 懒加载：展开节点时才加载子节点（连接 → 数据库列表；数据库 → 表/视图/对象）
const lazyLoad = async (node, resolve) => {
  const data = node?.data
  const key = data?.id
  if (key) nodeLoadingIds.value.push(key)
  let delivered = false
  // 包装 resolve：撤圈与展开**同一时刻**发生（el-tree 收到 resolve 才展开节点）
  const done = (children) => {
    if (delivered) return
    delivered = true
    if (key) {
      const index = nodeLoadingIds.value.indexOf(key)
      if (index >= 0) nodeLoadingIds.value.splice(index, 1)
    }
    resolve(children)
  }
  try {
    await lazyLoadRaw(node, done)
  } catch (e) {
    // lazyLoadRaw 内部已兜底，这里只是保险：绝不让节点停在永久转圈
    done([])
  }
}
const lazyLoadRaw = async (node, resolve) => {
  const data = node.data
  try {
    if (data.kind === 'env-folder') {
      // 环境分组：lazy 模式下 el-tree 会忽略 data.children，需显式返回预置的连接子节点
      resolve(data.children || [])
    } else if (data.kind === 'conn') {
      const connId = String(data.connId)
      // catalog 层级（Doris）：连接下先出一层 catalog。
      //
      // ⚠️ 这段必须放在**所有库缓存判断之前** —— 否则第二次展开会命中"库列表缓存"，
      // 直接画库，catalog 那一层凭空消失（第一次有、第二次没有，很难查）。
      // 接口对没有 catalog 的类型返回 []，所以对另外 16 种类型是零影响。
      if (!(connId in catalogsByConn.value)) {
        try {
          const list = await listCatalogs(connId)
          catalogsByConn.value[connId] = Array.isArray(list) ? list : []
        } catch {
          catalogsByConn.value[connId] = []
        }
      }
      const cats = catalogsByConn.value[connId] || []
      // 只有一个 catalog 时**不画这一层**：Doris 绝大多数实例就只有 `internal`，
      // 为了一个名字多出一层，每次都要多点一次才看得到库 —— 纯噪音。
      //
      // 做法是把它当成连接的「隐含前缀」直接展开：库节点、请求参数与"点开那一层"
      // **完全一致**（库节点仍是 `internal.ods`、`/databases?catalog=` 照传），只是树上少一个节点。
      //
      // ⚠️ 这里**不能**图省事把前缀丢掉、改用裸库名。后端列 catalog 走的是
      // `select distinct table_catalog from information_schema.tables`（见 dialect.rs），
      // 而**一张表都没有的 catalog 不会出现在里面** —— 于是"只剩一个"的那个
      // 未必是默认的 `internal`，裸库名会静默查到**错误的 catalog** 上去。
      if (cats.length === 1) {
        const only = cats[0]
        // 只有默认 internal（用户没建自定义 catalog）→ **平铺裸库名**，与"没建过 catalog"
        // 的形态完全一致（ods，而不是 internal.ods）。请求仍带 catalog=only（后端列库
        // 必须指定），但树节点与所有下游的 database 标识都用裸名 —— 裸名经 scope::resolve
        // 会落到 internal 的同名库，两种形态在执行链路上等价。
        try {
          const list = await listDatabases(connId, only)
          resolve(finishLazyChildren(data, dbChildNodes(connId, data.connType, list)))
        } catch (e) {
          showConnFail(errMsg(e, t('mv.loadDbsFailed')))
          resolve(finishLazyChildren(data, dbChildNodes(connId, data.connType, [])))
        }
        return
      }
      // 两个以上 catalog 才把这一层画出来 —— 那时它是真的在区分东西
      if (cats.length > 1) {
        resolve(finishLazyChildren(data, cats.map(c => ({
          id: 'catalog:' + connId + ':' + c, label: c, kind: 'catalog',
          catalog: c, connId, connType: data.connType
        }))))
        return
      }
      // 已有真实子节点缓存（后台加载已完成）时直接返回
      if (data.children && data.children.length && data.children[0].kind !== 'placeholder') {
        resolve(data.children)
        return
      }
      // 已缓存过该连接的数据库列表：直接用缓存构建子节点，不再重复请求后端
      // 注意：缓存可能为空数组（连接无库），必须仅凭 "connId in dbsByConn" 判断，
      // 否则空库连接会在占位节点与重新请求之间死循环
      if (connId in dbsByConn.value) {
        const cachedDbs = dbsByConn.value[connId]
        resolve(finishLazyChildren(data, dbChildNodes(connId, data.connType, cachedDbs)))
        return
        }
      // 持久缓存命中：**立刻**渲染（每个连接首次建连要 5~10 秒，展开过的连接不该再等一次），
      // 同时后台拉一次真实列表校准 —— 缓存只负责快，对错由后端说了算，
      // 所以新建/删除的库几百毫秒后会自动出现，不会像"把列表存进会话"那样一次存错永远错。
      {
        const hit = readSchemaCache('dbs:' + connId)
        if (hit && Array.isArray(hit.value)) {
          dbsByConn.value[connId] = hit.value
          resolve(finishLazyChildren(data, dbChildNodes(connId, data.connType, hit.value)))
          calibrateDatabases(connId, data)
          return
        }
      }
      // **不再塞「加载中…」占位行**：那行出现在"展开后的节点里"，而"正在加载"这件事
      // 已经由**箭头位置的转圈**表达了（见 isNodeLoading）。这里直接等列表，一次 resolve 真内容 ——
      // 于是"展开"只在确实拿到结果之后发生，中间不会先亮出一个空节点。
      try {
        const isNosql = isNoSqlType(data.connType)
        const dbList = await (isNosql ? noSqlDatabases(data.connId) : listDatabases(data.connId))
        dbsByConn.value[connId] = dbList
        // 落进持久缓存：下次（含重启桌面版之后）展开这个连接就不用再等 5~10 秒的建连
        writeSchemaCache('dbs:' + connId, dbList)
        if (connId === currentConnId.value) {
          dbs.value = dbList
          // 恢复该连接上次选中的库。缓存值可能是失效的（库被删/改名，或存档由旧会话带来），
          // 所以必须拿本次真实返回的库列表校验一次 —— 否则会把不存在的库当成「当前库」，
          // 依赖它的表列表、AI 面板的数据源显示就全都跟着错。
          const cachedDb = currentDbByConn.value[connId]
          const configuredDb = (data.database || '').trim()
          if (!cachedDb || !dbList.includes(cachedDb)) {
            // 缓存缺失或已失效：回退到连接配置里的库（同样要存在），都没有就不打开任何库
            currentDbByConn.value[connId] =
              (configuredDb && dbList.includes(configuredDb)) ? configuredDb : ''
          }
          setCurrentDb(currentDbByConn.value[connId])
          // **表列表放后台，别挡住"展开"**：这条请求在有些库上要十几秒
          // （实测 `/tables?database=mysql` = 10.96 秒），而它和"列出库名"毫无关系。
          // 以前在这里 await，点开连接要等十几秒才看到库节点 —— 看着就像卡住没反应。
          // 库列表已经在手上了，先画出来；表列表回来再补齐右侧。
          refreshAllTables().catch((e) => console.warn('[展开] 表列表后台加载失败', e))
          }
        data.children = dbChildNodes(connId, data.connType, dbList)
        // 一次 resolve 真内容（不再有"先占位、后台替换"那一套）：
        // 占位原本是为了"展开即时生效"，而现在展开由**拿到结果**决定。
        resolve(data.children)
        // 只在"**已经加载好了**却没展开"时补一次展开。必须带 `n.loaded` 判断：
        // 不带的话，`expandNode` 会对还没标记 loaded 的节点再触发一轮懒加载，
        // 那正是"展开了却没东西 / 又冒出第二个转圈"的来源之一。
        const n = treeRef.value?.getNode(data.id)
        if (n && n.loaded && !n.expanded) nextTick(() => expandNode(data.id))
        // 记进 defaultExpanded：树重建（buildTree）后这个展开状态才留得住
        if (!defaultExpanded.value.includes(data.id)) defaultExpanded.value.push(data.id)
      } catch (e) {
        const sid = String(data.connId)
        // 若 selectConn 已报错并加入 connErrorSet，此处不再重复弹窗
        const alreadyReported = connErrorSet.value.has(sid)
        connErrorSet.value.add(sid)
        if (!alreadyReported) showConnFail(errMsg(e, t('mv.connFailedCheck')))
        // 失败时不再"先收起、再 resolve([])"：`resolve` 本身会让 el-tree 展开这个节点，
        // 于是先 collapse 再 resolve 的结果是**一个展开着的空节点** ——
        // 用户说的"显示展开了但没东西，其实有东西"就是这么来的（服务器上明明有库）。
        // 改成直接 resolve 一行**说明**：节点该展开就展开，但里面写清为什么没有内容。
        const hint = {
          id: 'ph:' + data.id + ':failload', kind: 'placeholder',
          connId: data.connId, connType: data.connType,
          label: t('mv.noDbListRetry')
        }
        resolve(finishLazyChildren(data, [hint]))
      }
    } else if (data.kind === 'catalog') {
      // catalog 展开：列这个 catalog 下的库（`?catalog=xxx`）。
      // 库节点带上 catalog 前缀，后续展开库、查表都走 `<catalog>.<库>`。
      const connId = String(data.connId)
      const list = await listDatabases(connId, data.catalog).catch((e) => {
        showConnFail(errMsg(e, t('mv.loadDbsFailed')))
        return []
      })
      resolve(finishLazyChildren(data, dbChildNodes(connId, data.connType, list, data.catalog)))
    } else if (data.kind === 'db') {
      resolve(finishLazyChildren(data, await buildDbDetailNodes(data)))
    } else if (data.kind === 'schema') {
      resolve(finishLazyChildren(data, await buildSchemaDetailNodes(data)))
    } else if (data.kind === 'category') {
      // 分类目录：**展开时才查它自己那一类**（Tables 才查 listTables）。
      // 这是"打开库快"的关键之一：以前展开库就把 5 类对象一次性全查，
      // 最慢的那条（在 mysql 库上 10.96 秒）会挡住整个库的展开。
      // 预取已经拿到清单的，直接用那份 —— 展开是秒开的；没拿到（或预取失败）才现查
      const items = Array.isArray(data.catItems) ? data.catItems : await loadCategoryItems(data)
      // 手动展开这条路也要写表：数字立刻正确，不必等预取（两条路写同一个来源）
      if (Array.isArray(items)) catCounts.value[data.id] = items.length
      resolve(finishLazyChildren(data, buildCatChildren({ cat: data.cat, connId: data.connId, db: data.db, items })))
    } else if (data.children && data.children.length) {
      // 懒加载模式下 el-tree 会忽略 data.children：分类目录（Tables/Views/Procedures 等）
      // 必须在这里显式返回预置子节点，否则最内层目录展开后为空
      resolve(data.children)
    } else {
      resolve([])
    }
  } catch (e) {
    resolve([])
  }
}

// 刷新按钮：重新拉取当前连接的 dbs，再重建树（懒加载缓存随之清除）
// 展开/收起状态由 buildTree 内部自动保持
const refreshTree = async () => {
  treeLoading.value = true
  // 纯分组也在「刷新」的语义里：后端删了分组，点一下刷新树上就要消失
  await loadPureFolders()
  // 没有任何连接时也要继续：空态下刷新同样该重画树（以前直接 return，
  // 空态下按钮点了没反应，用户以为坏了）
  if (conn.value) {
    // 用户点「刷新对象」的语义就是「我要看现在真实的东西」（同事刚建的表/库要能出来）：
    // 先把该连接的**前端结构缓存全部丢掉**。
    //
    // 不丢会怎样：这个函数下面确实会真查（兼容层一律 fresh），但拿到的结果不写回缓存，
    // 旧缓存又原封不动 ⇒ 下次展开还是先渲染那批旧的「刷新」等于只在本次生效。
    const cid = String(conn.value.id)
    invalidateSchemaCache(cid)
    try {
      dbs.value = await (isNoSql.value ? noSqlDatabases(conn.value.id) : listDatabases(conn.value.id))
      // 同上：空列表不进缓存（拉库失败时 dbs.value 就是 []）
      if (currentConnId.value && dbs.value.length) {
        dbsByConn.value[currentConnId.value] = dbs.value
        // 顺带写回持久缓存：本次拿到的就是最新的，不写的话下次展开又先渲染旧的
        writeSchemaCache('dbs:' + currentConnId.value, dbs.value)
      }
    } catch (e) {
      dbs.value = []
      ElMessage.error(t('mv.refreshDbsFailed', { detail: errMsg(e, t('common.unknownError')) }))
    }
  }
  await refreshAllTables()
  await buildTree()
  treeLoading.value = false
}


const onDbChange = () => { /* 切换库时由 onNodeClick 处理 */ }

// 树搜索过滤
const filterNode = (val, data) => {
  if (!val) return true
  const needle = val.toLowerCase()
  // 分类节点的 `label` 是**英文兜底**（Tables/Views…），界面上显示的是字典里的译文。
  // 只比 label 的话，中文界面下搜「表」一句都搜不出来 —— 看起来像"搜索坏了"。
  // 所以译名一起参与匹配（未翻译的节点译文=原名，这条判断自然退化成原行为）。
  const shown = data.i18nKey ? t(data.i18nKey) : (data.label || '')
  return String(data.label || '').toLowerCase().includes(needle)
    || String(shown).toLowerCase().includes(needle)
}
// 输入即过滤对大型已加载树仍有开销，加 150ms 防抖让输入更跟手
let filterTimer = 0
watch(filterText, (v) => {
  clearTimeout(filterTimer)
  filterTimer = setTimeout(() => treeRef.value?.filter(v), 150)
})

// 数据源类型标签（单一来源：src/types 注册表，与后端 DatabaseType.label 一致）
const typeLabel = (t) => labelOf(t)

// ====== 侧栏宽度拖拽 ======
let resizeStartX = 0
let resizeStartW = 0
const startResize = (e) => {
  resizeStartX = e.clientX
  resizeStartW = treeWidth.value
  document.body.classList.add('dc-col-resizing')
  window.addEventListener('mousemove', onResize)
  window.addEventListener('mouseup', stopResize)
}
const onResize = (e) => {
  const w = Math.min(620, Math.max(160, resizeStartW + e.clientX - resizeStartX))
  treeWidth.value = w
}
const stopResize = () => {
  window.removeEventListener('mousemove', onResize)
  window.removeEventListener('mouseup', stopResize)
  document.body.classList.remove('dc-col-resizing')
}

// 在 treeData 中查找 id 为 key 的节点的父节点 id（懒加载下父子链节点不会全部预渲染）
const findTreeParentKey = (key, nodes) => {
  const list = nodes || treeData.value
  for (const n of list) {
    if (!n.id || n.id === key) continue
    const kids = n.children || []
    if (kids.some(ch => ch.id === key)) return n.id
    const sub = findTreeParentKey(key, kids)
    if (sub) return sub
  }
  return null
}

// 可靠地展开 el-tree 节点（lazy 模式下直接 node.expand() 对未加载节点可能不生效）
// 若节点尚未创建（父级目录收起时其下子节点未渲染/未懒加载），先展开父级目录并轮询等待节点就绪
const expandNode = async (key) => {
  let node = treeRef.value?.getNode(key)
  if (!node) {
    const parentKey = findTreeParentKey(key)
    if (parentKey) {
      const p = treeRef.value?.getNode(parentKey)
      if (p && !p.expanded) expandNode(parentKey)
    }
    // 找不到节点时最多等 600ms（原先是 100ms×40 = 4 秒）。
    // 4 秒的轮询是"点了像没反应"的典型来源：调用方在等一个还没渲染出来的节点，
    // 而它没渲染出来通常意味着**父节点根本没展开** —— 再等 3 秒也不会变出来。
    let tries = 0
    while (!node && tries < 6) {
      await new Promise(r => setTimeout(r, 100))
      node = treeRef.value?.getNode(key)
      tries++
    }
    if (!node) return
  }
  const doExpand = () => {
    if (!node.expanded) node.expand()
    if (!defaultExpanded.value.includes(key)) defaultExpanded.value.push(key)
  }
  if (node.loaded) {
    doExpand()
  } else {
    node.loadData(doExpand)
  }
}

// 收起节点并同步 defaultExpanded，防止 tree 重新渲染后又自动展开
const collapseNode = (key) => {
  const node = treeRef.value?.getNode(key)
  if (node && node.expanded) node.collapse()
  const idx = defaultExpanded.value.indexOf(key)
  if (idx > -1) defaultExpanded.value.splice(idx, 1)
}

// ---- 展开 / 收起：由本文件**唯一**决定（el-tree 的 expand-on-click-node 已关闭）----
//
// 规则：① 只有"装得下子节点"的类型才画箭头；② 点箭头只切展开，
// 点行才执行语义动作（打开连接 / 打开库 / 预览）。
// 之前是 el-tree 的 expand-on-click-node 与本函数**各切一次**，两次互相抵消 ——
// 实测表现是"只会展开、再点收不起来"，也就是那个"不灵敏"。
// `catalog` 必须在这里：Doris 的 catalog 节点下面是库，它当然"装得下子节点"。
// 漏掉它的后果不只是"没箭头" —— 点击与右键也各自按 kind 分支，三处都不认它，
// 于是那个节点**点也没反应、右键也没反应**（用户反馈的就是这个）。
const EXPANDABLE_KINDS = new Set(['env-folder', 'conn', 'db', 'schema', 'category', 'catalog'])
/** 能不能展开：能装子节点的类型才算（Redis 库的 key 不进树，标了 leaf 就不算）。 */
const canExpandNode = (data) => !!data && !data.leaf && EXPANDABLE_KINDS.has(data.kind)
/** 是不是正在加载：el-tree 懒加载会把 node.loading 置位；连接另有自己的标记。 */
const isNodeLoading = (node, data) => {
  if (node && node.loading) return true
  // 每层展开的显式记账（见 nodeLoadingIds）：覆盖手动展开/预取等 node.loading 不可靠的路径
  if (data?.id && nodeLoadingIds.value.includes(data.id)) return true
  return data.kind === 'conn' && connLoadingSet.value.has(String(data.connId))
}
/**
 * 库是不是"已打开"：当前库就是打开的（树上一次只展开一个库的表）。
 * 它同时决定两件事：库图标满色还是变暗、
 * 行尾要不要那个绿点。
 */
const isDbOpened = (data) => {
  if (!data || data.kind !== 'db') return false
  if (inCurrentDb(data.db)) return true
  // 开着同一个库的**页签**，同样算"已打开"。
  // 为什么需要：Redis 的库不进树（点它是开一个「该库全部键」页签，见 `openRedisDb`），
  // 它也不会被设成 currentDb —— 只认 currentDb 的话，点了库图标永远是暗的
  // （用户反馈："点击一会图标不亮"）。口径与上面的 `isConnOpen` 保持一致。
  const db = String(data.db || '')
  if (!db) return false
  const nodeConn = String(data.connId || '')
  return tabs.value.some((t) => {
    if (String(t.database || '') !== db) return false
    // 两边都带 connId 时必须一致；一边缺失就不比这项（老节点 / 老页签的容错）
    const tabConn = String(t.connId || '')
    return !tabConn || !nodeConn || tabConn === nodeConn
  })
}
/** 点箭头：只切换展开，不执行"打开对象"。 */
const onArrowClick = (data, node) => {
  // 未打开的连接：点箭头 = 连接（连上后由 selectConn 展开）—— 不在这里展开，
  // 否则又会变成"先展开成一个空节点"。转圈由 isNodeLoading 画在箭头位置。
  if (data.kind === 'conn' && !isConnOpen(data.connId)) {
    selectConn(data.connId)
    return
  }
  if (node?.expanded) collapseNode(data.id)
  else expandNode(data.id)
}

/**
 * 单击树节点：先执行该类型的语义动作，最后**切一次**展开/收起。
 */
const onNodeClick = async (data, node, _component, event) => {
  // 拖拽过程中/刚结束的点击不当作选中（否则拖放换目录会顺带触发连接）
  if (treeDragActive.value || Date.now() - treeDragEndAt < 200) return
  // Ctrl / ⌘ + 单击 = 多选（不进"打开对象"的语义）；普通单击顺手清掉多选。
  // el-tree 的 node-click 第 4 个参数就是原生事件，不用再挂一层监听。
  if (event?.ctrlKey || event?.metaKey) {
    toggleMultiSelect(data)
    return
  }
  if (multiSel.value.length) clearMultiSelect()
  // 连接节点：未连接 → 单击即连接（连上后由 selectConn 展开）；
  // 已连接 → 单击展开/收起。**单击不会断开连接**，断开在右键菜单里。
  if (data.kind === 'conn') {
    if (!isConnOpen(data.connId) || connErrorSet.value.has(String(data.connId))) {
      await selectConn(data.connId)
    } else if (node?.expanded) {
      collapseNode(data.id)
    } else {
      expandNode(data.id)
    }
    return
  }
  // catalog 节点（Doris）：它自己不承载"当前库"的语义（那是库节点的事），
  // 单击只做一件事 —— 展开/收起，与文件夹一致。
  // 必须在这里显式切换：el-tree 的 expand-on-click-node 是关掉的（见文件头注释），
  // 不写这一段，点这个节点就是**完全没反应**。
  if (data.kind === 'catalog') {
    if (node?.expanded) collapseNode(data.id)
    else expandNode(data.id)
    return
  }
  // Redis 库：key 不进树，单击即在右侧打开 key 列表（不展开/收起空节点）
  if (data.kind === 'db' && data.connType === 'REDIS') {
    openRedisDb(data)
    return
  }
  if ((data.kind === 'db' || data.kind === 'schema') && data.db) {
    // 树里所有连接的库是摊在一起显示的，点到别的连接下的库时**必须先切连接**。
    // 原先只 setCurrentDb，会留下「连接还是 A、库却是 B 的」这种串台状态：
    // 表列表会拿 A 的连接去查 B 的库，AI 面板的数据源也会拼成 "A · B库名"。
    // 复用右键菜单同一套切换逻辑，避免两处行为不一致。
    const connChanged = !!data.connId && String(data.connId) !== String(currentConnId.value)
    if (connChanged) ensureDbConn(data)
    const dbChanged = data.db !== currentDb.value
    if (dbChanged) setCurrentDb(data.db)
    // **展开先行**：点击瞬间就转圈（转圈时机是用户明确要求的）——
    // 原来 `await refreshAllTables()` 挡在展开前面，慢请求会把转圈拖到几百毫秒甚至
    // 数秒之后才出现，看着就像"点了没反应，过一会儿才转"。
    // 表清单的语义动作放后台，不挡展开、不挡转圈。
    if (canExpandNode(data)) {
      if (node?.expanded) collapseNode(data.id)
      else expandNode(data.id)
      if (connChanged || dbChanged) {
        refreshAllTables().catch((e) => console.warn('[点击] 表列表后台加载失败', e))
      }
      return
    }
  }
  // 表 / 视图 / 集合 / 索引 / 过程 / 触发器 / 事件：单击不触发预览

  // 环境文件夹 / 数据库 / schema / 分类目录：单击 = 展开或收起（**只切这一次**）。
  // 库与 schema 在上面的分支里已经切好了当前连接 / 当前库的语义，这里只负责开合。
  if (canExpandNode(data)) {
    if (node?.expanded) collapseNode(data.id)
    else expandNode(data.id)
  }
  }

// 双击（事件委托）：叶子对象打开预览；目录节点已由单击处理展开/收起
const onTreeDoubleClick = async (e) => {
  const nodeEl = e.target.closest('.el-tree-node')
  if (!nodeEl) return
  const key = nodeEl.getAttribute('data-key')
  if (!key) return
  const treeNode = treeRef.value?.getNode(key)
  if (!treeNode) return
  const data = treeNode.data
  // 连接节点统一由单击处理（打开/展开/收起），双击不再单独处理
  if (data.kind === 'conn') return
  // 目录节点双击已由 onNodeClick 处理展开/收起，这里跳过（Redis db 除外）
  if (data.kind === 'env-folder' || data.kind === 'schema' || data.kind === 'category'
      || data.kind === 'catalog') {
    return
  }
  if (data.kind === 'db' && data.connType !== 'REDIS') {
    return
  }
  // 叶子对象打开预览
  if (data.kind === 'table' || data.kind === 'view') await openTable(data)
  else if (data.kind === 'collection') openNoSql(data)
  else if (data.kind === 'db' && data.connType === 'REDIS') openRedisDb(data)
  else if (data.kind === 'script') openScript(data)
  else if (data.kind === 'function') openObject(data, 'function', data.label)
  else if (data.kind === 'procedure') openObject(data, 'procedure', data.label)
  else if (data.kind === 'trigger') openObject(data, 'trigger', data.label)
  else if (data.kind === 'event') openObject(data, 'event', data.label)
  // 用户页签要带节点自己的连接/库：双击是「还没点过连接」也会走的入口
  // （详见 openUserTab 的注释 —— 不带就是一片空白）
  else if (data.kind === 'user') openUserTab(data.objectName, { connId: data.connId, db: data.db })
  // placeholder 不处理
}

// 双击脚本节点：在 SQL 编辑器中打开该脚本内容
const openScript = (data) => {
  const scripts = getDbScripts(data.connId || currentConnId.value, data.db)
  const s = scripts.find(x => x.id === data.scriptId)
  if (!s) { ElMessage.warning(t('mv.scriptGone')); return }
  // 切换到脚本所属库，确保编辑器下拉选中正确数据库
  setCurrentDb(data.db)
  // 记录 scriptName：保存/自动保存将覆盖更新同名脚本，而非重复新建
  openTab({ id: 'sql:' + (++tabSeq), type: 'sql', label: s.name, scriptName: s.name, dirty: false,
    connId: data.connId || currentConnId.value, database: data.db })
  // 等待 SqlQueryView 挂载后再注入内容
  setTimeout(() => {
    window.dispatchEvent(new CustomEvent('dc-insert-sql', { detail: s.content }))
  }, 350)
}

// 删除脚本节点（右键 → 删除脚本）
const deleteScriptNode = (data) => {
  try {
    const key = `dc_scripts:${data.connId || currentConnId.value}:${data.db}`
    const list = JSON.parse(localStorage.getItem(key) || '[]')
    localStorage.setItem(key, JSON.stringify(list.filter(x => x.id !== data.scriptId)))
  } catch { /* ignore */ }
  refreshScriptsNode(data.db, data.connId)
  ElMessage.success(t('mv.scriptDeleted', { name: data.label }))
}

// 只刷新某个库的 Scripts 目录（懒加载模式下：让该库节点重新加载，脚本分类随之更新）
const refreshScriptsNode = (db, connId) => { refreshDbNode(db, connId) }

// 监听 SqlQueryView 保存/删除脚本后刷新对应库的 Scripts 目录
const onScriptsChanged = (e) => {
  const { db } = e.detail || {}
  if (db) refreshScriptsNode(db)
}

const openObject = (data, type, label) => {
  // 对象详情页签：id 带连接，否则不同数据源的同名视图 / 存储过程会互相顶掉
  const cid = data.connId ? String(data.connId) : (currentConnId.value ? String(currentConnId.value) : '')
  openTab({
    id: tabKey(type + ':' + data.objectName, cid), type: 'object', objectType: type, objectName: data.objectName,
    connId: cid, database: data.db || currentDb.value, label
  })
}

const openTable = async (data) => {
  const db = data.db || currentDb.value
  const cid = data.connId ? String(data.connId) : (currentConnId.value ? String(currentConnId.value) : '')
  // id 带连接：页签栏全局之后，不同数据源的同名表必须能各开一个
  const id = tabKey('table:' + data.table, cid)
  // 目标连接非当前连接：先切换上下文（树高亮 / 当前库）；页签自带 connId，不受影响
  if (data.connId && currentConnId.value && cid !== String(currentConnId.value)) {
    await selectConn(cid)
  }
  const existing = tabs.value.find(t => t.id === id)
  if (!existing) {
    const isView = data.kind === 'view'
    tabs.value.push({ id, type: 'table', table: data.table, database: db, readOnly: isView, connId: cid, label: data.table })
  } else if (existing.database !== db) {
    // 同连接跨库打开同名表：把旧 tab 指向新库，避免混淆
    existing.database = db
  }
  activeTab.value = id
}

const openNoSql = (data) => {
  const cid = data.connId ? String(data.connId) : (currentConnId.value ? String(currentConnId.value) : '')
  openTab({
    id: tabKey('nosql:' + data.collection, cid), type: 'nosql', collection: data.collection,
    database: data.database, kind: data.kind, connId: cid, label: data.collection
  })
}

const openRedisDb = (data) => {
  openTab({
    id: 'redis:' + data.connId + ':' + data.db, type: 'nosql', collection: '*',
    database: data.db, kind: 'redis-db', label: data.db,
    connId: data.connId, connType: 'REDIS'
  })
}

const openDetail = async (data, kind) => {
  const db = data.db || currentDb.value
  const cid = data.connId ? String(data.connId) : (currentConnId.value ? String(currentConnId.value) : '')
  const id = tabKey('detail:' + data.table, cid)
  // 目标连接非当前连接：先切换上下文（树高亮 / 当前库）；页签自带 connId，不受影响
  if (data.connId && currentConnId.value && cid !== String(currentConnId.value)) {
    await selectConn(cid)
  }
  const existing = tabs.value.find(t => t.id === id)
  if (!existing) {
    tabs.value.push({ id, type: 'detail', table: data.table, database: db, connId: cid, label: data.table + t('mv.structureSuffix') })
  } else if (existing.database !== db) {
    existing.database = db
  }
  activeTab.value = id
}

const newQueryTab = (opts = {}) => {
  openTab({
    id: 'sql:' + (++tabSeq), type: 'sql', label: t('mv.scriptN', { n: tabSeq }), scriptName: '', dirty: false,
    connId: opts.connId || currentConnId.value,
    database: opts.database || currentDb.value,
    initialSql: opts.initialSql || '',
    // 新建脚本时若要求自动执行（AI 结果里点「插入并执行」），由 SqlQueryView 挂载后自己跑
    autoRun: !!opts.autoRun
  })
}

// ===== Ctrl/⌘ + K：唤起统一的 AI 助手入口（并直接切到「指令」页签）=====
// 命令面板的能力已整合进 AI 助手，界面只保留 AI 助手这一个入口
const onGlobalKey = (e) => {
  // Esc：退出多选（多选状态下最顺手的"取消"）
  if (e.key === 'Escape' && multiSel.value.length) {
    clearMultiSelect()
    return
  }
  if ((e.ctrlKey || e.metaKey) && (e.key === 'k' || e.key === 'K')) {
    e.preventDefault()
    aiOpen.value = true
    nextTick(() => aiPanelRef.value?.openCommand?.())
  }
}

/**
 * 执行命令面板产出的操作计划。
 * 安全原则：只做「无副作用」动作（打开面板 / 把 SQL 插入编辑器等待用户确认），
 * 高危动作一律降级为提示或二次确认，绝不代用户直接写库。
 */
const onRunPlan = (actions) => {
  if (!Array.isArray(actions) || !actions.length) return
  for (const a of actions) {
    const p = a.params || {}
    try {
      if (a.type === 'open_query' || a.type === 'run_sql') {
        const sqlText = p.sql || ''
        if (!sqlText) continue
        insertSqlToActive(sqlText)
        ElMessage.success(a.type === 'run_sql' ? t('mv.sqlInsertedRun') : t('mv.sqlInserted'))
      } else if (a.type === 'create_index') {
        if (p.ddl) insertSqlToActive(p.ddl)
        ElMessage.warning(t('mv.indexDdlInserted'))
      } else if (a.type === 'open_panel') {
        const panel = p.panel || ''
        if (panel === 'ai') aiOpen.value = true
        else if (GOVERNANCE_TABS.includes(panel)) {
          governanceTab.value = panel
          governanceAutoRun.value = GOVERNANCE_AUTORUN_TABS.includes(panel)
          governanceOpen.value = true
        }
        else if (panel === 'governance') { governanceAutoRun.value = false; governanceOpen.value = true }
        else if (panel === 'settings') settingsOpen.value = true
        else if (panel === 'compare') openCompare()
        else if (panel === 'sync') syncDialogOpen.value = true
        else if (panel === 'newConn') openNewConn('')
        else if (panel === 'editConn' && conn.value) openConnDialog(conn.value)
      } else if (a.type === 'analyze_table') {
        aiOpen.value = true
        ElMessage.info(p.table ? t('mv.aiOpenedFor', { table: p.table }) : t('mv.aiOpened'))
      } else if (a.type === 'search') {
        ElMessage.info(t('mv.searchInTree', { keyword: (p.keyword || '') }))
      } else if (a.type === 'export_table') {
        ElMessage.info(t('mv.useExportInGrid') + (p.table ? t('mv.tableSuffix', { table: p.table }) : ''))
      } else if (a.type === 'answer') {
        if (p.text) {
          ElMessageBox.alert(String(p.text), t('mv.aiAnswer'), { showConfirmButton: true, customClass: 'dc-cmd-answer' }).catch(() => {})
        }
      }
    } catch (e) {
      ElMessage.error(e?.message || t('mv.actionFailed'))
    }
  }
}

onBeforeUnmount(() => { window.removeEventListener('keydown', onGlobalKey) })

// 数据表格的「AI 筛选」结果 → 在新 SQL 页签中打开，便于用户确认后再执行
const onOpenQueryFromTable = (payload) => {
  if (!payload || !payload.sql) return
  newQueryTab({
    connId: payload.connId || (currentConnId.value ? String(currentConnId.value) : ''),
    database: payload.database || currentDb.value,
    initialSql: payload.sql
  })
}

const onScriptSave = (tabId, name) => {
  const t = tabs.value.find(x => x.id === tabId)
  if (t) { t.scriptName = name; t.dirty = false }
  // 保存后立刻存一次会话快照：保证刷新回来看到的是刚保存的内容
  scheduleSaveSession()
}

// 编辑器内容变化（防抖后上报）：更新页签快照里的 SQL
const onScriptSqlChange = (tabId, val) => {
  const t = tabs.value.find(x => x.id === tabId)
  if (t && typeof val === 'string') t.initialSql = val
  scheduleSaveSession()
}

const onScriptDirty = (tabId, dirty) => {
  const t = tabs.value.find(x => x.id === tabId)
  if (t) { t.dirty = dirty }
}

// 编辑器内切换连接
const onEditorConnChange = async (connId) => {
  if (!connId) return
  await selectConn(connId)
}

// 编辑器内切换数据库
const onEditorDbChange = (db) => {
  if (db && db !== currentDb.value) {
    setCurrentDb(db)
    refreshAllTables()
  }
}

const sqlViewRefs = {}
const setSqlRef = (id, el) => { if (el) sqlViewRefs[id] = el; else delete sqlViewRefs[id] }

const doClose = (id) => {
  const idx = tabs.value.findIndex(t => t.id === id)
  if (idx === -1) return
  tabs.value.splice(idx, 1)
  delete sqlViewRefs[id]
  // 只有点常驻页签（AI 助手 / 知识库）自己的 × 才会关掉它：同步「常驻」状态
  if (GLOBAL_TABS.some(g => g.id === id)) {
    globalTabState.value = {
      ...globalTabState.value,
      [id]: { open: false, index: globalTabState.value[id]?.index || 0 }
    }
  }
  if (activeTab.value === id) {
    activeTab.value = tabs.value[idx] ? tabs.value[idx].id : (tabs.value[idx - 1]?.id || '')
  }
}

const closeTab = async (id) => {
  const tab = tabs.value.find(x => x.id === id)
  if (tab && tab.type === 'sql' && tab.dirty) {
    let action = 'cancel'
    try {
      await ElMessageBox.confirm(
        t('mv.scriptUnsaved', { name: tabLabel(tab) }),
        t('mv.unsavedTitle'),
        {
          type: 'warning',
          confirmButtonText: t('common.save'),
          cancelButtonText: t('mv.dontSave'),
          distinguishCancelAndClose: true,
          showClose: true,
          closeOnClickModal: false
        }
      )
      action = 'save'
    } catch (e) {
      // cancel = 点「不保存」→ 直接弃用；close = 点右上角 X / ESC → 取消关闭
      action = e === 'cancel' ? 'discard' : 'cancel'
    }
    if (action === 'cancel') return
    if (action === 'save') {
      let ok
      try { ok = await (sqlViewRefs[id] ? sqlViewRefs[id].saveForClose() : true) } catch { ok = false }
      if (ok === false) return
    }
  }
  // 事务未提交：关页签 = 回滚 —— 先问一声（回滚并关闭 / 留下来自己提交）
  if (tab && tab.type === 'sql' && sqlViewRefs[id]) {
    let txOk = true
    try { txOk = await sqlViewRefs[id].txForClose() } catch { txOk = false }
    if (!txOk) return
  }
  doClose(id)
}

// ===== Tab 右键菜单 =====
const tabCtxMenu = ref({ show: false, x: 0, y: 0, tabId: '' })
const showTabCtxMenu = (e, tab) => {
  tabCtxMenu.value = { show: true, x: e.clientX, y: e.clientY, tabId: tab.id }
  nextTick(() => {
    const el = document.querySelector('.tab-ctx-menu')
    if (!el) return
    const rect = el.getBoundingClientRect()
    if (rect.right > window.innerWidth) tabCtxMenu.value.x = window.innerWidth - rect.width - 4
    if (rect.bottom > window.innerHeight) tabCtxMenu.value.y = window.innerHeight - rect.height - 4
  })
}
const closeTabCtxMenu = () => { tabCtxMenu.value.show = false }
window.addEventListener('click', closeTabCtxMenu)
window.addEventListener('scroll', closeTabCtxMenu, true)

const tabCtxClose = (mode) => {
  const targetId = tabCtxMenu.value.tabId
  const idx = tabs.value.findIndex(t => t.id === targetId)
  if (idx === -1) return
  // 常驻页签（AI 助手 / 知识库）：只认它自己页签上的 ×，
  // 「关闭其它 / 关闭左侧 / 关闭右侧 / 全部关闭」都不会把它们带走
  switch (mode) {
    case 'self':
      closeTab(targetId)
      break
    case 'others':
      tabs.value = tabs.value.filter(t => t.id === targetId || isGlobalTab(t))
      activeTab.value = targetId
      break
    case 'left':
      tabs.value = tabs.value.filter((t, i) => i >= idx || isGlobalTab(t))
      if (!tabs.value.find(t => t.id === activeTab.value)) activeTab.value = targetId
      break
    case 'right':
      tabs.value = tabs.value.filter((t, i) => i <= idx || isGlobalTab(t))
      if (!tabs.value.find(t => t.id === activeTab.value)) activeTab.value = targetId
      break
    case 'all':
      tabs.value = tabs.value.filter(isGlobalTab)
      if (!tabs.value.find(t => t.id === activeTab.value)) activeTab.value = tabs.value[0]?.id || ''
      break
  }
  closeTabCtxMenu()
}

// 表结构页里删除/重命名表后：关闭「对象详情/数据」页签并刷新树。
// 注意只关这类页签 —— 原来无脑关「当前活动页签」，若此刻活动的是 AI 助手页签会被误关
const onDetailDeleted = () => {
  const t = tabs.value.find(x => x.id === activeTab.value)
  if (t && (t.type === 'table' || t.type === 'detail' || t.type === 'object' || t.type === 'form')) {
    closeTab(activeTab.value)
  }
  refreshTree()
}

// 表格分页拿到精确 COUNT 后，同步刷新左侧树节点行数（仅当前打开的表，不开销批量查询）
const onUpdateTableRows = ({ connId, database, table, rows }) => {
  if (!connId || !database || !table) return
  // lazy 模式下表节点由 el-tree 内部管理，需通过 node-key 定位后修改 node.data
  const key = 'table:' + connId + ':' + database + ':' + table
  // 写进响应式表：COUNT(*) 回填的行数必须让模板一定看得到（只改 node.data 时实测不刷新）
  tableRows.value[key] = rows
  const node = treeRef.value?.getNode(key)
  if (node && node.data) {
    node.data.rows = rows
  }
}

// 展开库/schema 后异步回填各表的真实行数。
// 树里的行数来自 listTables 的 TABLE_ROWS，而它只是统计估算值：Doris、ClickHouse 等在未 ANALYZE
// 时恒为 0、MySQL 的 InnoDB 在没统计时干脆是 NULL，会出现「表里明明有数据、树上却没有数字」
// 或者「空表与取不到长得一样」。这里另发一次批量 COUNT(*) 把精确值写回节点。
//
// `onlyMissing = true`：只查**还不知道行数**的表。被动展开走这条 —— 否则每次展开
// （包括命中持久缓存那种）都会对着整库重跑一遍 COUNT(*)，300 张表的库就是 300 条查询，
// 而树上的数字多数时候没变。显式「刷新」传 false：用户要的就是最新值。
const fillRealRowCounts = async (connId, db, tables, onlyMissing = false) => {
  let names = (Array.isArray(tables) ? tables : [])
    .filter(t => (t.type || 'TABLE') === 'TABLE')
    .map(t => t.name)
    .filter(Boolean)
  if (onlyMissing) {
    names = names.filter(name => tableRows.value['table:' + connId + ':' + db + ':' + name] == null)
  }
  if (!connId || !db || !names.length) return
  try {
    const counts = await getTableCounts(connId, db, names)
    if (!counts) return
    // 等一帧让 el-tree 先按估算值建出节点，再回填精确行数
    await nextTick()
    Object.keys(counts).forEach(name => {
      const n = counts[name]
      if (typeof n === 'number' && n >= 0) {
        onUpdateTableRows({ connId, database: db, table: name, rows: n })
      }
    })
  } catch (e) {
    // 回填失败不影响主流程：保留估算值
  }
}

// 对象（过程/函数/视图/触发器/事件）重命名成功后：同步已打开的对象详情页签与可视化编辑页签
const applyRenameObjectTabs = (type, oldName, newName) => {
  if (!type || !oldName || !newName) return
  tabs.value.forEach((t) => {
    // 对象详情页签：type='object'，id 形如 "view:名" 或 "object:view:名"
    if (t.type === 'object' && t.objectName === oldName) {
      const oldId = t.id
      t.objectName = newName
      // id 形如 "view:名@连接"，重建时保留连接后缀
      t.id = tabKey(String(oldId).startsWith('object:') ? 'object:' + type + ':' + newName : type + ':' + newName, t.connId)
      if (t.label) t.label = String(t.label).replace(oldName, newName)
      if (activeTab.value === oldId) activeTab.value = t.id
    }
    // 可视化编辑页签（ObjectFormTab）：对象名挂在 tab.object.name 上
    if (t.type === 'form' && t.object && t.object.name === oldName) {
      t.object = { ...t.object, name: newName }
      if (t.label) t.label = String(t.label).replace(oldName, newName)
    }
  })
}

// 表重命名成功后：同步已打开的 数据/结构 页签（table/id/label）
const applyRenameTabs = (data, newName) => {
  if (!data || !newName) return
  const oldName = data.oldName || data.table
  const db = data.db || currentDb.value
  tabs.value.forEach((t) => {
    if (t.table === oldName && (t.database || currentDb.value) === db) {
      t.table = newName
      // id 形如 "table:名@连接"，前缀判断 + 保留连接后缀（重命名只换名字）
      const oldId = String(t.id || '')
      if (oldId.startsWith('detail:')) {
        t.id = tabKey('detail:' + newName, t.connId)
        t.label = newName + t('mv.structureSuffix')
      } else if (oldId.startsWith('table:')) {
        t.id = tabKey('table:' + newName, t.connId)
        t.label = newName
      }
      if (activeTab.value === oldId) activeTab.value = t.id
    }
  })
}

const onMainTabChange = () => {
  // 切换 tab 后，让内容区组件重新计算布局（表格高度等）
  nextTick(() => {
    window.dispatchEvent(new CustomEvent('dc-tab-change'))
    setTimeout(() => window.dispatchEvent(new CustomEvent('dc-tab-change')), 80)
  })
}

// ===== 页签拖拽排序 =====
// 只改 tabs 数组的顺序：页签栏、内容区、会话存档都从它派生，改一处即可。
// 用原生 HTML5 拖拽而不是引第三方库，页签这点需求不值得多一个依赖。
const dragTabId = ref('')
const dragOverTabId = ref('')
// 落点在目标页签的**左半还是右半**：决定插到它前面还是后面。
// 只知道"拖到了哪个页签"是不够的 —— 那样靠右拖也会插到前面去，指示条也就画不准。
const dragOverSide = ref('')

const onTabDragStart = (e, tab) => {
  dragTabId.value = tab.id
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = 'move'
    // Firefox 不设置数据就根本不触发拖拽，这行不能省
    e.dataTransfer.setData('text/plain', tab.id)
  }
}

const onTabDragOver = (e, tab) => {
  if (!dragTabId.value || dragTabId.value === tab.id) return
  e.preventDefault()   // 不阻止默认行为就不会触发 drop
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move'
  dragOverTabId.value = tab.id
  const el = e.currentTarget || e.target
  if (el && el.getBoundingClientRect) {
    const r = el.getBoundingClientRect()
    dragOverSide.value = e.clientX < r.left + r.width / 2 ? 'before' : 'after'
  }
}

const onTabDrop = (e, tab) => {
  e.preventDefault()
  const from = dragTabId.value
  const side = dragOverSide.value
  dragTabId.value = ''
  dragOverTabId.value = ''
  dragOverSide.value = ''
  if (!from || from === tab.id) return
  const arr = [...tabs.value]
  const fi = arr.findIndex(t => t.id === from)
  if (fi < 0) return
  const [moved] = arr.splice(fi, 1)
  // 先把自己摘掉再找落点：往右拖时目标索引会因此少一位，
  // 不重算的话「落在右半」就会插到左边去（这正是改之前的表现）
  let at = arr.findIndex(t => t.id === tab.id)
  if (at < 0) return
  if (side === 'after') at += 1
  arr.splice(at, 0, moved)
  tabs.value = arr
  // 常驻页签的位置记录要跟着更新，否则切走再切回来会被按旧位置插回
  saveConnState()
}

const onTabDragEnd = () => { dragTabId.value = ''; dragOverTabId.value = ''; dragOverSide.value = '' }

// ===== 整条页签栏都是落点 =====
// 页签自己的 dragover 只覆盖「标签文字」那一小块 —— 页签之间的缝隙、最后一个页签右边的
// 空白都落不下去（想"挪到最后"就得精准压在最后一个页签的右半上，很难）。
// 所以再加一层"按指针位置解析落点"的监听，挂在内容区上、用「指针是否在页签栏那条带内」当闸门
// （指针在下面的数据区时不插手）。
const TABS_STRIP_GUARD = 8   // 页签栏下沿再多给 8px，正好压在边界上也不至于失效

const tabStripHeaderEl = () => {
  const host = document.querySelector('.dc-tabs')
  return host ? host.querySelector('.el-tabs__header') : null
}

const inTabStrip = (e) => {
  const header = tabStripHeaderEl()
  if (!header) return false
  return e.clientY <= header.getBoundingClientRect().bottom + TABS_STRIP_GUARD
}

/**
 * 按指针横坐标解出「落在哪个页签的哪一侧」：
 * 落在某页签左半 → before、右半 → after；落在两页签之间的缝隙 → 归到右边那个的 before；
 * 落在最后一个页签右边（那片空白）→ 追加到末尾。DOM 里的页签顺序与 tabs 数组一致，按索引取即可。
 */
const resolveTabDropTarget = (clientX) => {
  const host = document.querySelector('.dc-tabs')
  if (!host) return null
  const items = [...host.querySelectorAll('.el-tabs__item')]
  if (!items.length) return null
  for (let i = 0; i < items.length; i++) {
    const r = items[i].getBoundingClientRect()
    if (clientX < r.left) return { tab: tabs.value[i], side: 'before' }
    if (clientX <= r.right) {
      return { tab: tabs.value[i], side: clientX < r.left + r.width / 2 ? 'before' : 'after' }
    }
  }
  return { tab: tabs.value[items.length - 1], side: 'after' }
}

const onTabsStripDragOver = (e) => {
  if (!dragTabId.value || !inTabStrip(e)) return
  const hit = resolveTabDropTarget(e.clientX)
  if (!hit || !hit.tab) return
  e.preventDefault()   // 不阻止默认行为就不会触发 drop
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move'
  dragOverTabId.value = hit.tab.id
  dragOverSide.value = hit.side
}

const onTabsStripDrop = (e) => {
  if (!dragTabId.value || !inTabStrip(e)) return
  const hit = resolveTabDropTarget(e.clientX)
  if (!hit || !hit.tab) return
  dragOverSide.value = hit.side   // 落点方向按"位置解析"的结果为准，再交给同一个 onTabDrop
  onTabDrop(e, hit.tab)
}

const onTabsStripDragLeave = (e) => {
  if (!dragTabId.value) return
  // 在页签栏内部元素之间挪动也会触发 dragleave，只有**真的离开**这条带才清掉落点
  if (inTabStrip(e)) return
  dragOverTabId.value = ''
  dragOverSide.value = ''
}

/**
 * 顶部「AI 助手」：打开中央工作区形态（与「表数据 / SQL 查询」并列的 Tab）。
 * 侧栏形态（Ctrl/⌘+K）保留用于「边看数据边问」，两者共用同一个 AiPanel 组件。
 */
const toggleAi = () => {
  openTab({ id: 'ai:studio', type: 'ai', label: t('empty.aiAssistant') })
}

/**
 * AI 助手输入框里的「斜杠命令」要求打开某个面板时由此接管。
 * 面板归属留在 MainView，AI 助手只发意图，避免两个组件互相耦合。
 */
/** 首页「数据工具」直达治理页签：质量规则 / 质量分析（与 AI 面板命令同一套定位逻辑） */
const openGovernance = (tab) => {
  governanceTab.value = tab
  governanceAutoRun.value = GOVERNANCE_AUTORUN_TABS.includes(tab)
  governanceOpen.value = true
}
const onAiPanelCommand = (key) => {
  // 治理的四个页签可以直接被点名打开（敏感数据 / 数据容量 / 质量规则 / 质量分析）；
  // 敏感数据 / 数据容量属于「点开即出结果」的扫描，跳过去就直接开跑
  if (GOVERNANCE_TABS.includes(key)) {
    governanceTab.value = key
    governanceAutoRun.value = GOVERNANCE_AUTORUN_TABS.includes(key)
    governanceOpen.value = true
  }
  else if (key === 'governance') { governanceAutoRun.value = false; governanceOpen.value = true }
  else if (key === 'settings') settingsOpen.value = true
  else if (key === 'compare') openCompare()
  else if (key === 'sync') syncDialogOpen.value = true
  // 连接管理：新建走数据源选择器，编辑直接打开当前连接的表单
  else if (key === 'newConn') openNewConn('')
  else if (key === 'editConn') {
    if (conn.value) openConnDialog(conn.value)
    else ElMessage.warning(t('mv.pickConnFirst'))
  }
}
const insertSqlToActive = (sql, autoRun = false) => {
  const active = tabs.value.find(t => t.id === activeTab.value)
  if (active && active.type === 'sql') {
    // 已有 SQL 标签页：通过事件广播；autoRun 时让编辑器写入后直接执行
    window.dispatchEvent(new CustomEvent(autoRun ? 'dc-run-sql' : 'dc-insert-sql', { detail: sql }))
  } else {
    // 新建标签页：通过 initialSql prop 传递，避免 Monaco 未初始化导致注入失败
    newQueryTab({ initialSql: sql, autoRun })
  }
}

// 树节点右键：表/视图/对象走 .ctx-menu；folder/conn/category 走 .tree-ctx-menu
const onTreeNodeContextMenu = (e, data) => {
  if (data.kind === 'table' || data.kind === 'view'
      || data.kind === 'function' || data.kind === 'procedure'
      || data.kind === 'trigger' || data.kind === 'event' || data.kind === 'user') {
    ctxMenu.value = {
      show: true, x: e.clientX, y: e.clientY,
      subKey: null,
      data: { ...data, objectKind: data.kind }
    }
    nextTick(() => {
      const el = document.querySelector('.ctx-menu')
      if (!el) return
      const rect = el.getBoundingClientRect()
      if (rect.right > window.innerWidth) ctxMenu.value.x = window.innerWidth - rect.width - 4
      if (rect.bottom > window.innerHeight) ctxMenu.value.y = window.innerHeight - rect.height - 4
    })
    treeCtxMenu.value.show = false
  } else if (data.kind === 'env-folder') {
    showTreeCtxMenu(e, 'env-folder', data.env, null)
    ctxMenu.value.show = false
  } else if (data.kind === 'conn') {
    // 新建/新建脚本等操作继承连接所在的"目录"（environment），避免新数据源落到不可见分组
    showTreeCtxMenu(e, 'conn', data.environment || data.env, data)
    ctxMenu.value.show = false
  } else if (data.kind === 'catalog') {
    // catalog 节点右键：它下面那层库是单独查出来的（`?catalog=`），
    // 所以能做的事就是"刷新这份列表"和"在这个连接下新建查询"。
    // 库那一套动作（打开数据库 / 查看建库语句 / 转储）对 catalog 不适用，别硬套 ——
    // 套了只会是点下去报错或打开一个不存在的库。
    showTreeCtxMenu(e, 'catalog', '', data)
    ctxMenu.value.show = false
  } else if (data.kind === 'db') {
    // 数据库节点右键
    showTreeCtxMenu(e, 'db', '', data)
    ctxMenu.value.show = false
  } else if (data.kind === 'schema') {
    // schema 节点没有专属动作：能做的事与所属库基本一致（新建查询 / 查看建库语句 / 转储 /
    // 查找…），所以复用「库」的菜单。
    // 但有两处必须换值：菜单里的动作都按 `db` 取库名，而 schema 节点的 `db` 是「库.schema」
    // （树节点 id 需要它），直接复用会让「转储 / 查找」拿到一个带点的库名。换成 `realDb`。
    // 之前这里**根本没有 schema 分支** —— 模板里那段 schema 菜单是死的，右键 schema 毫无反应。
    showTreeCtxMenu(e, 'schema', '', { ...data, db: data.realDb || data.db })
    ctxMenu.value.show = false
  } else if (data.kind === 'collection') {
    // NoSQL collection / key / index 右键
    ctxMenu.value = {
      show: true, x: e.clientX, y: e.clientY,
      subKey: null,
      data: { ...data, objectKind: 'collection' }
    }
    nextTick(() => {
      const el = document.querySelector('.ctx-menu')
      if (!el) return
      const rect = el.getBoundingClientRect()
      if (rect.right > window.innerWidth) ctxMenu.value.x = window.innerWidth - rect.width - 4
      if (rect.bottom > window.innerHeight) ctxMenu.value.y = window.innerHeight - rect.height - 4
    })
    treeCtxMenu.value.show = false
  } else if (data.kind === 'category') {
    // 分类目录：新建对象 / 刷新
    showTreeCtxMenu(e, 'category', '', data)
    ctxMenu.value.show = false
  } else if (data.kind === 'script') {
    // 脚本节点：右键弹确认删除
    ctxMenu.value.show = false
    treeCtxMenu.value.show = false
    e.preventDefault?.()
    ElMessageBox.confirm(t('mv.deleteScriptBody', { name: data.label }), t('mv.deleteScriptTitle'), { type: 'warning', confirmButtonText: t('common.delete'), cancelButtonText: t('common.cancel') })
      .then(() => deleteScriptNode(data))
      .catch(() => {})
  } else {
    // db / placeholder 等不弹菜单
    e.preventDefault?.()
  }
}

// 树空白区右键：新建连接 / 新建分组
const onTreeAreaContextMenu = (e) => {
  // 只在没点到节点时触发（el-tree 的 @node-contextmenu 已先 stop）
  if (e.target.closest('.el-tree-node__content')) return
  showTreeCtxMenu(e, 'blank', '', null)
  ctxMenu.value.show = false
}

const showTreeCtxMenu = (e, kind, env, data) => {
  treeCtxMenu.value = {
    show: true, x: e.clientX, y: e.clientY, kind, env,
    cat: data?.cat || '', db: data?.db || '', data
  }
  // 防止超出右/下边界
  nextTick(() => {
    const el = document.querySelector('.tree-ctx-menu')
    if (!el) return
    const rect = el.getBoundingClientRect()
    if (rect.right > window.innerWidth) treeCtxMenu.value.x = window.innerWidth - rect.width - 4
    if (rect.bottom > window.innerHeight) treeCtxMenu.value.y = window.innerHeight - rect.height - 4
  })
}

const ctxSub = (key) => { ctxMenu.value.subKey = key }
const closeCtxMenu = () => { ctxMenu.value.show = false; ctxMenu.value.subKey = null }
// 具名而非匿名：匿名函数没法 removeEventListener，卸载后就永远留在 window 上了
const onCtxMenuEscape = (e) => { if (e.key === 'Escape') closeCtxMenu() }
window.addEventListener('click', closeCtxMenu)
window.addEventListener('keydown', onCtxMenuEscape)

// ====== 表右键菜单操作 ======
const ctxOpenData = async () => {
  if (!ctxMenu.value.data) return
  await openTable(ctxMenu.value.data)
  closeCtxMenu()
}
const ctxOpenDetail = async () => {
  if (!ctxMenu.value.data) return
  await openDetail(ctxMenu.value.data, ctxMenu.value.data.objectKind)
  closeCtxMenu()
}
const ctxRenameTable = () => {
  const d = ctxMenu.value.data
  if (!d || !d.connId) return
  renameTableCtx.value = { connId: d.connId, db: d.db || currentDb.value, oldName: d.table }
  closeCtxMenu()
  renameTableVisible.value = true
}
// 重命名表（表单 / 执行在 RenameTableDialog.vue，成功后同步已打开页签并刷新分类节点）
const onTableRenamed = ({ connId, db, oldName, newName }) => {
  applyRenameTabs({ db, oldName }, newName)
  refreshCatNode(db, 'tables', connId)
}

// 重命名对象（存储过程/函数/视图/触发器/事件）：右键菜单入口，成功后同步页签并刷新对应分类节点。
// 对象节点不带 connId/db，回退到当前连接的库上下文（与 objEditDdl / objDrop 一致）
const ctxRenameObject = () => {
  const d = ctxMenu.value.data
  if (!d) return
  renameObjectCtx.value = {
    connId: d.connId || currentConnId.value,
    db: d.db || currentDb.value,
    type: d.objectKind,
    oldName: objName(d)
  }
  closeCtxMenu()
  renameObjectVisible.value = true
}
// 重命名对象（表单 / 预览 / 执行在 RenameObjectDialog.vue）
const onObjectRenamed = ({ connId, db, type, oldName, newName }) => {
  applyRenameObjectTabs(type, oldName, newName)
  const cat = OBJ_CAT[type]
  if (cat) refreshCatNode(db, cat, connId)
}
const ctxShowDdl = async () => {
  if (!ctxMenu.value.data) return
  const d = ctxMenu.value.data
  closeCtxMenu()
  try {
    const res = await getTableDdl(conn.value.id, d.db || currentDb.value, d.table)
    ctxDdl.value = res.ddl || ''
    ctxDdlVisible.value = true
  } catch (e) { ElMessage.error(t('mv.ddlFailed', { detail: errMsg(e) })) }
}
const ctxOpenImport = () => {
  const d = ctxMenu.value.data
  if (!d) return
  ctxImportTarget.value = { ...d, db: d.db || currentDb.value }
  closeCtxMenu()
  ctxImportVisible.value = true
}
// 导入数据成功后刷新树（文件选择 / 上传在 ImportDataDialog.vue）
const onImportDone = () => refreshTree()

// 打开「数据生成」弹窗：快照右键命中的表（含所属连接，避免跨连接串台）
const ctxOpenDataGen = () => {
  const d = ctxMenu.value.data
  if (!d) return
  const cid = d.connId || currentConnId.value
  ctxDataGenTarget.value = { ...d, connId: cid, db: d.db || currentDb.value }
  closeCtxMenu()
  ctxDataGenVisible.value = true
}
// 生成成功后刷新树（字段规则与数据在 DataGenDialog.vue）
const onDataGenDone = () => refreshTree()

// 复制文本到剪贴板（navigator.clipboard 不可用时降级 textarea + execCommand）
const copyText = (text, tip) => {
  const fallback = () => {
    const ta = document.createElement('textarea')
    ta.value = text
    ta.style.position = 'fixed'
    ta.style.opacity = '0'
    document.body.appendChild(ta)
    ta.select()
    try { document.execCommand('copy'); ElMessage.success(tip) } catch { ElMessage.error(t('sqlq.copyFailed')) }
    document.body.removeChild(ta)
  }
  if (navigator.clipboard?.writeText) {
    navigator.clipboard.writeText(text).then(() => ElMessage.success(tip)).catch(fallback)
  } else {
    fallback()
  }
}

// 复制表/对象 DDL（弹窗内复制按钮）
const copyCtxDdl = () => copyText(ctxDdl.value || '', t('mv.ddlCopied'))

// 复制建库语句（弹窗内复制按钮）
const copyDbDdl = () => copyText(dbDdlText.value || '', t('mv.createDbCopied'))

// 按数据库类型生成标识符引用（反引号 / 方括号 / 双引号），引号风格来自类型注册表
// type 可选：不传时回退到当前连接类型（兼容原有调用方）；传了则按目标连接类型生成（如跨连接生成 SQL）
const quoteIdent = (name, type) => {
  const s = String(name ?? '')
  const style = quoteStyleOf(type || conn.value?.type)
  if (style === 'BACKTICK') return '`' + s.replace(/`/g, '``') + '`'
  if (style === 'BRACKET') return '[' + s + ']'
  return '"' + s.replace(/"/g, '""') + '"'
}

const ctxCopyName = () => {
  if (!ctxMenu.value.data) return
  const d = ctxMenu.value.data
  closeCtxMenu()
  copyText(d.table, t('mv.tableNameCopied'))
}

// 视图 / 存储过程 / 函数 / 触发器 / 事件：复制对象名称
const ctxCopyObjName = () => {
  const d = ctxMenu.value.data
  if (!d) return
  closeCtxMenu()
  copyText(objName(d), t('mv.nameCopied'))
}

// ====== 右键「查看索引」（只读快照）======
// 拿该连接**整库**的索引清单，再按表名过滤 —— 而不是让后端"按表查"：
// 各库的索引元数据 SQL 差异极大（已在 dialect 里逐库写好），加一个 table 参数就得把
// 那些 SQL 全再写一遍；而索引总量很小（一张表几条、一个库几十条），UI 侧过滤是廉价的。
const ctxIndexVisible = ref(false)
const ctxIndexLoading = ref(false)
const ctxIndexTable = ref('')
const ctxIndexCols = ref([])
const ctxIndexRows = ref([])

const ctxShowIndexes = async () => {
  const d = ctxMenu.value.data
  if (!d) return
  const cid = conn.value?.id || currentConnId.value
  const want = String(d.table || d.label || '')
  closeCtxMenu()
  ctxIndexTable.value = want
  ctxIndexCols.value = []
  ctxIndexRows.value = []
  ctxIndexVisible.value = true
  ctxIndexLoading.value = true
  try {
    const res = await listIndexes(cid, d.db || currentDb.value)
    const rows = Array.isArray(res?.rows) ? res.rows : (Array.isArray(res) ? res : [])
    const cols = (Array.isArray(res?.columns) && res.columns.length)
      ? res.columns
      : Object.keys(rows[0] || {})
    // 表名所在的列名各库不一（sqlite/sqlserver 叫 `table`、oracle 亦同、部分实现带 schema 前缀），
    // 所以不按固定列名取，而是「任一字段等于表名」；再不行退一步做包含匹配。
    let filtered = want ? rows.filter(r => Object.values(r || {}).some(v => String(v) === want)) : rows
    if (!filtered.length && want) {
      const low = want.toLowerCase()
      filtered = rows.filter(r => Object.values(r || {}).some(v => String(v).toLowerCase().includes(low)))
    }
    ctxIndexCols.value = cols
    ctxIndexRows.value = filtered
  } catch (e) {
    ctxIndexVisible.value = false
    ElMessage.error(t('mv.loadIndexesFailed', { detail: errMsg(e) }))
  } finally {
    ctxIndexLoading.value = false
  }
}

const copyCtxIndexes = () => {
  if (!ctxIndexRows.value.length) return
  const cols = ctxIndexCols.value
  const text = cols.join('\t') + '\n' +
    ctxIndexRows.value.map(r => cols.map(c => (r?.[c] ?? '')).join('\t')).join('\n')
  copyText(text, t('mv.indexesCopied'))
}

const ctxCopyDdl = async () => {
  if (!ctxMenu.value.data) return
  const d = ctxMenu.value.data
  closeCtxMenu()
  try {
    const res = await getTableDdl(conn.value.id, d.db || currentDb.value, d.table)
    copyText(res.ddl || '', t('mv.createTableCopied'))
  } catch (e) { ElMessage.error(t('mv.ddlFailed', { detail: errMsg(e) })) }
}

const ctxCopySelect = () => {
  if (!ctxMenu.value.data) return
  const d = ctxMenu.value.data
  closeCtxMenu()
  copyText(`SELECT * FROM ${quoteIdent(d.table)};`, t('mv.selectCopied'))
}

// 视图专用：复制 SELECT 语句
const ctxCopyViewSelect = () => {
  const d = ctxMenu.value.data
  if (!d) return
  closeCtxMenu()
  copyText(`SELECT * FROM ${quoteIdent(d.table || d.label)};`, t('mv.selectCopied'))
}

// 视图专用：复制视图定义（DDL）
const ctxCopyViewDdl = async () => {
  const d = ctxMenu.value.data
  if (!d) return
  closeCtxMenu()
  try {
    const res = await getTableDdl(conn.value.id, d.db || currentDb.value, d.table || d.label)
    copyText(res.ddl || '', t('mv.viewDefCopied'))
  } catch (e) { ElMessage.error(t('mv.loadViewDefFailed', { detail: errMsg(e) })) }
}

// 生成 SQL 模板并注入 SQL 编辑器（基于真实列结构）
const ctxGenSql = async (tpl) => {
  if (!ctxMenu.value.data) return
  const d = ctxMenu.value.data
  // 关键：使用右键命中表所属的连接（tab 节点已带 connId），而非当前活动连接，
  // 否则在别的库/连接上右键会串到当前连接（如 clickhouse 表生成出 sqlserver 语法）。
  const cid = d.connId || currentConnId.value
  const connType = connTypeOf(cid)
  const db = d.db || currentDb.value
  const name = d.table || d.label
  closeCtxMenu()
  let cols = []
  try {
    const list = await listColumns(cid, db, name)
    cols = list.map(c => c.name).filter(Boolean)
  } catch { /* 列信息获取失败时生成无列模板 */ }
  const t = quoteIdent(name, connType)
  let sql = ''
  if (tpl === 'select') {
    sql = `SELECT * FROM ${t};\n`
  } else if (tpl === 'insert') {
    sql = cols.length
      ? `INSERT INTO ${t} (${cols.map(c => quoteIdent(c, connType)).join(', ')})\nVALUES (${cols.map(() => '?').join(', ')});\n`
      : `INSERT INTO ${t} (...);\n`
  } else if (tpl === 'update') {
    const sets = cols.length ? cols.map(c => `  ${quoteIdent(c, connType)} = ?`).join(',\n') : '  col = ?'
    const where = cols.length ? `\nWHERE ${quoteIdent(cols[0], connType)} = ?` : ''
    sql = `UPDATE ${t}\nSET\n${sets}${where};\n`
  } else if (tpl === 'delete') {
    const where = cols.length ? `\nWHERE ${quoteIdent(cols[0], connType)} = ?` : ''
    sql = `DELETE FROM ${t}${where};\n`
  }
  newQueryTab({ connId: cid, database: db, initialSql: sql })
}

// 危险操作：清空 / 截断 / 删除表
const ctxClearTable = () => {
  if (!ctxMenu.value.data) return
  const d = ctxMenu.value.data
  closeCtxMenu()
  ElMessageBox.confirm(
    t('mv.clearTableBody', { name: d.table }),
    t('mv.clearTableTitle'),
    { type: 'warning', confirmButtonText: t('mv.clearTableConfirm'), cancelButtonText: t('common.cancel') }
  ).then(() => doTableAction('clear', t('mv.clearTableTitle'))).catch(() => {})
}
const ctxTruncateTable = () => {
  if (!ctxMenu.value.data) return
  const d = ctxMenu.value.data
  closeCtxMenu()
  ElMessageBox.confirm(
    t('mv.truncateBody', { name: d.table }),
    t('mv.truncateTitle'),
    { type: 'warning', confirmButtonText: t('mv.truncateConfirm'), cancelButtonText: t('common.cancel') }
  ).then(() => doTableAction('truncate', t('mv.truncateTitle'))).catch(() => {})
}
const ctxDropTable = () => {
  if (!ctxMenu.value.data) return
  const d = ctxMenu.value.data
  closeCtxMenu()
  ElMessageBox.confirm(
    t('mv.dropTableBody', { name: d.table }),
    t('mv.dropTableTitle'),
    { type: 'error', confirmButtonText: t('mv.dropConfirm'), cancelButtonText: t('common.cancel') }
  ).then(() => doTableAction('drop', t('mv.dropTableTitle'))).catch(() => {})
}
// ====== 危险操作进度（单个 DDL / 批量删除）======
// 以前是一个全屏转圈：看不出"在干活"还是"卡死了"，也没法中止。现在统一有计时 + 逐条日志。
//
// 两类口径**必须区分**（混在一起就是骗用户）：
//   · 单个 DDL（drop / truncate / clear）与单条删除连接：语句已经交给数据库，
//     客户端**中断不了** → 不给取消按钮，只显示用时与"请勿重复操作"。
//   · 批量删除连接：循环在**前端**，取消是真的停 → 给取消按钮，并写明已删的不回滚。
//
// 另外**延迟 400ms 才弹窗**：秒级操作根本看不到窗口，只有真慢的时候才出现 ——
// 否则每次删张表都闪一个对话框，比转圈还烦。
const dangerVisible = ref(false)
const dangerTitle = ref('')
const dangerTargetName = ref('')
const dangerStatus = ref('running')
const dangerMessage = ref('')
const dangerLogs = ref([])
const dangerCancellable = ref(false)
const dangerCanceled = ref(false)
let dangerTimer = null
let dangerShowTimer = null
let dangerStartedAt = 0
let dangerShown = false

const dangerNow = () => new Date().toLocaleTimeString('zh-CN', { hour12: false })
const dangerLog = (text) => { dangerLogs.value.push({ time: dangerNow(), text }) }

const dangerBegin = (title, target, { cancellable = false, message = '' } = {}) => {
  dangerTitle.value = title
  dangerTargetName.value = target || ''
  dangerStatus.value = 'running'
  dangerMessage.value = message
  dangerLogs.value = []
  dangerCancellable.value = cancellable
  dangerCanceled.value = false
  dangerShown = false
  dangerStartedAt = Date.now()
  dangerLog(t('mv.submitted'))
  if (dangerTimer) clearInterval(dangerTimer)
  // 每秒刷新"已用时"那一行（就地改最后一行，不刷屏）
  dangerTimer = setInterval(() => {
    if (dangerStatus.value !== 'running') return
    const text = t('mv.elapsedSec', { s: ((Date.now() - dangerStartedAt) / 1000).toFixed(0) })
    const last = dangerLogs.value[dangerLogs.value.length - 1]
    // 前缀必须与上面生成时用同一个键：写死中文的话换语言后这里就认不出"计时那一行"了
  if (last && last.text.startsWith(t('mv.elapsedPrefix'))) { last.text = text; last.time = dangerNow() }
    else dangerLog(text)
  }, 1000)
  if (dangerShowTimer) clearTimeout(dangerShowTimer)
  dangerShowTimer = setTimeout(() => {
    if (dangerStatus.value === 'running') {
      dangerShown = true
      dangerVisible.value = true
    }
  }, 400)
}

const dangerFinish = (ok, message) => {
  if (dangerTimer) { clearInterval(dangerTimer); dangerTimer = null }
  if (dangerShowTimer) { clearTimeout(dangerShowTimer); dangerShowTimer = null }
  const seconds = ((Date.now() - dangerStartedAt) / 1000).toFixed(1)
  dangerStatus.value = ok ? 'success' : 'error'
  dangerMessage.value = message || ''
  if (ok) dangerLog(t('mv.doneInSec', { s: seconds }))
  // 没弹过窗（快操作）就别弹了：只留 ElMessage，不然会被一个"已经完成"的对话框拦住
  if (!dangerShown) dangerVisible.value = false
}

const dangerCancel = () => {
  dangerCanceled.value = true
  dangerMessage.value = t('mv.stopping')
}

const doTableAction = async (action, label) => {
  const d = ctxMenu.value.data
  if (!d || !conn.value) return
  const target = ((d.db || currentDb.value) ? (d.db || currentDb.value) + '.' : '') + d.table
  dangerBegin(label + '：' + target, d.table, {
    message: t('mv.stmtSubmitted')
  })
  try {
    const res = await tableAction(conn.value.id, d.db || currentDb.value, d.table, action)
    if (res.success) {
      ElMessage.success(res.message || t('mv.opSuccess', { label }))
      if (action === 'drop') {
        // 关闭该表相关的数据/结构页签
        const db = d.db || currentDb.value
        const dc = String(d.connId || currentConnId.value || '')
        // 只关这个连接下的同名表页签：页签栏全局之后，其它数据源的同名表不能被牵连
        const idx = tabs.value.findIndex(t => (t.type === 'table' || t.type === 'detail') && t.table === d.table && (t.database || currentDb.value) === db && String(t.connId || '') === dc)
        if (idx !== -1) tabs.value.splice(idx, 1)
        if (!tabs.value.find(t => t.id === activeTab.value)) {
          activeTab.value = tabs.value[tabs.value.length - 1]?.id || ''
        }
      }
      if (action === 'drop') {
        refreshCatNode(d.db || currentDb.value, 'tables')
      } else {
        refreshCatNode(d.db || currentDb.value, 'tables')
      }
      dangerFinish(true, res.message || t('mv.opSuccess', { label }))
    } else {
      dangerFinish(false, (res.message || label) + t('mv.checkEditorResult'))
      ElMessage.error((res.message || label) + t('mv.checkEditorResultErr'))
    }
  } catch (e) {
    dangerFinish(false, t('mv.opFailed', { label, detail: errMsg(e) }))
    ElMessage.error(t('mv.opFailed', { label, detail: errMsg(e) }))
  }
}

// 刷新表信息：只刷新 Tables 分类节点
const ctxRefreshRows = () => {
  closeCtxMenu()
  const d = ctxMenu.value.data
  refreshCatNode(d?.db || currentDb.value, 'tables')
}

// ====== NoSQL collection 右键菜单操作 ======
const ctxOpenNoSqlData = () => {
  if (!ctxMenu.value.data) return
  openNoSql(ctxMenu.value.data)
  closeCtxMenu()
}
const ctxDropCollection = () => {
  if (!ctxMenu.value.data) return
  const d = ctxMenu.value.data
  closeCtxMenu()
  const label = d.connType === 'REDIS' ? 'Key' : (d.connType === 'ELASTICSEARCH' ? t('mv.catIndex') : t('mv.catCollection'))
  ElMessageBox.confirm(t('mv.deleteItemBody', { kind: label, name: d.label }), t('mv.deleteConfirmTitle'), {
    type: 'warning', confirmButtonText: t('mv.dropConfirm'), cancelButtonText: t('common.cancel')
  }).then(() => doDropCollection(d)).catch(() => {})
}
const doDropCollection = async (d) => {
  try {
    await noSqlDeleteCollection(d.connId, d.database, d.collection)
    ElMessage.success(t('mv.deleteOk'))
    refreshDbNode(d.database, d.connId)
  } catch (e) {
    ElMessage.error(t('mv.deleteFailed', { detail: errMsg(e) }))
  }
}
const ctxRefreshCollection = () => {
  closeCtxMenu()
  const d = ctxMenu.value.data
  if (d?.database && d?.connId) refreshDbNode(d.database, d.connId)
}

// AI 生成查询：打开 AI 面板并自动生成 SELECT
const ctxAiQuery = async () => {
  const d = ctxMenu.value.data
  closeCtxMenu()
  if (!conn.value) { ElMessage.warning(t('mv.pickConn')); return }
  aiOpen.value = true
  const tableName = d?.label || d?.objectName || ''
  const prompt = t('mv.aiGenQuery', { table: tableName })
  try {
    const res = await aiNl2sql({ prompt, connectionId: conn.value.id, database: d.db || currentDb.value })
    if (res.success) {
      insertSqlToActive(res.sql)
      ElMessage.success(t('mv.aiGenerated'))
    } else {
      ElMessage.error(res.message || t('mv.genFailed'))
    }
  } catch (e) { ElMessage.error(t('mv.aiReqFailed', { detail: (e?.message || e) })) }
}

/**
 * 「把表结构存入知识库」：生成这张表的结构说明并收进知识库。
 *
 * <p>用途是把散落在数据库里的隐性知识固化下来 —— 表结构、字段含义这些问 AI 反而不如
 * 直接查资料准确，存进知识库后每次提问都能被召回。
 */
const saveKbOpen = ref(false)
const saveKbContent = ref('')
const saveKbTitle = ref('')
const saveKbSource = ref('')

/** 「把表结构存入知识库」：内容由「数据字典」生成，再交给 SaveToKbDialog 选目标库 */
const ctxSaveTableToKb = async () => {
  const d = ctxMenu.value.data
  closeCtxMenu()
  if (!conn.value) { ElMessage.warning(t('mv.pickConn')); return }
  const table = d?.name || d?.table || d?.label
  if (!table) { ElMessage.warning(t('mv.noTableIdentified')); return }
  const db = currentDb.value
  const loading = ElLoading.service({ lock: true, text: t('mv.generatingSchemaDoc'), background: 'rgba(0,0,0,0.5)' })
  try {
    const res = await aiDataDict({ connectionId: conn.value.id, database: db, tables: [table] })
    if (!res?.success) throw new Error(res.message || t('mv.genFailed'))
    saveKbContent.value = res.content || ''
    saveKbTitle.value = t('mv.schemaDocTitle', { table })
    saveKbSource.value = [conn.value.name || conn.value.type, db, table].filter(Boolean).join(' · ')
    saveKbOpen.value = true
  } catch (e) {
    ElMessage.error(t('mv.genFailedDetail', { detail: (e?.message || e) }))
  } finally {
    loading.close()
  }
}

// ===== 对象树里的 AI 入口 =====
// 取舍：只保留「能在原地出结果」的两个 —— 解释对象、数据洞察，点击即弹窗给结论。
// 原先还有一个「在 AI 助手里继续问」（只预填上下文、不发送），入口已按要求移除。

/** 「AI 解释这个对象」：取对象 DDL 交给 AI 逐段解释；表与视图共用同一套流程 */
const aiExplainObject = async (d, kindLabel) => {
  if (!conn.value) { ElMessage.warning(t('mv.pickConn')); return }
  const name = d?.label || d?.objectName || ''
  const db = d?.db || currentDb.value
  if (!name) return
  const loading = ElLoading.service({ lock: true, text: t('mv.loadingObjectDef'), background: 'rgba(0,0,0,0.5)' })
  try {
    const ddlRes = await getTableDdl(conn.value.id, db, name)
    const ddl = ddlRes?.ddl || ''
    if (!ddl || ddl.startsWith('-- 获取 DDL 失败')) {
      ElMessage.error(t('mv.loadObjectDefFailed'))
      return
    }
    loading.setText(t('mv.aiExplaining'))
    const res = await aiExplain({ sql: ddl, connectionId: conn.value.id, database: db })
    if (res?.success) {
      aiExplainTitle.value = t('mv.aiExplainTitle', { kind: kindLabel, name })
      aiExplainContent.value = renderMarkdown(res.content || '')
      aiExplainVisible.value = true
    } else {
      ElMessage.error(res?.message || t('mv.explainFailed'))
    }
  } catch (e) { ElMessage.error(t('mv.aiReqFailed', { detail: (e?.message || e) })) } finally {
    loading.close()
  }
}

/**
 * 「解释这张表」。
 *
 * 注：表右键的 AI 子菜单目前只保留「数据洞察 / 把表结构存入知识库」，本入口已移除、
 * 暂无调用方；底层 aiExplainObject 仍被视图菜单的「解释视图」使用。
 */
const ctxAiExplainTable = () => {
  const d = ctxMenu.value.data
  closeCtxMenu()
  aiExplainObject(d, t('mv.catTable'))
}

// AI 解释视图（视图菜单的既有入口，改为复用同一套流程）
const ctxAiExplainView = () => {
  const d = ctxMenu.value.data
  closeCtxMenu()
  aiExplainObject(d, t('mv.catView'))
}

/**
 * 「AI 数据洞察」：后端先采集统计画像（行数 / 空值率 / 枚举分布…），再让 AI 给结论。
 * 只发统计摘要、不发原始数据行——这点在隐私模式下尤其重要。
 */
const ctxAiInsightTable = async () => {
  const d = ctxMenu.value.data
  closeCtxMenu()
  if (!conn.value) { ElMessage.warning(t('mv.pickConn')); return }
  const table = d?.label || d?.objectName || ''
  const db = d?.db || currentDb.value
  if (!table) return
  const loading = ElLoading.service({ lock: true, text: t('mv.collectingProfile'), background: 'rgba(0,0,0,0.5)' })
  try {
    loading.setText(t('mv.aiAnalyzing'))
    const res = await aiInsight({ connectionId: conn.value.id, database: db, table })
    if (res?.success) {
      aiExplainTitle.value = t('mv.aiInsightTitle', { table })
      aiExplainContent.value = renderMarkdown(res.content || '')
      aiExplainVisible.value = true
    } else {
      ElMessage.error(res?.message || t('mv.analyzeFailed'))
    }
  } catch (e) { ElMessage.error(t('mv.aiReqFailed', { detail: (e?.message || e) })) } finally {
    loading.close()
  }
}

// ====== 分类目录（Tables/Views/Procedures/...）右键菜单操作 ======
// 根据分类生成对应的 CREATE 模板并填入 SQL 编辑器；Scripts 分类直接新建脚本
const catCreate = (cat) => {
  const tb = treeCtxMenu.value?.data
  const db = treeCtxMenu.value.db || currentDb.value
  closeTreeCtxMenu()
  if (cat === 'scripts') {
    newQueryTab()
    ElMessage.success(t('mv.newScriptCreated'))
    return
  }
  if (db) setCurrentDb(db)
  const connId = tb?.connId || currentConnId.value
  // 连接类型解析失败时回退到当前连接类型，避免 ClickHouse 误显示"存储过程"
  const connType = connTypeOf(connId) || conn.value?.type
  openTab({ id: 'form:' + cat + ':' + (++tabSeq), type: 'form', cat, mode: 'create', object: {}, database: db, connId, connType, label: t('mv.newObjectTitle', { kind: objCatLabel(cat, connType) }) })
}

const catCreateUser = () => {
  const db = treeCtxMenu.value.db || currentDb.value
  closeTreeCtxMenu()
  if (db) setCurrentDb(db)
  openTab({ id: 'user-form:create:' + (++tabSeq), type: 'user-form', mode: 'create', editData: {}, connId: currentConnId.value ? String(currentConnId.value) : '', database: db, label: t('mv.newUserTitle') })
}

// 刷新某分类所在的库节点
const catRefresh = (db) => {
  const data = treeCtxMenu.value.data
  closeTreeCtxMenu()
  const cat = data?.cat
  if (cat) refreshCatNode(db, cat, data?.connId)
  else refreshDbNode(db, data?.connId)
}

// 重载某个库节点（懒加载模式下置为未加载并重新拉取，各分类目录随之刷新）
const refreshDbNode = (db, connId) => {
  const tree = treeRef.value
  const cid = connId || currentConnId.value
  if (!tree || !cid || !db) return
  // 整库重载等同于「库列表要重新看」：连库列表的持久缓存一起丢，
  // 否则重载出来的还是那份旧库列表（新建的库要等 15 分钟 TTL 才出现）
  removeSchemaCache('dbs:' + cid)
  const key = 'db:' + cid + ':' + db
  const node = tree.store?.nodesMap?.[key]
  if (!node || !node.loaded) return
  // 递归重置所有子节点的 loaded 状态，确保 Tables / Views / Procedures 等分类重新拉取
  const resetLoaded = (n) => {
    if (n.childNodes) {
      for (const child of n.childNodes) {
        child.loaded = false
        child.isLeaf = false
        resetLoaded(child)
      }
    }
  }
  resetLoaded(node)
  node.loaded = false
  node.loadData(() => {})
}

const refreshCatNode = async (db, cat, connId) => {
  const tree = treeRef.value
  const cid = connId || currentConnId.value
  if (!tree || !cid || !db || !cat) return
  const key = 'cat-' + cid + '-' + db + ':' + cat
  const node = tree.store?.nodesMap?.[key]
  if (!node) return
  // 这份清单对应的持久缓存键（只有表/视图这一类用了持久缓存，键与 loadCategoryItems 一致）
  const catKey = (cat === 'tables' || cat === 'views') ? 'tables:' + cid + ':' + db : ''
  try {
    // 这个函数的语义就是「我要最新的」：先把这份旧缓存丢掉。
    //
    // 不丢会怎样（实测）：建完表点刷新、清单是新的，但持久缓存里还是旧的 ——
    // 用户下次展开这个库时**又先渲染旧清单**，于是「计数是新的、清单里却找不到那张表」，
    // 而且 15 分钟 TTL 内反复展开都一样。后端在 DDL 后已经作废了自己的缓存，
    // 前端这份是独立的一份，必须在这里一起丢。
    if (catKey) removeSchemaCache(catKey)
    // 先拉取该分类的原始数据；节点映射统一走 buildCatChildren，与树首次构建保持字段一致
    let items = []
    if (cat === 'tables' || cat === 'views') items = await listTables(cid, db).catch(() => [])
    // 过程与函数同源：这里取**整份**例程清单，按分类筛选交给下面的 buildCatChildren
    // （它按 routineType 分，两处保持同一判据，不必在这里再写一遍）
    else if (cat === 'procs' || cat === 'functions') items = await listProcedures(cid, db).catch(() => [])
    else if (cat === 'triggers') items = await listTriggers(cid, db).catch(() => [])
    else if (cat === 'events') items = await listEvents(cid, db).catch(() => [])
    else if (cat === 'users') items = await listUsers(cid, db).catch(() => [])
    else if (cat === 'scripts') items = getDbScripts(cid, db)
    else return
    if (cat === 'tables' && db === currentDb.value) {
      // 同步「表名下拉」数据源（与 refreshAllTables 同构：仅表、不含视图）
      allTables.value = (Array.isArray(items) ? items : []).filter(t => (t.type || 'TABLE') === 'TABLE')
    }
    // 「刷新」拿到的就是最新清单，顺手把精确行数也回填。
    // 不回填的后果：刚建的表刷新后仍然没有数字（回填只发生在"第一次取到列表"那几条路上，
    // 而刷新走的是这里）。与上面 loadCategoryItems 的口径一致。
    if (cat === 'tables') fillRealRowCounts(cid, db, items)
    // 把刚拿到的清单写回持久缓存：刷新之后这次的结果就是「已知的最新」，
    // 下次展开直接用它渲染，不必再打一次库
    if (catKey && Array.isArray(items)) writeSchemaCache(catKey, items)
    const children = buildCatChildren({ cat, connId: cid, db, items })
    node.data.children = children
    const oldKeys = node.childNodes.map(c => c.key)
    for (const k of oldKeys) {
      try { tree.remove(k) } catch (e) { /* ignore */ }
    }
    for (const child of children) {
      tree.append(child, key)
    }
  } catch (e) {
    console.error('刷新分类节点失败:', e)
  }
}

// ====== 对象（视图/索引/存储过程/触发器/事件）右键菜单操作 ======
const objName = (d) => d.objectName || d.table || d.label || ''

const objViewDetail = () => {
  const d = ctxMenu.value.data
  if (!d) return
  closeCtxMenu()
  if (d.objectKind === 'view') {
    // 视图：打开对象详情页展示 SHOW CREATE VIEW（id 带连接，避免跨数据源同名视图顶掉）
    const cid = d.connId ? String(d.connId) : (currentConnId.value ? String(currentConnId.value) : '')
    openTab({
      id: tabKey('object:view:' + (d.table || d.label), cid), type: 'object', objectType: 'view',
      objectName: d.table || d.label, connId: cid, database: d.db || currentDb.value, label: d.label + t('mv.definitionSuffix')
    })
  } else {
    openObject(d, d.objectKind, d.label)
  }
}

const objCopyName = () => {
  const d = ctxMenu.value.data
  if (!d) return
  closeCtxMenu()
  copyText(objName(d), d.objectKind === 'view' ? t('mv.viewNameCopied') : t('mv.nameCopied'))
}

// ====== 可视化新建 / 编辑对象（ObjectFormDialog） ======
const objForm = ref({ visible: false, cat: 'tables', mode: 'create', object: {} })
const objFormDb = ref('')
// 对象 kind → 表单分类
const OBJ_CAT = { view: 'views', function: 'procs', procedure: 'procs', trigger: 'triggers', event: 'events' }
// 当前连接类型（根据连接 id 从连接列表查，避免依赖 conn 的赋值时序）
const connTypeOf = (connId) => {
  const cid = connId || currentConnId.value
  return allConnections.value.find(x => String(x.id) === String(cid))?.type
}
// 分类简体标题（函数不是存储过程：kind=function 或 ClickHouse 的 procs 显示为"函数"）
const objCatLabel = (cat, connType, kind) => {
  const m = {
    tables: t('mv.catTable'), views: t('mv.catView'), procs: t('mv.catProc'),
    triggers: t('mv.catTrigger'), events: t('mv.catEvent')
  }
  if (cat === 'procs' && (kind === 'function' || connType === 'CLICKHOUSE')) return t('mv.catFunc')
  return m[cat] || cat
}
// tab 标题渲染：procs 表单在函数场景下把"存储过程"替换为"函数"
const tabLabel = (tab) => {
  const ct = tab.connType || conn.value?.type
  const isFn = tab.object?.kind === 'function' || tab.formKind === 'function' || ct === 'CLICKHOUSE'
  let base = tab.type === 'sql' ? (tab.scriptName || tab.label) : tab.label
  if (tab.type === 'form' && tab.cat === 'procs' && isFn) {
    // 标题里的「存储过程」在函数场景下要换成「函数」：两边都取字典 —— 写死中文的话，
    // 切到英文后这个正则匹配不到任何东西，标题会一直留着 procedure 不改
    const proc = t('mv.catProc')
    base = base.replace(new RegExp(proc.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'g'), t('mv.catFunc'))
  }
  // **路径放全**（用户口径）：带数据源上下文的页签，标题前缀「连接 · 库」——
  // 多连接 / 多库各开同名的页签，一眼能分清是哪条路径下的。脚本页签是用户自己
  // 命名的、AI / 知识库是全局页签，都不带数据源语境，保持原样。
  if (!isGlobalTab(tab) && tab.type !== 'sql') {
    const connName = allConnections.value.find(x => String(x.id) === String(tab.connId || ''))?.name
    const parts = [connName, tab.database].filter(Boolean)
    // 页签文本与库名相同时不再重复拼一份（Redis 的 db 页签 label 就是库名，
    // 原来会显示成「本地Redis · db0 · db0」）
    if (!parts.length) { /* 无前缀，保持原样 */ }
    else if (base === tab.database) base = parts.join(' · ')
    else base = parts.join(' · ') + ' · ' + base
  }
  return base
}

const onObjFormSaved = (db, cat) => {
  // 刷新对象所属分类节点（精确刷新，不刷新整个库）
  const database = db || objFormDb.value || currentDb.value
  const category = cat || objForm.value.cat
  if (database && category) refreshCatNode(database, category)
  else if (database) refreshDbNode(database)
}

// 对象定义 → 可编辑脚本。各引擎「改定义体」的语法差异很大，不能统一把 CREATE 换成 ALTER：
//   PG / Oracle / DB2        CREATE OR REPLACE 本身即可重定义 → 原样保留
//   SQL Server               视图/过程/函数/触发器都支持 ALTER
//   MySQL / MariaDB / Doris  只有视图支持 ALTER VIEW；存储过程/函数/触发器/事件必须 DROP + CREATE
//   SQLite / ClickHouse      视图、触发器（ClickHouse 的函数）没有 ALTER → DROP + CREATE
// 另外 MySQL 定义里的 ALGORITHM / DEFINER / SQL SECURITY 在 ALTER 里不合法，重建时也避免绑定
// 可能不存在的账号，所以统一剔除。
const DDL_DROP_KW = { view: 'VIEW', procedure: 'PROCEDURE', function: 'FUNCTION', trigger: 'TRIGGER', event: 'EVENT' }
const QUOTE_CHAR = { BACKTICK: '`', DOUBLE_QUOTE: '"' }
const isPgFamily = (t) => t === 'POSTGRESQL' || t === 'KINGBASE'
// 返回该连接类型下「必须 DROP + CREATE」的对象 kind；空数组表示可用 ALTER / CREATE OR REPLACE
const dropCreateKindsOf = (connType) => {
  const t = String(connType || '').toUpperCase()
  if (t === 'MYSQL' || t === 'MARIADB' || t === 'DORIS') return ['procedure', 'function', 'trigger', 'event']
  if (t === 'SQLITE') return ['view', 'trigger']
  if (t === 'CLICKHOUSE') return ['view', 'procedure', 'function', 'event']
  // PG 的 ALTER TRIGGER 只能改名 / 改依赖，不能改触发定义体 → 必须 DROP + CREATE
  if (isPgFamily(t)) return ['trigger']
  return []
}
// PG 的 DROP TRIGGER 必须带所属表：从 CREATE TRIGGER ... ON <表> FOR EACH ROW 里取表名（保留原引号写法）
const pgTriggerTable = (s) => {
  const m = /\bON\s+((?:"[^"]+"|`[^`]+`|\[[^\]]+\]|[A-Za-z_][\w$]*)(?:\s*\.\s*(?:"[^"]+"|`[^`]+`|\[[^\]]+\]|[A-Za-z_][\w$]*))?)\s+FOR\s+EACH\s+ROW/i.exec(String(s))
  return m ? m[1].replace(/\s+/g, '') : ''
}
// SQL Server 的定义来自 sys.sql_modules，常带 SSMS 生成的 `SET ANSI_NULLS ON / GO` 前缀：
// GO 是客户端批分隔符（不是 T-SQL，执行会报语法错误）→ 去掉；CREATE 可能排在 SET 之后 → 定位后改 ALTER。
// CREATE OR ALTER（2016 SP1+）本身可重复执行，保持原样。
const sqlServerEditable = (s) => {
  const out = String(s).replace(/^[ \t]*GO[ \t]*;?[ \t]*\r?\n/gim, '')
  if (/CREATE\s+OR\s+ALTER/i.test(out)) return out
  return out.replace(/^([ \t]*)CREATE([ \t]+(?:PROC(?:EDURE)?|FUNCTION|VIEW|TRIGGER))\b/im, '$1ALTER$2')
}
// 剔除 ALTER 不接受的 MySQL 前缀子句（ALGORITHM / DEFINER / SQL SECURITY）
const stripDdlPrefix = (s) => String(s).replace(
  /^(CREATE\s+)(?:ALGORITHM\s*=\s*\w+\s+)?(?:DEFINER\s*=\s*(?:`[^`]*`|'[^']*'|[^\s@]+)\s*(?:@\s*(?:`[^`]*`|'[^']*'|[^\s]+))?\s+)?(?:SQL\s+SECURITY\s+\w+\s+)?/i, '$1')
const toEditableSql = (ddl, { connType, kind, db, name } = {}) => {
  const s = String(ddl || '').trim()
  if (!s) return ''
  const k = String(kind || '').toLowerCase()
  const t = String(connType || '').toUpperCase()
  const kw = DDL_DROP_KW[k]
  if (kw && name && dropCreateKindsOf(t).includes(k)) {
    const q = QUOTE_CHAR[quoteStyleOf(t)] || '`'
    if (isPgFamily(t) && k === 'trigger') {
      // 约束触发器（CREATE CONSTRAINT TRIGGER）不能用 DROP TRIGGER 删，保持原样交给用户处理
      if (/CONSTRAINT\s+TRIGGER/i.test(s)) return s
      const tbl = pgTriggerTable(s)
      // 取不到所属表时保持原样，避免生成一条必然失败的 DROP
      return tbl ? 'DROP TRIGGER IF EXISTS ' + q + name + q + ' ON ' + tbl + ';\n\n' + s : s
    }
    // 库/schema 限定：MySQL、ClickHouse 视图支持；SQLite 的触发器/视图直接按名删除；
    // ClickHouse 的函数名不接受库前缀
    const withDb = db && !(t === 'CLICKHOUSE' && k !== 'view') && t !== 'SQLITE'
    const target = withDb ? q + db + q + '.' + q + name + q : q + name + q
    // ClickHouse 的视图/物化视图统一用 DROP TABLE 删除（DROP VIEW 对物化视图已废弃）
    const dropKw = t === 'CLICKHOUSE' ? 'TABLE' : kw
    return 'DROP ' + dropKw + ' IF EXISTS ' + target + ';\n\n' + stripDdlPrefix(s)
  }
  if (t === 'SQLSERVER') return sqlServerEditable(s)
  if (/CREATE\s+OR\s+REPLACE/i.test(s)) return s
  return stripDdlPrefix(s).replace(/^CREATE\b/i, 'ALTER')
}

// 对象定义带入编辑器前按当前连接方言自动格式化（关键字大小写/换行缩进取「设置 → SQL 格式化」）。
// 未取到定义（提示性注释）或格式化异常时原样带入，保证编辑器一定有可用内容。
const autoFormatSql = (text, connType) => {
  const s = String(text || '')
  if (!s.trim() || s.trimStart().startsWith('--')) return s
  try {
    return smartFormatSql(s, getEditorSettings(), connDialectOf(connType))
  } catch (e) {
    return s
  }
}
// 格式化防呆：例程块（BEGIN...END）里含分号，个别方言的解析器可能拆错结构，
// 一旦分号/括号/BEGIN/END/引号数量发生变化说明破坏语义，直接退回未格式化文本。
const fmtStructureChanged = (a, b) => {
  const count = (s, re) => (String(s).match(re) || []).length
  return count(a, /;/g) !== count(b, /;/g)
    || count(a, /\bBEGIN\b/gi) !== count(b, /\bBEGIN\b/gi)
    || count(a, /\bEND\b/gi) !== count(b, /\bEND\b/gi)
    || count(a, /\(/g) !== count(b, /\(/g)
    || count(a, /\)/g) !== count(b, /\)/g)
    || count(a, /'/g) !== count(b, /'/g)
}
const autoFormatDdl = (text, connType) => {
  const s = String(text || '')
  const out = autoFormatSql(s, connType)
  return out !== s && fmtStructureChanged(s, out) ? s : out
}

// 编辑（脚本）：读对象定义，打开 SQL 页签展示可执行定义（ALTER 或 DROP + CREATE）
const objEditScript = async () => {
  const d = ctxMenu.value.data
  if (!d) return
  const kind = d.objectKind
  const name = kind === 'view' ? (d.table || d.label) : objName(d)
  const db = d.db || currentDb.value
  const cid = d.connId || currentConnId.value
  const cat = OBJ_CAT[kind] || 'views'
  closeCtxMenu()
  if (cid && currentConnId.value && String(cid) !== String(currentConnId.value)) await selectConn(String(cid))
  if (db && db !== currentDb.value) setCurrentDb(db)
  let ddl = ''
  try {
    const res = await getObjectInfo(cid, db, kind, name)
    ddl = (res && res.ddl) || ''
  } catch (e) {
    ElMessage.error(t('mv.loadDefFailed', { detail: errMsg(e) }))
  }
  const ctype = connTypeOf(cid)
  const editable = toEditableSql(ddl, { connType: ctype, kind, db, name })
  openTab({
    id: 'sql:obj:' + kind + ':' + name + ':' + (++tabSeq), type: 'sql',
    label: t('mv.editLabel', { kind: objCatLabel(cat, ctype, kind), name }),
    scriptName: '', dirty: false,
    connId: cid, database: db,
    initialSql: editable ? autoFormatDdl(editable, ctype) : t('mv.noDefComment', { name })
  })
}

// 编辑定义：打开可视化编辑 Tab（预填现有定义）
const objEditDdl = () => {
  const d = ctxMenu.value.data
  if (!d) return
  const db = d.db || currentDb.value
  closeCtxMenu()
  if (db) setCurrentDb(db)
  const cat = OBJ_CAT[d.objectKind] || 'views'
  const connId = d.connId || currentConnId.value
  const connType = connTypeOf(connId)
  openTab({
    id: 'form:' + cat + ':edit:' + (++tabSeq), type: 'form', cat, mode: 'edit',
    object: { name: objName(d), table: d.table || '', kind: d.objectKind },
    database: db, connId, connType,
    label: t('mv.editLabel', { kind: objCatLabel(cat, connType, d.objectKind), name: objName(d) })
  })
}

// 对象详情页点击"可视化编辑"→ 同样打开编辑 Tab
const onObjDetailEdit = ({ cat, name, kind, table }) => {
  const db = currentDb.value
  const connId = currentConnId.value ? String(currentConnId.value) : ''
  const connType = connTypeOf(currentConnId.value)
  openTab({
    id: 'form:' + cat + ':edit:' + (++tabSeq), type: 'form', cat, mode: 'edit',
    object: { name, table: table || '', kind },
    database: db, connId, connType,
    label: t('mv.editLabel', { kind: objCatLabel(cat, connType, kind), name })
  })
}

// ====== 用户管理 ======
// 打开用户详情页签（用户列表右键"查看" / 树双击用户节点）
//
// 必须带上**节点自己的**连接与库：入口不止「右键当前节点」一种 —— 启动后直接双击
// 用户节点时 `currentConnId` 还是空的，老写法会开出一个没有 connId 的页签；
// `connFor('')` 回退到 `conn.value` 也是空 → `UserDetailView.load()` 里
// `if (!props.conn) return` 直接返回：页面只剩外壳，字段与 SQL 预览全空。
const openUserTab = (name, ctx = {}) => {
  const cid = String(ctx.connId || currentConnId.value || '')
  const db = ctx.db || currentDb.value
  openTab({
    id: tabKey('user:' + name, cid), type: 'user', userName: name,
    connId: cid, database: db, label: name
  })
}

const openUserDetail = () => {
  const d = ctxMenu.value.data
  if (!d) return
  closeCtxMenu()
  openUserTab(d.objectName || d.label, { connId: d.connId, db: d.db })
}

// 组装用户编辑数据：按数据库类型拉取并解析角色/权限（MySQL / SQL Server / ClickHouse 结构不同；解析器在 src/utils/grants.js）
//
// `connId` 必须由调用方给（节点的连接），不能拿全局 `conn`：右键菜单可以作用于
// **非当前连接**的对象（见 connFor 的注释），用全局 conn 会去问错误的服务器；
// 而当前连接为空时（启动后直接双击用户节点）`conn.value.id` 直接抛 TypeError，
// 被下面的 catch 吞掉 → 编辑表单里权限全空、还看不出哪里错了。
const buildUserEditData = async ({ fullName, db, name, host, connId }) => {
  const parts = String(fullName).split('@')
  const editData = { name: name || parts[0], host: host || (parts.length > 1 ? parts[1] : '') }
  const target = connFor(connId ?? currentConnId.value)
  if (!target?.id) {
    ElMessage.error(t('mv.userConnUnknown'))
    return editData
  }
  const isMssql = target.type === 'SQLSERVER'
  const isClickhouse = target.type === 'CLICKHOUSE'
  try {
    const info = await getUserInfo(target.id, db, fullName)
    if (isMssql) {
      const parsed = parseSqlServerPerms(info)
      editData.serverRoles = parsed.serverRoles
      editData.roles = parsed.roles
      editData.schemaPrivileges = parsed.schemaPrivileges
      editData.objectPrivileges = parsed.objectPrivileges
    } else if (isClickhouse && info.grants && Array.isArray(info.grants)) {
      editData.grants = info.grants.filter(g => g && !g.startsWith('--'))
    } else if (info.privileges || info.dbPrivileges || info.tablePrivileges) {
      // Doris 等方言直接返回结构化权限（其 SHOW GRANTS 不含权限明细）
      editData.privileges = info.privileges || []
      editData.dbPrivileges = info.dbPrivileges || []
      editData.tablePrivileges = info.tablePrivileges || []
    } else if (info.grants && Array.isArray(info.grants)) {
      const parsed = parseMySqlGrants(info.grants)
      editData.privileges = parsed.privileges
      editData.dbPrivileges = parsed.dbPrivileges
      editData.tablePrivileges = parsed.tablePrivileges
    }
    if (info.roles && !isMssql) editData.roles = info.roles
  } catch (e) {
    // 不能静默：拿不到详情时表单会显示成「没有权限」，与「确实没有权限」无法区分
    ElMessage.warning(t('mv.loadUserPrivFailed', { detail: errMsg(e) }))
  }
  return editData
}

const openUserEdit = async () => {
  const d = ctxMenu.value.data
  if (!d) return
  closeCtxMenu()
  const fullName = d.objectName || d.label
  const db = d.db || currentDb.value
  const host = d.host || ''
  const cid = String(d.connId || currentConnId.value || '')
  const editData = await buildUserEditData({
    fullName, db, connId: cid,
    name: d.name || (host ? String(fullName).split('@')[0] : fullName),
    host
  })
  openTab({ id: tabKey('user-form:edit:' + fullName, cid), type: 'user-form', mode: 'edit', editData, connId: cid, database: db, label: t('mv.editUserLabel', { name: fullName }) })
}

const userDrop = () => {
  const d = ctxMenu.value.data
  if (!d) return
  const db = d.db || currentDb.value
  closeCtxMenu()
  const name = d.objectName || d.label
  const parts = name.split('@')
  const userName = parts[0]
  const host = parts.length > 1 ? parts[1] : ''
  ElMessageBox.confirm(
    t('mv.deleteUserBody', { name }),
          t('mv.deleteUserTitle'),
    { type: 'error', confirmButtonText: t('mv.dropConfirm'), cancelButtonText: t('common.cancel') }
  ).then(async () => {
    try {
      const res = await userAction(conn.value.id, { action: 'drop', database: db, userName, host })
      if (res.success) {
        ElMessage.success(t('mv.userDeleted', { name }))
        refreshCatNode(db, 'users')
      } else {
        ElMessage.error(t('mv.deleteFailed', { detail: (res.message || t('common.unknownError')) }))
      }
    } catch (e) { ElMessage.error(t('mv.deleteFailed', { detail: errMsg(e) })) }
  }).catch(() => {})
}

// 详情页的「编辑用户」：连接/库跟着**详情页自己**（用户可能早已切走当前连接）
const onUserDetailEdit = ({ name, connId, database }) => {
  openUserEditByName(name, connId, database)
}

const openUserEditByName = async (fullName, connId, db) => {
  const cid = String(connId || currentConnId.value || '')
  const database = db || currentDb.value
  const editData = await buildUserEditData({ fullName, db: database, connId: cid })
  openTab({ id: tabKey('user-form:edit:' + fullName, cid), type: 'user-form', mode: 'edit', editData, connId: cid, database, label: t('mv.editUserLabel', { name: fullName }) })
}

const onUserFormSaved = (db) => {
  if (db) refreshCatNode(db, 'users')
}

const objDrop = () => {
  const d = ctxMenu.value.data
  if (!d) return
  const db = d.db || currentDb.value
  closeCtxMenu()
  const map = {
    view: t('mv.catView'), function: t('mv.catFunc'), procedure: t('mv.catProc'),
    trigger: t('mv.catTrigger'), event: t('mv.catEvent')
  }
  const label = map[d.objectKind] || t('mv.catObject')
  ElMessageBox.confirm(
    t('mv.deleteItemBody', { kind: label, name: d.label }),
          t('mv.deleteKindTitle', { kind: label }),
    { type: 'error', confirmButtonText: t('mv.dropConfirm'), cancelButtonText: t('common.cancel') }
  ).then(() => doDropObject(d, db, label)).catch(() => {})
}

const doDropObject = async (d, db, label) => {
  const name = objName(d)
  let sql
  if (d.objectKind === 'view') sql = `DROP VIEW IF EXISTS ${quoteIdent(name)}`
  else if (d.objectKind === 'function') sql = `DROP FUNCTION IF EXISTS ${quoteIdent(name)}`
  else if (d.objectKind === 'procedure') sql = `DROP PROCEDURE IF EXISTS ${quoteIdent(name)}`
  else if (d.objectKind === 'trigger') sql = `DROP TRIGGER IF EXISTS ${quoteIdent(name)}`
  else if (d.objectKind === 'event') sql = `DROP EVENT IF EXISTS ${quoteIdent(name)}`
  else {
    // 原来这里是光秃秃的 `return`：右键点「删除」什么都不发生，也没有任何提示，
    // 用户只会以为界面卡住了。索引不在这个树的管辖范围内（它在表结构的「索引」页签里管），
    // 就把这句话直接说出来。
    ElMessage.warning(t('mv.dropUnsupported', { kind: (d.objectKind || t('mv.thisObject')) }))
    return
  }
  try {
    const res = await alterTable(conn.value.id, db, sql)
    if (res.success) {
      ElMessage.success(t('mv.kindDeleted', { kind: label, name: d.label }))
      // 关闭该对象的详情页签
      const tabId = (d.objectKind === 'view' ? 'object:view:' + name : d.objectKind + ':' + (d.objectName || name))
      const idx = tabs.value.findIndex(x => x.id === tabId)
      if (idx !== -1) tabs.value.splice(idx, 1)
      const catMap = { view: 'views', function: 'procs', procedure: 'procs', trigger: 'triggers', event: 'events' }
      const cat = catMap[d.objectKind]
      if (cat) refreshCatNode(db, cat)
      else refreshDbNode(db)
    } else {
      ElMessage.error(t('mv.deleteFailed', { detail: (res.message || t('common.unknownError')) }))
    }
  } catch (e) { ElMessage.error(t('mv.deleteFailed', { detail: errMsg(e) })) }
}

const objRefresh = () => {
  const d = ctxMenu.value.data
  if (!d) return
  closeCtxMenu()
  const db = d.db || currentDb.value
  const catMap = { table: 'tables', view: 'views', function: 'procs', procedure: 'procs', trigger: 'triggers', event: 'events', user: 'users' }
  const cat = catMap[d.objectKind]
  if (cat) refreshCatNode(db, cat)
  else refreshDbNode(db)
}

const ctxExport = async (format) => {
  if (!ctxMenu.value.data) return
  const d = ctxMenu.value.data
  const db = d.db || currentDb.value
  const name = d.table || d.label
  closeCtxMenu()
  exportTargetName.value = name
  exportStatus.value = 'running'
  exportDone.value = 0
  exportTotal.value = -1
  exportPhase.value = t('mv.exportPreparing')
  exportMessage.value = t('mv.submittingExport')
  exportLogs.value = []
  exportState.seenLogs = 0
  exportCanceling.value = false
  try {
    const payload = { format, table: name, database: db }
    // SQL UPDATE/DELETE 导出：附带主键列用于生成精确 WHERE 条件（仅物理表）
    if (format === 'sql-update' || format === 'sql-delete') {
      try {
        const cols = await listColumns(conn.value.id, db, name)
        const pks = cols.filter(c => c.primaryKey).map(c => c.name)
        if (pks.length) payload.pkColumns = pks.join(',')
      } catch { /* 主键获取失败则退化为整行条件 */ }
    }
    const res = await exportStart(conn.value.id, payload)
    if (!res.success || !res.taskId) {
      throw new Error(res.message || t('mv.exportSubmitFailed'))
    }
    exportTaskId.value = res.taskId
    exportProgressVisible.value = true
    startExportPolling(name, format)
  } catch (e) {
    ElMessage.error(t('mv.exportFailed', { detail: errMsg(e) }))
  }
}

// 导出进度状态袋：与 useExportTask 共用 applyTaskSnapshot（字段口径、日志增量逻辑一处维护）
const exportState = {
  status: exportStatus, done: exportDone, total: exportTotal,
  phase: exportPhase, message: exportMessage, logs: exportLogs,
  seenLogs: 0
}

const startExportPolling = (name, format) => {
  if (exportPollTimer) clearInterval(exportPollTimer)
  const extMap = { csv: 'csv', excel: 'xlsx', json: 'json', 'sql-insert': 'sql', 'sql-update': 'sql', 'sql-delete': 'sql', ddl: 'sql' }
  exportPollTimer = setInterval(async () => {
    if (!exportTaskId.value) return
    try {
      const r = await exportTask(conn.value.id, exportTaskId.value)
      // 与 useExportTask 共用同一套「应用快照」：日志按 logsSeq 增量追加（不再每轮重建时间戳）
      applyTaskSnapshot(exportState, r)
      if (exportStatus.value !== 'running') {
        clearInterval(exportPollTimer)
        exportPollTimer = null
        // 与 useExportTask 共用同一套收尾：下载 / 落盘提示 / 打开文件夹
        await finishExport(exportState, conn.value.id, exportTaskId.value, name, format)
      }
    } catch (e) {
      console.error('导出进度轮询失败', e)
    }
  }, 300)
}

const cancelExportTask = async () => {
  if (!exportTaskId.value) return
  exportCanceling.value = true
  try {
    await exportCancel(conn.value.id, exportTaskId.value)
  } catch (e) {
    ElMessage.error(t('mv.cancelFailed', { detail: errMsg(e) }))
  } finally {
    setTimeout(() => { exportCanceling.value = false }, 2000)
  }
}

const onExportProgressClose = () => {
  exportProgressVisible.value = false
  if (exportPollTimer) { clearInterval(exportPollTimer); exportPollTimer = null }
}
// 判断当前右键的数据库是否已打开（是当前连接的当前库）
const isDbCurrentOpen = computed(() => {
  const d = treeCtxMenu.value.data
  if (!d) return false
  return String(d.connId || currentConnId.value) === String(currentConnId.value) && d.db === currentDb.value
})

// ====== 数据库节点右键菜单操作 ======
const dbCtxToggleOpen = () => {
  const d = treeCtxMenu.value.data
  if (!d) return
  closeTreeCtxMenu()
  const isOpen = String(d.connId || currentConnId.value) === String(currentConnId.value) && d.db === currentDb.value
  if (isOpen) {
    // 关闭数据库：清空当前库，关闭该库相关的 tab，收起节点
            // **真正断开会话**：该库的语句跑在它的影子连接上，调后端按库断掉 ——
            // 只清前端状态的话，内核池里那条会话还挂数据库上（用户要求真正关闭）
            const closeSid = String(d.connId || currentConnId.value || '')
            if (closeSid && d.db) {
              disconnectDatabase(closeSid, d.db).catch(() => {})
            }
            if (currentConnId.value && currentDbByConn.value[currentConnId.value] === d.db) {
      currentDb.value = ''
      delete currentDbByConn.value[currentConnId.value]
    }
    // 关闭属于该数据库的 tab（表数据、表结构、对象、表单等）。
    // 只关这个连接下的 —— 页签栏全局之后，别的数据源的同名库不能被牵连
    const sid = String(d.connId || currentConnId.value || '')
    tabs.value = tabs.value.filter(t => {
      // 保留不依赖特定数据库的 tab（如不带 database 的 sql 脚本）
      if (!t.database) return true
      if (String(t.connId || '') !== sid) return true
      return t.database !== d.db
    })
    // 如果当前激活的 tab 被关闭了，切换到第一个可用 tab
    if (activeTab.value && !tabs.value.find(t => t.id === activeTab.value)) {
      activeTab.value = tabs.value[0]?.id || ''
    }
    nextTick(() => {
      collapseNode(d.id)
    })
  } else {
    // 打开数据库：先切换到目标连接（若不同），再设为当前库，展开节点
    ensureDbConn(d)
    setCurrentDb(d.db)
    refreshAllTables()
    nextTick(() => {
      expandNode(d.id)
    })
  }
}

// 确保当前连接与目标数据库所在连接一致（用于数据库右键操作）
const ensureDbConn = (d) => {
  const targetConnId = String(d.connId)
  if (currentConnId.value !== targetConnId) {
    saveConnState()
    currentConnId.value = targetConnId
    const c = allConnections.value.find(x => String(x.id) === targetConnId)
    conn.value = c || null
    sessionStorage.setItem('dbmind.currentConnId', targetConnId)
    dbs.value = dbsByConn.value[targetConnId] || []
  }
}

const dbCtxNewQuery = () => {
  const d = treeCtxMenu.value.data
  if (!d) return
  closeTreeCtxMenu()
  ensureDbConn(d)
  setCurrentDb(d.db)
  newQueryTab({ connId: String(d.connId), database: d.db })
}

const dbCtxShowDdl = async () => {
  const d = treeCtxMenu.value.data
  if (!d) return
  closeTreeCtxMenu()
  ensureDbConn(d)
  // 查看建库语句的查询 SQL 由各类型模块提供；返回 null 表示该类型无建库语句
  const type = conn.value?.type || ''
  const sql = byType(type).showDbDdlSql(d.realDb || d.db)
  if (sql === null) {
    dbDdlText.value = t('mv.noCreateDbComment', { kind: labelOf(type) })
    dbDdlVisible.value = true
    return
  }
  try {
    const res = await executeSql(conn.value.id, sql, d.db, null, null, null, null, true)
    if (res.success && res.rows && res.rows.length) {
      // 结果可能多列（MySQL SHOW CREATE DATABASE 第一列为库名），取含 CREATE 的列
      const row = res.rows[0]
      const vals = Object.values(row)
      const ddlVal = vals.find(v => typeof v === 'string' && /CREATE\s+(DATABASE|SCHEMA)/i.test(v))
        || vals[vals.length - 1] || vals[0] || ''
      dbDdlText.value = ddlVal
    } else if (res.success) {
      dbDdlText.value = t('mv.createDbComment', { db: d.db }) + '\nCREATE DATABASE ' + quoteIdent(d.db) + ';'
    } else {
      dbDdlText.value = t('mv.fetchFailedComment', { detail: (res.message || t('common.unknownError')) }) + '\nCREATE DATABASE ' + quoteIdent(d.db) + ';'
    }
  } catch (e) {
    dbDdlText.value = t('mv.fetchFailedComment', { detail: (e?.message || e) }) + '\nCREATE DATABASE ' + quoteIdent(d.db) + ';'
  }
  dbDdlVisible.value = true
}

// 数据库右键弹窗的目标库（打开弹窗时保存，避免 menu.data 被后续右键覆盖）
let dbCtxTarget = null

const dbCtxRunSqlFile = () => {
  const d = treeCtxMenu.value.data
  if (!d) return
  ensureDbConn(d)
  dbCtxTarget = d.db
  closeTreeCtxMenu()
  dbRunSqlVisible.value = true
}

// 运行 SQL 文件执行成功后刷新库节点（文件读取与执行在 RunSqlFileDialog.vue）
const onRunSqlFileDone = () => {
  const db = dbCtxTarget
  if (db) refreshDbNode(db)
}

// 打开转储弹窗（对象列表的拉取与导出逻辑已随 DbDumpDialog.vue 一并拆出）
const dbCtxDumpSql = () => {
  const d = treeCtxMenu.value.data
  if (!d) return
  ensureDbConn(d)
  dbCtxTarget = d.db
  closeTreeCtxMenu()
  dbDumpVisible.value = true
}

const dbCtxSearch = () => {
  const d = treeCtxMenu.value.data
  if (!d) return
  ensureDbConn(d)
  dbCtxTarget = d.db
  closeTreeCtxMenu()
  dbSearchVisible.value = true
}

// 打开「可视化 ER 关系图」tab（基于外键元数据自动布局 + 外键连线）
const dbCtxEr = () => {
  const d = treeCtxMenu.value.data
  if (!d) return
  closeTreeCtxMenu()
  openErTab(d.connId, d.db || d.label)
}
const openErTab = (connId, db) => {
  const cid = connId ? String(connId) : ''
  const id = 'er:' + cid + ':' + (db || '')
  openTab({ id, type: 'er', label: t('mv.erDiagramLabel', { db: (db || 'ER') }), connId: cid, database: db })
}

const dbCtxDrop = () => {
  const d = treeCtxMenu.value.data
  if (!d) return
  ensureDbConn(d)
  closeTreeCtxMenu()
  ElMessageBox.confirm(
    t('mv.dropDbBody', { name: d.db }),
    t('mv.dropDbTitle'),
    { type: 'error', confirmButtonText: t('mv.dropConfirm'), cancelButtonText: t('common.cancel') }
  ).then(() => doDbDrop(d)).catch(() => {})
}

/**
 * 哪些类型真有 `DROP DATABASE` 语法。
 *
 * 之前这里对所有类型都拼 `DROP DATABASE`，于是 Oracle / DM / H2 / Derby / SQLite 上
 * 用户点了「删除数据库」只会拿到一句数据库语法错误 —— 那既不是用户的问题，也帮不上他。
 * 现在这些类型直接说清「为什么没有这条语句、该怎么做」。
 */
const DROP_DB_TYPES = ['MYSQL', 'MARIADB', 'POSTGRESQL', 'KINGBASE', 'SQLSERVER', 'CLICKHOUSE', 'DORIS', 'DB2']
const dropDbHint = (type) => {
  const ty = String(type || '').toUpperCase()
  if (ty === 'SQLITE') return t('mv.dropDbSqlite')
  if (ty === 'ORACLE' || ty === 'DM') return t('mv.dropDbOracle')
  if (ty === 'H2' || ty === 'DERBY') return t('mv.dropDbH2')
  return t('mv.dropDbNone')
}

const doDbDrop = async (d) => {
  const type = String(conn.value?.type || '').toUpperCase()
  if (!DROP_DB_TYPES.includes(type)) {
    ElMessage.warning(`${labelOf(type)}：${dropDbHint(type)}`)
    return
  }
  try {
    // catalog 层级（Doris）的库名是 `internal.bg_tgt` **全限定** —— 必须分段引号：
    // 整体引住会被当成一个叫 "internal.bg_tgt" 的名字找（真机踩过：报 doesn't exist）。
    // 删库也不需要「切到该库」（切进去反而可能阻止删除），database 参数留空
    const qualified = d.db.includes('.')
    const dropSql = qualified
      ? `DROP DATABASE ${d.db.split('.').map(p => quoteIdent(p)).join('.')}`
      : `DROP DATABASE ${quoteIdent(d.db)}`
    const bare = qualified ? d.db.split('.').pop() : d.db
    // **先清占用连接**：SQL Server / PG 在库被占用时会拒绝删除（3702 / "being accessed"）——
    // 我们自己的浏览/同步会话就占着，必须先踢掉：
    // - SQL Server：踢成单用户（ROLLBACK IMMEDIATE 回滚未提交事务）→ 同批 DROP
    //   （SINGLE_USER 只放行一个会话，SET 和 DROP 必须同一批才进得去）
    // - PG / KingBase：terminate 该库上除自身外的所有后端 → 再 DROP
    // 均为内核内部链路（internal），多语句批已放行
    if (type === 'SQLSERVER') {
      await alterTable(conn.value.id, '',
        `ALTER DATABASE ${quoteIdent(bare)} SET SINGLE_USER WITH ROLLBACK IMMEDIATE; ${dropSql}`)
    } else if (type === 'POSTGRESQL' || type === 'KINGBASE') {
      await alterTable(conn.value.id, '',
        `SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '${bare.replace(/'/g, "''")}' AND pid <> pg_backend_pid()`)
      await alterTable(conn.value.id, '', dropSql)
    } else {
      // MySQL 系 / ClickHouse / Doris / DB2：删库不受连接占用限制，直接删
      await alterTable(conn.value.id, '', dropSql)
    }
    const res = { success: true }
    ElMessage.success(t('mv.dbDeleted', { name: d.db }))
    refreshTree()
  } catch (e) {
    ElMessage.error(t('mv.deleteFailed', { detail: (e?.message || e) }))
  }
}

const dbCtxRefresh = () => {
  const d = treeCtxMenu.value.data
  if (!d) return
  closeTreeCtxMenu()
  refreshDbNode(d.db, d.connId)
}

// ====== catalog 节点右键菜单操作（Doris）======
/**
 * 刷新一个 catalog 下的库列表。
 *
 * 与「刷新连接」刻意不同：那个清的是**连接级**缓存（库列表 + 各库表清单），
 * 而 catalog 这一层库压根不经过 `dbsByConn` —— 它是 `?catalog=` 单独查的
 * （见 lazyLoad 的 catalog 分支）。用连接级去清，结果是把别的 catalog 的东西
 * 一起冲掉，而这里真正要刷的那份反而没动。
 */
const catalogCtxRefresh = async () => {
  const d = treeCtxMenu.value.data
  if (!d) return
  closeTreeCtxMenu()
  const node = treeRef.value?.getNode(d.id)
  if (node && !node.loaded) {
    // 还没展开过：直接展开就是最新的（懒加载会现查一次），不必再走"替换子节点"
    expandNode(d.id)
    return
  }
  try {
    const list = await listDatabases(String(d.connId), d.catalog)
    const children = dbChildNodes(String(d.connId), d.connType, list, d.catalog)
    d.children = children
    treeRef.value?.updateKeyChildren(d.id, children)
    ElMessage.success(t('mv.refreshed'))
  } catch (e) {
    ElMessage.error(t('mv.refreshFailed', { detail: errMsg(e) }))
  }
}

/** catalog 上「新建查询」：catalog 不参与"当前库"，直接复用连接的建查询逻辑。 */
const catalogCtxNewQuery = () => {
  const d = treeCtxMenu.value.data
  if (!d) return
  closeTreeCtxMenu()
  treeCtxNewQueryConn(d.connId)
}

/** catalog 上「删除」：`DROP CATALOG`。只删 Doris 里的**映射**，
 *  外部数据库（如 mysql_216 背后的 MySQL）里的数据原封不动，之后可重建。
 *  内置 internal 装着用户真正的库，Doris 不允许删 —— 菜单层已门控不显示。 */
const catalogCtxDrop = () => {
  const d = treeCtxMenu.value.data
  if (!d) return
  closeTreeCtxMenu()
  ElMessageBox.confirm(
    t('mv.dropCatalogBody', { name: d.catalog }),
    t('mv.dropCatalogTitle'),
    { type: 'warning', confirmButtonText: t('common.confirm'), cancelButtonText: t('common.cancel') }
  ).then(async () => {
    try {
      const res = await executeSql(String(d.connId), `DROP CATALOG ${quoteIdent(d.catalog)}`, '', null, null, null, null, true)
      if (!res.success) throw new Error(res.message || t('common.unknownError'))
      ElMessage.success(t('mv.catalogDropped', { name: d.catalog }))
      // 从树上摘掉该 catalog 节点（它下面的库/表节点随子树一起消失）
      treeRef.value?.remove(d.id)
    } catch (e) {
      ElMessage.error(t('mv.dropCatalogFailed', { detail: errMsg(e) }))
    }
  }).catch(() => {})
}

onMounted(async () => {
  window.addEventListener('dc-scripts-changed', onScriptsChanged)
  window.addEventListener('keydown', onGlobalKey)
  await loadAllConnections()
  // 仅渲染树（展示所有配置好的连接），不自动连接任何数据源
  await buildTree()
  // 先把上次的页签快照灌回内存，再走「恢复连接」——
  // selectConn 内部会按连接取出页签，顺序反了就只能拿回一个空列表
  restoreSession()
  // 只有「刷新」这种加载才会走到这里（restoreSession 已保证）：把记忆里的连接自动 test 并展开
  //
  // ⚠️ 必须**自己包一层 try/catch**：这里失败（记忆里的连接已经连不上是最常见的）以前会
  // 直接把整个 onMounted 打断 —— 后面的初始化全都不执行，而且一点痕迹都没有。
  // 实测踩到：末尾那句「静默预热」的 setTimeout 因此从未注册，预热等于没做。
  const memId = sessionStorage.getItem('dbmind.currentConnId')
  if (memId) {
    const c = allConnections.value.find(x => String(x.id) === String(memId))
    if (c) {
      try {
        await selectConn(c.id)
        await nextTick()
        expandNode('conn:' + c.id)
      } catch (e) {
        console.warn('[启动] 自动展开上次的连接失败（不影响其它初始化）', e)
      }
    }
  }
  // 没有记忆连接时也要把常驻页签摆回来（AI 助手 / 知识库那类与连接无关的）
  if (!memId && Object.keys(globalTabState.value).length) {
    for (const g of openGlobalTabs()) {
      if (!tabs.value.some(t => t.id === g.id)) tabs.value.push({ ...g })
    }
  }
  // 静默预热（不 await）：延迟 2.5 秒再发，避开首屏渲染最忙的那一下。
  // 这一步让"用户手动点开第一条连接"从 5~10 秒变成 ~250ms —— 上面那条自动展开
  // 走的是同一条路，所以它仍要等一次冷启动（那本来就是"第一次"，绕不过去）。
  setTimeout(() => { warmUpBeforeFirstClick() }, 2500)
})

// 页签变化就落盘：关掉浏览器再打开（或刷新）时能原样回来
watch([tabs, activeTab, globalTabState], scheduleSaveSession, { deep: true })
onBeforeUnmount(() => {
  clearTimeout(sessionTimer)
  saveSession()   // 离开前补一次，避免防抖窗口内的改动丢失

  // 摘掉挂在 window 上的监听。这些原本是「注册了就不管」的，
  // 而 MainView 在某些页签切换场景下会整个卸载再挂回来，
  // 于是每重挂一次就多叠一份回调 —— 表现是点一下、滚一下，同一段逻辑被执行 N 次。
  window.removeEventListener('click', closeTreeCtxMenu)
  window.removeEventListener('scroll', closeTreeCtxMenu, true)
  window.removeEventListener('click', closeTabCtxMenu)
  window.removeEventListener('scroll', closeTabCtxMenu, true)
  window.removeEventListener('click', closeCtxMenu)
  window.removeEventListener('keydown', onCtxMenuEscape)
  window.removeEventListener('dc-scripts-changed', onScriptsChanged)
  window.removeEventListener('keydown', onGlobalKey)
  if (exportPollTimer) { clearInterval(exportPollTimer); exportPollTimer = null }
})

// 监听 URL id 变化（如外部链接跳转）
watch(() => route.query.id, (id) => {
  if (id && id !== conn.value?.id && allConnections.value.find(c => c.id === id)) {
    selectConn(id)
  }
})

// ---------- 首页 KPI：总连接数 / 已连接数 / 连接类型 ----------
// 已连接的口径与树里连接名旁的绿点一致（连上且没报过错）——
// 同一个「连上了」不该有两套判断。
const connectedCount = computed(() =>
  allConnections.value.filter(c => isConnOpen(c.id) && !connErrorSet.value.has(String(c.id))).length
)
const connTypeCount = computed(() => new Set(allConnections.value.map(c => String(c.type || ''))).size)
// 悬停看具体类型名：数量是概览，名字才是想确认的东西
const connTypeSummary = computed(() =>
  [...new Set(allConnections.value.map(c => typeLabel(c.type)))].join(' / ')
)
</script>

<style scoped>
.main-view { height: 100%; display: flex; flex-direction: column; }
.dlg-title { display: flex; align-items: center; gap: 8px; flex: 1; }
.dlg-title > span:nth-child(2) { flex: 1; }
.dlg-copy-btn { flex-shrink: 0; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}
.topbar {
  /* 顶栏高度 48 → 40px：这一条只有「图标 + 文字」的导航（约 32px 高），48 显得空。
     整页是 flex 纵向布局，高度变化会自动让给下方内容，没有任何按 48px 硬算的偏移。 */
  height: 40px; display: flex; align-items: center; justify-content: flex-start;
  padding: 0 16px 0 10px; flex-shrink: 0;
  background: linear-gradient(180deg, var(--dc-bg-card), var(--dc-bg-soft));
  border-bottom: 1px solid var(--dc-border);
  box-shadow: 0 1px 8px var(--dc-shadow-sm);
  position: relative; z-index: 5;
}
/* ---- 桌面壳：系统标题栏已隐藏，这一条顶栏充当标题栏 ----
   右侧窗口按钮贴边排布，悬停态与系统标题栏保持一致的习惯。
   拖动的实现在 script 里（onTopbarMouseDown / Move / Up 三段式 → 壳的 startDragging）。
   ⚠️ 这里**刻意不写 `-webkit-app-region`**：它是 Chromium 的私有拖动特性，WebView2 同为
   Chromium 内核，写上去会被当成"系统级拖动区"与那套三段式互相打架（实测拖不动），
   而它原本只是留给**已不在仓库里的 Electron 壳**用的 —— 留着弊大于利。 */
.topbar.is-desktop { padding-right: 0; }
/* macOS 的系统红绿灯按钮占着左上角，给品牌区让位 */
.topbar.is-mac { padding-left: 84px; }
.win-acts { margin-left: auto; align-self: stretch; display: flex; align-items: stretch; }
/* 窗口按钮：图标 16px 与左侧图标组一致（真机反馈 14px 偏小不协调） */
.win-act {
  width: 56px; height: 100%; flex-shrink: 0;
  display: inline-flex; align-items: center; justify-content: center;
  border: none; background: transparent; padding: 0; cursor: pointer;
  color: var(--dc-text-dim);
  transition: background .15s ease, color .15s ease;
}
.win-act svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 1.4; }
.win-act:hover { background: var(--dc-bg-hover); color: var(--dc-text); }
.win-act.is-close:hover { background: #e81123; color: #fff; }
/* 顶栏品牌 logo：显示 22px，源图 64px（2 倍屏不糊，体积几 KB）。
   macOS 上顶栏左侧留了 84px 给系统红绿灯，logo 自然落在它右侧。 */
.topbar-logo {
  width: 22px; height: 22px; flex-shrink: 0;
  margin-right: 10px; border-radius: 6px;
  object-fit: contain; user-select: none; -webkit-user-drag: none;
}
.top-nav { display: flex; align-items: center; gap: 6px; }
.top-nav-item {
  display: inline-flex; align-items: center; gap: 5px;
  padding: 6px 11px; margin-right: 2px;
  font-size: 14px; font-weight: 500; color: var(--dc-text);
  border-radius: 6px; cursor: pointer; transition: all .15s;
  user-select: none;
}
.top-nav-item:hover { background: var(--dc-primary-wash); color: var(--dc-text-strong); }
.top-nav-item.dc-top-active { color: var(--dc-primary); font-weight: 600; }
.top-nav-arrow { font-size: 14px; color: var(--dc-text-dim); }
.top-nav-item .el-icon { font-size: 16px; }
.conn-label { display: flex; align-items: center; gap: 6px; font-weight: 500; font-size: 14px; color: var(--dc-text); }
.conn-label.t-mysql { }
.conn-label.t-doris { }
.conn-label.t-postgresql { }
.conn-label.t-sqlite { }
.body { flex: 1; display: flex; min-height: 0; position: relative; }
.sidebar {
  background: linear-gradient(180deg, var(--dc-bg-sidebar), var(--dc-bg-input));
  border-right: 1px solid var(--dc-border);
  display: flex; flex-direction: column; flex-shrink: 0; min-width: 0;
}
.tree-resizer { width: 4px; flex-shrink: 0; cursor: col-resize; background: transparent; transition: background .15s; }
.tree-resizer:hover, body.dc-col-resizing .tree-resizer { background: var(--dc-primary-light); }

/* 浮动折叠按钮：sidebar 显示时贴底部居中，隐藏时贴左边缘中央 */
.sidebar-toggle {
  position: absolute;
  bottom: 10px;
  transform: translateX(-50%);
  width: 28px; height: 28px;
  display: inline-flex; align-items: center; justify-content: center;
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border);
  border-radius: 6px;
  color: var(--dc-text-dim);
  cursor: pointer;
  transition: all .2s;
  font-size: 16px;
  padding: 0;
  z-index: 6;
}
.sidebar-toggle:hover {
  background: var(--dc-primary-wash);
  border-color: var(--dc-primary);
  color: var(--dc-primary);
}
.sidebar-toggle.toggle-floating {
  top: 50%;
  bottom: auto;
  left: 4px !important;
  transform: translateY(-50%);
}

/* 数据源卡片 */
.ds-card { display: flex; align-items: center; gap: 10px; padding: 12px 12px; border-bottom: 1px solid var(--dc-border); background: linear-gradient(180deg, var(--dc-hover-wash), transparent); }
.ds-info { flex: 1; min-width: 0; }
.ds-name { font-size: 14px; font-weight: 700; color: var(--dc-text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.ds-type { font-size: 12px; color: var(--dc-text-dim); margin-top: 3px; }
.ds-host { color: var(--dc-text-dim); font-family: "SF Mono", Consolas, monospace; }
.ds-switch { width: 24px; height: 24px; border: 1px solid var(--dc-border); border-radius: 6px; background: transparent; color: var(--dc-text-dim); cursor: pointer; display: flex; align-items: center; justify-content: center; flex-shrink: 0; }
.ds-switch:hover { color: var(--dc-text); border-color: var(--dc-border-strong); background: var(--dc-bg-soft); }

/* 表树区：整块不滚动，标题/搜索框固定，仅下方树列表内部滚动 */
.tree-area { flex: 1; display: flex; flex-direction: column; overflow: hidden; padding: 10px 6px 8px; min-height: 0; }
.tree-title { display: flex; align-items: center; justify-content: space-between; padding: 0 8px 10px; flex-shrink: 0; }
.tree-title-label {
  display: flex; align-items: center; gap: 6px;
  font-size: 14px; font-weight: 700; color: var(--dc-text);
  letter-spacing: .3px;
}
.tree-title-label .el-icon { font-size: 16px; color: var(--dc-primary); }
.tree-title-btn {
  height: 28px; padding: 0 11px;
  font-size: 14px; font-weight: 500;
  background: var(--dc-primary-wash) !important;
  border: 1px solid var(--dc-primary) !important;
  color: var(--dc-primary) !important;
  border-radius: 6px;
  transition: all .2s ease;
}
.tree-title-btn:hover {
  background: var(--dc-primary-wash) !important;
  border-color: var(--dc-primary-light) !important;
  color: var(--dc-primary-light) !important;
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}
.tree-empty { padding: 20px; text-align: center; color: var(--dc-text-weak); font-size: 13px; }
/* 树列表滚动容器：纵向滚动，横向不外溢（节点名靠省略号收尾） */
.tree-loading { flex: 1; min-height: 0; overflow-y: auto; overflow-x: hidden; position: relative; contain: layout paint; }
.placeholder-spin { animation: spin 1s linear infinite; }
@keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
/* 左侧树 loading：去掉遮罩背景，只保留旋转图标 */
.tree-loading :deep(.el-loading-mask) { background: transparent; }
.tree-loading :deep(.el-loading-spinner) { background: transparent; }
.tree-search { margin: 0 8px 10px; width: calc(100% - 16px); flex-shrink: 0; }
.tree-search :deep(.el-input__wrapper) {
  background: var(--dc-bg-input);
  box-shadow: inset 0 0 0 1px var(--dc-border-strong), 0 1px 3px var(--dc-shadow-sm);
  border-radius: 8px;
  padding: 2px 10px;
  transition: box-shadow .2s, border-color .2s;
}
.tree-search :deep(.el-input__wrapper:hover) { box-shadow: inset 0 0 0 1px var(--dc-border-hover), 0 1px 3px var(--dc-shadow-sm); }
.tree-search :deep(.el-input__wrapper.is-focus) { box-shadow: inset 0 0 0 1px var(--dc-primary), 0 0 0 3px var(--dc-primary-wash); }
.tree-search :deep(.el-input__inner) { font-size: 13px; color: var(--dc-text); }
.tree-search :deep(.el-input__prefix-inner) { color: var(--dc-text-weak); font-size: 14px; }
.tree-search :deep(.el-input__inner::placeholder) { color: var(--dc-text-weak); font-size: 13px; }
.tree-search :deep(.el-input__clear) { color: var(--dc-text-weak); }
.tree-search :deep(.el-input__clear:hover) { color: var(--dc-text-dim); }
.cat-count { font-size: 11px; color: var(--dc-text-dim); background: var(--dc-bg-soft); border-radius: 999px; padding: 0 7px; height: 16px; display: inline-flex; align-items: center; flex-shrink: 0; margin-left: auto; }
.content { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: linear-gradient(180deg, var(--dc-bg), var(--dc-bg-deep)); }
.conn-empty { flex: 1; display: flex; align-items: center; justify-content: center; }
.conn-switcher { width: 220px; }
.conn-switcher :deep(.el-input__prefix) { color: var(--dc-primary); margin-left: 4px; }
.conn-switcher :deep(.el-input__prefix .cs-icon) { font-size: 14px; }

/* 数据库 Logo 在树中：与树内 14px 的 el-icon 图标视觉一致，去掉灰色方块背景 */
.tree-logo { width: 14px; height: 14px; border-radius: 3px; flex-shrink: 0; }
.tree-logo :deep(img),
.tree-logo :deep(svg) {
  padding: 0 !important;
  background: transparent !important;
  filter: none !important;
  box-shadow: none !important;
  border-radius: 3px !important;
}

/* conn 节点行：与 tree-node 同构，确保 flex 填充 */
.conn-row { display: flex; align-items: center; gap: 5px; flex: 1; min-width: 0; }
.conn-label { flex: 1; min-width: 0; }
/* 拖拽：数据源行可抓取 */
/* 原来这里是 grab / grabbing —— 也就是树上那个"抓取手型"光标。
   拖拽换目录仍然可用，但不再用一只手去提示（按需求去掉）。 */
.dc-tree :deep(.el-tree-node__content .conn-row) { cursor: default; }
.dc-tree :deep(.el-tree-node__content:active .conn-row) { cursor: default; }
/* 被拖起的节点半透明 */
.dc-tree :deep(.el-tree-node.is-dragging > .el-tree-node__content) { opacity: .45; }
/* 放置目标：目录节点高亮，提示"可放入此文件夹" */
.dc-tree :deep(.el-tree-node.is-drop-inner > .el-tree-node__content) {
  background: var(--dc-primary-wash, rgba(79,140,255,.15)) !important;
  box-shadow: inset 0 0 0 1px var(--dc-primary, #4f8cff);
}

/* logo 容器：承载角标 */
.logo-wrap { position: relative; display: inline-flex; flex-shrink: 0; }

/* 「只读」徽标：跟在连接名后面（环境角标之前）。
   warning 色系 —— 它是一句**提醒**（这条连接写不进去），不是分类标签。 */
.ro-badge {
  flex: none; display: inline-flex; align-items: center;
  font-size: 10px; line-height: 1; padding: 2px 4px; border-radius: 4px;
  color: var(--dc-warning); background: var(--dc-warning-wash);
  border: 1px solid var(--dc-warning);
}

/* 环境角标：与文字水平居中 */
.env-corner {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 15px;
  height: 13px;
  padding: 0 4px;
  margin-left: 3px;
  font-size: 11px;
  font-weight: 700;
  line-height: 1;
  letter-spacing: 0.2px;
  border-radius: 3px;
  vertical-align: middle;
  flex-shrink: 0;
  user-select: none;
}
.env-corner-DEV { background: rgba(61,220,151,.15); color: #3ddc97; border: 1px solid rgba(61,220,151,.45); }
.env-corner-TEST { background: rgba(245,158,11,.15); color: #f59e0b; border: 1px solid rgba(245,158,11,.45); }
.env-corner-PROD { background: rgba(239,68,68,.16); color: #ef4444; border: 1px solid rgba(239,68,68,.5); }
.env-corner-STAGING { background: rgba(167,139,250,.15); color: #a78bfa; border: 1px solid rgba(167,139,250,.45); }
.env-corner-UAT { background: rgba(6,182,212,.15); color: #06b6d4; border: 1px solid rgba(6,182,212,.45); }
.env-corner-none { background: rgba(107,114,128,.15); color: #8b95a3; border: 1px solid rgba(107,114,128,.4); }
/* 环境文件夹节点：靠文字加粗 + 顶部分隔（通过 el-tree 第一层选择器） */
.dc-tree :deep(.el-tree > .el-tree-node > .el-tree-node__content) {
  border-top: 1px solid var(--dc-border);
  margin-top: 2px;
  background: var(--dc-hover-wash);
}
.dc-tree :deep(.el-tree > .el-tree-node:first-child > .el-tree-node__content) { border-top: none; margin-top: 0; }
.env-folder-label { font-weight: 700; font-size: 14px; letter-spacing: .3px; color: var(--dc-text); }
.env-count { font-size: 11px; color: var(--dc-text-dim); background: var(--dc-bg-soft); border-radius: 999px; padding: 0 7px; height: 16px; display: inline-flex; align-items: center; flex-shrink: 0; margin-left: auto; }


/* 树右键菜单浮层 */
.tree-ctx-menu {
  position: fixed; z-index: 3000; min-width: 180px;
  background: var(--dc-bg-sidebar); border: 1px solid var(--dc-border);
  border-radius: 6px; padding: 4px 0; box-shadow: var(--dc-shadow);
  list-style: none; margin: 0; font-size: 14px;
}
.tree-ctx-menu .ctx-item {
  display: flex; align-items: center; gap: 8px;
  padding: 7px 14px; cursor: pointer; user-select: none;
  color: var(--dc-text, #e4e6eb);
}
.tree-ctx-menu .ctx-item:hover { background: var(--dc-primary-wash); color: var(--dc-text-strong); }
.tree-ctx-menu .ctx-item.danger { color: var(--dc-danger); }
.tree-ctx-menu .ctx-item.danger:hover { background: var(--dc-danger-wash); color: var(--dc-danger); }
.tree-ctx-menu .ctx-item .el-icon { font-size: 14px; }
.tree-ctx-menu .ctx-item .el-icon { font-size: 14px; }
.tree-ctx-menu .divider { height: 1px; background: var(--dc-border); margin: 4px 6px; }
.tab-ctx-menu {
  position: fixed; z-index: 3000; min-width: 180px;
  background: var(--dc-bg-sidebar); border: 1px solid var(--dc-border);
  border-radius: 6px; padding: 4px 0; box-shadow: var(--dc-shadow);
  list-style: none; margin: 0; font-size: 14px;
}
.tab-ctx-menu .ctx-item {
  display: flex; align-items: center; gap: 8px;
  padding: 7px 14px; cursor: pointer; user-select: none;
  color: var(--dc-text, #e4e6eb);
}
.tab-ctx-menu .ctx-item:hover { background: var(--dc-primary-wash); color: var(--dc-text-strong); }
.tab-ctx-menu .ctx-item .el-icon { font-size: 14px; }
.tab-ctx-menu .divider { height: 1px; background: var(--dc-border); margin: 4px 6px; }
.dc-tabs { flex: 1; min-height: 0; display: flex !important; flex-direction: column; border: none; }
.dc-tabs :deep(.el-tabs__header) { margin: 0; background: var(--dc-bg-input); padding: 0; border: none; border-left: 1px solid var(--dc-border); }
.dc-tabs :deep(.el-tabs__nav-wrap) { margin-bottom: 0; }
.dc-tabs :deep(.el-tabs__nav-prev),
.dc-tabs :deep(.el-tabs__nav-next) {
  top: 0;
  height: 100%;
  line-height: normal !important;
  display: flex;
  align-items: center;
  color: var(--dc-text-dim);
}
.dc-tabs :deep(.el-tabs__nav-prev:hover),
.dc-tabs :deep(.el-tabs__nav-next:hover) { color: var(--dc-text); }
.dc-tabs :deep(.el-tabs__nav-wrap::after) { background-color: var(--dc-border); height: 1px; }
.dc-tabs :deep(.el-tabs__item) {
  border: none !important;
  border-radius: 0 !important;
  background: var(--dc-bg-input) !important;
  color: var(--dc-text-dim) !important;
  margin-right: 0 !important;
  height: 34px !important;
  padding: 0 14px !important;
  box-sizing: border-box;
  display: inline-flex !important;
  align-items: center !important;
  justify-content: center;
  line-height: normal;
  font-size: 14px !important;
  position: relative;
  transition: color .15s, background .15s;
}
.dc-tabs :deep(.el-tabs__item.is-active) {
  background: var(--dc-primary-wash) !important;
  color: var(--dc-primary) !important;
  font-weight: 600;
}

.dc-tabs :deep(.el-tabs__item.is-active:hover) { color: var(--dc-primary) !important; background: var(--dc-primary-wash) !important; }
.dc-tabs :deep(.el-tabs__item .el-icon.is-icon-close) { margin-left: 8px; vertical-align: middle; cursor: default; }
.dc-tabs :deep(.el-tabs__item:not(.is-active):hover) { color: var(--dc-text) !important; background: var(--dc-bg-hover) !important; }
.dc-tabs :deep(.el-tabs__content) { flex: 1 1 auto !important; min-height: 0; overflow: hidden; padding: 0; border: 1px solid var(--dc-border); border-top: none; background: var(--dc-bg-deep); }
.dc-tabs :deep(.el-tab-pane) { height: 100%; display: flex; flex-direction: column; }

/* ---- 页签拖拽排序 ----
   鼠标移到页签上**不要手型光标**：`grab`/`grabbing` 都是"手"，Element 默认给页签的 `pointer`
   同样是"手"，所以这里显式写 `default`。拖拽排序照旧可用 —— 拖动本身不依赖光标形状。 */
.dc-tabs :deep(.el-tabs__item) { cursor: default; }
.dc-tabs :deep(.el-tabs__item:active) { cursor: default; }
.tab-label { display: inline-flex; align-items: center; position: relative; }
/* PROD 连接的页签红 P 标：防误连 */
.tab-prod-dot {
  display: inline-flex; align-items: center; justify-content: center;
  width: 14px; height: 14px; margin-right: 4px;
  border-radius: 4px; background: #e34d4d; color: #fff;
  font-size: 10px; font-weight: 700; line-height: 1;
}
/* 被拎起来的页签：整个页签淡下去（不只是标签文字）、底色抽掉、加一圈虚线 —— 读起来就是
   "它被拎离原位了"。用 :has() 是因为 el-tabs__item 由 Element Plus 渲染，我们没法直接给它加类。 */
.dc-tabs :deep(.el-tabs__item:has(.tab-dragging)) {
  opacity: .42;
  background: transparent !important;
  outline: 1px dashed var(--dc-primary);
  outline-offset: -2px;
}
/* 落点提示：一根"插入指示条"，明确表达**插到左边还是右边** —— 并且**正好落在两个页签中间那条分界线**上。
   为什么不挂在 .tab-label 上按偏移量算：标签外面还夹着右内边距和关闭钮（实测左 14px、右 36px），
   要贴着分界线就得写死 14/36，一旦改主题或改内边距就会偏（之前那几个像素的偏差就是这么来的）。
   改挂到**页签项**上（它本身就是 position: relative，且 ::before/::after 没被 Element 占用），
   于是 left:0 / right:0 天然就是分界线本身，零魔法数字。
   两侧只给匹配的那一侧写 content，另一侧伪元素不存在，所以不会两边同时冒出来。 */
.dc-tabs :deep(.el-tabs__item:has(.tab-drop-target)) { background: var(--dc-primary-wash) !important; }
.dc-tabs :deep(.el-tabs__item:has(.tab-drop-before))::before,
.dc-tabs :deep(.el-tabs__item:has(.tab-drop-after))::after {
  content: ''; position: absolute; top: 50%; transform: translateY(-50%);
  width: 3px; height: 26px; border-radius: 2px; background: var(--dc-primary);
  box-shadow: 0 0 0 1px var(--dc-primary-soft, transparent);
}
/* 位置：**跨在两个页签的分界线上**，而不是贴在某一侧的内边。
   这两条边在布局上是重合的（前一项的右缘 / 后一项的左缘），所以两种情况要落在同一根线上：
   3px 宽的条再向外挪 1px，中心就正好压在分界线（见下方算式）：
     before: [item.left-1, item.left+2] ，after: [item.right-2, item.right+1]
     而 item[1].right ≈ item[2].left + 1，两边中心因此重合在同一处。
   抬高 z-index 是必须的：同一层里 DOM 靠后的页签会盖住靠前的，不提层级的话
   "跨到邻居那一侧"的半截会被邻居的背景吃掉，看起来又变成贴在一侧了。 */
.dc-tabs :deep(.el-tabs__item:has(.tab-drop-before)),
.dc-tabs :deep(.el-tabs__item:has(.tab-drop-after)) { z-index: 3; }
.dc-tabs :deep(.el-tabs__item:has(.tab-drop-before))::before { left: -1px; }
.dc-tabs :deep(.el-tabs__item:has(.tab-drop-after))::after { right: -1px; }


.empty-hint {
  height: 100%;
  /* 内容高于可视区时要能滚：原先垂直居中 + 无法滚动，高出来的部分被上下切掉，
     三个 KPI 卡的标题行就是这么"看不见"的（数字在、标题没了）。 */
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  /* stretch：子块铺满宽度（原来 center 会让它们缩成内容宽，两侧就是大片空白） */
  align-items: stretch;
  justify-content: flex-start;
  gap: 10px;
  text-align: center;
  padding: 10px 14px 12px;
  /* 不再设 1000px：列宽本来就窄，再框一层只会让左右空出来；
     超宽屏由 1680px 兜底，别铺成看不出结构的一条 */
  max-width: 1680px;
  margin: 0 auto;
  width: 100%;
  }
.empty-head { text-align: center; }
.empty-head h3 { font-size: 18px; font-weight: 650; color: var(--dc-text); margin: 0 0 6px; letter-spacing: .5px; }
.empty-head p { font-size: 13px; color: var(--dc-text-dim); margin: 0; }
/* 卡片等分整行：与下面两栏左右对齐（之前是居中，和通栏的两栏对不上，看着就乱） */
.empty-cards {
  display: flex; gap: 14px; flex-wrap: wrap; width: 100%;
}
/* 空态是「第一次打开见到的那一屏」：卡片太小 + 没有阴影时，整块像是浮在空白里。
   放大一点、给一层浅阴影，悬停时图标转主色（点得动这件事要看得出来）。 */
.empty-card {
  /* 等分整行（4 张卡铺满与下方两栏同宽），不再是四个固定宽的小方块 */
  flex: 1 1 0; min-width: 150px; padding: 24px 16px;
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border);
  border-radius: 12px;
  display: flex; flex-direction: column; align-items: center; gap: 8px;
  color: var(--dc-text-dim);
  cursor: pointer;
  transition: all .2s ease;
  user-select: none;
  box-shadow: 0 1px 2px var(--dc-shadow-sm);
}
.empty-card:hover {
  background: var(--dc-bg-hover);
  border-color: var(--dc-border-hover);
  color: var(--dc-text);
  transform: translateY(-2px);
  box-shadow: 0 6px 18px var(--dc-shadow-sm);
}
.empty-card .el-icon { color: var(--dc-text-dim); }
.empty-card:hover .el-icon { color: var(--dc-primary); }
.empty-card span { font-size: 14px; font-weight: 600; }
.empty-card small { font-size: 12px; color: var(--dc-text-dim); opacity: .8; }
/* ---- 首页仪表盘：内容块 / 操作项 / MCP 片段 ----
   .empty-stack 承担行间距并**垂直居中**：内容不满一屏时上下留白均分，
   超出一屏时 margin:auto 归零 + 容器 overflow 可滚，顶部不会被裁。 */
.empty-stack {
  display: flex; flex-direction: column; gap: 10px;
  width: 100%; margin: auto 0; text-align: left;
}
/* 每个内容块是一张卡：里面的行必须**扁平**（卡里再套卡很难看） */
.empty-block {
  width: 100%; text-align: left; padding: 9px 12px;
  background: var(--dc-bg-card); border: 1px solid var(--dc-border); border-radius: 10px;
}
.empty-acts { display: flex; flex-direction: column; }
.empty-act {
  display: flex; align-items: center; gap: 9px; width: 100%; text-align: left;
  padding: 5px 8px; border: none; border-radius: 8px; cursor: pointer;
  background: transparent; font: inherit; color: var(--dc-text);
  transition: background .15s ease;
}
.empty-act:hover { background: var(--dc-bg-hover); color: var(--dc-primary); }

/* 说明文字并入标题行（省一行高度，也不再有"孤零零一句提示"） */
.mcp-inline { font-size: 11.5px; color: var(--dc-text-dim); letter-spacing: 0; }
.mcp-code {
  position: relative; padding: 8px 10px; border-radius: 8px;
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border);
  /* 缩进版 JSON 是固定宽度的：窄屏让它自己横向滚，不撑破版面 */
  overflow-x: auto;
}
.mcp-code code {
  display: block; font-family: 'SF Mono', ui-monospace, Consolas, monospace;
  font-size: 12px; line-height: 1.4; white-space: pre; color: var(--dc-text);
}
/* 多行文本时按钮放右上角（竖居中的话会压住中间那几行 JSON） */
.mcp-copy { position: absolute; right: 4px; top: 4px; }

/* ---- 数据源速览（空态下半部分）：真实数据 + 点一下即打开 ---- */
.empty-dss { width: 100%; }
.empty-dss-head {
  display: flex; align-items: center; gap: 8px; margin-bottom: 8px;
  font-size: 12px; color: var(--dc-text-dim); letter-spacing: .06em;
}
/* 标题后面拉一条细线到底，把「分组标题」和卡片区分开 */
.empty-dss-head::after { content: ''; flex: 1; height: 1px; background: var(--dc-border); }
.empty-dss-count {
  order: 3; min-width: 22px; text-align: center; padding: 0 6px; border-radius: 999px;
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border);
}
/* 「清空历史」：排在计数徽标之后（标题与细线由 flex 撑开，这里只定顺序与尺寸）。
   用 text 型按钮 —— 它是卡片标题行上的一个次要动作，不该跟列表抢注意力。 */
.empty-head-btn { order: 4; padding: 0 4px; height: 20px; color: var(--dc-text-dim); }
/* 单列列表：正好显示 4 行（4×36 + 3×2 的间距 = 150px），第 5 个起在块内滚动。
   块本身不随连接数变高 —— 连接再多首页也不会被撑长。 */
.empty-dss-list {
  display: grid; grid-template-columns: 1fr; gap: 2px;
  max-height: 150px; overflow-y: auto; padding-right: 2px;
}
.empty-ds {
  display: flex; align-items: center; gap: 10px; width: 100%; text-align: left;
  /* 固定行高 36px：列表"正好 4 行"的高度才算得准，也不会因字体变化跳动 */
  height: 36px; padding: 0 10px; border-radius: 8px; cursor: pointer;
  background: transparent; border: none;
  transition: background .15s ease; font: inherit; color: var(--dc-text);
}
/* 斑马纹：行多了靠底色分区比靠间距清楚，也顺便让"列表"这件事一眼可见 */
.empty-ds:nth-child(odd) { background: var(--dc-bg-soft); }
.empty-ds:hover { background: var(--dc-bg-hover); }
.empty-ds:hover .ds-go { color: var(--dc-primary); }
/* 列表形态：名字在左、类型·主机靠右同一行（单列宽度够，一行比两行更好扫读） */
.ds-main { display: flex; flex-direction: row; align-items: baseline; gap: 12px; min-width: 0; flex: 1; }
.ds-sub { margin-left: auto; font-size: 11.5px; color: var(--dc-text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ds-go { color: var(--dc-text-dim); font-size: 13px; flex-shrink: 0; }
.ds-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; font-weight: 550; flex: 0 1 auto; }
.ds-type { margin-left: auto; font-size: 11.5px; color: var(--dc-text-dim); flex-shrink: 0; }
.ds-env {
  font-size: 11px; color: var(--dc-text-dim); flex-shrink: 0;
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border); border-radius: 999px; padding: 0 6px;
}
.empty-ds-more { grid-column: 1 / -1; font-size: 12px; color: var(--dc-text-dim); opacity: .8; }

/* 下半部分两栏：左边数据源、右边最近查询（都是真实内容，把宽度用起来）
   `minmax(0, 1fr)` 不能写成 `1fr`：grid item 的**自动最小尺寸**由内容决定，
   实测右侧那条长 SQL 会把右列顶到 1218px、左列被压成 195px（两栏直接塌掉）。 */
.empty-grid {
  display: grid; grid-template-columns: minmax(0, 1.08fr) minmax(0, 1fr); gap: 10px;
  /* stretch（默认）：左右两张卡等高，右边不会在底部空出一截 */
  width: 100%; align-items: stretch; text-align: left;
}
@media (max-width: 1100px) { .empty-grid { grid-template-columns: minmax(0, 1fr); } }
/* 第四行常显：与其它行一起压缩竖向空间（行距/内边距/图高），整体塞进一屏 ——
   首页的原则是不出滚动条，也不是大屏专属（矮屏藏掉它下方照样空一大块） */
/* 半栏宽度下，数据源一行一个更清楚（两列会把名字挤断） */
.empty-dss .empty-dss-list { grid-template-columns: 1fr; }
.empty-hist { width: 100%; }
.empty-hist-list { display: flex; flex-direction: column; gap: 4px; }
.empty-hi {
  display: flex; align-items: center; gap: 8px; width: 100%; text-align: left;
  padding: 3px 8px; border-radius: 8px; cursor: pointer;
  background: transparent; border: none;
  transition: background .15s ease; font: inherit; color: var(--dc-text);
}
.empty-hi:hover { background: var(--dc-bg-hover); }
.hi-dot { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; background: #cbd5e1; }
.hi-dot.ok { background: #10b981; }
.hi-dot.bad { background: #f43f5e; }
/* 不要 flex:1：那会把「连接·时间」顶到最右边，SQL 与它之间拖出半行空白。
   让 SQL 按内容占位（上限 74%），元信息紧跟其后。 */
.hi-sql {
  flex: 0 1 auto; max-width: 74%; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  font-family: 'SF Mono', ui-monospace, Consolas, monospace; font-size: 11.5px; color: var(--dc-text);
}

.hi-meta { flex-shrink: 0; font-size: 11px; color: var(--dc-text-dim); }
.empty-hs-none { font-size: 12px; color: var(--dc-text-dim); opacity: .85; padding: 10px 2px; }
.tree-node { display: flex; align-items: center; gap: 6px; flex: 1; min-width: 0; padding: 2px 0; }
/* 近 7 天趋势：7 根柱等分一行，成功/失败两段堆叠（失败在下段，红一眼可见）。
   趋势卡与右侧「数据工具」等高（grid stretch），图区 flex:1 吃掉剩余高度、
   内容贴底 —— 柱子和日期沉到卡片下缘，不再悬在半空留一截尾巴 */
.trend-block { display: flex; flex-direction: column; }
/* 「前往 MCP 服务设置」直达链接：主题色小按钮，hover 提亮 */
.mcp-go {
  border: none; background: transparent; cursor: pointer; padding: 2px 6px; border-radius: 6px;
  font: inherit; font-size: 12px; color: var(--dc-primary); white-space: nowrap;
  transition: background .15s ease;
}
.mcp-go:hover { background: var(--dc-bg-soft); }
/* 图例放标题行右端：拉线（::after）order 1、图例 order 2 —— 线在标题与图例之间 */
.trend-block .empty-dss-head::after { order: 1; }
.trend-legend {
  order: 2; display: inline-flex; align-items: center; gap: 4px;
  font-size: 11px; color: var(--dc-text-dim); letter-spacing: 0;
}
.trend-legend .lg { width: 9px; height: 9px; border-radius: 2px; display: inline-block; }
.trend-legend .lg.ok { background: var(--dc-primary); opacity: .75; }
.trend-legend .lg.bad { background: var(--el-color-danger, #e25c5c); }
.trend-block .empty-trend { flex: 1; justify-content: flex-end; padding-bottom: 2px; }
.empty-trend {
  display: flex; align-items: flex-end; justify-content: space-between;
  gap: 8px; padding: 6px 2px 0;
}
.trend-col { flex: 1 1 0; display: flex; flex-direction: column; align-items: center; gap: 3px; min-width: 0; }
.trend-bars {
  display: flex; flex-direction: column; justify-content: flex-end;
  align-items: stretch; width: 26px; height: 72px;
  border-radius: 5px; overflow: hidden; background: var(--dc-bg, transparent);
}
.trend-bar { width: 100%; }
.trend-bar.ok { background: var(--dc-primary); opacity: .75; border-radius: 0; }
.trend-bar.bad { background: var(--el-color-danger, #e25c5c); }
.trend-n { font-size: 12px; font-weight: 600; color: var(--dc-text-mid); line-height: 1; min-height: 12px; }
.trend-label { font-size: 11px; color: var(--dc-text-dim); line-height: 1; }

/* 树内重命名：输入框就地替换分组名，宽度跟着名字走、不撑满整行 */
.tree-rename-input { width: 132px; flex-shrink: 0; }
.tree-rename-input :deep(.el-input__wrapper) { padding: 0 6px; }
/* 多选浮条：出现在工具条与树之间；选中的节点整行加一层主色浅底 */
.multi-bar { display: flex; align-items: center; gap: 6px; margin: 6px 8px 0; padding: 4px 6px; border-radius: 6px; background: var(--dc-primary-wash); }
.multi-count { flex: 1; font-size: 12px; color: var(--dc-primary); white-space: nowrap; }
.multi-act { font-size: 12px; padding: 2px 8px; border: 1px solid var(--dc-border); border-radius: 4px; background: var(--dc-bg-card); color: var(--dc-text); cursor: pointer; }
.multi-act:hover:not(:disabled) { border-color: var(--dc-primary); color: var(--dc-primary); }
.multi-act.danger:hover:not(:disabled) { border-color: #ef4444; color: #ef4444; }
.multi-act:disabled { opacity: .55; cursor: default; }
.tree-node.multi-sel { background: var(--dc-primary-wash); border-radius: 4px; }
/* 索引元数据弹窗：内容区限高（索引多的表不至于把弹窗撑到天上去） */
.ctx-index-wrap { max-height: 56vh; overflow: auto; }
.ctx-index-empty { padding: 24px 4px; font-size: 13px; color: var(--dc-text-dim); text-align: center; }
.tree-icon { font-size: 15px; }
.node-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; min-width: 0; font-size: 14px; line-height: 1.4; font-weight: 400; color: var(--dc-text); }
/* 文字轻重分层：连接名比对象名重一档，当前库名再加粗。
   这样"连接 → 库 → 对象"三层在长列表里能一眼分开，不必靠颜色去猜。 */
.conn-label { font-weight: 500; color: var(--dc-text-strong); }
.db-label-current { font-weight: 600; color: var(--dc-text-strong); }
/* 说明性占位行（不是真对象）：小一号、灰一点，一眼看得出"这是一句说明" */
.hint-label { color: var(--dc-text-dim); font-size: 12.5px; font-weight: 400; }
.row-tag {
  font-size: 11px !important; line-height: 16px !important; border: none !important;
  border-radius: 999px !important; padding: 0 7px !important; height: 16px !important;
  display: inline-flex !important; align-items: center !important; margin-left: auto !important;
  background: var(--dc-bg-soft) !important;
  color: var(--dc-text-dim) !important;
}
.dc-tree :deep(.el-tree-node__content) { height: 30px; border-radius: 6px; }
.dc-tree :deep(.el-tree-node__content:hover) { background: var(--dc-bg-hover); }
.dc-tree :deep(.el-tree-node.is-current > .el-tree-node__content) {
  background: linear-gradient(90deg, var(--dc-primary-wash), transparent);
  box-shadow: inset 2px 0 0 var(--dc-primary);
}
.dc-tree { background: transparent; --el-tree-text-color: var(--dc-text); --el-tree-node-hover-bg-color: var(--dc-bg-hover); font-size: 14px; }
/* 字号加大后，行高放宽一点更舒展 */
.dc-tree :deep(.el-tree-node__content) { height: 30px; }
/* 树展开/折叠：直接瞬间开合，不要 element 默认 300ms 的 max-height 过渡（旧写法覆盖的 .collapse-transition 类名对 el-tree 无效） */
.dc-tree :deep(.el-collapse-transition-enter-active),
.dc-tree :deep(.el-collapse-transition-leave-active) { transition: none !important; }
/* 自绘展开箭头：固定 15px 位，展开时旋 90°，加载中转圈。
   el-tree 自带的那个箭头必须藏掉 —— 它 pointer-events:none（用户点不动），
   留着就会出现"两个箭头"，而且用户点的恰好是点不动的那一个。 */
.dc-tree :deep(.el-tree-node__expand-icon),
.dc-tree :deep(.el-tree-node__loading-icon) { display: none !important; }
.tree-arrow {
  font-size: 15px; flex-shrink: 0; cursor: pointer; border-radius: 3px;
  color: var(--dc-text-dim); transition: transform .12s ease, color .12s ease;
}
.tree-arrow:hover { color: var(--dc-text-strong); background: var(--dc-bg-soft); }
.tree-arrow-expanded { transform: rotate(90deg); }
.tree-arrow-spin { animation: dc-tree-spin 1s linear infinite; }
.tree-arrow-spacer { width: 15px; flex-shrink: 0; }
@keyframes dc-tree-spin { to { transform: rotate(360deg); } }
/* 图标变暗 = "还没打开"：
   只有库会用，且颜色恒为琥珀 —— 让明暗专门说状态，不再和类型颜色混在一起。 */
.tree-icon-dim { filter: grayscale(100%); opacity: .42; }
/* 行尾状态点：连接=已连接、库=已打开 */
.tree-dot {
  width: 6px; height: 6px; border-radius: 50%; background: #3ddc97;
  flex-shrink: 0; margin-left: 6px;
}

/* 右侧栏形态的 AI 面板：固定宽度不参与收缩。
   用 :not(.is-studio) 排除「中央工作区」形态——那种形态要占满 Tab，不能收缩，
   也不需要左侧分隔线（它自身已带左侧能力导航栏）。 */
.ai-panel:not(.is-studio) { flex-shrink: 0; border-left: 1px solid var(--dc-border); background: var(--dc-bg-sidebar); }

/* 表右键菜单 */
.ctx-menu {
  position: fixed; z-index: 9999; min-width: 180px; padding: 4px 0;
  background: var(--dc-bg-sidebar); border: 1px solid var(--dc-border); border-radius: 6px;
  box-shadow: var(--dc-shadow);
  list-style: none; margin: 0;
}
.ctx-menu li {
  position: relative; display: flex; align-items: center; gap: 8px;
  padding: 7px 12px; font-size: 14px; color: var(--dc-text); cursor: pointer; user-select: none;
}
.ctx-menu li:hover { background: var(--dc-primary-wash); color: var(--dc-text-strong); }
.ctx-menu li .el-icon { font-size: 14px; color: var(--dc-text-dim); flex-shrink: 0; }
.ctx-menu li:hover .el-icon { color: var(--dc-text-strong); }
.ctx-menu li .arrow { margin-left: auto; }
.ctx-menu li.divider { height: 1px; padding: 0; margin: 4px 6px; background: var(--dc-border); cursor: default; }
.ctx-menu li.divider:hover { background: var(--dc-border); }
.ctx-menu li.has-sub { padding-right: 8px; }
.ctx-menu li.danger { color: var(--dc-danger); }
.ctx-menu li.danger:hover { background: var(--dc-danger-wash); color: var(--dc-danger); }
.ctx-menu li.danger .el-icon { color: var(--dc-danger); }
.ctx-menu li.danger:hover .el-icon { color: var(--dc-danger); }
/* AI 入口：沿用原有的紫色强调，和普通项、危险项区分开（不抢危险色的注意力） */
.ctx-menu li.ai-item,
.ctx-menu li.ai-item .el-icon,
.ctx-menu li.ai-item:hover,
.ctx-menu li.ai-item:hover .el-icon { color: #a78bfa; }
.ctx-menu li .el-icon.ai-ic { color: #a78bfa; }
.ctx-submenu {
  position: absolute; left: 100%; top: 0; min-width: 150px; padding: 4px 0;
  background: var(--dc-bg-pop); border: 1px solid var(--dc-border); border-radius: 6px;
  box-shadow: var(--dc-shadow-sm); list-style: none; margin: 0;
}
.ctx-submenu li { padding: 7px 12px; }
.ctx-pre {
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border); border-radius: 6px;
  padding: 14px; overflow: auto; max-height: 480px;
  font-family: "SF Mono", Consolas, monospace; font-size: 13px; line-height: 1.6;
  color: var(--dc-link); white-space: pre-wrap; word-break: break-all; margin: 0;
  user-select: text; cursor: text;
}
/* ===== AI 解释视图弹窗 ===== */
/* ===== 检查更新弹窗 ===== */
/* 注意：el-dialog 是 teleport 渲染的，内部元素不带 scoped 的 data-v，
   高度/滚动这类「影响内部盒子」的规则必须用 :deep() 穿透，否则写了不生效（弹窗会被内容撑满屏） */
.upd-dialog :deep(.el-dialog__body) { max-height: 50vh; overflow-y: auto; padding: 4px 20px 8px; }
.upd-dialog :deep(.el-dialog__header) { padding: 16px 20px 12px; }
.upd-dialog :deep(.el-dialog__footer) { padding: 10px 20px 14px; }
.upd-lead { margin: 0 0 10px; font-size: 13px; color: var(--dc-text-mid, #555); }
.upd-notes { font-size: 12.5px; line-height: 1.7; color: var(--dc-text, #333); }
.upd-notes :deep(h1), .upd-notes :deep(h2), .upd-notes :deep(h3),
.upd-notes :deep(h4), .upd-notes :deep(h5) { margin: 12px 0 6px; font-size: 13.5px; font-weight: 700; }
.upd-notes :deep(h1:first-child), .upd-notes :deep(h2:first-child), .upd-notes :deep(p:first-child) { margin-top: 0; }
.upd-notes :deep(p) { margin: 0 0 8px; }
.upd-notes :deep(ul), .upd-notes :deep(ol) { margin: 0 0 10px; padding-left: 20px; }
.upd-notes :deep(li) { margin-bottom: 4px; }
.upd-notes :deep(code) {
  background: var(--dc-bg-soft, #f4f5f7); border-radius: 4px; padding: 1px 5px;
  font-family: var(--dc-mono, Consolas, monospace); font-size: 12px;
}
.upd-notes :deep(pre) {
  background: var(--dc-bg-soft, #f4f5f7); border-radius: 6px; padding: 10px 12px;
  overflow-x: auto; margin: 8px 0 12px;
}
.upd-notes :deep(pre code) { background: none; padding: 0; font-size: 11.5px; line-height: 1.6; }
.upd-notes :deep(table) { width: 100%; border-collapse: collapse; margin: 8px 0 12px; font-size: 12.5px; }
.upd-notes :deep(th), .upd-notes :deep(td) {
  border: 1px solid var(--dc-border, #e3e6eb); padding: 6px 10px; text-align: left;
}
.upd-notes :deep(th) { background: var(--dc-bg-soft, #f4f5f7); font-weight: 600; }
.upd-notes :deep(blockquote) {
  margin: 8px 0; padding: 6px 12px; border-left: 3px solid var(--dc-border, #dcdfe6);
  background: var(--dc-bg-soft, #fafbfc); color: var(--dc-text-dim, #888); border-radius: 0 6px 6px 0;
}
.upd-notes :deep(hr) { border: none; border-top: 1px solid var(--dc-border, #e3e6eb); margin: 12px 0; }
.upd-notes :deep(a) { color: var(--dc-link, #409eff); }
.upd-dir { display: flex; align-items: center; gap: 8px; margin: 0 0 12px; padding: 8px 10px; border: 1px solid var(--dc-border, #e3e6eb); border-radius: 8px; background: var(--dc-bg-soft, #fafbfc); }
.upd-dir-hint { margin: 8px 0 0; font-size: 12px; color: var(--dc-text-dim, #999); line-height: 1.6; }
.upd-dir-icon { color: var(--dc-text-dim, #999); font-size: 15px; }
.upd-footer { display: flex; align-items: center; justify-content: flex-end; gap: 8px; }
.upd-dl-body { padding: 4px 2px 2px; }
.upd-dl-meta { margin-top: 10px; font-size: 12.5px; color: var(--dc-text-mid, #666); text-align: center; }
.upd-dl-err { color: var(--el-color-danger, #f56c6c); word-break: break-all; }
.upd-dl-tip { margin-top: 8px; font-size: 12px; color: var(--dc-text-dim, #888); text-align: center; line-height: 1.6; }
.upd-more {
  display: inline-block; margin-top: 6px; font-size: 12.5px;
  color: var(--dc-link, #409eff); text-decoration: none;
}
.upd-more:hover { text-decoration: underline; }
.ai-explain-dialog :deep(.el-dialog) {
  max-height: 640px;
  display: flex;
  flex-direction: column;
}
.ai-explain-dialog :deep(.el-dialog__body) {
  padding: 0;
  overflow: hidden;
  flex: 1;
}
.ai-explain-body {
  padding: 20px 24px;
  font-size: 14px;
  line-height: 1.8;
  color: var(--dc-text);
  max-height: 520px;
  overflow-y: auto;
}
.ai-explain-body :deep(h1),
.ai-explain-body :deep(h2),
.ai-explain-body :deep(h3) {
  color: var(--dc-text-strong);
  margin: 16px 0 8px;
  font-weight: 600;
}
.ai-explain-body :deep(p) {
  margin: 8px 0;
}
.ai-explain-body :deep(ul),
.ai-explain-body :deep(ol) {
  margin: 8px 0;
  padding-left: 20px;
}
.ai-explain-body :deep(li) {
  margin: 4px 0;
}
.ai-explain-body :deep(code) {
  background: var(--dc-primary-wash);
  color: var(--dc-link);
  padding: 2px 6px;
  border-radius: 4px;
  font-family: "SF Mono", Consolas, monospace;
  font-size: 13px;
}
.ai-explain-body :deep(pre) {
  background: var(--dc-bg-code);
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  padding: 12px 16px;
  overflow: auto;
  margin: 10px 0;
}
.ai-explain-body :deep(pre code) {
  background: transparent;
  padding: 0;
  color: var(--dc-code-text);
}
.ai-explain-body :deep(blockquote) {
  border-left: 3px solid var(--dc-primary);
  margin: 10px 0;
  padding: 8px 16px;
  background: var(--dc-primary-wash);
  border-radius: 0 6px 6px 0;
}
</style>

<style>
/* 顶栏「数据」下拉菜单（teleport 到 body，需全局样式） */
.dc-top-menu { min-width: 160px; }
.dc-top-menu .el-dropdown-menu__item { min-height: 36px; }
.dc-conn-error-box {
  width: 720px !important;
  max-width: 92vw !important;
  border-radius: 12px !important;
  box-shadow: var(--dc-shadow) !important;
  background: var(--dc-bg-pop) !important;
  border: 1px solid var(--dc-border-strong) !important;
}
.dc-conn-error-box .el-message-box__header {
  padding: 20px 24px 12px !important;
  border-bottom: 1px solid var(--dc-border) !important;
}
.dc-conn-error-box .el-message-box__title {
  font-size: 16px !important;
  font-weight: 600 !important;
  color: var(--dc-danger) !important;
  letter-spacing: .3px;
}
.dc-conn-error-box .el-message-box__headerbtn {
  top: 16px !important;
  right: 18px !important;
}
.dc-conn-error-box .el-message-box__headerbtn .el-message-box__close {
  color: var(--dc-text-dim) !important;
  font-size: 18px !important;
  transition: color .2s;
}
.dc-conn-error-box .el-message-box__headerbtn .el-message-box__close:hover {
  color: var(--dc-text-strong) !important;
}
.dc-conn-error-box .el-message-box__status {
  display: none !important;
}
.dc-conn-error-box .el-message-box__content {
  padding: 18px 24px 22px !important;
}
.dc-conn-error-box .el-message-box__message {
  max-height: 60vh;
  overflow-y: auto;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--dc-text-mid) !important;
  font-size: 14px !important;
  line-height: 1.7 !important;
}
.dc-conn-error-box .el-message-box__btns {
  padding: 0 24px 20px !important;
}

/* ==================== 后台任务中心（顶栏时钟图标） ==================== */
.bg-task-btn .el-badge__content { z-index: 1; }
.bg-task-btn { position: relative; }
.bg-count { position: absolute; top: -3px; right: -4px; min-width: 14px; height: 14px; line-height: 14px; border-radius: 7px; background: var(--dc-primary); color: #fff; font-size: 10px; text-align: center; padding: 0 3px; box-sizing: border-box; }
.bg-toolbar { display: flex; justify-content: flex-start; margin-bottom: 10px; }
.bg-task-list { max-height: 300px; overflow-y: auto; }
.bg-task-head { font-size: 13px; font-weight: 600; color: var(--dc-text); padding-bottom: 8px; border-bottom: 1px solid var(--dc-border-soft); margin-bottom: 6px; }
.bg-task-empty { font-size: 12px; color: var(--dc-text-dim); padding: 14px 0; text-align: center; }
/* 执行记录表格：状态点在单元格里内联；行可点（查看） */
.bg-dot-inline { display: inline-block; margin-right: 6px; vertical-align: middle; }
:deep(.bg-row-click) { cursor: pointer; }
.bg-pager { margin-top: 10px; justify-content: flex-end; }
/* 时钟图标的悬浮下拉：正在执行的任务（纯 CSS hover，挂在图标正下方，不弹窗不飘位） */
.bg-hover { position: relative; display: inline-flex; }
.bg-running-panel {
  display: none;
  position: absolute; top: calc(100% + 8px); right: 0;
  width: 380px; z-index: 3000;
  background: var(--dc-bg, #fff);
  border: 1px solid var(--dc-border); border-radius: 10px;
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.14);
  padding: 12px 14px;
  cursor: default;
}
/* 桥接图标与面板之间的 8px 空隙：鼠标移过去穿过空隙时 hover 不断，
   面板才点得到（否则一移走就消失，任务项根本点不中） */
.bg-running-panel::before {
  content: ''; position: absolute; top: -9px; left: 0; right: 0; height: 9px;
}
.bg-hover:hover .bg-running-panel { display: block; }
.bg-running-head { font-size: 13px; font-weight: 600; color: var(--dc-text); margin-bottom: 8px; }
.bg-task-item { display: flex; align-items: center; gap: 8px; padding: 8px 6px; border-radius: 6px; cursor: pointer; font-size: 13px; }
.bg-task-item:hover { background: var(--dc-bg-hover); }
.bg-dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; background: var(--dc-primary); }
.bg-dot.success { background: var(--dc-success); }
.bg-dot.error { background: var(--dc-danger); }
.bg-dot.canceled { background: var(--dc-warning); }
.bg-kind { font-size: 11px; padding: 1px 6px; border-radius: 4px; flex-shrink: 0; background: rgba(59, 130, 246, 0.14); color: #3b82f6; border: 1px solid rgba(59, 130, 246, 0.4); }
.bg-kind.is-cmp { color: #f59e0b; border-color: rgba(245, 158, 11, 0.45); }
.bg-title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--dc-text); }
.bg-state { font-size: 12px; color: var(--dc-text-dim); flex-shrink: 0; }
</style>
