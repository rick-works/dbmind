<template>
  <div class="form-tab">
    <!-- 顶部标题栏（与「编辑表结构」同一形态） -->
    <div class="form-tab-header">
      <div class="header-title">
        <el-icon :size="16" class="header-icon"><User /></el-icon>
        <span class="header-text">{{ editMode ? $t('upv.editUser') : $t('upv.newUser') }}</span>
        <!-- 路径：**数据源名称在前**，中间用 `.` 连接（原来是「用户名」在前、连接名单一个标签） -->
        <el-tag size="small" effect="plain" type="info">{{ headPath }}</el-tag>
      </div>
    </div>

    <!-- 主体：上方分页签的表单 + 下方 SQL 预览（与「编辑表结构」同一套布局） -->
    <div class="form-tab-body">
      <div class="form-area">
        <!-- 卡片：页签（35px 那一行）与内容**同一张卡**，与「编辑表结构」一个形态 -->
        <div class="tab-card">
        <el-tabs v-model="tab" class="table-tabs">
          <!-- ===== 基本信息 ===== -->
          <el-tab-pane :label="$t('upv.tabBasic')" name="basic">
            <div class="tab-inner basic-tab">
              <!-- 该类型还没有自动授权：如实说明，别摆一堆点了会报错的控件 -->
              <el-alert v-if="!hasAnyGrantTab" type="info" :closable="false" show-icon class="grant-alert">
                <template #title>{{ $t('upv.grantUnsupported', { type: connTypeLabel }) }}</template>
              </el-alert>
              <!-- 基本信息：与「编辑表结构」同一张「标签 / 值」表（行高、对齐一致） -->
              <table class="form-table">
                <colgroup>
                  <col style="width:180px" />
                  <col />
                </colgroup>
                <thead>
                  <tr><th>{{ $t('tdet.item') }}</th><th>{{ $t('tdet.value') }}</th></tr>
                </thead>
                <tbody>
                  <tr>
                    <td class="ft-label">{{ $t('upv.fieldUser') }}</td>
                    <td><el-input v-model="form.name" size="small" clearable :disabled="editMode" /></td>
                  </tr>
                  <tr>
                    <td class="ft-label">{{ $t('upv.fieldPassword') }}</td>
                    <td>
                      <el-input v-model="form.password" size="small" type="password" show-password clearable
                                :placeholder="editMode ? $t('upv.phPasswordKeep') : $t('upv.phPassword')" />
                    </td>
                  </tr>
                  <tr v-if="isMysql">
                    <td class="ft-label">{{ $t('upv.fieldHost') }}</td>
                    <!-- ⚠️ 编辑态禁用：MySQL 的账号身份就是「用户名@主机」，而 ALTER USER **改不了 host**
                         —— 允许改的话，SQL 会指向一个不存在的账号（报错 or 什么都不发生）。
                         主机只影响"另一个账号"，要换主机是新建一个账号的事，不是改这一个。 -->
                    <td><el-input v-model="form.host" size="small" clearable :disabled="editMode"
                                  :placeholder="$t('upv.phHost')" /></td>
                  </tr>
                  <!-- 能改的项给控件（用户要求"能更改的都放开"）：改完点保存，
                       由后端按方言拼 ALTER 子句执行 -->
                  <tr v-for="d in editableAttrs" :key="d.key">
                    <td class="ft-label">{{ d.label }}</td>
                    <td class="ft-value">
                      <el-switch v-if="d.kind === 'bool'" v-model="form.attrs[d.key]"
                                 :active-text="$t('udv.yes')" :inactive-text="$t('udv.no')" />
                      <span v-else-if="d.kind === 'num'" class="attr-num">
                        <el-input-number v-model="form.attrs[d.key]" size="small" :min="0" :step="1" controls-position="right" />
                        <span v-if="!form.attrs[d.key]" class="attr-hint">{{ $t('udv.unlimited') }}</span>
                      </span>
                      <span v-else-if="d.kind === 'plugin'" class="attr-num">
                        <el-select class="attr-select" :model-value="authPluginModel" size="small"
                                   :placeholder="$t('upv.attrDefault')"
                                   @update:model-value="(v) => { form.attrs.authPlugin = v }">
                          <el-option v-for="p in pluginOptions" :key="p" :label="p" :value="p" />
                        </el-select>
                        <span v-if="form.attrs.authPlugin !== attrsBase.authPlugin" class="attr-hint">{{ $t('upv.pluginNeedsPassword') }}</span>
                      </span>
                      <el-input v-else v-model="form.attrs[d.key]" size="small" />
                    </td>
                  </tr>
                  <!-- 跟着数据源类型来的只读明细（Oracle 的表空间/状态、SQL Server 的登录名/默认架构、
                       PG 的 rol*、ClickHouse 的 storage/auth、MySQL 的插件/账号锁定…）。
                       这些列后端本来就返回了（见 dialect.rs 的 user_info），原来前端只取了 name / host，
                       于是不管哪种库都只有「用户名 / 密码 / 主机」三行。 -->
                  <tr v-for="it in detailItems" :key="it.key">
                    <td class="ft-label">{{ it.label }}</td>
                    <td class="ft-value" :class="it.cls">{{ it.value }}</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </el-tab-pane>

          <!-- ===== 服务器权限 ===== -->
          <el-tab-pane v-if="hasServerTab" :label="serverTabLabel" name="server">
            <div class="tab-inner grant-tab">
              <!-- MySQL 系：全局权限 —— 「权限项当列」。
                   分组写在**表头第一行**（colspan 跨它自己的那几列，带计数与全选/清空），
                   第二行才是权限名；主体只有一行（就是这个用户）。
                   原来是"一行一个分组"：每组只在自己那几列上打勾，34 列 × 4 行里大半是空的，
                   读起来一片空洞，也看不出"这一格为什么不给勾"。 -->
              <table v-if="isMysql" class="form-table mx-table mx-grouped">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1" rowspan="2">{{ $t('tdet.item') }}</th>
                    <th v-for="g in globalGrouped" :key="g.key" :colspan="g.items.length" class="mx-grp" :title="g.label">
                      <span class="ft-group-name">{{ g.label }}</span>
                      <span class="priv-group-count">{{ countIn('privileges', g.items) }} / {{ g.items.length }}</span>
                      <span class="priv-group-actions">
                        <button :disabled="countIn('privileges', g.items) === g.items.length"
                                @click="setList('privileges', g.items, true)">{{ $t('common.selectAll') }}</button>
                        <button :disabled="countIn('privileges', g.items) === 0"
                                @click="setList('privileges', g.items, false)">{{ $t('common.clear') }}</button>
                      </span>
                    </th>
                  </tr>
                  <tr>
                    <th v-for="p in globalPrivColumns" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr>
                    <td class="mx-fix mx-f1 ft-label">{{ $t('upv.privileges') }}</td>
                    <td v-for="p in globalPrivColumns" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasIn('privileges', p)"
                                   @change="(v) => toggleList('privileges', p, v)" />
                    </td>
                  </tr>
                </tbody>
              </table>

              <!-- SQL Server 服务器角色 / Oracle 系统权限 / PostgreSQL 角色：
                   勾选项本身就是名字，一个一列，只有一行 -->
              <table v-else class="form-table mx-table mx-grouped">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1" rowspan="2">{{ $t('tdet.item') }}</th>
                    <th v-for="g in serverGrouped" :key="g.key" :colspan="g.items.length" class="mx-grp" :title="g.label">
                      <span class="ft-group-name">{{ g.label }}</span>
                      <span class="priv-group-count">{{ countIn(serverModelKey, g.items) }} / {{ g.items.length }}</span>
                      <span class="priv-group-actions">
                        <button :disabled="countIn(serverModelKey, g.items) === g.items.length"
                                @click="setList(serverModelKey, g.items, true)">{{ $t('common.selectAll') }}</button>
                        <button :disabled="countIn(serverModelKey, g.items) === 0"
                                @click="setList(serverModelKey, g.items, false)">{{ $t('common.clear') }}</button>
                      </span>
                    </th>
                  </tr>
                  <tr>
                    <th v-for="p in serverColumns" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr>
                    <td class="mx-fix mx-f1 ft-label">{{ serverRowLabel }}</td>
                    <td v-for="p in serverColumns" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasIn(serverModelKey, p)"
                                   @change="(v) => toggleList(serverModelKey, p, v)" />
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </el-tab-pane>

          <!-- ===== 角色（Oracle）===== -->
          <el-tab-pane v-if="hasRolesTab" :label="$t('upv.tabRoles')" name="roles">
            <div class="tab-inner grant-tab">
              <table class="form-table mx-table mx-grouped">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1" rowspan="2">{{ $t('tdet.item') }}</th>
                    <th v-for="g in rolesGrouped" :key="g.key" :colspan="g.items.length" class="mx-grp" :title="g.label">
                      <span class="ft-group-name">{{ g.label }}</span>
                      <span class="priv-group-count">{{ countIn('roles', g.items) }} / {{ g.items.length }}</span>
                      <span class="priv-group-actions">
                        <button :disabled="countIn('roles', g.items) === g.items.length"
                                @click="setList('roles', g.items, true)">{{ $t('common.selectAll') }}</button>
                        <button :disabled="countIn('roles', g.items) === 0"
                                @click="setList('roles', g.items, false)">{{ $t('common.clear') }}</button>
                      </span>
                    </th>
                  </tr>
                  <tr>
                    <th v-for="p in oracleRoles" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr>
                    <td class="mx-fix mx-f1 ft-label">{{ $t('upv.tabRoles') }}</td>
                    <td v-for="p in oracleRoles" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasIn('roles', p)" @change="(v) => toggleList('roles', p, v)" />
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </el-tab-pane>

          <!-- ===== 数据库权限 ===== -->
          <el-tab-pane v-if="hasDbTab" :label="dbTabLabel" name="database">
            <!-- 动作条：与页签**同一行**、贴右（照「编辑表结构」的排法）：
                 绝对定位到卡片右上角，表格里不再单独占一行 -->
            <div class="tab-actions">
              <el-button v-if="isMysql" size="small" type="primary" :icon="Plus" @click="addDbPriv">{{ $t('upv.addDbPriv') }}</el-button>
            </div>
            <div class="tab-inner grant-tab">
              <!-- MySQL 系：一行一个库，每个库级权限一列 -->
              <table v-if="isMysql" class="form-table mx-table mx-grouped">
                <thead>
                  <!-- 分类行（colspan 跨它那几列）：一行一个库，每组的"已勾数"随行而变，这里只写分类名 -->
                  <tr>
                    <th class="mx-fix mx-f1" rowspan="2">{{ $t('upv.colDb') }}</th>
                    <th v-for="g in dbGrouped" :key="g.key" :colspan="g.items.length" class="mx-grp" :title="g.label">
                      <span class="ft-group-name">{{ g.label }}</span>
                      <span class="priv-group-count">{{ rowsCount(form.dbPrivileges, g.items) }} / {{ rowsTotal(form.dbPrivileges, g.items) }}</span>
                      <span class="priv-group-actions">
                        <button :disabled="rowsCount(form.dbPrivileges, g.items) === rowsTotal(form.dbPrivileges, g.items)"
                                @click="setRowsPrivileges(form.dbPrivileges, g.items, true)">{{ $t('common.selectAll') }}</button>
                        <button :disabled="rowsCount(form.dbPrivileges, g.items) === 0"
                                @click="setRowsPrivileges(form.dbPrivileges, g.items, false)">{{ $t('common.clear') }}</button>
                      </span>
                    </th>
                    <th class="mx-act" rowspan="2">{{ $t('upv.colAction') }}</th>
                  </tr>
                  <tr>
                    <th v-for="p in dbPrivColumns" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-if="form.dbPrivileges.length === 0">
                    <td :colspan="dbPrivColumns.length + 2" class="ft-empty">{{ $t('upv.noDbPriv') }}</td>
                  </tr>
                  <tr v-for="(row, i) in form.dbPrivileges" :key="rowKey(row)">
                    <td class="mx-fix mx-f1">
                      <el-select v-model="row.database" size="small" :placeholder="$t('upv.phPickDb')" filterable>
                        <el-option v-for="db in databases" :key="db" :label="db" :value="db" />
                      </el-select>
                      <el-tag v-if="isDbMissing(row.database)" size="small" type="warning" effect="plain">{{ $t('upv.dbMissing') }}</el-tag>
                      <el-tag v-if="isDorisSystemDb(row.database)" size="small" type="info" effect="plain">{{ $t('upv.dorisSystemDb') }}</el-tag>
                    </td>
                    <td v-for="p in dbPrivColumns" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasObjectPriv(row, p)" @change="(v) => toggleObjectPriv(row, p, v)" />
                    </td>
                    <td class="mx-cell">
                      <el-button type="danger" size="small" :icon="Delete" text @click="removeDbPriv(i)" />
                    </td>
                  </tr>
                </tbody>
              </table>

              <!-- SQL Server：数据库角色 —— 一个角色一列 -->
              <table v-if="isMssql" class="form-table mx-table mx-grouped">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1" rowspan="2">{{ $t('tdet.item') }}</th>
                    <th v-for="g in mssqlDbRolesGrouped" :key="g.key" :colspan="g.items.length" class="mx-grp" :title="g.label">
                      <span class="ft-group-name">{{ g.label }}</span>
                      <span class="priv-group-count">{{ countIn('roles', g.items) }} / {{ g.items.length }}</span>
                      <span class="priv-group-actions">
                        <button :disabled="countIn('roles', g.items) === g.items.length"
                                @click="setList('roles', g.items, true)">{{ $t('common.selectAll') }}</button>
                        <button :disabled="countIn('roles', g.items) === 0"
                                @click="setList('roles', g.items, false)">{{ $t('common.clear') }}</button>
                      </span>
                    </th>
                  </tr>
                  <tr>
                    <th v-for="p in mssqlRoles" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr>
                    <td class="mx-fix mx-f1 ft-label">{{ $t('upv.segDbRoles') }}</td>
                    <td v-for="p in mssqlRoles" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasIn('roles', p)" @change="(v) => toggleList('roles', p, v)" />
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </el-tab-pane>

          <!-- ===== 细粒度权限 =====
               MySQL 系 = 表权限一页、Oracle = 对象权限一页；**SQL Server 拆成两页**
               （Schema 级 / 对象级）—— 原来两张表挤在同一页里上下叠着，读起来分不清
               是"同一份权限的两个视图"还是"两件事"（用户要求拆开）。 -->
          <el-tab-pane v-if="hasGranularTab && !isMssql" :label="granularTabLabel" name="granular">
            <!-- 动作条：与页签同一行、贴右 -->
            <div class="tab-actions">
              <el-button v-if="isOracle" size="small" type="primary" :icon="Plus" @click="addTablePriv">{{ $t('upv.addObjectPrivOracle') }}</el-button>
              <el-button v-if="isMysql" size="small" type="primary" :icon="Plus" @click="addTablePriv">{{ $t('upv.addTablePriv') }}</el-button>
            </div>
            <div class="tab-inner grant-tab">
              <!-- Oracle：对象权限（「模式 · 名称」两列冻结，权限项当列横向滚动） -->
              <table v-if="isOracle" class="form-table mx-table mx-grouped">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1" rowspan="2">{{ $t('upv.colSchema') }}</th>
                    <th class="mx-fix mx-f2" rowspan="2">{{ $t('upv.colObject') }}</th>
                    <th v-for="g in objectGrouped" :key="g.key" :colspan="g.items.length" class="mx-grp" :title="g.label">
                      <span class="ft-group-name">{{ g.label }}</span>
                      <span class="priv-group-count">{{ rowsCount(form.tablePrivileges, g.items) }} / {{ rowsTotal(form.tablePrivileges, g.items) }}</span>
                      <span class="priv-group-actions">
                        <button :disabled="rowsCount(form.tablePrivileges, g.items) === rowsTotal(form.tablePrivileges, g.items)"
                                @click="setRowsPrivileges(form.tablePrivileges, g.items, true)">{{ $t('common.selectAll') }}</button>
                        <button :disabled="rowsCount(form.tablePrivileges, g.items) === 0"
                                @click="setRowsPrivileges(form.tablePrivileges, g.items, false)">{{ $t('common.clear') }}</button>
                      </span>
                    </th>
                    <th class="mx-act" rowspan="2">{{ $t('upv.colAction') }}</th>
                  </tr>
                  <tr>
                    <th v-for="p in objectPrivColumns" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-if="form.tablePrivileges.length === 0">
                    <td :colspan="objectPrivColumns.length + 3" class="ft-empty">{{ $t('upv.noObjectPrivOracle') }}</td>
                  </tr>
                  <tr v-for="(row, i) in form.tablePrivileges" :key="rowKey(row)">
                    <td class="mx-fix mx-f1">
                      <el-select v-model="row.database" size="small" :placeholder="$t('upv.phPickSchema')" filterable @change="onTablePrivDbChange(i)">
                        <el-option v-for="s in schemas" :key="s" :label="s" :value="s" />
                      </el-select>
                    </td>
                    <td class="mx-fix mx-f2">
                      <el-select v-model="row.table" size="small" :placeholder="$t('upv.phPickObject')" filterable :loading="row._loading">
                        <el-option v-for="t in row._tables || []" :key="t" :label="t" :value="t" />
                      </el-select>
                    </td>
                    <td v-for="p in objectPrivColumns" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasObjectPriv(row, p)" @change="(v) => toggleObjectPriv(row, p, v)" />
                    </td>
                    <td class="mx-cell">
                      <el-button type="danger" size="small" :icon="Delete" text @click="removeTablePriv(i)" />
                    </td>
                  </tr>
                </tbody>
              </table>

              <!-- MySQL 系：表级（「数据库 · 表」两列冻结） -->
              <table v-if="isMysql" class="form-table mx-table mx-grouped">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1" rowspan="2">{{ $t('upv.colDb') }}</th>
                    <th class="mx-fix mx-f2" rowspan="2">{{ $t('upv.colTableName') }}</th>
                    <th v-for="g in tableGrouped" :key="g.key" :colspan="g.items.length" class="mx-grp" :title="g.label">
                      <span class="ft-group-name">{{ g.label }}</span>
                      <span class="priv-group-count">{{ rowsCount(form.tablePrivileges, g.items) }} / {{ rowsTotal(form.tablePrivileges, g.items) }}</span>
                      <span class="priv-group-actions">
                        <button :disabled="rowsCount(form.tablePrivileges, g.items) === rowsTotal(form.tablePrivileges, g.items)"
                                @click="setRowsPrivileges(form.tablePrivileges, g.items, true)">{{ $t('common.selectAll') }}</button>
                        <button :disabled="rowsCount(form.tablePrivileges, g.items) === 0"
                                @click="setRowsPrivileges(form.tablePrivileges, g.items, false)">{{ $t('common.clear') }}</button>
                      </span>
                    </th>
                    <th class="mx-act" rowspan="2">{{ $t('upv.colAction') }}</th>
                  </tr>
                  <tr>
                    <th v-for="p in tablePrivColumns" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-if="form.tablePrivileges.length === 0">
                    <td :colspan="tablePrivColumns.length + 3" class="ft-empty">{{ $t('upv.noTablePriv') }}</td>
                  </tr>
                  <tr v-for="(row, i) in form.tablePrivileges" :key="rowKey(row)">
                    <td class="mx-fix mx-f1">
                      <el-select v-model="row.database" size="small" :placeholder="$t('upv.phPickDb')" filterable @change="onTablePrivDbChange(i)">
                        <el-option v-for="db in databases" :key="db" :label="db" :value="db" />
                      </el-select>
                    </td>
                    <td class="mx-fix mx-f2">
                      <el-select v-model="row.table" size="small" :placeholder="$t('upv.phPickTable')" filterable :loading="row._loading">
                        <el-option v-for="t in row._tables || []" :key="t" :label="t" :value="t" />
                      </el-select>
                    </td>
                    <td v-for="p in tablePrivColumns" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasObjectPriv(row, p)" @change="(v) => toggleObjectPriv(row, p, v)" />
                    </td>
                    <td class="mx-cell">
                      <el-button type="danger" size="small" :icon="Delete" text @click="removeTablePriv(i)" />
                    </td>
                  </tr>
                </tbody>
              </table>

            </div>
          </el-tab-pane>
          <!-- SQL Server：Schema 级与对象级**各占一页**（原来两张表挤在同一页里上下叠着） -->
          <el-tab-pane v-if="isMssql" :label="$t('upv.tabSchema')" name="schema">
            <div class="tab-actions">
              <el-button size="small" type="primary" :icon="Plus" @click="addSchemaPriv">{{ $t('upv.addSchemaPriv') }}</el-button>
            </div>
            <div class="tab-inner grant-tab">
              <!-- SQL Server：Schema 级（模式冻结） -->
              <table v-if="isMssql" class="form-table mx-table mx-grouped">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1" rowspan="2">{{ $t('upv.colSchema') }}</th>
                    <th v-for="g in schemaGrouped" :key="g.key" :colspan="g.items.length" class="mx-grp" :title="g.label">
                      <span class="ft-group-name">{{ g.label }}</span>
                      <span class="priv-group-count">{{ rowsCount(form.schemaPrivileges, g.items) }} / {{ rowsTotal(form.schemaPrivileges, g.items) }}</span>
                      <span class="priv-group-actions">
                        <button :disabled="rowsCount(form.schemaPrivileges, g.items) === rowsTotal(form.schemaPrivileges, g.items)"
                                @click="setRowsPrivileges(form.schemaPrivileges, g.items, true)">{{ $t('common.selectAll') }}</button>
                        <button :disabled="rowsCount(form.schemaPrivileges, g.items) === 0"
                                @click="setRowsPrivileges(form.schemaPrivileges, g.items, false)">{{ $t('common.clear') }}</button>
                      </span>
                    </th>
                    <th class="mx-act" rowspan="2">{{ $t('upv.colAction') }}</th>
                  </tr>
                  <tr>
                    <th v-for="p in schemaPrivColumns" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-if="form.schemaPrivileges.length === 0">
                    <td :colspan="schemaPrivColumns.length + 2" class="ft-empty">{{ $t('upv.noSchemaPriv') }}</td>
                  </tr>
                  <tr v-for="(row, i) in form.schemaPrivileges" :key="rowKey(row)">
                    <td class="mx-fix mx-f1">
                      <el-select v-model="row.schema" size="small" :placeholder="$t('upv.phPickSchema')" filterable>
                        <el-option v-for="s in schemas" :key="s" :label="s" :value="s" />
                      </el-select>
                    </td>
                    <td v-for="p in schemaPrivColumns" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasObjectPriv(row, p)" @change="(v) => toggleObjectPriv(row, p, v)" />
                    </td>
                    <td class="mx-cell">
                      <el-button type="danger" size="small" :icon="Delete" text @click="removeSchemaPriv(i)" />
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </el-tab-pane>

          <el-tab-pane v-if="isMssql" :label="$t('upv.tabObjects')" name="object">
            <div class="tab-actions">
              <el-button size="small" type="primary" :icon="Plus" @click="addObjectPriv">{{ $t('upv.addObjectPriv') }}</el-button>
            </div>
            <div class="tab-inner grant-tab">
              <!-- SQL Server：对象级（「模式 · 对象」两列冻结，两者都可手写） -->
              <table v-if="isMssql" class="form-table mx-table mx-grouped">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1" rowspan="2">{{ $t('upv.colSchema') }}</th>
                    <th class="mx-fix mx-f2" rowspan="2">{{ $t('upv.colObject') }}</th>
                    <th v-for="g in objectGroupedMssql" :key="g.key" :colspan="g.items.length" class="mx-grp" :title="g.label">
                      <span class="ft-group-name">{{ g.label }}</span>
                      <span class="priv-group-count">{{ rowsCount(form.objectPrivileges, g.items) }} / {{ rowsTotal(form.objectPrivileges, g.items) }}</span>
                      <span class="priv-group-actions">
                        <button :disabled="rowsCount(form.objectPrivileges, g.items) === rowsTotal(form.objectPrivileges, g.items)"
                                @click="setRowsPrivileges(form.objectPrivileges, g.items, true)">{{ $t('common.selectAll') }}</button>
                        <button :disabled="rowsCount(form.objectPrivileges, g.items) === 0"
                                @click="setRowsPrivileges(form.objectPrivileges, g.items, false)">{{ $t('common.clear') }}</button>
                      </span>
                    </th>
                    <th class="mx-act" rowspan="2">{{ $t('upv.colAction') }}</th>
                  </tr>
                  <tr>
                    <th v-for="p in objectPrivColumnsMssql" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-if="form.objectPrivileges.length === 0">
                    <td :colspan="objectPrivColumnsMssql.length + 3" class="ft-empty">{{ $t('upv.noObjectPriv') }}</td>
                  </tr>
                  <tr v-for="(row, i) in form.objectPrivileges" :key="rowKey(row)">
                    <td class="mx-fix mx-f1"><el-input v-model="row.schema" size="small" placeholder="Schema" /></td>
                    <td class="mx-fix mx-f2"><el-input v-model="row.object" size="small" :placeholder="$t('upv.phObject')" /></td>
                    <td v-for="p in objectPrivColumnsMssql" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasObjectPriv(row, p)" @change="(v) => toggleObjectPriv(row, p, v)" />
                    </td>
                    <td class="mx-cell">
                      <el-button type="danger" size="small" :icon="Delete" text @click="removeObjectPriv(i)" />
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </el-tab-pane>

        </el-tabs>
        </div>
      </div>

      <!-- 分隔条：与「编辑表结构」同一套（向上拖 = 预览变高，双击最大/最小） -->
      <div v-show="!sqlCollapsed" class="sql-resizer"
           :title="$t('tdet.resizerTitle')"
           @mousedown.prevent="onSqlResizeStart" @dblclick="onSqlResizeDblClick"></div>

      <!-- SQL 预览：挪到**下方**（原来在右侧固定栏），点标题行折叠/展开 -->
      <div ref="sqlAreaRef" class="sql-area" :class="{ collapsed: sqlCollapsed }"
           :style="sqlCollapsed ? null : { height: sqlHeight + 'px' }">
        <div class="sql-head" @click="sqlCollapsed = !sqlCollapsed"
             :title="sqlCollapsed ? $t('tdet.expandSql') : $t('tdet.collapseSql')">
          <el-icon class="sql-fold" :class="{ folded: sqlCollapsed }"><CaretRight /></el-icon>
          <el-icon><DocumentCopy /></el-icon>
          <span>{{ $t('upv.sqlPreview') }}</span>
          <span class="sql-fold-tx">{{ sqlCollapsed ? $t('tdet.expand') : $t('tdet.collapse') }}</span>
        </div>
        <div v-show="!sqlCollapsed" class="sql-body">
          <!-- 默认不显示 SQL 文本：勾选/取消之后（有差量了）才出语句；框本身一直留着 -->
          <pre><code>{{ previewSql || $t('upv.sqlPlaceholder') }}</code></pre>
        </div>
        <!-- 与「编辑表结构」同一排法：左边一句说明，右边一排动作按钮 -->
        <div class="sql-foot">
          <span class="sql-tip">{{ $t('upv.previewTip') }}</span>
          <div class="sql-foot-actions">
            <el-button size="small" text :icon="CopyDocument" :disabled="!previewSql" @click="copySql">{{ $t('upv.copySql') }}</el-button>
            <el-button size="small" type="primary" :icon="CaretRight" :loading="saving" :disabled="!previewOk" @click="submit">
              {{ editMode ? $t('upv.saveChanges') : $t('upv.execCreate') }}
            </el-button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onBeforeUnmount } from 'vue'
import { ElMessage } from 'element-plus'
import { t } from '../../utils/i18n'
import { User, CaretRight, CopyDocument, DocumentCopy, Plus, Delete } from '@element-plus/icons-vue'
import { userAction, listDatabases, listTables, listSchemas } from '../../api'
import { rowKey } from '../../utils/rowKey'
import { userDetailItems } from '../../utils/userFields'

const props = defineProps({
  conn: { type: Object, required: true },
  database: { type: String, default: '' },
  editMode: { type: Boolean, default: false },
  editData: { type: Object, default: () => ({}) }
})

const emit = defineEmits(['saved', 'close'])

const form = ref({
  name: '',
  password: '',
  host: '%',
  privileges: [],
  roles: [],
  dbPrivileges: [],
  tablePrivileges: [],
  serverRoles: [],
  schemaPrivileges: [],
  objectPrivileges: [],
  // 可编辑的用户属性（账户锁定 / 配额 / 插件 / 表空间…），见 editableAttrs
  attrs: {}
})

const saving = ref(false)
const databases = ref([])
const schemas = ref([])
// 当前页签：基本信息 / 服务器权限 / 数据库权限 / 细粒度权限
// （原来是每张卡里再放一个"分段按钮"，层级与「编辑表结构」不一致，用户要求改成页签形态）
const tab = ref('basic')
const mssqlServerRoles = [
  'sysadmin', 'securityadmin', 'serveradmin', 'setupadmin',
  'processadmin', 'diskadmin', 'dbcreator', 'bulkadmin'
]

// 标题栏的路径：数据源名称在前 + `.` 连接（新建时还没有用户名，只显示数据源）
const headPath = computed(() => {
  // MySQL 系的账号是「用户名@主机」—— 同名不同主机是两个账号，头里带全才分得清
  // （页签条上仍然只显示最后一层，全路径在悬浮提示里）
  const who = isMysql.value && form.value.host ? `${form.value.name}@${form.value.host}` : form.value.name
  return [props.conn?.name, who].filter(Boolean).join('.')
})

// ===== 基本信息里「能改的那些」=====
// 各库能改的子集不同（执行那侧按方言拼 ALTER 子句，见后端 dialect.rs 的 user_action_sql）：
//   MySQL 系    : ACCOUNT LOCK/UNLOCK、PASSWORD EXPIRE、WITH MAX_*、IDENTIFIED WITH <插件>
//   Oracle / DM : ACCOUNT LOCK/UNLOCK、默认 / 临时表空间、概要文件
//   SQL Server  : WITH DEFAULT_SCHEMA、ALTER LOGIN DISABLE/ENABLE
//   PG/Kingbase : LOGIN / SUPERUSER / CREATEDB
// 提交只带**改动过的**项（attrsBase 是打开表单那一刻的基准），没动过的不会出现在 SQL 里。
const ATTR_PLUGINS = ['mysql_native_password', 'caching_sha2_password', 'sha256_password', 'unix_socket']

// 从用户详情的明细列里取值：各家键名大小写与下划线都不一样（`account_locked` / `ACCOUNT_LOCKED`），
// 去掉非字母数字再比（与 utils/userFields.js 的 normKey 同一口径）。
const attrRaw = (name) => {
  const want = String(name).toUpperCase().replace(/[^A-Z0-9]/g, '')
  for (const [k, v] of Object.entries(props.editData?.fields || {})) {
    if (String(k).toUpperCase().replace(/[^A-Z0-9]/g, '') === want) return v
  }
  return undefined
}
const attrYes = (v) => v === true || v === 1 || v === '1' || v === 'Y' || String(v).toLowerCase() === 'yes'
const attrNum = (v) => { const n = Number(v); return Number.isFinite(n) ? n : 0 }
const attrText = (v) => (v === undefined || v === null ? '' : String(v))

// 每个可改项在明细列里的原始键名（大小写两套都列上）：改控件的不再在下面重复显示一遍
const ATTR_RAW_KEYS = {
  accountLocked: ['account_locked', 'ACCOUNT_LOCKED', 'account_status', 'ACCOUNT_STATUS'],
  passwordExpired: ['password_expired', 'PASSWORD_EXPIRED'],
  maxQueriesPerHour: ['max_questions', 'MAX_QUESTIONS'],
  maxUpdatesPerHour: ['max_updates', 'MAX_UPDATES'],
  maxConnectionsPerHour: ['max_connections', 'MAX_CONNECTIONS'],
  maxUserConnections: ['max_user_connections', 'MAX_USER_CONNECTIONS'],
  authPlugin: ['plugin', 'PLUGIN'],
  defaultTablespace: ['default_tablespace', 'DEFAULT_TABLESPACE'],
  temporaryTablespace: ['temporary_tablespace', 'TEMPORARY_TABLESPACE'],
  profile: ['profile', 'PROFILE'],
  disabled: ['disabled', 'is_disabled'],
  defaultSchema: ['default_schema', 'default_schema_name'],
  canLogin: ['can_login', 'rolcanlogin'],
  isSuperuser: ['is_superuser', 'rolsuper'],
  canCreateDb: ['can_create_db', 'rolcreatedb']
}

const attrsBase = ref({})
const seedAttrs = () => {
  const o = {}
  if (isMysql.value) {
    o.accountLocked = attrYes(attrRaw('account_locked'))
    o.passwordExpired = attrYes(attrRaw('password_expired'))
    o.maxQueriesPerHour = attrNum(attrRaw('max_questions'))
    o.maxUpdatesPerHour = attrNum(attrRaw('max_updates'))
    o.maxConnectionsPerHour = attrNum(attrRaw('max_connections'))
    o.maxUserConnections = attrNum(attrRaw('max_user_connections'))
    o.authPlugin = attrText(attrRaw('plugin'))
  } else if (isOracle.value) {
    // Oracle 的状态是 OPEN / LOCKED / EXPIRED… —— 这里只管"锁没锁"
    o.accountLocked = attrText(attrRaw('account_status')).toUpperCase().indexOf('LOCKED') >= 0
    o.defaultTablespace = attrText(attrRaw('default_tablespace'))
    o.temporaryTablespace = attrText(attrRaw('temporary_tablespace'))
    o.profile = attrText(attrRaw('profile'))
  } else if (isMssql.value) {
    o.disabled = attrYes(attrRaw('disabled'))
    o.defaultSchema = attrText(attrRaw('default_schema'))
  } else if (isPgsql.value) {
    // 新建时默认"可登录" —— 后端 CREATE ROLE 拼的就是 login，界面别显示成"否"让人误会
    o.canLogin = props.editMode ? attrYes(attrRaw('can_login')) : true
    o.isSuperuser = attrYes(attrRaw('is_superuser'))
    o.canCreateDb = attrYes(attrRaw('can_create_db'))
  }
  form.value.attrs = o
  attrsBase.value = { ...o }
}

// 可改项清单。**新建时同样摆出来** —— 用户口径"新建用户的信息也加全"：
// 建号时就该能直接指定账户锁定 / 配额 / 认证插件 / 表空间，而不是建完再进编辑页改一遍。
// 新建时没有"原值"，基准就是各家的默认值（见 seedAttrs），提交时只带改过的那些。
const editableAttrs = computed(() => {
  const defs = []
  if (isMysql.value) {
    defs.push(
      { key: 'accountLocked', label: t('udv.fAccountLocked'), kind: 'bool' },
      { key: 'passwordExpired', label: t('udv.fPwdExpired'), kind: 'bool' },
      { key: 'maxQueriesPerHour', label: t('udv.fMaxQuestions'), kind: 'num' },
      { key: 'maxUpdatesPerHour', label: t('udv.fMaxUpdates'), kind: 'num' },
      { key: 'maxConnectionsPerHour', label: t('udv.fMaxConnections'), kind: 'num' },
      { key: 'maxUserConnections', label: t('udv.fMaxUserConnections'), kind: 'num' },
      { key: 'authPlugin', label: t('udv.fPlugin'), kind: 'plugin' }
    )
  } else if (isOracle.value) {
    defs.push(
      { key: 'accountLocked', label: t('udv.fAccountLocked'), kind: 'bool' },
      { key: 'defaultTablespace', label: t('udv.fTablespace'), kind: 'text' },
      { key: 'temporaryTablespace', label: t('udv.fTempTablespace'), kind: 'text' },
      { key: 'profile', label: t('udv.fProfile'), kind: 'text' }
    )
  } else if (isMssql.value) {
    defs.push({ key: 'defaultSchema', label: t('udv.fSchema'), kind: 'text' })
    // 只有"带登录名"的用户（不是包含数据库用户）才 DISABLE 得了 —— 没有登录名就不摆这个开关
    if (attrText(attrRaw('login_name')).trim()) {
      defs.push({ key: 'disabled', label: t('udv.fDisabled'), kind: 'bool' })
    }
  } else if (isPgsql.value) {
    defs.push(
      { key: 'canLogin', label: t('udv.fCanLogin'), kind: 'bool' },
      { key: 'isSuperuser', label: t('udv.fSuperuser'), kind: 'bool' },
      { key: 'canCreateDb', label: t('udv.fCanCreateDb'), kind: 'bool' }
    )
  }
  return defs
})

// 插件下拉的选项：**当前值一定要在选项里** —— el-select 找不到匹配项时就显示空，
// 看着像"没回显"（实际是库里那个插件名不在我们的候选清单里，比如某个第三方插件）
const pluginOptions = computed(() => {
  const cur = String(form.value.attrs.authPlugin || '').trim()
  return cur && !ATTR_PLUGINS.includes(cur) ? [cur].concat(ATTR_PLUGINS) : ATTR_PLUGINS
})

// 认证插件的绑定：**取不到种子值时直接回落到明细列** —— 不管哪种入口打开的表单
// （右键编辑 / 用户管理页里点编辑），只要 fields 里有 plugin，这一格就不会是空的。
const authPluginModel = computed({
  get: () => String(form.value.attrs.authPlugin || attrText(attrRaw('plugin')) || ''),
  set: (v) => { form.value.attrs.authPlugin = v },
})

// 只交改动过的项
const attrDelta = computed(() => {
  const out = {}
  for (const d of editableAttrs.value) {
    const v = form.value.attrs[d.key]
    if (v === attrsBase.value[d.key]) continue
    out[d.key] = v
  }
  return out
})

// MySQL 的属性子句（预览用；执行的子句在后端 dialect.rs，两边同一套写法）。
// 顺序按 MySQL 的 ALTER USER 语法：auth_option → WITH 资源项 → PASSWORD EXPIRE → ACCOUNT LOCK，
// 顺序错了直接报语法错。
const mysqlAttrClause = () => {
  const a = attrDelta.value
  let s = ''
  if (a.authPlugin) s += ` IDENTIFIED WITH ${a.authPlugin} BY '***'`
  else if (form.value.password) s += " IDENTIFIED BY '***'"
  const quotas = []
  if (a.maxQueriesPerHour !== undefined) quotas.push(`MAX_QUERIES_PER_HOUR ${a.maxQueriesPerHour}`)
  if (a.maxUpdatesPerHour !== undefined) quotas.push(`MAX_UPDATES_PER_HOUR ${a.maxUpdatesPerHour}`)
  if (a.maxConnectionsPerHour !== undefined) quotas.push(`MAX_CONNECTIONS_PER_HOUR ${a.maxConnectionsPerHour}`)
  if (a.maxUserConnections !== undefined) quotas.push(`MAX_USER_CONNECTIONS ${a.maxUserConnections}`)
  if (quotas.length) s += ` WITH ${quotas.join(' ')}`
  if (a.passwordExpired !== undefined) s += a.passwordExpired ? ' PASSWORD EXPIRE' : ' PASSWORD EXPIRE NEVER'
  if (a.accountLocked !== undefined) s += a.accountLocked ? ' ACCOUNT LOCK' : ' ACCOUNT UNLOCK'
  return s
}

// 「基本信息」里跟着数据源类型来的**只读**项（已摆成控件的那些不再重复显示）。
// 权限类列（MySQL 的 t.* 里有几十个 *_PRIV）不在这里列（它们各自有页签）；
// 凭据列与授权文本的屏蔽在 utils/userFields.js 里统一做。
const detailItems = computed(() => userDetailItems(props.editData?.fields, {
  // 已经摆成控件的项不再重复显示；但**取不到值的项除外** ——
  // 那种情况下藏着它就等于整项都看不见了（宁可多显示一行只读值）
  hide: editableAttrs.value
    .filter((d) => d.kind !== 'text' || String(form.value.attrs[d.key] || '').trim())
    .flatMap((d) => ATTR_RAW_KEYS[d.key] || []),
  hideRe: /PRIV$/
}))

const isMysql = computed(() => {
  // 局部名不叫 t：本文件已 import { t } 做翻译，叫 t 会在本作用域遮蔽它
  const type = props.conn?.type
  return type === 'MYSQL' || type === 'MARIADB' || type === 'DORIS'
})
// Doris 会对新建用户自动授予系统库（information_schema/mysql）的只读权限
const isDoris = computed(() => props.conn?.type === 'DORIS')
const isDorisSystemDb = (db) => isDoris.value && ['information_schema', 'mysql'].includes(String(db || '').toLowerCase())
// 库列表已加载且当前授权的库不在其中：MySQL 删库不会清理 mysql.db 的授权行，属于残留授权
const isDbMissing = (db) => {
  const d = String(db || '').trim()
  return !!d && databases.value.length > 0 && !databases.value.includes(d)
}

const isMssql = computed(() => props.conn?.type === 'SQLSERVER')
const isPgsql = computed(() => props.conn?.type === 'POSTGRESQL')
// Oracle：系统权限 / 角色 / 对象权限三层。
// ⚠️ 下面三张清单与后端 `dialect.rs` 的 ORACLE_SYSTEM_PRIVS / ORACLE_ROLES /
// ORACLE_OBJECT_PRIVS **必须一致**：后端会再校验一遍并拒绝清单外的名字，
// 这里只决定界面能勾什么 —— 不一致的表现是"勾了却报不在白名单"。
const isOracle = computed(() => props.conn?.type === 'ORACLE')
const oracleSystemPrivs = [
  'CREATE SESSION', 'CREATE TABLE', 'CREATE ANY TABLE', 'ALTER ANY TABLE', 'DROP ANY TABLE',
  'SELECT ANY TABLE', 'INSERT ANY TABLE', 'UPDATE ANY TABLE', 'DELETE ANY TABLE',
  'CREATE VIEW', 'CREATE ANY VIEW', 'CREATE SEQUENCE', 'CREATE PROCEDURE', 'CREATE ANY PROCEDURE',
  'CREATE TRIGGER', 'CREATE ANY TRIGGER', 'CREATE TYPE', 'CREATE SYNONYM', 'CREATE ANY SYNONYM',
  'CREATE DATABASE LINK', 'CREATE JOB', 'CREATE MATERIALIZED VIEW', 'CREATE ROLE', 'CREATE USER',
  'ALTER USER', 'DROP USER', 'ALTER SESSION', 'UNLIMITED TABLESPACE', 'DEBUG CONNECT SESSION',
  'QUERY REWRITE'
]
const oracleRoles = [
  'CONNECT', 'RESOURCE', 'DBA', 'EXP_FULL_DATABASE', 'IMP_FULL_DATABASE',
  'SELECT_CATALOG_ROLE', 'EXECUTE_CATALOG_ROLE', 'DELETE_CATALOG_ROLE', 'RECOVERY_CATALOG_OWNER',
  'SCHEDULER_ADMIN', 'OEM_MONITOR', 'AQ_ADMINISTRATOR_ROLE'
]
// 表格的列 = 白名单里的权限 ∪ 现有授权里出现的其它名字（一个都不能隐身：
// 服务器返回了哪个名字，界面就必须能看见、能取消，否则保存时会把它悄悄撤掉）
const objectPrivColumns = computed(() => {
  const known = new Set(oracleObjectPrivs)
  const extras = []
  for (const row of form.value.tablePrivileges || []) {
    for (const p of row.privileges || []) {
      const name = String(p || '').trim()
      if (name && !known.has(name) && !extras.includes(name)) extras.push(name)
    }
  }
  return [...oracleObjectPrivs, ...extras]
})
const hasObjectPriv = (row, priv) => (row.privileges || []).includes(priv)
const toggleObjectPriv = (row, priv, checked) => {
  const list = row.privileges || (row.privileges = [])
  const at = list.indexOf(priv)
  if (checked && at < 0) list.push(priv)
  if (!checked && at >= 0) list.splice(at, 1)
}
const oracleObjectPrivs = [
  'SELECT', 'INSERT', 'UPDATE', 'DELETE', 'ALTER', 'INDEX', 'REFERENCES', 'READ',
  'EXECUTE', 'DEBUG', 'FLASHBACK', 'ON COMMIT REFRESH', 'QUERY REWRITE'
]
const isClickhouse = computed(() => props.conn?.type === 'CLICKHOUSE')

// ===== 页签可见性：**按数据库类型出对应界面** =====
// MySQL 系：服务器(全局) / 数据库(库级) / 细粒度(表级)；
// SQL Server：服务器角色 / 数据库角色 / 细粒度(Schema + 对象)；
// PG、Kingbase：角色是集群级的，只有服务器权限这一页。
const hasServerTab = computed(() => isMysql.value || isMssql.value || isPgsql.value || isOracle.value)
const hasDbTab = computed(() => isMysql.value || isMssql.value)
const hasGranularTab = computed(() => isMysql.value || isMssql.value || isOracle.value)
// Oracle 的「角色」是单独一层（与系统权限、对象权限并列）
const hasRolesTab = computed(() => isOracle.value)
// 页签标题按类型取：同一层在不同家的说法不一样（Oracle 的"服务器级"其实就是系统权限）
// 同一层在不同家叫法不同，按数据库类型取**准确**的名字：
//   MySQL/MariaDB/Doris → 全局权限（GLOBAL PRIVILEGES，它没有"服务器"这一层）
//   SQL Server → 服务器角色（它这一层给的就是固定服务器角色）
//   Oracle → 系统权限（SYSTEM PRIVILEGES）
//   PostgreSQL / Kingbase → 角色（集群级只有角色，没有直接的权限授予）
const serverTabLabel = computed(() => (
  isOracle.value ? t('upv.tabSystemPrivs')
    : isMssql.value ? t('upv.tabServerRoles')
      : isPgsql.value ? t('upv.tabRoles')
        : t('upv.tabGlobal')
))
// 数据库那一页：MySQL 系是"库级权限"，SQL Server 给的是数据库角色
const dbTabLabel = computed(() => (isMssql.value ? t('upv.segDbRoles') : t('upv.tabDb')))
const granularTabLabel = computed(() => (
  isOracle.value ? t('upv.tabObjects')
    : isMssql.value ? t('upv.tabSchemaObject')
      : t('upv.tabTables')
))
const hasAnyGrantTab = computed(() => hasServerTab.value || hasDbTab.value)
// 类型名（给"该类型还没实现自动授权"那句提示用）
const connTypeLabel = computed(() => {
  const map = {
    MYSQL: 'MySQL', MARIADB: 'MariaDB', DORIS: 'Apache Doris', POSTGRESQL: 'PostgreSQL',
    KINGBASE: 'KingbaseES', SQLSERVER: 'SQL Server', ORACLE: 'Oracle', DM: '达梦',
    CLICKHOUSE: 'ClickHouse', SQLITE: 'SQLite', REDIS: 'Redis', MONGODB: 'MongoDB', H2: 'H2'
  }
  const key = String(props.conn?.type || '').toUpperCase()
  return map[key] || key || ''
})
const isH2 = computed(() => props.conn?.type === 'H2')
const isDerby = computed(() => props.conn?.type === 'DERBY')

// 全局权限的固定清单 = MySQL 的静态权限全集。
//
// 之前只有 14 项，而库里 `root@%`（GRANT ALL PRIVILEGES ON *.*）有 **28 项** ——
// 差额那十几项在界面上**根本没有勾选框**：看不见、也没法取消。
// 保存是「以表单为准」的，少一项就等于把那一项撤掉，所以清单必须与**
// `information_schema.user_privileges` 会返回的名字**对齐。
// （并且下面 `withExtras` 还会把清单外的名字兜底渲染出来，防止再出现"隐身"的权限。）
const mysqlPrivs = [
  { value: 'ALL PRIVILEGES', labelKey: 'upv.all' },
  { value: 'SELECT', labelKey: 'upv.select' },
  { value: 'INSERT', labelKey: 'upv.insert' },
  { value: 'UPDATE', labelKey: 'upv.update' },
  { value: 'DELETE', labelKey: 'upv.delete' },
  { value: 'CREATE', labelKey: 'upv.create' },
  { value: 'DROP', labelKey: 'upv.drop' },
  { value: 'INDEX', labelKey: 'upv.index' },
  { value: 'ALTER', labelKey: 'upv.alter' },
  { value: 'CREATE VIEW', labelKey: 'upv.createView' },
  { value: 'SHOW VIEW', labelKey: 'upv.showView' },
  { value: 'TRIGGER', labelKey: 'upv.trigger' },
  { value: 'EVENT', labelKey: 'upv.event' },
  { value: 'EXECUTE', labelKey: 'upv.execute' },
  { value: 'REFERENCES', labelKey: 'upv.references' },
  { value: 'LOCK TABLES', labelKey: 'upv.lockTables' },
  { value: 'CREATE TEMPORARY TABLES', labelKey: 'upv.createTemporaryTables' },
  { value: 'CREATE ROUTINE', labelKey: 'upv.createRoutine' },
  { value: 'ALTER ROUTINE', labelKey: 'upv.alterRoutine' },
  { value: 'CREATE USER', labelKey: 'upv.createUser' },
  { value: 'CREATE TABLESPACE', labelKey: 'upv.createTablespace' },
  { value: 'PROCESS', labelKey: 'upv.process' },
  { value: 'SHOW DATABASES', labelKey: 'upv.showDatabases' },
  { value: 'RELOAD', labelKey: 'upv.reload' },
  { value: 'SHUTDOWN', labelKey: 'upv.shutdown' },
  { value: 'SUPER', labelKey: 'upv.super' },
  { value: 'FILE', labelKey: 'upv.file' },
  { value: 'REPLICATION CLIENT', labelKey: 'upv.replicationClient' },
  { value: 'REPLICATION SLAVE', labelKey: 'upv.replicationSlave' },
  { value: 'GRANT OPTION', labelKey: 'upv.grantOption' },
  { value: 'CREATE ROLE', labelKey: 'upv.createRole' },
  { value: 'DROP ROLE', labelKey: 'upv.dropRole' }
]

// 勾选项 = 固定清单 ∪ 数据里出现的其它权限名。
//
// 固定清单负责中文标签与展示顺序，兜底负责"一个都不能隐身"：
// 只要服务器返回了它（换了大版本、MariaDB 加了别的名字、动态权限…），
// 界面就必须能看见、能取消 —— 否则保存时会把它悄悄撤掉。
const withExtras = (base, values) => {
  const known = new Set(base.map((p) => p.value))
  const extras = [...new Set(values || [])].filter((v) => v && !known.has(v))
  // 表里存的是 labelKey（见上方清单），在这里就地取出当前语言的文案；
  // 兜底项是服务器返回的原始权限名，没有译文，直接用名字本身。
  return [
    ...base.map((p) => ({ value: p.value, label: t(p.labelKey) })),
    ...extras.map((v) => ({ value: v, label: v }))
  ]
}
const globalPrivOptions = computed(() => withExtras(mysqlPrivs, form.value.privileges))
const objectPrivOptions = (row) => withExtras(objectPrivs, row?.privileges)

// 权限按用途分组 —— 三十二个勾选框平铺着没人看得下去，按「数据读写 / 对象与结构 /
// 服务器管理」分开后一眼就能定位（这也是各家客户端的老做法）。
//
// 分组**只决定怎么摆**，不改变集合：清单里有、而这里没提到的值会落进最后的「其它」组。
// 少摆一个就等于保存时把它撤掉（这是这一块反复出问题的地方，宁可多一组兜底）。
const privGroupDefs = [
  { key: 'data', labelKey: 'upv.grpData', values: ['SELECT', 'INSERT', 'UPDATE', 'DELETE'] },
  {
    key: 'object',
    labelKey: 'upv.grpObject',
    values: [
      'CREATE', 'ALTER', 'DROP', 'INDEX', 'REFERENCES', 'TRIGGER', 'EVENT',
      'CREATE VIEW', 'SHOW VIEW', 'CREATE ROUTINE', 'ALTER ROUTINE', 'EXECUTE',
      'CREATE TEMPORARY TABLES', 'LOCK TABLES'
    ]
  },
  {
    key: 'admin',
    labelKey: 'upv.grpAdmin',
    values: [
      'PROCESS', 'SHOW DATABASES', 'RELOAD', 'SHUTDOWN', 'SUPER', 'FILE',
      'CREATE USER', 'CREATE TABLESPACE', 'REPLICATION CLIENT', 'REPLICATION SLAVE',
      'CREATE ROLE', 'DROP ROLE', 'GRANT OPTION'
    ]
  }
]
const groupPrivOptions = (base) => {
  const groups = []
  const used = new Set()
  const all = base.find((p) => p.value === 'ALL PRIVILEGES')
  if (all) {
    used.add(all.value)
    groups.push({ key: 'all', label: t('upv.grpQuick'), items: [all] })
  }
  for (const def of privGroupDefs) {
    const items = base.filter((p) => def.values.includes(p.value))
    if (!items.length) continue
    items.forEach((p) => used.add(p.value))
    groups.push({ key: def.key, label: t(def.labelKey), items })
  }
  const rest = base.filter((p) => !used.has(p.value))
  if (rest.length) groups.push({ key: 'other', label: t('upv.grpOther'), items: rest })
  return groups
}
const globalPrivGroups = computed(() =>
  groupPrivOptions(globalPrivOptions.value).map((group) => ({
    ...group,
    total: group.items.length,
    checked: group.items.filter((p) => form.value.privileges.includes(p.value)).length,
  })),
)
const objectPrivGroups = (row) => groupPrivOptions(objectPrivOptions(row))

// 分组级「全选 / 清空」：只动这一组的权限项，别的组不受影响。
// 赋值一个新数组（而不是原地 splice）——`form` 是 ref，换引用才会触发重渲染。
const setGroupPrivileges = (values, checked) => {
  const set = new Set(form.value.privileges)
  values.forEach((value) => (checked ? set.add(value) : set.delete(value)))
  form.value.privileges = [...set]
}

const objectPrivs = [
  { value: 'ALL PRIVILEGES', labelKey: 'upv.all' },
  { value: 'SELECT', labelKey: 'upv.select' },
  { value: 'INSERT', labelKey: 'upv.insert' },
  { value: 'UPDATE', labelKey: 'upv.update' },
  { value: 'DELETE', labelKey: 'upv.delete' },
  { value: 'CREATE', labelKey: 'upv.create' },
  { value: 'DROP', labelKey: 'upv.drop' },
  { value: 'INDEX', labelKey: 'upv.index' },
  { value: 'ALTER', labelKey: 'upv.alter' },
  { value: 'REFERENCES', labelKey: 'upv.references' },
  { value: 'TRIGGER', labelKey: 'upv.trigger' },
  { value: 'EXECUTE', labelKey: 'upv.execute' },
  { value: 'CREATE VIEW', labelKey: 'upv.createView' },
  { value: 'SHOW VIEW', labelKey: 'upv.showView' },
  { value: 'EVENT', labelKey: 'upv.event' },
  { value: 'LOCK TABLES', labelKey: 'upv.lockTables' },
  { value: 'CREATE TEMPORARY TABLES', labelKey: 'upv.createTemporaryTables' },
  { value: 'CREATE ROUTINE', labelKey: 'upv.createRoutine' },
  { value: 'ALTER ROUTINE', labelKey: 'upv.alterRoutine' },
  // 授权能力也是会被撤销的（revoke ... grant option），必须有对应可勾项
  { value: 'GRANT OPTION', labelKey: 'upv.grantOption' }
]

const mssqlRoles = [
  'db_owner', 'db_securityadmin', 'db_accessadmin', 'db_backupoperator',
  'db_ddladmin', 'db_datawriter', 'db_datareader',
  'db_denydatawriter', 'db_denydatareader'
]

const mssqlSchemaPrivs = [
  { value: 'SELECT', labelKey: 'upv.select' },
  { value: 'INSERT', labelKey: 'upv.insert' },
  { value: 'UPDATE', labelKey: 'upv.update' },
  { value: 'DELETE', labelKey: 'upv.delete' },
  { value: 'EXECUTE', labelKey: 'upv.execute' },
  { value: 'REFERENCES', labelKey: 'upv.references' }
]

const mssqlObjectPrivs = [
  { value: 'SELECT', labelKey: 'upv.select' },
  { value: 'INSERT', labelKey: 'upv.insert' },
  { value: 'UPDATE', labelKey: 'upv.update' },
  { value: 'DELETE', labelKey: 'upv.delete' },
  { value: 'EXECUTE', labelKey: 'upv.execute' },
  { value: 'REFERENCES', labelKey: 'upv.references' },
  { value: 'VIEW DEFINITION', labelKey: 'upv.viewDefinition' }
]

const pgRoles = [
  'pg_read_all_data', 'pg_write_all_data', 'pg_read_server_files',
  'pg_write_server_files', 'pg_execute_server_program'
]

// ===== 矩阵表（权限项当列）=====
// 列 = 固定清单 ∪ 数据里出现的名字 —— 与「一个都不能隐身」同一条原则：
// 服务器返回了哪个权限名，界面就必须有一列能看见、能取消（否则保存时会把它悄悄撤掉）。
const privColumns = (base, rows) => {
  const seen = new Set()
  const cols = []
  const push = (v) => {
    const name = String(v || '').trim()
    if (name && !seen.has(name)) { seen.add(name); cols.push(name) }
  }
  ;(base || []).forEach(push)
  ;[...(rows || [])].forEach((r) => (r && r.privileges ? r.privileges : []).forEach(push))
  return cols
}
// MySQL 全局权限：所有分组的并集（分组之间不重叠，但列取并集才能一张表放下）
// 分组写在**表头第一行**（colspan 跨它自己的那几列），所以还要知道每组占了几列 ——
// 按"首次出现"计数，万一清单里有重名也不会把表头撑歪。
const globalPrivColumns = computed(() => {
  const seen = new Set()
  const cols = []
  for (const g of globalPrivGroups.value) {
    for (const p of g.items) {
      if (!seen.has(p.value)) { seen.add(p.value); cols.push(p.value) }
    }
  }
  return cols
})
// ===== 表头分类：把权限名按「这一类是干什么的」分组，写在表头第一行（colspan 跨它那几列）。
// 分组**只决定表头怎么摆**，不改变集合：清单里提到但这里没列到的名字会落进「其它」。
const groupByDefs = (names, defs) => {
  const used = new Set()
  const groups = []
  for (const def of defs) {
    const items = names.filter((n) => def.values.includes(n))
    if (!items.length) continue
    items.forEach((n) => used.add(n))
    groups.push({ key: def.key, label: t(def.labelKey), items })
  }
  const rest = names.filter((n) => !used.has(n))
  if (rest.length) {
    // 定义里已经有一个「其它」时把兜底项并进去 —— 否则表头会并排出现两个「其它」
    // （服务器返回的额外权限名如 USE 会落进兜底那组，真机实测就是这样）。
    const at = groups.findIndex((g) => g.key === 'other')
    if (at >= 0) groups[at] = { ...groups[at], items: [...groups[at].items, ...rest] }
    else groups.push({ key: 'other', label: t('upv.grpOther'), items: rest })
  }
  return groups
}
// MySQL 系（库级/表级与全局同一套清单）：快捷(ALL) + 数据读写 / 对象与结构 / 服务器管理
const mysqlGroupDefs = [{ key: 'all', labelKey: 'upv.grpQuick', values: ['ALL PRIVILEGES'] }, ...privGroupDefs]
// Oracle 对象权限
const oracleObjGroupDefs = [
  { key: 'data', labelKey: 'upv.grpData', values: ['SELECT', 'INSERT', 'UPDATE', 'DELETE'] },
  { key: 'object', labelKey: 'upv.grpObject', values: ['ALTER', 'INDEX', 'REFERENCES'] },
  { key: 'exec', labelKey: 'upv.grpExec', values: ['EXECUTE', 'DEBUG'] },
  { key: 'other', labelKey: 'upv.grpOther', values: ['READ', 'FLASHBACK', 'ON COMMIT REFRESH', 'QUERY REWRITE'] }
]
// Oracle 系统权限
const oracleSysGroupDefs = [
  { key: 'session', labelKey: 'upv.grpSession', values: ['CREATE SESSION', 'ALTER SESSION'] },
  {
    key: 'data',
    labelKey: 'upv.grpData',
    values: ['SELECT ANY TABLE', 'INSERT ANY TABLE', 'UPDATE ANY TABLE', 'DELETE ANY TABLE']
  },
  {
    key: 'object',
    labelKey: 'upv.grpObject',
    values: [
      'CREATE TABLE', 'CREATE ANY TABLE', 'ALTER ANY TABLE', 'DROP ANY TABLE',
      'CREATE VIEW', 'CREATE ANY VIEW', 'CREATE SEQUENCE',
      'CREATE PROCEDURE', 'CREATE ANY PROCEDURE', 'CREATE TRIGGER', 'CREATE ANY TRIGGER',
      'CREATE TYPE', 'CREATE SYNONYM', 'CREATE ANY SYNONYM', 'CREATE MATERIALIZED VIEW'
    ]
  },
  {
    key: 'admin',
    labelKey: 'upv.grpAdmin',
    values: [
      'CREATE ROLE', 'CREATE USER', 'ALTER USER', 'DROP USER', 'UNLIMITED TABLESPACE',
      'CREATE DATABASE LINK', 'CREATE JOB', 'DEBUG CONNECT SESSION'
    ]
  },
  { key: 'other', labelKey: 'upv.grpOther', values: ['QUERY REWRITE'] }
]
// Oracle 角色
const oracleRoleGroupDefs = [
  { key: 'common', labelKey: 'upv.grpCommon', values: ['CONNECT', 'RESOURCE', 'DBA'] },
  {
    key: 'catalog',
    labelKey: 'upv.grpCatalog',
    values: ['SELECT_CATALOG_ROLE', 'EXECUTE_CATALOG_ROLE', 'DELETE_CATALOG_ROLE', 'RECOVERY_CATALOG_OWNER']
  },
  { key: 'impexp', labelKey: 'upv.grpImpExp', values: ['EXP_FULL_DATABASE', 'IMP_FULL_DATABASE'] },
  { key: 'admin', labelKey: 'upv.grpAdmin', values: ['SCHEDULER_ADMIN', 'OEM_MONITOR', 'AQ_ADMINISTRATOR_ROLE'] }
]
// SQL Server 服务器角色 / 数据库角色
const mssqlServerRoleGroupDefs = [
  { key: 'admin', labelKey: 'upv.grpAdmin', values: ['sysadmin', 'securityadmin', 'serveradmin', 'setupadmin'] },
  { key: 'object', labelKey: 'upv.grpObject', values: ['dbcreator', 'diskadmin', 'processadmin', 'bulkadmin'] }
]
const mssqlDbRoleGroupDefs = [
  { key: 'data', labelKey: 'upv.grpData', values: ['db_owner', 'db_datawriter', 'db_datareader'] },
  {
    key: 'object',
    labelKey: 'upv.grpObject',
    values: ['db_ddladmin', 'db_securityadmin', 'db_accessadmin', 'db_backupoperator']
  },
  { key: 'other', labelKey: 'upv.grpOther', values: ['db_denydatawriter', 'db_denydatareader'] }
]
// SQL Server Schema 级 / 对象级权限
const mssqlPrivGroupDefs = [
  { key: 'data', labelKey: 'upv.grpData', values: ['SELECT', 'INSERT', 'UPDATE', 'DELETE'] },
  { key: 'exec', labelKey: 'upv.grpExec', values: ['EXECUTE', 'REFERENCES', 'VIEW DEFINITION'] }
]
// PostgreSQL 角色
const pgRoleGroupDefs = [
  { key: 'data', labelKey: 'upv.grpData', values: ['pg_read_all_data', 'pg_write_all_data'] },
  {
    key: 'admin',
    labelKey: 'upv.grpAdmin',
    values: ['pg_read_server_files', 'pg_write_server_files', 'pg_execute_server_program']
  }
]

const globalGrouped = computed(() => groupByDefs(globalPrivColumns.value, mysqlGroupDefs))
const dbGrouped = computed(() => groupByDefs(dbPrivColumns.value, mysqlGroupDefs))
const tableGrouped = computed(() => groupByDefs(tablePrivColumns.value, mysqlGroupDefs))
const serverGrouped = computed(() =>
  groupByDefs(
    serverColumns.value,
    isOracle.value ? oracleSysGroupDefs : isMssql.value ? mssqlServerRoleGroupDefs : pgRoleGroupDefs,
  ),
)
const rolesGrouped = computed(() => groupByDefs(oracleRoles, oracleRoleGroupDefs))
const mssqlDbRolesGrouped = computed(() => groupByDefs(mssqlRoles, mssqlDbRoleGroupDefs))
const objectGrouped = computed(() => groupByDefs(objectPrivColumns.value, oracleObjGroupDefs))
const schemaGrouped = computed(() => groupByDefs(schemaPrivColumns.value, mssqlPrivGroupDefs))
const objectGroupedMssql = computed(() => groupByDefs(objectPrivColumnsMssql.value, mssqlPrivGroupDefs))

// 分类表头里的「已勾 / 共几项」与「整组全选 / 清空」（只对"权限存在一个数组里"的那几页用；
// 库级/表级/对象级是**一行一个对象**，每组数量随行而变，那里只写分类名）
const countIn = (key, values) => (form.value[key] || []).filter((v) => values.includes(v)).length
// 行级页签（库 / 表 / Schema / 对象）是"一行一个对象"，所以分类表头里的计数按**格子**算：
//   分子 = 已勾的格子数（这些行里勾了多少个该组权限），分母 = 行数 × 该组权限数。
// 「全选 / 清空」作用于**所有行** —— 与网格里的勾选是同一件事，只是批量做。
const rowsCount = (rows, values) => {
  let n = 0
  for (const r of rows || []) {
    for (const v of values || []) {
      if ((r.privileges || []).includes(v)) n += 1
    }
  }
  return n
}
const rowsTotal = (rows, values) => (rows || []).length * (values || []).length
const setRowsPrivileges = (rows, values, checked) => {
  for (const r of rows || []) {
    const set = new Set(r.privileges || [])
    values.forEach((v) => (checked ? set.add(v) : set.delete(v)))
    r.privileges = [...set]
  }
}
const setList = (key, values, checked) => {
  const set = new Set(form.value[key] || [])
  values.forEach((v) => (checked ? set.add(v) : set.delete(v)))
  form.value[key] = [...set]
}
// 库级 / 表级（MySQL 系）
const dbPrivColumns = computed(() => privColumns(objectPrivs.map((p) => p.value), form.value.dbPrivileges))
const tablePrivColumns = computed(() => privColumns(objectPrivs.map((p) => p.value), form.value.tablePrivileges))
// SQL Server 的 Schema 级 / 对象级
const schemaPrivColumns = computed(() => privColumns(mssqlSchemaPrivs.map((p) => p.value), form.value.schemaPrivileges))
const objectPrivColumnsMssql = computed(() => privColumns(mssqlObjectPrivs.map((p) => p.value), form.value.objectPrivileges))
// 服务器权限那一页：勾选项本身就是「角色名 / 权限名」，一个一列，只有一行
const serverColumns = computed(() => (
  isOracle.value ? oracleSystemPrivs : isMssql.value ? mssqlServerRoles : isPgsql.value ? pgRoles : []
))
const serverRowLabel = computed(() => (
  isOracle.value ? t('upv.tabSystemPrivs') : isMssql.value ? t('upv.segServer') : t('upv.roleAssignPg')
))
// 这一页写的是 form 里的哪个数组（Oracle 系统权限 → privileges，SQL Server → serverRoles，PG → roles）
const serverModelKey = computed(() => (isMssql.value ? 'serverRoles' : isPgsql.value ? 'roles' : 'privileges'))
const hasIn = (key, value) => (form.value[key] || []).includes(value)
// 换新数组而不是原地改 —— form 是 ref，换引用才会触发重渲染（本文件多处踩过）
const toggleList = (key, value, checked) => {
  const set = new Set(form.value[key] || [])
  if (checked) set.add(value); else set.delete(value)
  form.value[key] = [...set]
}

const previewOk = computed(() => {
  if (!form.value.name.trim()) return false
  if (!props.editMode && !form.value.password) return false
  return true
})

// ===== 「只出差量」的授权 =====
// 打开表单时服务端给的那份授权存进 orig，之后拿当前表单逐项比对：
//   · 勾选了一个原本没有的   → 只出这一条的 GRANT
//   · 取消勾选了一个原本有的 → 只出这一条的 REVOKE
// 不再"先 REVOKE ALL PRIVILEGES, GRANT OPTION 再按表单全量重授"（用户要求：授权机制不要全删）。
// ⚠️ 同一份差量既用来**预览**、也交给**后端执行** —— 两处必须同源，
//    否则"看到的"和"真正执行的"会对不上（这是这一块最容易骗到人的地方）。
const orig = ref({ privileges: [], dbPrivileges: [], tablePrivileges: [], serverRoles: [], roles: [], schemaPrivileges: [], objectPrivileges: [], grants: [] })
const asSet = (arr) => new Set((arr || []).filter(Boolean))
const listDiff = (before, after) => {
  const b = asSet(before)
  const a = asSet(after)
  return { added: [...a].filter((x) => !b.has(x)), removed: [...b].filter((x) => !a.has(x)) }
}
// 行级（库 / 表 / 模式 / 对象）：按"行键"配对，逐行算差；
// 行键变了（比如把某行的库名改了）就当成"旧行整条撤销 + 新行整条授予"。
const rowsDiff = (before, after, keyOf) => {
  const out = { added: [], removed: [] }
  const bmap = new Map((before || []).filter((r) => keyOf(r)).map((r) => [keyOf(r), r]))
  const amap = new Map((after || []).filter((r) => keyOf(r)).map((r) => [keyOf(r), r]))
  for (const [k, a] of amap) {
    const d = listDiff(bmap.get(k) ? bmap.get(k).privileges : [], a.privileges)
    if (d.added.length) out.added.push({ key: k, row: a, privs: d.added })
  }
  for (const [k, b] of bmap) {
    const d = listDiff(b.privileges, amap.get(k) ? amap.get(k).privileges : [])
    if (d.removed.length) out.removed.push({ key: k, row: b, privs: d.removed })
  }
  return out
}
const dbKey = (r) => (r && r.database ? String(r.database) : '')
const tableKey = (r) => (r && r.database && r.table ? r.database + '.' + r.table : '')
const schemaKey = (r) => (r && r.schema ? String(r.schema) : '')
const objectKey = (r) => (r && r.schema && r.object ? r.schema + '.' + r.object : '')
const allRows = (rows, withKey) => (rows || [])
  .filter((r) => withKey(r) && r.privileges && r.privileges.length)
  .map((r) => ({ key: withKey(r), row: r, privs: [...r.privileges] }))

const changes = computed(() => {
  const o = orig.value
  const f = form.value
  // 新建用户：没有"原状态"，勾了什么就授什么
  if (!props.editMode) {
    return {
      global: { added: [...asSet(f.privileges)], removed: [] },
      serverRoles: { added: [...asSet(f.serverRoles)], removed: [] },
      roles: { added: [...asSet(f.roles)], removed: [] },
      db: { added: allRows(f.dbPrivileges, dbKey), removed: [] },
      table: { added: allRows(f.tablePrivileges, tableKey), removed: [] },
      schema: { added: allRows(f.schemaPrivileges, schemaKey), removed: [] },
      object: { added: allRows(f.objectPrivileges, objectKey), removed: [] }
    }
  }
  return {
    global: listDiff(o.privileges, f.privileges),
    serverRoles: listDiff(o.serverRoles, f.serverRoles),
    roles: listDiff(o.roles, f.roles),
    db: rowsDiff(o.dbPrivileges, f.dbPrivileges, dbKey),
    table: rowsDiff(o.tablePrivileges, f.tablePrivileges, tableKey),
    schema: rowsDiff(o.schemaPrivileges, f.schemaPrivileges, schemaKey),
    object: rowsDiff(o.objectPrivileges, f.objectPrivileges, objectKey)
  }
})
// 差量里有没有"要执行的语句"（撤销也算）
const hasChanges = computed(() => {
  const c = changes.value
  return ['global', 'serverRoles', 'roles', 'db', 'table', 'schema', 'object'].some((k) => {
    const g = c[k]
    return (g.added && g.added.length) || (g.removed && g.removed.length)
  }) || !!form.value.password || !props.editMode
})
// 交给后端的撤销清单（scope 与 Oracle 那套同一形状：owner/object 为空的按目标层级理解）
const revokePayload = computed(() => {
  const c = changes.value
  const out = []
  for (const p of c.global.removed) out.push({ scope: 'global', privilege: p, owner: '', object: '' })
  for (const p of c.serverRoles.removed) out.push({ scope: 'server-role', privilege: p, owner: '', object: '' })
  for (const p of c.roles.removed) out.push({ scope: 'role', privilege: p, owner: '', object: '' })
  for (const r of c.db.removed) for (const p of r.privs) out.push({ scope: 'db', privilege: p, owner: r.key, object: '' })
  for (const r of c.table.removed) for (const p of r.privs) out.push({ scope: 'table', privilege: p, owner: r.row.database, object: r.row.table })
  for (const r of c.schema.removed) for (const p of r.privs) out.push({ scope: 'schema', privilege: p, owner: r.key, object: '' })
  for (const r of c.object.removed) for (const p of r.privs) out.push({ scope: 'object', privilege: p, owner: r.row.schema, object: r.row.object })
  return out
})

const previewSql = computed(() => {
  if (!previewOk.value) return ''
  const action = props.editMode ? 'ALTER' : 'CREATE'
  let sql = ''
  if (isMysql.value) {
    const h = form.value.host || '%'
    // 没填密码就不要拼 `ALTER USER 'x'@'%';` —— MySQL 里这是语法错误。
    // 「只改权限、不改密码」是常见操作，那时本来就只该有 revoke/grant。
    const who = `'${form.value.name}'@'${h}'`
    const lines = []
    if (!props.editMode) {
      lines.push(`CREATE USER ${who}${form.value.password ? " IDENTIFIED BY '***'" : ''};`)
    } else {
      // 属性（插件 / 配额 / 密码过期 / 账户锁定）与改密合成一条 ALTER USER —— 见 mysqlAttrClause
      const clause = mysqlAttrClause()
      if (clause) lines.push(`ALTER USER ${who}${clause};`)
    }
    const c = changes.value
    // 先撤销（只撤"取消了勾选"的那些），再授予（只授"新勾上"的那些）
    const grantOption = (privs) => privs.filter((p) => p !== 'GRANT OPTION')
    const hasGrantOption = (privs) => privs.includes('GRANT OPTION')
    for (const p of c.global.removed) lines.push(`REVOKE ${p} ON *.* FROM ${who};`)
    for (const r of c.db.removed) for (const p of r.privs) lines.push(`REVOKE ${p} ON \`${r.key}\`.* FROM ${who};`)
    for (const r of c.table.removed) for (const p of r.privs) lines.push(`REVOKE ${p} ON \`${r.row.database}\`.\`${r.row.table}\` FROM ${who};`)
    const g = grantOption(c.global.added)
    if (g.length) lines.push(`GRANT ${g.join(', ')} ON *.* TO ${who}${hasGrantOption(c.global.added) ? ' WITH GRANT OPTION' : ''};`)
    else if (hasGrantOption(c.global.added)) lines.push(`GRANT USAGE ON *.* TO ${who} WITH GRANT OPTION;`)
    for (const r of c.db.added) lines.push(`GRANT ${r.privs.join(', ')} ON \`${r.key}\`.* TO ${who};`)
    for (const r of c.table.added) lines.push(`GRANT ${r.privs.join(', ')} ON \`${r.row.database}\`.\`${r.row.table}\` TO ${who};`)
    sql = lines.join('\n')
  } else if (isMssql.value) {
    if (!props.editMode && form.value.password) {
      sql = `CREATE LOGIN [${form.value.name}] WITH PASSWORD='***';\nCREATE USER [${form.value.name}] FOR LOGIN [${form.value.name}];`
    } else if (props.editMode && form.value.password) {
      sql = `ALTER LOGIN [${form.value.name}] WITH PASSWORD='***';`
    }
    const c = changes.value
    const u = `[${form.value.name}]`
    // 属性：默认架构；禁用走 ALTER LOGIN（要有登录名，包含数据库用户没有）
    const a = attrDelta.value
    if (a.defaultSchema !== undefined) sql += `\nALTER USER ${u} WITH DEFAULT_SCHEMA = [${a.defaultSchema}];`
    if (a.disabled !== undefined) {
      const login = attrText(attrRaw('login_name')).trim()
      if (login) sql += `\nALTER LOGIN [${login}] ${a.disabled ? 'DISABLE' : 'ENABLE'};`
    }
    for (const r of c.serverRoles.removed) sql += `\nALTER SERVER ROLE [${r}] DROP MEMBER ${u};`
    for (const r of c.serverRoles.added) sql += `\nALTER SERVER ROLE [${r}] ADD MEMBER ${u};`
    for (const r of c.roles.removed) sql += `\nALTER ROLE [${r}] DROP MEMBER ${u};`
    for (const r of c.roles.added) sql += `\nALTER ROLE [${r}] ADD MEMBER ${u};`
    for (const r of c.schema.removed) for (const p of r.privs) sql += `\nREVOKE ${p} ON SCHEMA::[${r.key}] FROM ${u};`
    for (const r of c.schema.added) sql += `\nGRANT ${r.privs.join(', ')} ON SCHEMA::[${r.key}] TO ${u};`
    for (const r of c.object.removed) for (const p of r.privs) sql += `\nREVOKE ${p} ON [${r.row.schema}].[${r.row.object}] FROM ${u};`
    for (const r of c.object.added) sql += `\nGRANT ${r.privs.join(', ')} ON [${r.row.schema}].[${r.row.object}] TO ${u};`
  } else if (isPgsql.value) {
    if (!props.editMode) {
      sql = `CREATE USER "${form.value.name}"`
      if (form.value.password) sql += ` WITH PASSWORD '***'`
      sql += ';'
    } else if (form.value.password) {
      sql = `ALTER USER "${form.value.name}" WITH PASSWORD '***';`
    }
    const c = changes.value
    const who = `"${form.value.name}"`
    const flags = []
    const a = attrDelta.value
    if (a.canLogin !== undefined) flags.push(a.canLogin ? 'LOGIN' : 'NOLOGIN')
    if (a.isSuperuser !== undefined) flags.push(a.isSuperuser ? 'SUPERUSER' : 'NOSUPERUSER')
    if (a.canCreateDb !== undefined) flags.push(a.canCreateDb ? 'CREATEDB' : 'NOCREATEDB')
    if (flags.length) sql += `\nALTER ROLE ${who} ${flags.join(' ')};`
    for (const r of c.roles.removed) sql += `\nREVOKE "${r}" FROM ${who};`
    for (const r of c.roles.added) sql += `\nGRANT "${r}" TO ${who};`
  } else if (isClickhouse.value) {
    if (!props.editMode) {
      sql = `CREATE USER \`${form.value.name}\``
      if (form.value.password) sql += ` IDENTIFIED BY '***'`
      sql += ';'
    } else if (form.value.password) {
      sql = `ALTER USER \`${form.value.name}\` IDENTIFIED BY '***';`
    }
    if (form.value.grants?.length) {
      for (const g of form.value.grants) {
        if (g && !g.startsWith('--')) sql += `\nGRANT ${g} TO \`${form.value.name}\`;`
      }
    }
  } else if (isOracle.value) {
    // Oracle：建号 / 改密 + 三层（系统权限 / 角色 / 对象权限）+ 撤销差异。
    // 与 MySQL 那段同一写法（本地预览，口令一律显示 ***）。
    const who = `"${form.value.name}"`
    const lines = []
    if (!props.editMode) {
      lines.push(`CREATE USER ${who} IDENTIFIED BY "***";`)
      lines.push(`GRANT CREATE SESSION TO ${who};`)
    } else if (form.value.password) {
      lines.push(`ALTER USER ${who} IDENTIFIED BY "***";`)
    }
    // 属性：锁定 / 默认表空间 / 临时表空间 / 概要文件（一次改一项最稳）
    const oa = attrDelta.value
    if (oa.accountLocked !== undefined) lines.push(`ALTER USER ${who} ${oa.accountLocked ? 'ACCOUNT LOCK' : 'ACCOUNT UNLOCK'};`)
    if (oa.defaultTablespace !== undefined) lines.push(`ALTER USER ${who} DEFAULT TABLESPACE ${oa.defaultTablespace};`)
    if (oa.temporaryTablespace !== undefined) lines.push(`ALTER USER ${who} TEMPORARY TABLESPACE ${oa.temporaryTablespace};`)
    if (oa.profile !== undefined) lines.push(`ALTER USER ${who} PROFILE ${oa.profile};`)
    for (const r of oracleRevokes()) {
      lines.push(r.scope === 'object'
        ? `REVOKE ${r.privilege} ON "${r.owner}"."${r.object}" FROM ${who};`
        : `REVOKE ${r.privilege} FROM ${who};`)
    }
    for (const p of form.value.privileges || []) lines.push(`GRANT ${p} TO ${who};`)
    for (const r of form.value.roles || []) lines.push(`GRANT ${r} TO ${who};`)
    for (const row of form.value.tablePrivileges || []) {
      if (row.database && row.table && row.privileges?.length) {
        lines.push(`GRANT ${row.privileges.join(', ')} ON "${row.database}"."${row.table}" TO ${who};`)
      }
    }
    sql = lines.join('\n')
  } else if (isH2.value) {
    if (!props.editMode) {
      sql = `CREATE USER ${form.value.name} PASSWORD '***';`
    } else if (form.value.password) {
      sql = `ALTER USER ${form.value.name} SET PASSWORD '***';`
    }
  } else if (isDerby.value) {
    if (!props.editMode) {
      sql = `CALL SYSCS_UTIL.SYSCS_CREATE_USER('${form.value.name}', '***');`
    } else if (form.value.password) {
      sql = `CALL SYSCS_UTIL.SYSCS_RESET_PASSWORD('${form.value.name}', '***');`
    }
  } else {
    sql = t('upv.unsupportedType')
  }
  return sql
})

const copySql = async () => {
  if (!previewSql.value) return
  try {
    await navigator.clipboard.writeText(previewSql.value)
    ElMessage.success(t('upv.sqlCopied'))
  } catch { ElMessage.warning(t('sqlq.copyFailed')) }
}

const addDbPriv = () => {
  form.value.dbPrivileges.push({ database: '', privileges: [] })
}
const removeDbPriv = (i) => {
  form.value.dbPrivileges.splice(i, 1)
}

const addTablePriv = () => {
  form.value.tablePrivileges.push({ database: '', table: '', privileges: [], _tables: [], _loading: false })
}
const removeTablePriv = (i) => {
  form.value.tablePrivileges.splice(i, 1)
}

const onTablePrivDbChange = async (i) => {
  const row = form.value.tablePrivileges[i]
  row.table = ''
  row._tables = []
  if (!row.database || !props.conn) return
  row._loading = true
  try {
    const res = await listTables(props.conn.id, row.database)
    row._tables = (res || []).map(t => t.name || t)
  } catch {
    row._tables = []
  } finally {
    row._loading = false
  }
}

const addSchemaPriv = () => {
  form.value.schemaPrivileges.push({ schema: '', privileges: [] })
}
const removeSchemaPriv = (i) => {
  form.value.schemaPrivileges.splice(i, 1)
}

const addObjectPriv = () => {
  form.value.objectPrivileges.push({ schema: '', object: '', privileges: [] })
}
const removeObjectPriv = (i) => {
  form.value.objectPrivileges.splice(i, 1)
}

const loadDatabases = async () => {
  if (!props.conn) return
  if (isMysql.value) {
    try {
      const res = await listDatabases(props.conn.id)
      databases.value = (res || []).map(d => d.name || d)
    } catch {
      databases.value = []
    }
  }
  if (isMssql.value) {
    await loadSchemas()
  }
}

const loadSchemas = async () => {
  if (!props.conn || !isMssql.value) return
  try {
    const res = await listSchemas(props.conn.id, props.database)
    schemas.value = (res || []).map(s => s.name || s)
  } catch {
    schemas.value = []
  }
}

/**
 * 算「要撤销的授权」：界面里**原来有、现在没有了**的那些。
 *
 * 为什么是界面来算：打开表单时已经读过一遍现有授权（`editData.oracleRows`），
 * 用户在这份状态上增删勾选，差异只有界面知道；后端拿到的只是"最终状态"。
 * 逐条点名是因为 Oracle 没有 `revoke all privileges from u` 这种一把全撤。
 */
const oracleRevokes = () => {
  const all = (props.editData && props.editData.oracleRows) || []
  if (!Array.isArray(all) || all.length === 0) return []
  const up = (v) => String(v || '').trim().toUpperCase()
  const desired = new Set()
  for (const p of form.value.privileges || []) desired.add('global|' + up(p))
  for (const r of form.value.roles || []) desired.add('role|' + up(r))
  for (const row of form.value.tablePrivileges || []) {
    for (const p of row.privileges || []) {
      desired.add('object|' + up(row.database) + '|' + up(row.table) + '|' + up(p))
    }
  }
  return all
    .filter((r) => r && r.privilege)
    .filter((r) => {
      const key = r.scope === 'object'
        ? 'object|' + up(r.owner) + '|' + up(r.object) + '|' + up(r.privilege)
        : String(r.scope) + '|' + up(r.privilege)
      return !desired.has(key)
    })
    .map((r) => ({ scope: r.scope, privilege: r.privilege, owner: r.owner || '', object: r.object || '' }))
}

const submit = async () => {
  if (!previewOk.value) return
  saving.value = true
  try {
    const opts = {}
    if (isMysql.value) {
      // 撤销清单与预览同源：只交"取消了勾选"的那些，后端不再 revoke all
      opts.revoke = revokePayload.value
      opts.privileges = form.value.privileges
      opts.dbPrivileges = form.value.dbPrivileges.map(r => ({ database: r.database, privileges: r.privileges }))
      opts.tablePrivileges = form.value.tablePrivileges.map(r => ({ database: r.database, table: r.table, privileges: r.privileges }))
    }
    if (isMssql.value) {
      opts.serverRoles = form.value.serverRoles
      opts.roles = form.value.roles
      opts.schemaPrivileges = form.value.schemaPrivileges.map(r => ({ schema: r.schema, privileges: r.privileges }))
      opts.objectPrivileges = form.value.objectPrivileges.map(r => ({ schema: r.schema, object: r.object, privileges: r.privileges }))
      // 撤销：**按差量点名**（"原来有、现在没有了"的那些）—— 与预览同源。
      // 不再"先全删再重授"：那样会把用户别处拿到的授权也一并抹掉再重建。
      const mc = changes.value
      opts.revoke = [
        ...mc.serverRoles.removed.map((r) => ({ scope: 'serverrole', privilege: r, owner: '', object: '' })),
        ...mc.roles.removed.map((r) => ({ scope: 'role', privilege: r, owner: '', object: '' })),
        ...mc.schema.removed.flatMap((r) => r.privs.map((p) => ({ scope: 'schema', privilege: p, owner: r.key, object: '' }))),
        ...mc.object.removed.flatMap((r) => r.privs.map((p) => ({ scope: 'object', privilege: p, owner: r.row.schema, object: r.row.object }))),
      ]
    }
    if (isPgsql.value) {
      opts.roles = form.value.roles
    }
    if (isClickhouse.value) {
      opts.grants = form.value.grants || []
    }
    if (isOracle.value) {
      // 三层一起交：系统权限（privileges）/ 角色（roles）/ 对象权限（tablePrivileges，
      // 这里的 database 字段对 Oracle 就是**模式名**）。
      opts.privileges = form.value.privileges
      opts.roles = form.value.roles
      opts.tablePrivileges = form.value.tablePrivileges.map(r => ({ database: r.database, table: r.table, privileges: r.privileges }))
      // Oracle 没有"一把全撤"（不像 MySQL 的 revoke all privileges），所以由这里把
      // **原来有、现在没有了**的那些算出来交给后端逐条 revoke。
      opts.revoke = oracleRevokes()
    }
    // 属性（配额 / 锁定 / 表空间 / 默认架构 / rol* 开关）：只交改动过的项
    const attrs = { ...attrDelta.value }
    if (Object.keys(attrs).length) {
      if (attrs.authPlugin && !form.value.password && props.editMode) {
        // 改插件却不给口令：MySQL 的 `IDENTIFIED WITH x`（不带 BY）会把口令置空 —— 那是把人锁在门外
        ElMessage.warning(t('upv.pluginNeedsPassword'))
        return
      }
      if (attrs.disabled !== undefined) attrs.loginName = attrText(attrRaw('login_name'))
      opts.attrs = attrs
    }
    const res = await userAction(props.conn.id, {
      action: props.editMode ? 'alter' : 'create',
      database: props.database,
      userName: form.value.name.trim(),
      password: form.value.password,
      host: form.value.host || '%',
      options: opts
    })
    if (res.success) {
      ElMessage.success(props.editMode ? t('upv.userUpdated') : t('upv.userCreated'))
      emit('saved', props.database)
      emit('close')
    } else {
      ElMessage.error(res.message || t('upv.opFailed'))
    }
  } catch (e) {
    ElMessage.error(t('upv.opFailedDetail', { detail: (e.message || e) }))
  } finally {
    saving.value = false
  }
}

// ===== 改动检测：打开表单时 SQL 预览**不展示**，动过任何一处才出现 =====
// 快照只取"用户能改的那些字段"，并按值排序 —— 否则勾选顺序不同会被当成改动。
// 打开时（editData 的 watch，immediate）存一份基准，之后比对即知有没有动过。
const formSnapshot = () => JSON.stringify({
  name: form.value.name,
  password: form.value.password,
  host: form.value.host,
  privileges: [...(form.value.privileges || [])].sort(),
  roles: [...(form.value.roles || [])].sort(),
  serverRoles: [...(form.value.serverRoles || [])].sort(),
  dbPrivileges: (form.value.dbPrivileges || []).map((r) => ({
    database: r.database, privileges: [...(r.privileges || [])].sort(),
  })),
  tablePrivileges: (form.value.tablePrivileges || []).map((r) => ({
    database: r.database, table: r.table, privileges: [...(r.privileges || [])].sort(),
  })),
  schemaPrivileges: (form.value.schemaPrivileges || []).map((r) => ({
    schema: r.schema, privileges: [...(r.privileges || [])].sort(),
  })),
  objectPrivileges: (form.value.objectPrivileges || []).map((r) => ({
    schema: r.schema, object: r.object, privileges: [...(r.privileges || [])].sort(),
  })),
  attrs: form.value.attrs,
})
const baseSnapshot = ref('')
const dirty = computed(() => formSnapshot() !== baseSnapshot.value)

watch(() => props.editData, (d) => {
  if (d && d.name) {
    form.value.name = d.name
    form.value.host = d.host || '%'
    form.value.password = ''
    form.value.privileges = d.privileges || []
    form.value.roles = d.roles || []
    form.value.dbPrivileges = (d.dbPrivileges || []).map(r => ({ ...r }))
    form.value.tablePrivileges = (d.tablePrivileges || []).map(r => ({ ...r, _tables: [], _loading: false }))
    form.value.serverRoles = d.serverRoles || []
    form.value.schemaPrivileges = (d.schemaPrivileges || []).map(r => ({ ...r }))
    form.value.objectPrivileges = (d.objectPrivileges || []).map(r => ({ ...r }))
    form.value.grants = d.grants || []
  } else {
    form.value = { name: '', password: '', host: '%', privileges: [], roles: [], dbPrivileges: [], tablePrivileges: [], serverRoles: [], schemaPrivileges: [], objectPrivileges: [], grants: [], attrs: {} }
  }
  // 属性基准（可改项的"原值"）：与权限基准同一时刻存下，之后按差提交
  seedAttrs()
  // 刚打开（或切换用户）时存基准与原状态：此刻"没动过"，预览不展示 SQL 文本；
  // 之后所有 GRANT/REVOKE 都按"与这份原状态的差"来出（见 changes 计算）。
  orig.value = JSON.parse(JSON.stringify({
    privileges: form.value.privileges,
    dbPrivileges: form.value.dbPrivileges,
    tablePrivileges: form.value.tablePrivileges,
    serverRoles: form.value.serverRoles,
    roles: form.value.roles,
    schemaPrivileges: form.value.schemaPrivileges,
    objectPrivileges: form.value.objectPrivileges,
    grants: form.value.grants
  }))
  baseSnapshot.value = formSnapshot()
}, { immediate: true })

// fields 晚到时补种一次（attrsBase 已种过就不再动 —— 免得覆盖用户正在改的值）
watch(() => props.editData?.fields, (f) => {
  if (!f || !Object.keys(f).length) return
  if (Object.keys(attrsBase.value).length) return
  seedAttrs()
})

watch(() => props.conn?.id, loadDatabases, { immediate: true })
watch(() => props.database, loadSchemas, { immediate: true })
// ===== 底部 SQL 预览的折叠 / 拖拽（与「编辑表结构」同一套手感）=====
const SQL_H_DEFAULT = 380
// 最低 = **0**：用户要求"最低可以再低点"，于是允许把整个预览拖走（只留下面那条 6px 分隔条）。
// 不会找不回来：分隔条一直在（悬停变主色、有 tooltip"双击最大/最小"），往上拖即可还原；
// 另外点标题行的折叠态（.collapsed）也仍然保留。
const SQL_H_MIN = 0
// 上方表单**不预留**高度：往上拖可以把预览拉到顶（0 = 允许压到没有）。
// 表单区自己有 overflow: auto，压扁了也不会把内容挤丢。
const FORM_MIN = 0
const sqlAreaRef = ref(null)
const sqlCollapsed = ref(false)
const sqlHeight = ref(SQL_H_DEFAULT)
const sqlMaxH = () => {
  const parent = sqlAreaRef.value?.parentElement
  const available = (parent ? parent.clientHeight : Math.round(window.innerHeight * 0.7)) - FORM_MIN
  return Math.max(SQL_H_MIN, available)
}
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
const onSqlResizeDblClick = () => {
  const max = sqlMaxH()
  sqlHeight.value = sqlHeight.value >= max - 8 ? SQL_H_MIN : max
}
onBeforeUnmount(onSqlResizeEnd)

// 页签在类型之间切换时可能变得不可见（如从 MySQL 切到 Oracle）：拉回「基本信息」，
// 否则 el-tabs 会停在空内容上（看着像页面坏了）
watch([hasServerTab, hasDbTab, hasGranularTab], () => {
  const ok = ['basic', 'server', 'database', 'granular', 'schema', 'object'].filter((k) => (
    k === 'basic'
    || (k === 'server' && hasServerTab.value)
    || (k === 'database' && hasDbTab.value)
    || (k === 'granular' && hasGranularTab.value && !isMssql.value)
    // SQL Server 的细粒度分成两页（Schema 级 / 对象级）
    || ((k === 'schema' || k === 'object') && isMssql.value)
  ))
  if (!ok.includes(tab.value)) tab.value = 'basic'
})
</script>

<style scoped>
/* ===== 与 ObjectFormTab 一致的顶层布局 ===== */
.form-tab {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--dc-bg-deep);
  /* 四周留 5px：贴着面板边缘时，卡片的 8px 圆角会被面板边线"压平"，看起来就像没做圆角
     （「编辑表结构」那边也是这么处理的，注释写着"用户反馈『都加圆角啊』正是这个"）。 */
  padding: 5px;
  box-sizing: border-box;
  gap: 5px;
}
/* 顶部标题栏：**独立成一张小卡**（8px 圆角 + 完整边框 + 浅底 + 同款轻阴影），
   与下面的页签卡之间留 5px 缝（见 .form-tab 的 gap）。 */
.form-tab-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
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
}
.header-icon { color: var(--dc-primary); }
.header-actions { display: flex; gap: 6px; }

.form-tab-body {
  flex: 1;
  display: flex;
  overflow: hidden;
  min-height: 0;
}

/* 左侧表单 */
.form-area {
  flex: 1;
  overflow-y: auto;
  padding: 14px;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

/* 右侧 SQL */
.sql-area {
  width: 380px;
  border-left: 1px solid var(--dc-border-soft);
  background: var(--dc-bg-code);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}
.sql-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 10px 12px;
  font-size: 13px;
  color: var(--dc-text-dim);
  /* 一档浅底 + 下边框，把"标题 / 代码 / 底栏"三层分开（与「编辑表结构」一致） */
  background: var(--dc-bg-soft);
  border-bottom: 1px solid var(--dc-border-soft);
  flex-shrink: 0;
}
.sql-head:hover { background: var(--dc-bg-hover); color: var(--dc-text); }
.sql-state { font-size: 12px; margin-left: auto; }
.sql-state.ok { --el-tag-bg-color: var(--dc-success-wash); --el-tag-border-color: var(--dc-success); --el-tag-text-color: var(--dc-success); }
.sql-state.warn { --el-tag-bg-color: var(--dc-warning-wash); --el-tag-border-color: var(--dc-warning); --el-tag-text-color: var(--dc-warning); }
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
  justify-content: flex-end;
  gap: 8px;
  flex-shrink: 0;
}

/* ===== 表单卡片（与 objectforms 统一风格） ===== */
.form-card {
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border-soft);
  border-radius: 10px;
  padding: 12px 14px;
}
.card-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 14px;
  font-weight: 600;
  color: var(--dc-text);
  margin-bottom: 10px;
}
.card-title-icon {
  width: 26px; height: 26px; border-radius: 6px;
  background: var(--dc-primary-wash);
  border: 1px solid var(--dc-primary);
  display: flex; align-items: center; justify-content: center;
  color: var(--dc-link);
  font-size: 14px;
}
.card-title-icon.purple {
  background: var(--dc-purple, rgba(168, 85, 247, .12));
  border-color: var(--dc-purple, rgba(168, 85, 247, .2));
  color: var(--dc-purple);
}
.form-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 0 16px; }

/* 权限相关 */
.perm-grid { display: flex; flex-wrap: wrap; gap: 4px; }
.perm-grid :deep(.el-checkbox) {
  margin-right: 0;
  color: var(--dc-text);
  background: rgba(255,255,255,.03);
  border: 1px solid var(--dc-border-soft);
  border-radius: 6px;
  padding: 6px 10px;
  margin: 2px;
  transition: all .15s;
}
.perm-grid :deep(.el-checkbox:hover) {
  background: rgba(79, 140, 255, .08);
  border-color: rgba(79, 140, 255, .3);
}
.perm-grid :deep(.el-checkbox.is-checked) {
  background: rgba(79, 140, 255, .12);
  border-color: rgba(79, 140, 255, .35);
}
.perm-grid :deep(.el-checkbox__label) { font-size: 13px; padding-left: 6px; }

/* 分组展示：每组一个标题 + 一行计数（勾了几项 / 共几项）+「全选 / 清空」 */
.priv-groups { display: flex; flex-direction: column; gap: 12px; }
.priv-group-title {
  display: flex; align-items: center; gap: 8px;
  font-size: 12px;
  color: var(--dc-text-dim);
  margin-bottom: 6px;
  letter-spacing: .3px;
}
/* 分隔线用真实元素而不是 ::after —— 按钮要排在它右边 */
.priv-group-line { flex: 1; height: 1px; background: var(--dc-border-soft); }
.priv-group-count { font-size: 12px; color: var(--dc-text-dim); opacity: .8; }
.priv-group-actions { display: flex; align-items: center; gap: 8px; }
.priv-group-actions button {
  appearance: none;
  border: none;
  background: none;
  padding: 0 2px;
  /* 字体要**显式跟着走**：原生 <button> 不继承父级字体，不写这一行它就用浏览器的
     默认 UI 字体 —— 同一行里"分组名 / 计数 / 全选 清空"会呈现三种字形
     （用户反馈"这个字体也保持一致啊"）。行高也钉一下：button 默认 line-height: normal，
     会把这一行撑得比旁边略高。 */
  font-family: inherit;
  font-size: 12px;
  line-height: 1.2;
  color: var(--dc-link);
  cursor: pointer;
  transition: opacity .15s;
}
.priv-group-actions button:hover:not(:disabled) { text-decoration: underline; }
.priv-group-actions button:disabled { color: var(--dc-text-dim); opacity: .45; cursor: default; }

.priv-row {
  display: flex; align-items: center; gap: 8px;
  margin-bottom: 8px;
}
.priv-row:last-child { margin-bottom: 0; }
.empty-tip {
  font-size: 13px; color: var(--dc-text-dim);
  padding: 12px 0; text-align: center;
}
.add-row-btn { margin-top: 8px; }

.priv-seg {
  display: inline-flex;
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border-soft);
  border-radius: 8px;
  padding: 3px;
  margin-bottom: 12px;
}
.seg-btn {
  appearance: none;
  border: none;
  background: transparent;
  color: var(--dc-text-dim);
  font-size: 13px;
  padding: 5px 14px;
  border-radius: 6px;
  cursor: pointer;
  transition: all .15s;
}
.seg-btn:hover { color: var(--dc-text); }
.seg-btn.active {
  background: var(--dc-primary);
  color: var(--dc-on-primary);
  font-weight: 500;
}

/* ===== 与「编辑表结构」(TableDetailView) 统一的形态：上方页签 + 下方 SQL 预览 ===== */

/* 主体改成纵向：表单在上、预览在下（原来是左右分栏） */
.form-tab-body { flex-direction: column; }

/* 表单区：内容多，自己滚动（预览固定在下） */
.form-area { overflow: auto; padding: 0 14px 10px; gap: 0; }

/* ===== 卡片 + 页签一体（与「编辑表结构」同一形态）=====
   页签就是卡片的第一行（35px，带一条分隔线）：纯文字、激活项主色 + 加粗，
   内容区在它下面 —— 不再是"浮在卡片上方的一条独立页签"。 */
.tab-card {
  position: relative;
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  box-shadow: 0 1px 3px rgba(16, 24, 40, .06), 0 1px 2px rgba(16, 24, 40, .04);
  overflow: hidden;
}
.tab-card :deep(.el-tabs) { flex: 1; min-height: 0; display: flex; flex-direction: column; }
.tab-card :deep(.el-tabs__header) {
  height: 35px; margin: 0; padding: 0; flex: 0 0 35px;
  /* 分隔线用 --dc-border（不是浅一档的 soft）：参照物那里这条线由表格表头的下边框给出，
     用的就是 --dc-border，浅一档在部分屏幕上几乎看不见。 */
  border-bottom: 1px solid var(--dc-border);
  background: transparent;
}
.tab-card :deep(.el-tabs__nav-wrap) { height: 35px; padding: 0 8px; }
.tab-card :deep(.el-tabs__nav-wrap::after) { display: none; }
.tab-card :deep(.el-tabs__active-bar) { display: none; }
.tab-card :deep(.el-tabs__nav) { height: 35px; display: flex; align-items: center; }
.tab-card :deep(.el-tabs__item) {
  height: 30px !important;
  line-height: 30px;
  padding: 0 14px !important;
  /* 字号必须 !important：全局还有一条 .el-tabs__item 是 14px，会把组件样式压掉
     （真机实测本组件算出来就是 14px，比参照物大一档） */
  font-size: 13px !important;
  color: var(--dc-text-mid);
  border-radius: 6px !important;
  /* 必须 !important：MainView 里有一条全局 .dc-tabs .el-tabs__item.is-active 会命中这里 */
  background: transparent !important;
  transition: background .15s ease, color .15s ease;
}
.tab-card :deep(.el-tabs__item:hover) { color: var(--dc-text-strong); }
/* 激活项：**白底 + 一层轻阴影**（与「编辑表结构」一致）。
   原来是灰底（--dc-bg-soft），在纯白卡片里像一块没擦干净的印子。 */
.tab-card :deep(.el-tabs__item.is-active) {
  color: var(--dc-primary) !important;
  font-weight: 600;
  background: var(--dc-bg-card) !important;
  box-shadow: 0 1px 2px rgba(16, 24, 40, .10);
}
/* Element 给页签内容区铺了一层淡灰蓝（--el-color-info-light-9，实测 rgb(228,233,242)）：
   表体/表头都是白的，于是**表格下面**透出这块底色，看着就像"表底下压着一个背景块"，
   与「编辑表结构」的纯白卡不一样（用户反馈"背景样式还是不一样"）。显式盖回透明。 */
/* position/overflow 也要放开（Element 给 content 预置了 relative + hidden）：
   右上角那条 .tab-actions 要**上浮到页签这一行**、锚点是卡片，中间这几层若不是
   static/visible，它就会被定位到内容区里、还可能被裁掉。
   滚动因此交给页签内容自己（.tab-inner { overflow: auto }）—— 与「编辑表结构」同一处理。 */
.tab-card :deep(.el-tabs__content) {
  flex: 1;
  min-height: 0;
  position: static;
  overflow: visible;
  background: transparent !important;
}
.tab-card :deep(.el-tab-pane) { height: 100%; position: static; overflow: visible; }
/* 页签内容：与卡片同一列宽，卡片之间留缝 */
.tab-inner {
  height: 100%;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 10px 12px;
  overflow: auto;      /* 滚动在这里（内容区已放开 overflow，好让动作条能上浮到页签行） */
}

/* 页签行右侧的动作条（「+ 添加…」）：绝对定位到卡片右上角、与页签同一行、纵向居中。
   右缩进 13px 让按钮右缘落在纵向滚动条左侧，与表格右缘基本齐平。
   max-width 给左侧页签留位置，窄屏时不会叠到页签上。 */
.tab-actions {
  position: absolute;
  top: 0;
  right: 13px;
  height: 35px;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  max-width: calc(100% - 320px);
  z-index: 5;
  flex-shrink: 0;
}
/* 基本信息页签里是一张**表格**：不留内边距，表格直接贴到卡片内缘
   （左右那两条"框线"就是卡片自身那条边框 —— 与「编辑表结构」的字段表同一处理）。
   选择器要写 `.tab-inner.basic-tab`：两个类在同一个元素上，写成后代选择器匹配不到。 */
.tab-inner.basic-tab { padding: 0; }

/* 基本信息那张「项目 / 值」表：照「编辑表结构」的 .field-table 那套 ——
   表头浅底 + 横竖网格 + 26px 紧凑行。行高不是 padding 决定的、是行内控件决定的，
   所以控件也压到同一高度（--ctl-h）。 */
.tab-card { --grid-row-h: 26px; --ctl-h: 20px; }
.form-table {
  width: 100%;
  border-collapse: separate;   /* 不能用 collapse：与吸顶表头同用会出残影（字段表里踩过） */
  border-spacing: 0;
  table-layout: fixed;
  font-size: 13px;
}
.form-table th {
  position: sticky;
  top: 0;
  z-index: 3;
  background: var(--dc-bg-soft);
  color: var(--dc-text-strong);
  font-weight: 600;
  font-size: 12px;
  letter-spacing: .02em;
  text-align: left;
  padding: 5px 9px;
  border-bottom: 1px solid var(--dc-border);
  border-right: 1px solid var(--dc-border);
  white-space: nowrap;
}
.form-table td {
  padding: 1px 9px;
  height: var(--grid-row-h, 26px);
  background: var(--dc-bg-card);
  border-bottom: 1px solid var(--dc-border);
  border-right: 1px solid var(--dc-border);
  vertical-align: middle;
}
/* 最后一列不画竖线：否则会与卡片外框并成两条线 */
.form-table th:last-child,
.form-table td:last-child { border-right: none; }
/* 左列「项目」：读作行标题（字号略小、颜色偏灰），与表头一样左对齐。
   权限页签里它还要放「计数 + 全选/清空」，所以允许换行。 */
/* 明细值：与用户详情页同一套口径（允许 / 是 用主色，禁止 / 否 用弱化色） */
.ft-value { color: var(--dc-text); }
.ft-value.allow, .ft-value.yes { color: var(--dc-accent); }
.ft-value.deny, .ft-value.no { color: var(--dc-text-dim); }
/* 可改项：数字输入 + 提示（0 = 不限制）；插件下拉 + "要同时填口令"的提示 */
.attr-num { display: inline-flex; align-items: center; gap: 8px; }
.attr-hint { font-size: 12px; color: var(--dc-text-dim); }
/* ⚠️ Element 的 select 默认 width:100%，放进 inline-flex 的 .attr-num 里会被缩到只剩箭头
   （实测 34px，选中项 opacity:0 —— 值在 DOM 里但看不见，用户看到的就是"没回显"）。
   给一个明确宽度，选中项就会显出来。 */
.attr-select { width: 210px; }
/* ⚠️ 这里**不能** display:flex：.ft-label 是 <td>，一旦它不再是 table-cell，
   盒高就不再跟整行齐平 —— 于是每格的右边框各画一段、上下**接不上**
   （用户反馈"表格竖线没连上"就是这么来的）。居中交给 .form-table td 的
   vertical-align: middle，间距交给 padding。 */
.ft-label {
  font-size: 12.5px;
  color: var(--dc-text-mid);
}
.ft-group-name { color: var(--dc-text); font-weight: 500; }
/* 右列「值」里是成片的复选框：行高交给内容（表头/其它行仍是 26px 紧凑行） */
.ft-perms { padding: 4px 8px !important; height: auto; }
.ft-center { text-align: center; }
/* 空态：照字段表的 .empty-row（居中、上下 18px、13px 字） */
.ft-empty { padding: 18px 0 !important; text-align: center; color: var(--dc-text-dim); font-size: 13px; }
/* 权限页签：不留内边距，表格直接贴卡片内缘（与「编辑表结构」同一处理） */
.grant-alert { margin: 10px 12px 0; }
.tab-inner.grant-tab { padding: 0; }
/* 注：「+ 添加…」已挪到页签行右上角（.tab-actions），表格里不再占一行 */

/* ===== 矩阵表：**权限项当列**，主体列冻结，权限列横向滚动（与各家客户端一致）=====
   列宽不能用 fixed：权限列要按内容（`CREATE TEMPORARY TABLES` 这种名字）自己撑开。
   冻结列的 left 偏移必须写死，所以第一列要用 min/max 把宽度钉住 ——
   否则第二列的 left 会随第一列实际宽度漂移，滚动时两列互相盖住。 */
.mx-table { table-layout: auto; min-width: 0; }
.mx-table th,
.mx-table td { white-space: nowrap; }
.mx-fix {
  position: sticky;
  z-index: 2;                      /* 盖住横向滚过来的权限列 */
  background: var(--dc-bg-card);   /* 必须不透明，否则下面的内容会透出来 */
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 表头那格的冻结要注意层级：表头本身已是 sticky top + z-index 3，
   冻结列表头要再高一层（4）才能同时盖住"横向滚来的表头"和"纵向滚来的数据格" */
.mx-table thead .mx-fix { background: var(--dc-bg-soft); z-index: 4; }
.mx-f1 {
  left: 0;
  width: 190px; min-width: 190px; max-width: 190px;
}
.mx-f2 {
  left: 190px;                     /* = 第一列的宽度 */
  width: 220px; min-width: 220px; max-width: 220px;
}
/* 冻结线：两列右侧各画一条，滚动时能看清冻到哪儿（`!important` 压过最后列不画竖线那条） */
.mx-f1, .mx-f2 { border-right: 1px solid var(--dc-border) !important; }

/* ===== 分组表头（MySQL 全局权限）=====
   第一行是分组名（colspan 跨它的列，带计数与全选/清空），第二行才是权限名。
   第一行**不吸顶**：它带计数与按钮，实测高 59px，硬给它 top: 0 再让第二行 top: 27px，
   两行纵向滚动时会叠在一起。这一页主体只有一行、纵向本来就不会滚动，
   所以让第一行随表头滚走、只把权限名列吸在顶上更稳（也不会和冻结列表头打架）。 */
/* 分类行不吸顶（它带计数与按钮，比权限行高，硬吸顶会和下一行叠住）；
   但**冻结列**那一格要照旧吸顶 —— 它 rowspan=2 跨两行，横向滚动时得一直贴左边。 */
.mx-grouped thead tr:first-child th:not(.mx-fix) { position: static; }
.mx-grouped thead tr:last-child th { top: 0; }
/* 分类名**居中**在它跨的那几列中间（用户要求）；计数与全选/清空跟在名字后面同一行。
   选择器必须把 th 带上：`.form-table th { text-align: left }` 是"类+元素"，只写 .mx-grp
   优先级更低，会被它压掉（真机实测压成 left，居中不生效）。 */
.form-table th.mx-grp { text-align: center; }
/* 分类行整体 12px：分组名、计数、全选/清空 三处同一字号（原来是 13 / 11.5 / 12 三种）。 */
.mx-grp { font-size: 12px; }
.mx-grp .ft-group-name { color: var(--dc-text-strong); font-size: 12px; }
.mx-grp .priv-group-count { font-size: 12px; }
.mx-grp .priv-group-actions { margin-left: 2px; }
/* 分类表头分**两行**：第一行「分组名 + 已勾/共几项」，第二行「全选 / 清空」。
   动作条用**块级 flex**，效果是两件事一起成立：
     · 它必然另起一行（不会跟第一行挤在一起 —— 之前用 inline-flex 就是挤在一行）；
     · justify-content: center 让它整行居中（cell 上的 text-align 管不到块级 flex）。 */
.mx-grp .priv-group-actions {
  display: flex;
  justify-content: center;
  align-items: center;
  gap: 8px;
  margin-top: 2px;
}
.mx-grp .priv-group-actions button { vertical-align: middle; }
.mx-priv { text-align: center; min-width: 74px; padding-left: 6px !important; padding-right: 6px !important; }
.mx-cell { text-align: center; padding-left: 2px !important; padding-right: 2px !important; }
.mx-act { width: 48px; }
.mx-cell :deep(.el-checkbox) { margin-right: 0; height: auto; }
.mx-table :deep(.el-select) { width: 100%; }
.mx-table :deep(.el-tag) { margin-top: 2px; }
.form-table :deep(.el-input__inner) { height: var(--ctl-h, 20px); }
.form-table :deep(.el-input__wrapper) { min-height: var(--ctl-h, 20px); }
/* ===== 让表格"像表格"：单元格里的输入控件默认**隐形**（透明底 + 无描边），
   整张表读起来是一列列文字；鼠标移到该行才浮出输入框的样子，聚焦时给主色描边。
   （与「编辑表结构」的字段表同一处理 —— 用户反馈"框不要留"就是指这个。）
   必须 !important：index.css 有一套全局的 `.el-input__wrapper { background-color: …!important }`。 */
.form-table :deep(.el-input__wrapper),
.form-table :deep(.el-select__wrapper) {
  background-color: transparent !important;
  box-shadow: none !important;
  transition: background .15s ease, box-shadow .15s ease;
}
.form-table tbody tr:hover :deep(.el-input__wrapper),
.form-table tbody tr:hover :deep(.el-select__wrapper) {
  background-color: var(--dc-bg-hover) !important;
  box-shadow: 0 0 0 1px var(--dc-border) inset !important;
}
.form-table :deep(.el-input__wrapper.is-focus),
.form-table :deep(.el-select__wrapper.is-focused) {
  background-color: var(--dc-bg-hover) !important;
  box-shadow: 0 0 0 1px var(--dc-primary) inset !important;
}
/* 行悬停：与字段表同色 */
.form-table tbody tr:hover td { background: var(--dc-bg-hover); }
/* 底栏：左提示 + 右按钮（与「编辑表结构」同一排法） */
.sql-tip { font-size: 12px; color: var(--dc-text-dim); margin-right: auto; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.grant-alert { margin-bottom: 2px; }

/* SQL 预览：通栏在下方（原来是右侧 380px 固定栏）——与上方的表格卡同一套外观：
   1px 细边框 + 8px 圆角 + 卡片底 + 同一层轻阴影；overflow: hidden 让圆角不被头部/底栏顶成直角 */
.sql-area {
  width: auto;
  margin: 0;
  flex-shrink: 0;
  min-height: 0;
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  background: var(--dc-bg-card);
  box-shadow: 0 1px 3px rgba(16, 24, 40, .06), 0 1px 2px rgba(16, 24, 40, .04);
  overflow: hidden;
}
.sql-area.collapsed { height: auto !important; }
.sql-area .sql-body { flex: 1; min-height: 0; overflow: auto; }
/* 分隔条：就是上下两块之间那道缝（6px + 透明底），悬停变主色 */
.sql-resizer { height: 6px; flex: 0 0 6px; cursor: row-resize; position: relative; z-index: 2; background: transparent; }
.sql-resizer::before {
  content: ''; position: absolute; left: 50%; top: 50%;
  transform: translate(-50%, -50%);
  width: 48px; height: 4px; border-radius: 3px;
  background: var(--dc-text-dim); opacity: .45;
  transition: background .15s ease, opacity .15s ease;
}
.sql-resizer:hover::before { background: var(--dc-primary); opacity: 1; }
.sql-fold { transition: transform .15s ease; }
.sql-fold.folded { transform: rotate(-90deg); }
.sql-fold-tx { margin-left: auto; font-size: 12px; color: var(--dc-text-dim); }
.sql-foot-actions { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
.sql-head { cursor: pointer; }

/* ===== 间距收紧（用户反馈：边距太大）===== */
/* 高度分配钉死：预览只认内联那一个 height（旧布局给过 flex: 1，不钉死时拖到很矮会被
   flex-grow 重新拉大 —— 真机实测"拖到 0 却量出 705px"）；表单区吃掉剩下的那部分。 */
/* 左右**不留内边距**：与「编辑表结构」同一口径（那里若两侧缩进，页签卡就比下方的
   SQL 预览窄一圈，实测 890 vs 914，两块面板看着宽度不一致）。 */
.form-area { padding: 0; flex: 1 1 auto; min-height: 0; }
/* 左右**不留外边距**：与「编辑表结构」同一口径 —— 那边表单卡与预览区同宽
   （都贴着面板左右边），这里若缩进 8px 就会比上方的卡片窄一圈（实测差 16px）。 */
.sql-area { margin: 0; flex: 0 0 auto; }
.tab-inner { gap: 8px; }

/* ===== 对象权限表格（权限做列，勾选即授予）===== */
/* 对象权限表：本身就用 .form-table（表头浅底 + 横竖网格 + 吸顶），这里只补它自己的列宽与
   「自动列宽」—— 权限列有十几列，fixed 均分会把「模式/名称」压扁，交给内容定宽更合适。 */
.obj-grid { font-size: 12px; table-layout: auto; min-width: 0; }
.obj-grid tbody tr:hover { background: var(--dc-primary-wash); }
.og-schema { min-width: 120px; }
.og-object { min-width: 160px; }
.og-priv { width: 82px; text-align: center; }
.og-cell { text-align: center; }
.obj-grid :deep(.el-select) { width: 100%; }
.obj-grid :deep(.el-checkbox) { margin-right: 0; height: auto; }
</style>
