//! 方言能力（`DialectOps`）：按**语法族**实现，与「数据库类型」多对一。
//!
//! 为什么要有这一层：`dialect.rs` 里 45 处 `match self.key() { "mysql" | "mariadb" => … }`
//! 是典型的「新增一种库要改 80 个地方」结构 —— 加 TiDB 只因为它是 MySQL 系才没改代码，
//! 换个非同源的方言（Firebird / GaussDB 原生模式）就得在几十个 match 里逐个加分支。
//!
//! 这里把「一种方言怎么说话」收进一个 impl：
//! - 新增**同源**类型（TiDB / OceanBase-MySQL 模式）⇒ 只加 YAML + 前端 js，本文件零改动；
//! - 新增**新方言** ⇒ 写一个 `impl DialectOps`，在 [`of`] 里加一行。
//!
//! 注册表的兜底顺序是「先按 `dialect` 归族、再按单个类型覆盖例外」：
//! `dialect: MySQL` 的新类型自动落到 [`MySqlOps`]，不需要任何人记得来改这里。
//! 这也解释了为什么 [`of`] 里只有 `Doris` 这类**同族但能力不同**的例外需要显式列出。

use dbmind_core::ConnectionKind;

use crate::api::dialect::{is_reserved, DirectRefDepth, GrantRevoke, Meta, UserAttrs, UserPrivileges};

pub mod editor;
mod mssql;
mod mysql;
mod oracle;
mod others;
mod postgres;

/// 金仓的目录 SQL 改名器（`pg_*` → `sys_*`）。
///
/// 只给「绕开 `DialectOps`、把 PG 语句写死在 `match kind` 里」的几处复用：
/// 监控总览、容量排行、同步行数估算、比对外键定义。**绝不能用在用户 SQL 上**。
pub(crate) use postgres::kb_catalog_sql;

/// 一种方言族的全部「按库分支」能力。
///
/// 默认实现取**最保守**的形态（没有原生 upsert、没有分页扩展、查询版本要用各家自己的
/// 写法），这样新写一个 impl 只需要覆盖「我这族到底能做什么」，而不是把 40 个 false 抄一遍。
pub trait DialectOps: Sync + Send {
    /// 语法族短名（`mysql` / `postgresql` …）。只用于日志与错误文案，
    /// 与 `ConnectionKind::key()` 的区别是：多个类型共用一个族。
    fn family(&self) -> &'static str;

    /// PG 族有没有 `reltuples` 与容量那一族（`pg_total_relation_size` /
    /// `pg_size_pretty` / `pg_database_size`）。
    ///
    /// RisingWave 是 PG 协议的**子集**，这几样都没有 —— 照 PG 拼就直接
    /// `Failed to bind expression: c.reltuples`，连表清单都打不开。2026-11-19 在实验室
    /// `lab-risingwave` 上逐项探测过：`relkind` / `relowner` / `has_table_privilege` /
    /// `pg_get_userbyid` / `obj_description` / `pg_indexes` / `col_description` 都在，
    /// 缺的正是 `reltuples` 与那三个容量函数。
    ///
    /// 返回 false 的方言把这些列改成 null / 空串：界面上留空，行数由前端 COUNT(*) 回填
    /// （与「拿不到就留空，不拿 0 冒充空表」同一口径），库级统计如实报「未接入」。
    fn supports_pg_catalog_stats(&self) -> bool {
        true
    }

    /// PG 族「这张表/视图对我可见吗」用的权限清单（`has_table_privilege` 的第二个参数）。
    ///
    /// 默认七种表级权限，**任意一种**即可见 —— 只授了 INSERT 的表也该在树里找得到。
    /// 逗号列表是 PG / 金仓支持的写法。
    ///
    /// RisingWave 只实现了 `SELECT,INSERT,UPDATE,DELETE` 四种：另外三种会在**执行期**
    /// 报 `Invalid parameter privilege: unrecognized ...`（不是绑定错误，一行都读不到，
    /// 症状与「表清单打不开」一样）。真机逐种探测过，别把三种加回去。
    fn pg_table_visibility_privileges(&self) -> &'static str {
        "SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER"
    }

    /// 能不能用**一条语句**完成 upsert（原生 upsert）。
    ///
    /// 不能的话调用方回退「先把目标主键读进内存 → 逐行 UPDATE」，行为不变但慢得多。
    fn supports_native_upsert(&self) -> bool {
        false
    }

    /// 原生 upsert 单条语句最多带多少行（调用方按它拆批）。
    fn upsert_row_limit(&self) -> usize {
        usize::MAX
    }

    /// Oracle 风格的 `rownum` 包裹分页（没有 `OFFSET/FETCH` 的老写法）。
    fn wrap_paging(&self, _base: &str, _offset: u64, _size: u64) -> Option<String> {
        None
    }

    /// 分页子句（`offset` 从 0 开始）。
    fn limit_clause(&self, offset: u64, size: u64) -> String {
        format!("limit {size} offset {offset}")
    }

    /// 取服务器版本的一行 SQL（列别名 `v`）。没有统一写法就返回 `None`。
    fn version_sql(&self) -> Option<&'static str> {
        None
    }

    /// 分页的 `OFFSET` 是否必须跟 `ORDER BY`（SQL Server 的硬性要求）。
    fn needs_order_by_for_paging(&self) -> bool {
        false
    }

    /// 是否支持**行值比较** `(a, b) > (x, y)`（keyset 分页靠它走索引 range）。
    fn supports_row_value_compare(&self) -> bool {
        false
    }

    /// 同实例直拷：源表在**目标会话**上的限定层级。
    fn direct_ref_depth(&self) -> DirectRefDepth {
        DirectRefDepth::None
    }

    /// 「两个连接指向同一实例」的指纹 SQL。只有指纹确实唯一的方言才给。
    fn instance_fingerprint_sql(&self) -> Option<&'static str> {
        None
    }

    /// 清空一张表的语句（"清空后导入"模式）。
    fn clear_table_sql(&self, table: &str) -> String {
        format!("truncate table {table}")
    }

    /// 更新一行的语句（upsert 的更新分支）。
    fn update_sql(&self, table: &str, assignments: &str, condition: &str) -> String {
        format!("update {table} set {assignments} where {condition}")
    }

    // ---- 标识符与层级 ----

    /// 给一个**已经决定要加引号**的标识符套引号（不做大小写折叠）。
    fn wrap_ident(&self, text: &str) -> String {
        format!("\"{}\"", text.replace('"', "\"\""))
    }

    /// 未加引号的标识符是否被折成**小写**（PG 系）。
    fn folds_ident_lower(&self) -> bool {
        false
    }

    /// 未加引号的标识符是否被折成**大写**（Oracle / DM / DB2 / H2 / Derby）。
    fn folds_ident_upper(&self) -> bool {
        false
    }

    /// 字符串字面量里的 `\` 是否当转义符（MySQL / ClickHouse）。
    fn escapes_backslash(&self) -> bool {
        false
    }

    /// 是否按「库.模式」两级取元数据。
    fn is_schema_aware(&self) -> bool {
        false
    }

    /// 树里「库」这一层装的其实是**模式**（Oracle）。
    fn db_is_schema(&self) -> bool {
        false
    }

    /// 元数据 SQL 执行前要补一句 `use [库];`（SQL Server）。
    fn wants_use_switch(&self) -> bool {
        false
    }

    /// `database` 是「服务器上的库、可切换」还是连接固有的目标。
    fn switchable_database(&self) -> bool {
        false
    }

    /// 是否 catalog → 库 → 表 三层结构（Doris）。
    fn catalog_level(&self) -> bool {
        false
    }

    // ---- 层级清单（返回 SQL 常量；None 的含义见门面方法的文档）----

    /// catalog 清单 SQL。`None` = 这个类型**确实没有** catalog 这一层（`Meta::Absent`）。
    fn catalogs_sql(&self) -> Option<&'static str> {
        None
    }

    /// 服务器库清单 SQL。`None` = 还没写（`Meta::Unwritten`，调用方退回单库）。
    fn databases_sql(&self) -> Option<&'static str> {
        None
    }

    /// 标识符引号字符：反引号 / 方括号 / 双引号。
    ///
    /// 用于「把源端定义里的引号换成目标方言」这类**字面替换** ——
    /// 不能拿 `quote("_")` 反推（它对朴素名字不加引号，首字符会取成 `_` 本身）。
    fn quote_mark(&self) -> char {
        '"'
    }

    /// 整库同步时「同时搬几张表」的默认并发数。
    ///
    /// 管的是对象级并行（同时同步几张表），不是单表开几条写连接：
    /// - 嵌入式/文件引擎只有一条物理连接，并行只会互相排队 → 1；
    /// - Doris 每个导入落一个 part，并发高会压小 part 合并 → 3；
    /// - Oracle / DM / DB2 会话开销大、锁行为保守 → 3；
    /// - 其余网络型库 → 8（固定成本被摊薄，实测 48 张小表快一倍）。
    fn sync_object_concurrency(&self) -> usize {
        8
    }

    /// 表行数**估算** SQL（整库同步的调度排序用）。
    ///
    /// 只取统计视图，绝不 `count(*)` 全表扫 —— 那是把「为了快」变成「先慢一次」。
    /// `None` = 这类型没有现成的统计视图（排序退化成清单原序）。
    fn table_rows_estimate_sql(&self, database: &str, schema: &str) -> Option<String> {
        let _ = (database, schema);
        None
    }

    /// 一条 `INSERT` 能不能带多组 `VALUES (…), (…)`。
    ///
    /// 只有 Oracle / DM 不支持（它们的标准写法是 `INSERT ALL` 或 `UNION ALL`），
    /// 其余方言（含 DB2 / Derby / DuckDB）都支持。
    ///
    /// 三处调用点（导入 / 造数 / 备份的 DDL 导出）历史上各存了一份名单，
    /// 而且口径互不相同 —— 导入侧是 20 个品牌的白名单、造数侧是 13 个、
    /// 备份侧是「非 oracle/dm」的黑名单，同一个类型在三处可能得到相反结论。
    fn supports_multi_row_insert(&self) -> bool {
        true
    }

    /// 建库语句（目标库不存在时的自动建库）。默认是通用 ANSI 写法。
    fn create_database_sql(&self, name: &str) -> String {
        format!("create database {}", self.quote(name))
    }

    /// 删表语句（整库重同步前的清场）。
    ///
    /// 默认**不带** `if exists`：Oracle / DB2 / Derby / DuckDB 上多出来的子句是语法错，
    /// 「清场」失败会把整张表的同步带成红灯。支持 `if exists` 的方言自己覆盖。
    fn drop_table_sql(&self, quoted_table: &str) -> String {
        format!("drop table {quoted_table}")
    }

    // ---- 字面量 ----

    /// 布尔值在**写数据**时该写成什么（`insert`/`update` 的取值）。
    ///
    /// 只有 MySQL 系 / SQL Server / ClickHouse / SQLite 收 `0`/`1`；
    /// PG / Oracle / DB2 / H2 / Derby 的 boolean 列只认 `true`/`false`。
    fn bool_literal(&self, truth: bool) -> &'static str {
        if truth {
            "true"
        } else {
            "false"
        }
    }

    /// 高级搜索关键字里 `_`/`%` 的 LIKE 转义尾巴。
    ///
    /// `escape_like()` 产出的是 `\%`/`\_`，但**只有部分方言默认把 `\` 当 LIKE 转义符**
    /// （MySQL 系 / ClickHouse / PG 系）。没有默认转义符的方言（SQLite / SQL Server /
    /// Oracle / DM / DB2 / H2 / Derby）里 `\_` 会被理解成「反斜杠 + 任意一个字符」，
    /// 关键字带 `_`/`%` 时一条都搜不到 —— 必须显式 `escape '\'`。
    /// 默认 = 要显式转义；默认转义符就是 `\` 的方言覆盖成空串（多写一句反而可能
    /// 被某些版本判成重复声明）。
    fn like_escape_suffix(&self) -> &'static str {
        " escape '\\'"
    }

    /// 行编辑取值时日期/时间字符串的字面量翻译（`to_date`/`to_timestamp` 显式格式串）。
    ///
    /// Oracle 会话 NLS 默认 `DD-MON-RR`，ISO 串直接写会 `ORA-01861`，必须显式给格式串；
    /// DM 同语法。其它方言的 ISO 串会话都认 → 默认 `None`（调用方退回普通字符串字面量）。
    /// 与 [`DialectOps::portable_time_literal`]（表**默认值**的翻译）是两条独立入口，别混。
    fn temporal_row_literal(&self, _hint: &str, _value: &str) -> Option<String> {
        None
    }

    /// 授权回显行 → 一条 `GRANT ...;` 文本（user-info 的「原始授权」列表）。
    ///
    /// 入参是**按名字抽好**的字段（调用方负责 pick_field 与「单列文本直通」判定）。
    /// 默认 = MySQL 系通用形状（`ON \`库\`.\`表\` TO 'u'@'h'`）；
    /// Oracle / PG 系 / H2 的 grantee 与 ON 子句形状不同，各自覆盖。
    fn grant_line_text(
        &self,
        scope: &str,
        privilege: &str,
        owner: &str,
        object: &str,
        account_user: &str,
        account_host: &str,
    ) -> Option<String> {
        let target = match scope {
            "global" => "*.*".to_string(),
            "schema" if !owner.is_empty() => format!("{}.*", self.quote(owner)),
            "table" if !owner.is_empty() && !object.is_empty() => {
                format!("{}.{}", self.quote(owner), self.quote(object))
            }
            // 例程级：object 列是 `类型:名字`（user_grants 里 concat 出来的），还原成
            // `ON PROCEDURE \`db\`.\`p\`` —— 类型字是 MySQL 语法的一部分，必须带上
            "routine" if !owner.is_empty() && !object.is_empty() => {
                let (kind, routine) = object.split_once(':').unwrap_or(("PROCEDURE", object));
                let routine_type = if kind.eq_ignore_ascii_case("function") {
                    "function"
                } else {
                    "procedure"
                };
                format!("{routine_type} {}.{}", self.quote(owner), self.quote(routine))
            }
            // 角色（MySQL 8 / MariaDB）：角色本身就是账号形状，语句里没有 ON 子句
            "role" if !owner.is_empty() => {
                let role_host = if object.is_empty() { "%" } else { object };
                return Some(format!(
                    "GRANT {}@{} TO {}@{};",
                    self.literal(owner),
                    self.literal(role_host),
                    self.literal(account_user),
                    self.literal(account_host)
                ));
            }
            _ => return None,
        };
        Some(format!(
            "GRANT {privilege} ON {target} TO {}@{};",
            self.literal(account_user),
            self.literal(account_host)
        ))
    }

    /// 「授予角色」语句的形状（user-info 的角色层补行）。
    ///
    /// 三档：MySQL 的角色是账号形状（`'r'@'h'`，接受者也是 `'u'@'h'`）；
    /// PG 系的角色是**标识符**（`"r"`，单引号不合法）；其余（MariaDB/CH/Oracle/Doris）
    /// 角色没有 host，账号用字面量。
    fn role_grant_text(
        &self,
        role_name: &str,
        _role_host: &str,
        account_user: &str,
        _account_host: &str,
    ) -> String {
        format!(
            "GRANT {} TO {};",
            self.literal(role_name),
            self.literal(account_user)
        )
    }

    /// user-action 支持的授权入参 key（`privileges`/`dbPrivileges`/... 的白名单）。
    ///
    /// 不在清单里的入参**在动数据库之前**明确拒绝 —— 界面会以为权限也授好了
    /// 而实际上什么都没发生，那比报错糟得多。默认空 = 全拒。
    /// Doris 不含 `routinePrivileges`（连 mysql.procs_priv 都没有）。
    fn grant_input_scopes(&self) -> &'static [&'static str] {
        &[]
    }

    /// 全局/系统层权限名白名单（会被**拼进 SQL**，逐个过）。
    /// 返回函数指针：调用方把它喂给通用的 `checked_privileges`。
    fn grantable_privilege_check(&self) -> fn(&str) -> bool {
        crate::api::dialect::is_mysql_privilege
    }

    /// 对象/表层权限名白名单（与上面分开：Oracle 两本白名单、H2 只有表级）。
    fn object_privilege_check(&self) -> fn(&str) -> bool {
        crate::api::dialect::is_mysql_privilege
    }

    /// 撤销行的 scope 校验。privilege/owner/object/kind 都会拼进 `revoke ...`，
    /// 逐项过白名单；返回 false = 整行拒绝。
    /// 空 scope 的缺省语义由各族自己定（MySQL/Oracle 缺省当 global）。
    fn is_revocable(&self, scope: &str, privilege: &str, owner: &str, object: &str, kind: &str) -> bool {
        let _ = (scope, privilege, owner, object, kind);
        false
    }

    /// 撤销校验失败时提示里「缺少什么」的措辞（只影响文案）。
    fn revoke_missing_hint(&self) -> &'static str {
        "模式/对象名"
    }

    /// 布尔**默认值**在 DDL 里的写法。
    ///
    /// 真机实测：Doris 的 `boolean default false` 是语法错（只收 `default 0`），
    /// SQL Server 的 `bit default false` 报「The name "false" is not permitted」。
    /// 其余（MySQL / PG / H2 / Derby / DB2）两种都收。
    fn boolean_literal(&self, value: bool) -> String {
        if value { "true" } else { "false" }.to_string()
    }

    // ---- 元数据补全 ----
    //
    // 这一组方法的入参都是**已经转义过**的字面量/引用（门面调 `literal()` / `quote()`），
    // 实现只管拼 SQL；返回 `Meta` 三态，语义与门面文档一致：
    // `Sql` = 有这条查询、`Absent` = 这个方言根本没有这个概念、`Unwritten` = 还没写。

    /// 一张表的列注释：返回 `column_name` / `comment` 两列。
    fn table_comments_sql(&self, quoted: &str) -> Meta {
        let _ = quoted;
        Meta::Unwritten
    }

    /// 库级统计（表数 + 占用空间，列名 `tables` / `sizeText`）。
    ///
    /// `None` = 还没写（`Meta::Unwritten`）。文件型库拿不到占用空间时**照样给 SQL**，
    /// 只是把 `sizeText` 写成 `'—'` —— 「没有这个概念」和「没实现」是两件事。
    fn database_stats_sql(&self) -> Option<&'static str> {
        None
    }

    /// 一张表的**完整类型文本**：返回 `column_name` / `type_text` / `comment` /
    /// `is_auto` / `ordinal` 五列（个别方言缺 `is_auto` 就少给，调用方按列名取）。
    ///
    /// 为什么要单独查一次：内核的列元数据只有类型名（`nvarchar`），**不带长度/精度**。
    /// 界面按 `nvarchar(50)` 这种文本拆分出「长度/精度」两个输入框 ——
    /// 只给 `nvarchar` 的后果是：用户在那一行改任何别的属性（比如可空），
    /// 生成的 ALTER 就变成 `ALTER TABLE t ALTER COLUMN c nvarchar NULL`（没有长度），
    /// SQL Server / MySQL / Oracle 上这要么直接报错，要么把列悄悄变成 1 个字符宽。
    /// 所以类型文本必须带参数，`nvarchar(max)` 这种边界也要如实表达。
    fn column_types_sql(&self, quoted: &str) -> Meta {
        let _ = quoted;
        Meta::Unwritten
    }

    /// 结构编辑器的能力（`ddlStyle` / 列可改 / 字符集 …）。
    ///
    /// 默认是最保守的「只能看」：新方言不覆盖就等于**界面如实告诉用户这里改不了**，
    /// 而不是误开一堆点了会报错的开关。
    fn edit_capabilities(&self) -> editor::EditCapabilities {
        editor::EditCapabilities::generic()
    }

    /// 列类型下拉清单。空 = 前端下拉没有选项（只能手打）。
    fn column_type_names(&self) -> &'static [&'static str] {
        &[]
    }

    /// 索引类型下拉清单。空 = 前端不显示「类型」那一列。
    fn index_type_names(&self) -> &'static [&'static str] {
        &[]
    }

    /// 存储引擎下拉清单。空 = 该行退化成纯手输。
    fn storage_engines(&self) -> &'static [&'static str] {
        &[]
    }

    // ---- 标识符 ----

    /// 标识符加引号（门面 `Dialect::quote` 的唯一实现）。简单名不加，
    /// 避免把 MySQL 的大小写敏感表名改写掉；带 `.` / `-` / 保留字 / 大小写敏感的才加。
    ///
    /// **保留字必须加引号**：一张叫 `user` 的表会让服务端拼出的 `delete from user`
    /// 直接语法错（SQL Server 156，MySQL 的 `order`/`key` 同理）。
    ///
    /// 几条刻意的规矩（每条都对应过真机故障）：
    /// - 未加引号的标识符必须**字母开头**，`_`/数字开头一律走引号分支（ORA-00911）；
    /// - **下折方言**（PG / SQLite）里含大写的普通名也要加引号，否则被折成小写指向别的列；
    /// - **上折方言**（Oracle / DB2 / H2 / Derby）里要加引号的普通名先折大写，
    ///   否则 `rename to user` 会悄悄造出小写对象；但 `_` 开头的名字**不折**
    ///   （字典里是小写说明当初就是带引号建的小写对象）。
    fn quote(&self, name: &str) -> String {
        let wrap = |s: &str| -> String {
            if s.is_empty() {
                return String::new();
            }
            let plain = s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
            // 未加引号的标识符必须**字母开头**（Oracle/DB2/Derby 直接拒绝 `_`/数字开头
            // 的裸名 —— ORA-00911 无效字符；MySQL 数字开头同样不合法），这样的名字
            // 一律走引号分支。
            let starts_alpha = s.starts_with(|c: char| c.is_ascii_alphabetic());
            // **下折方言**（PG / Kingbase / SQLite）里未加引号的名字会被折成小写：
            // 目标表上叫 `Name` 的列，裸写 `Name` 会被解析成 `name` —— 真机报
            // `column "name" of relation "t" does not exist`，而表结构明明是好的。
            // 这类「含大写字母」的普通名必须逐字加引号；全小写的普通名照旧裸写（行为不变）。
            let needs_case_quote =
                plain && self.folds_ident_lower() && s.chars().any(|c| c.is_ascii_uppercase());
            if plain && starts_alpha && !is_reserved(s) && !needs_case_quote {
                return s.to_string();
            }
            // 上折方言（Oracle / DM / DB2 / H2 / Derby）里，未加引号的名字会被折成大写；
            // 一旦加引号就等于固定大小写。所以要加引号的**普通名**先折一次，
            // 让 `rename to user` 仍然建出 `USER`，而不是悄悄造一个小写对象。
            // 但**字母开头之外**的 plain 名（`_report_xingwei` 这种 `_` 开头的表）
            // 不折：它进引号分支恰恰因为它裸写不合法 —— 而字典里是小写就说明当初
            // 是带引号建的小写对象，折成大写反而指向一个不存在的表（豫园 11g 实测）。
            let text = if plain && starts_alpha && self.folds_ident_upper() {
                s.to_ascii_uppercase()
            } else {
                s.to_string()
            };
            self.wrap_ident(&text)
        };
        name.split('.').map(wrap).collect::<Vec<_>>().join(".")
    }
    /// 逐字加引号：**不动大小写**，简单名也加引号。
    ///
    /// 与 [`Self::quote`] 分工不同：`quote()` 翻译「用户想写的名字」，
    /// 这里的名字来自**目录元数据**（`list_columns` / `list_tables` 的返回值），
    /// 那就是库里真实存在的大小写，加引号锁死才指得准。
    /// 反例（Oracle 11g 实测）：列是小写带引号建的 `"day"`，排序走 `quote()` →
    /// 拼出 `order by day` → 服务端折成 `DAY` → `ORA-00904`。
    fn quote_exact(&self, name: &str) -> String {
        let wrap = |s: &str| -> String {
            if s.is_empty() {
                return String::new();
            }
            self.wrap_ident(s)
        };
        name.split('.').map(wrap).collect::<Vec<_>>().join(".")
    }
    // ---- 对象级语句 ----

    /// 这个方言能不能用 `drop <kind> if exists`。`kind` 是 `view` / `procedure` /
    /// `function` / `trigger` / `event`（小写）。
    ///
    /// `if exists` 不是到处都认（Oracle / DM / DB2 / Derby 没有），
    /// 与其拼一句「看着像、执行就报错」的语句，不如返回 `false` 让调用方跳过并说明白。
    /// SQL 字符串字面量（`'…'`）。
    ///
    /// 反斜杠要不要翻倍按族定：MySQL 系默认当转义符（`C:\temp\` 不翻倍会把收尾引号吃掉、
    /// 语句被截断），其余方言不翻（PG 的 `standard_conforming_strings` 默认开）。
    fn literal(&self, value: &str) -> String {
        let body = if self.escapes_backslash() {
            value.replace('\\', "\\\\")
        } else {
            value.to_string()
        };
        format!("'{}'", body.replace('\'', "''"))
    }

    // ---- 用户与角色 ----

    /// 是不是 MariaDB（同族里那几处真差异用：`is_role` / `roles_mapping` / 角色没有 host）。
    ///
    /// 不做成两个 impl：MariaDB 与 MySQL 在 50 个方法上一字不差，拆开只会让两边慢慢漂。
    fn mariadb_flag(&self) -> bool {
        false
    }

    /// 用户清单。
    ///
    /// 默认 `Absent`：SQLite / DuckDB 这类没有账号体系，树上不画「用户」节点。
    fn users(&self) -> Meta {
        Meta::Absent
    }

    /// 某个用户的详情：**一行**结果，含 `name` / `create_user_sql` / `grants_text`
    /// 以及若干明细列。
    ///
    /// 为什么把授权拼成换行文本而不是多行：调用方要的是一份「可执行脚本文本」，
    /// 多行结果还要自己逐行收集、顺序还可能变；一行内拼好，取数端只做 split。
    ///
    /// 各家系统表差别很大，**只写有把握的**：SQL Server / MySQL / PostgreSQL。
    /// 其余返回 `Unwritten` —— 用户管理涉及权限，猜错的代价比不显示大得多。
    fn user_info(&self, name: &str) -> Meta {
        let _ = name;
        Meta::Unwritten
    }

    /// 用户权限明细：**逐行**返回（全局 / 库级 / 表级各一档），由调用方拼成 `GRANT …` 文本。
    fn user_grants(&self, user: &str, host: &str) -> Meta {
        let _ = (user, host);
        Meta::Unwritten
    }

    /// 已授予该账号的**角色**（MySQL 8 / MariaDB 的 RBAC）。
    fn user_roles(&self, user: &str, host: &str) -> Meta {
        let _ = (user, host);
        Meta::Unwritten
    }

    /// 角色自己的「详情行」（界面复用用户表单来编辑**角色**的权限）。
    ///
    /// 角色和账号在有些方言里根本不在同一张目录（Oracle 的 `dba_roles`、ClickHouse 的
    /// `system.roles`、Doris 的 `SHOW ROLES`），所以不能拿 `user_info` 硬查 ——
    /// 查不到就是"没有找到用户"，角色页永远打不开。
    fn role_info(&self, name: &str) -> Meta {
        let _ = name;
        Meta::Unwritten
    }

    /// 角色**自身拥有**的权限（与 [`Self::user_grants`] 同形状：
    /// scope/privilege/owner/db_name/table_name/grant_option）。
    fn role_grants(&self, name: &str, host: &str) -> Meta {
        let _ = (name, host);
        Meta::Unwritten
    }

    /// 这台服务器上**出现过**的角色（界面要能列出「还没授的」，否则看不见也就授不了）。
    fn role_inventory(&self) -> Meta {
        Meta::Unwritten
    }

    /// 用户的**新建 / 改密 / 改属性 / 授权 / 撤销 / 删除**。
    ///
    /// `action` ∈ create / alter / drop / create-role / drop-role；调用方已把
    /// `alter-role` 归一成 `alter`，并把「这次保存的是**角色**」放进 `as_role`
    /// （角色没有口令 / 锁定 / 配额，所以角色模式只出差量授权）。
    ///
    /// **多条语句用 `;\n` 连接**，由调用方拆分逐条执行。
    ///
    /// 只生成有把握的那几件事：细粒度权限按层拼（全局 / 库 / 表 / 例程），权限名由调用方
    /// 过白名单；撤销是**差量**（界面算好"原来有、现在没有了"），不做事后全删重授。
    #[allow(clippy::too_many_arguments)]
    fn user_action_sql(
        &self,
        action: &str,
        as_role: bool,
        user: &str,
        password: &str,
        host: &str,
        roles: &[String],
        privileges: &UserPrivileges,
        revokes: &[GrantRevoke],
        reset_privileges: bool,
        attrs: &UserAttrs,
    ) -> Meta {
        let _ = (
            action,
            as_role,
            user,
            password,
            host,
            roles,
            privileges,
            revokes,
            reset_privileges,
            attrs,
        );
        Meta::Unwritten
    }

    // ---- 对象清单：schema / 表 / 例程 / 触发器 / 事件 ----

    /// schema 清单（只有 schema 层级的类型需要）。
    ///
    /// `None` = 这类型没有 schema 层，树上不会请求它。
    fn schemas_sql(&self) -> Option<&'static str> {
        None
    }

    /// 某个 schema 下的表/视图。`None` = 走内核的 `DatabaseMetaData.getTables`。
    fn tables_in_schema_sql(&self, schema: &str) -> Option<String> {
        let _ = schema;
        None
    }

    /// 例程清单（存储过程 / 函数）。`schema` = 树上点的那个模式，`None` = 交给会话当前模式。
    ///
    /// 这里刻意返回 `Meta` 而不是 `Option`：**「这库没有例程」（Absent，空数组是事实）
    /// 与「没实现」（Unwritten）对界面是两件事** —— 前者不该报错，后者才是缺口。
    fn procedures(&self, schema: Option<&str>) -> Meta {
        let _ = schema;
        Meta::Unwritten
    }

    /// 触发器清单。`schema` 含义同 [`Self::procedures`]。
    ///
    /// 默认 `Absent`：多数方言要么没有触发器、要么清单就是空 —— 只有明确知道的才返回 SQL。
    fn triggers(&self, schema: Option<&str>) -> Meta {
        let _ = schema;
        Meta::Absent
    }

    /// 事件清单（MySQL 的事件调度器）。默认 `Absent`：其余类型确实没有这种「事件」。
    fn events(&self) -> Meta {
        Meta::Absent
    }

    // ---- 元数据页：表清单 / 行数 / 表选项 / 索引 ----

    /// 「当前账号能用的表」清单；`None` = **不需要过滤**（清单查询本身已按权限收窄）。
    ///
    /// 只有目录不按权限收窄的方言（H2 / DB2 / Derby）才需要它：实测 H2 里给低权限账号
    /// 只授了一张表的 SELECT，`DatabaseMetaData.getTables` 仍把同模式里一张都读不了的
    /// 表全列出来。这两家的目录没有「是否可见」标志，只能把「属主 + 显式授权（含 PUBLIC）」
    /// 拼出来，再与 JDBC 结果求交。
    fn visible_tables(&self) -> Option<&'static str> {
        None
    }

    /// 一张表的**估算行数**（一次查询拿全库）。
    ///
    /// 精确值要 `count(*)` 扫表，树上不能对每张表都跑一次；这里取引擎自己维护的统计值。
    /// `None` = 这方言没有可用统计（调用方照旧不给数字）。
    fn table_rows_sql(&self, database: &str) -> Option<String> {
        let _ = database;
        None
    }

    /// 每张表的**真实**表选项：表名 / 引擎 / 排序规则 / 表注释（一次查询拿全库）。
    ///
    /// 不回填的话前端只能回退到「能力清单的第一项」—— 看着像回显、其实是默认值：
    /// 表是 MyISAM 也显示 InnoDB，改别的项保存时会顺手把引擎一起改掉。
    ///
    /// 列名规则：SQL 里用 `as <JSON 键>` 直接把最终键名定下来（`table_name` 只用于配对），
    /// 拿到什么给什么；`charset` 给的是**排序规则全名**（`utf8mb4_general_ci`），
    /// 前端据此拆出字符集与排序规则。
    fn table_options_sql(&self, database: &str) -> Option<String> {
        let _ = database;
        None
    }

    /// 索引清单（全库返回，每条带 `table`，结构页自己按表名过滤）。
    fn indexes_sql(&self) -> Option<&'static str> {
        None
    }

    fn supports_drop_object(&self, kind: &str) -> bool {
        let _ = kind;
        false
    }

    /// 视图 / 例程改名。默认没有改名语句（MySQL 系 / Oracle / DB2 / H2 / Derby / DM），
    /// 调用方改走「改写定义正文里的名字」这条路。
    fn rename_object_sql(&self, kind: &str, from: &str, to: &str, schema: Option<&str>) -> Option<String> {
        let _ = (kind, from, to, schema);
        None
    }

    // ---- 原生 upsert ----

    /// 一条语句完成「插入或更新」（调用方按 [`Self::upsert_row_limit`] 拆批）。
    ///
    /// 返回值里的列名引用由实现自己 `quote`，`rows` 里的值**已经**按本方言渲染好。
    /// 不支持的方言返回 `None`，调用方回退到「先读主键、再逐行 UPDATE」的两阶段。
    ///
    /// 拼错一个关键字，整张表的 upsert 会在真机上直接语法错；而少写 `when matched` /
    /// `on conflict` 则更坏 —— 语句仍然成功，只是把「更新」变成「插入」，行数悄悄翻倍。
    fn upsert_sql(
        &self,
        table: &str,
        columns: &[String],
        key_indexes: &[usize],
        rows: &[Vec<String>],
    ) -> Option<String> {
        let _ = (table, columns, key_indexes, rows);
        None
    }

    /// 表/视图的建表语句（一条查询带回**可直接执行**的 DDL 文本）。
    ///
    /// `schema` = 该对象所在的模式（请求里的「库.模式」拆出来的那半）。**不能只靠会话
    /// 默认模式**：DBMind 的查询会话刻意不按"树上选中的模式"切换（不赌代理有没有真的切
    /// schema），在 SYS 下点一张表、会话默认模式却是 SYSTEM 时会拿错对象。
    /// 没实现的方言返回 `Unwritten`（界面如实说，不猜）。
    fn ddl(&self, table: &str, schema: Option<&str>) -> Meta {
        let _ = (table, schema);
        Meta::Unwritten
    }

    /// 对象定义（右键「编辑」取 SQL 再转 ALTER）。默认 `Unwritten`（没实现的不猜）。
    ///
    /// 返回值的三态有讲究（导出循环按它区分处理）：
    ///   `Absent`    → 该方言**根本没有**这类对象，界面不摆「查看定义/编辑」，导出**静默跳过**；
    ///   `Unwritten` → 功能还没写，导出会逐个对象提示「定义查询尚未实现，未导出」。
    /// 各实现先答「有没有这种对象」（Absent），再给定义查询。
    fn object_source(&self, object_type: &str, name: &str, schema: Option<&str>) -> Meta {
        let _ = (object_type, name, schema);
        Meta::Unwritten
    }

    /// 「当前时间 / 当前日期」默认值的**目标方言改写**。默认不改写（None），
    /// 交给后续的通用翻译（`portable_time_literal`）。
    fn rewrite_time_default(&self, upper: &str, is_textual_type: bool) -> Option<String> {
        let _ = (upper, is_textual_type);
        None
    }

    /// 「当前时间 / 当前日期」的标准写法翻成**目标方言真能接受的默认值表达式**。
    /// 默认直通标准写法；返回 `None` 表示这个方言表达不了（省略默认值）。
    /// 实测矩阵见 `_uitest/ddl-time-default-check.cjs`（11 类目标库全跑）。
    fn portable_time_literal(&self, portable: &'static str) -> Option<&'static str> {
        Some(portable)
    }

    /// DDL 的「附加语句」：注释 / 索引与唯一约束 / 外键（每段是一次查询、
    /// 返回可直接执行的语句文本）。默认返回空：MySQL 的 `show create table` /
    /// SQLite 的 `sqlite_master.sql` 本来就把这些带在一条语句里；SQL Server
    /// 的注释是扩展属性，需要独立语句（族实现覆盖）。
    fn ddl_extras(&self, table: &str) -> Vec<(&'static str, String)> {
        let _ = table;
        Vec::new()
    }

    /// 建表后补注释（跨库同步用）。默认返回空：MySQL 系 / Doris / ClickHouse
    /// 的注释已随建表内联写好，SQLite / Derby 没有注释概念；
    /// PG 系 / Oracle / DM / DB2 / H2 走 `comment on …`（共用拼装件），
    /// SQL Server 是扩展属性 `sp_addextendedproperty`（族实现覆盖）。
    fn post_create_comments(
        &self,
        table: &str,
        column_comments: &std::collections::HashMap<String, String>,
        table_comment: Option<&str>,
        schema: &str,
    ) -> Vec<String> {
        let _ = (table, column_comments, table_comment, schema);
        Vec::new()
    }

    /// 表级动作（清空 / 截断 / 删除 / 重命名）的**通用**拼法。
    /// 例外在族实现里覆盖：CH 的 clear 是 TRUNCATE、SQLite 的 truncate 退化 DELETE、
    /// DB2 的 truncate 带 IMMEDIATE、rename 的方言写法差异。
    fn table_action(&self, table: &str, action: &str, new_name: Option<&str>) -> Meta {
        let t = self.quote(table);
        match action {
            "clear" => Meta::Sql(format!("delete from {t}")),
            "truncate" => Meta::Sql(format!("truncate table {t}")),
            "drop" => Meta::Sql(format!("drop table {t}")),
            "rename" => match new_name {
                Some(new_name) => Meta::Sql(format!("alter table {t} rename to {}", self.quote(new_name))),
                None => Meta::Unwritten,
            },
            _ => Meta::Unwritten,
        }
    }
}

// ==================== upsert 公共拼装件 ====================
//
// 所有方言都要算同样的四样东西（列引用、键、待更新列、值元组），差别只在
// 「用哪句 SQL 表达」。这几样放这里算一次，族实现只挑形态，不复制引用规则。

/// 一条 upsert 语句的公共拼装件。字段里的列名**都已按本方言 quote 过**。
pub(crate) struct UpsertParts {
    pub(crate) quoted: Vec<String>,
    pub(crate) col_list: String,
    pub(crate) keys: Vec<String>,
    /// 需要被更新的列（= 非键列下标）。为空时语句退化成「插入或忽略」。
    pub(crate) payload: Vec<usize>,
    pub(crate) tuples: String,
}

impl UpsertParts {
    pub(crate) fn col(&self, i: usize) -> &str {
        &self.quoted[i]
    }

    /// `src.a, src.b`：MERGE 的 insert 侧要带源别名。
    pub(crate) fn src_cols(&self) -> String {
        self.quoted
            .iter()
            .map(|c| format!("src.{c}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// `tgt.a = src.a and tgt.b = src.b`：MERGE 的匹配条件。
    pub(crate) fn on_clause(&self) -> String {
        self.keys
            .iter()
            .map(|c| format!("tgt.{c} = src.{c}"))
            .collect::<Vec<_>>()
            .join(" and ")
    }
}

/// 三个入参有一个为空就没有 upsert 可言（调用方也不会发这种请求）。
pub(crate) fn upsert_parts(
    ops: &dyn DialectOps,
    columns: &[String],
    key_indexes: &[usize],
    rows: &[Vec<String>],
) -> Option<UpsertParts> {
    if rows.is_empty() || columns.is_empty() || key_indexes.is_empty() {
        return None;
    }
    let quoted: Vec<String> = columns.iter().map(|c| ops.quote(c)).collect();
    let key_set: std::collections::HashSet<usize> = key_indexes.iter().copied().collect();
    Some(UpsertParts {
        col_list: quoted.join(", "),
        keys: key_indexes.iter().map(|i| quoted[*i].clone()).collect(),
        payload: (0..columns.len()).filter(|i| !key_set.contains(i)).collect(),
        tuples: rows
            .iter()
            .map(|row| format!("({})", row.join(", ")))
            .collect::<Vec<_>>()
            .join(", "),
        quoted,
    })
}

/// `insert … on conflict (keys) do update set c = excluded.c`
///
/// PG / Kingbase 与 SQLite 都是这个写法（SQLite 3.24+ 支持 upsert），所以放在公共层。
pub(crate) fn on_conflict_upsert(parts: &UpsertParts, table: &str) -> String {
    let target = parts.keys.join(", ");
    if parts.payload.is_empty() {
        return format!(
            "insert into {table} ({}) values {} on conflict ({target}) do nothing",
            parts.col_list, parts.tuples
        );
    }
    let sets = parts
        .payload
        .iter()
        .map(|i| {
            let c = parts.col(*i);
            format!("{c} = excluded.{c}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "insert into {table} ({}) values {} on conflict ({target}) do update set {sets}",
        parts.col_list, parts.tuples
    )
}

/// `merge into t as tgt using (values …) as src (cols) on (…)`
///
/// DB2 / Derby / SQL Server 三家同形，差别只有「尾分号」：T-SQL 的 MERGE 老解析器
/// 硬要求分号，DB2 反而不接受语句尾分号。`trailing_semicolon` 就是这个开关。
pub(crate) fn merge_values_upsert(parts: &UpsertParts, table: &str, trailing_semicolon: bool) -> String {
    let mut sql = format!(
        "merge into {table} as tgt using (values {}) as src ({}) on ({})",
        parts.tuples,
        parts.col_list,
        parts.on_clause()
    );
    if !parts.payload.is_empty() {
        let sets = parts
            .payload
            .iter()
            .map(|i| {
                let c = parts.col(*i);
                format!("{c} = src.{c}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" when matched then update set {sets}"));
    }
    sql.push_str(&format!(
        " when not matched then insert ({}) values ({})",
        parts.col_list,
        parts.src_cols()
    ));
    if trailing_semicolon {
        sql.push(';');
    }
    sql
}

// ---------------------------------------------------------------- MySQL 语法族

/// MySQL / MariaDB / TiDB：`insert … on duplicate key update`，`limit` 分页。
///
/// `mariadb` 只标出「同族里那几处真差异」，不是为了分支而分支：
/// MariaDB 有 `is_role` 列、角色表是 `roles_mapping`（MySQL 8 是 `role_edges`）、
/// 角色**没有 host**（`create role 'r'@'%'` 是 1064），还有几处 collation 与字面量
/// 不一致导致 `= 'Y'` 直接报 1267。除此之外 50 个方法与 MySQL 一字不差，
/// 再拆一个 impl 只会让两边慢慢漂开。
pub struct MySqlOps {
    pub mariadb: bool,
}

/// Doris：MySQL 协议，但**没有原生 upsert**（走两阶段），catalog 三级引用。
pub struct DorisOps {
    /// StarRocks 分叉后**没有 `mysql` 库**：用户/角色相关的 `mysql.user`、`mysql.role_edges`
    /// 全部 `Unknown database 'mysql'`（真机实测，整条清单 500）。这类能力对它如实 Absent，
    /// 其余（索引、表选项、DDL）与 Doris 同源，继续共用一份实现。
    pub starrocks: bool,
}

/// ClickHouse：MySQL 协议但**没有标准 UPDATE**（只能 `ALTER TABLE … UPDATE`），
/// 也没有原生 upsert / 行值比较。
pub struct ClickHouseOps;

// ---------------------------------------------------------------- PostgreSQL 语法族

/// PostgreSQL：`on conflict do update`，行值比较能走索引。
///
/// `risingwave` 是同一个 impl 里的例外：RisingWave 的 `pg_class` 是**裁剪过的视图**，
/// 没有 `reltuples`，容量那一族（`pg_total_relation_size` / `pg_size_pretty` /
/// `pg_database_size`）也没实现 —— 照 PG 写会 `Failed to bind expression`，
/// 整棵表清单都打不开（见 `supports_pg_catalog_stats`）。
pub struct PostgresOps {
    pub risingwave: bool,
}

/// KingbaseES：与 PG 同族，但**行值比较没有验证过能走索引**，保守关掉
/// （原 `match` 里只列了 `postgresql`，这里如实保留）。
pub struct KingbaseOps;

// ---------------------------------------------------------------- Oracle 语法族

/// Oracle：`merge into` upsert、`rownum` 分页、`v$version` 取版本。
pub struct OracleOps;

/// 达梦：语法与 Oracle 高度一致，批大小同样受 MERGE 源展开限制。
pub struct DmOps;

// ---------------------------------------------------------------- 其余 SQL 方言

/// SQL Server：`OFFSET … FETCH` 必须带 `ORDER BY`；两段名是「模式.表」，
/// 跨库引用要写三段名。
pub struct SqlServerOps;

/// DB2：不吃 `limit`（SQLCODE=-104），`truncate` 必须显式 `immediate`。
pub struct Db2Ops;

/// SAP HANA：标准 SQL + 双引号标识符，系统目录在 SYS schema。
pub struct SapHanaOps;

/// SQLite：没有 `TRUNCATE`，`on conflict do update` 可用。
pub struct SqliteOps;

/// H2：`merge into … key (…)` 形态的原生 upsert。
pub struct H2Ops;

/// Derby：没有 `LIMIT`，只能 `OFFSET … FETCH`；`values` 构造器上限 1000 行。
pub struct DerbyOps;

/// DuckDB：内嵌分析库，当前没有原生 upsert 与专属元数据语句。
pub struct DuckDbOps;

/// 非 SQL 类型（MongoDB / Redis / Elasticsearch）。
///
/// 它们根本不走 `Dialect`（查询走 `nosql.rs` 的文档/命令通道），但注册表仍要给它们
/// 一个归属 —— 否则 [`of`] 会把它们静默扔进 [`GenericOps`]，一旦将来有人误用
/// `dialect.count_sql()` 就会拼出一条 MySQL 语法发给 Mongo。
pub struct NoSqlOps;

/// 兜底：没写专属 impl 的方言一律走最保守的形态。
pub struct GenericOps;

// ---------------------------------------------------------------- 注册表

static MYSQL: MySqlOps = MySqlOps { mariadb: false };
/// MariaDB 与 MySQL 同族同 impl，只在用户管理那几条上分叉（见 `MySqlOps::mariadb`）。
static MARIADB: MySqlOps = MySqlOps { mariadb: true };
static DORIS: DorisOps = DorisOps { starrocks: false };
/// StarRocks：元数据/DDL 与 Doris 同源，只有用户管理缺席（见 `DorisOps::starrocks`）。
static STARROCKS: DorisOps = DorisOps { starrocks: true };
static CLICKHOUSE: ClickHouseOps = ClickHouseOps;
static POSTGRES: PostgresOps = PostgresOps { risingwave: false };
/// RisingWave：PG 协议的子集，`pg_class` 缺 `reltuples` 等统计列（见 `PostgresOps`）。
static RISINGWAVE: PostgresOps = PostgresOps { risingwave: true };
static KINGBASE: KingbaseOps = KingbaseOps;
static ORACLE: OracleOps = OracleOps;
static DM: DmOps = DmOps;
static SQLSERVER: SqlServerOps = SqlServerOps;
static DB2: Db2Ops = Db2Ops;
static SAPHANA: SapHanaOps = SapHanaOps;
static SQLITE: SqliteOps = SqliteOps;
static H2: H2Ops = H2Ops;
static DERBY: DerbyOps = DerbyOps;
static DUCKDB: DuckDbOps = DuckDbOps;
static NOSQL: NoSqlOps = NoSqlOps;
static GENERIC: GenericOps = GenericOps;

/// 类型 → 方言实现。
///
/// **先按 `dialect` 归族，再按单个类型覆盖例外**：`dialect: MySQL` 的新类型
/// （TiDB、OceanBase-MySQL 模式…）自动落到 [`MySqlOps`]，不需要任何人来改这张表。
/// 只有「同族但能力不同」的才需要显式列出（Doris 没有原生 upsert、ClickHouse 没有标准 UPDATE）。
pub fn of(kind: ConnectionKind) -> &'static dyn DialectOps {
    match kind.dialect() {
        "MySQL" => match kind {
            // Doris 系：SelectDB 是 Doris 的商业版（元数据与 SQL 全同源），StarRocks 分叉后
            // 元数据查询仍与 Doris 一样 —— 三家走同一份实现。
            // **漏了 SelectDB 的代价是真的会坏**：落到 MySqlOps 后 `indexes` 用
            // `group_concat(... order by seq_in_index separator ',')`（Doris 不认这个写法）、
            // `users` 读 `mysql.user.super_priv`（Doris 表里没这列），两处都直接 500
            //（2026-11-17 真机巡检实测）。
            // StarRocks 单独一支：它没有 `mysql` 库，用户/角色那几条查询会整片 500
            //（见 `DorisOps::starrocks`）；其余能力与 Doris 完全相同。
            ConnectionKind::Starrocks => &STARROCKS,
            ConnectionKind::Doris | ConnectionKind::Selectdb => &DORIS,
            ConnectionKind::Mariadb => &MARIADB,
            _ => &MYSQL,
        },
        "PostgreSQL" => match kind {
            ConnectionKind::Kingbase => &KINGBASE,
            ConnectionKind::Risingwave => &RISINGWAVE,
            _ => &POSTGRES,
        },
        "Oracle" => match kind {
            ConnectionKind::Dm => &DM,
            _ => &ORACLE,
        },
        "SqlServer" => &SQLSERVER,
        "Db2" => &DB2,
        "SapHana" => &SAPHANA,
        "ClickHouse" => &CLICKHOUSE,
        "SQLite" => &SQLITE,
        "H2" => &H2,
        "Derby" => &DERBY,
        "DuckDB" => &DUCKDB,
        "MongoShell" | "Redis" | "Elasticsearch" => &NOSQL,
        _ => &GENERIC,
    }
}

/// `comment on table/column` 风格的补注释（PG 系 / Oracle / DM / DB2 / H2 共用）。
/// 表名一律收**未加引号的裸名 + 模式**，引号由实现方的 `quote` 自己加。
pub(crate) fn comment_on_statements(
    ops: &dyn DialectOps,
    table: &str,
    column_comments: &std::collections::HashMap<String, String>,
    table_comment: Option<&str>,
    schema: &str,
) -> Vec<String> {
    let full = if schema.trim().is_empty() {
        ops.quote(table)
    } else {
        format!("{}.{}", ops.quote(schema), ops.quote(table))
    };
    let mut out = Vec::new();
    if let Some(tc) = table_comment.map(str::trim).filter(|c| !c.is_empty()) {
        out.push(format!("comment on table {full} is {}", ops.literal(tc)));
    }
    for (name, comment) in column_comments {
        if comment.trim().is_empty() {
            continue;
        }
        out.push(format!(
            "comment on column {full}.{} is {}",
            ops.quote(name),
            ops.literal(comment)
        ));
    }
    return out;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册表必须覆盖所有已声明的类型 —— 漏了就会静默退化成 GenericOps
    /// （症状是「这个库突然不支持原生 upsert 了」，而且没人会想到是注册表漏项）。
    #[test]
    fn every_kind_resolves() {
        for kind in ConnectionKind::ALL {
            let ops = of(*kind);
            assert!(!ops.family().is_empty(), "{kind:?} 没有方言实现");
            assert_ne!(
                ops.family(),
                "generic",
                "{kind:?} 落到了兜底实现（缺口：dialect={}）",
                kind.dialect()
            );
        }
    }

    /// TiDB 是「只加 YAML 不加代码」的样板：它必须自动获得 MySQL 族的全部能力。
    #[test]
    fn same_dialect_types_share_family() {
        assert_eq!(of(ConnectionKind::Tidb).family(), "mysql");
        assert!(of(ConnectionKind::Tidb).supports_native_upsert());
        assert_eq!(
            of(ConnectionKind::Tidb).limit_clause(10, 20),
            of(ConnectionKind::Mysql).limit_clause(10, 20)
        );
    }

    /// 同族里的例外必须分开：Doris 有 MySQL 的版本语句，但没有原生 upsert。
    #[test]
    fn family_exceptions_stay_distinct() {
        assert_eq!(of(ConnectionKind::Doris).family(), "doris");
        assert!(!of(ConnectionKind::Doris).supports_native_upsert());
        assert_eq!(
            of(ConnectionKind::Doris).version_sql(),
            of(ConnectionKind::Mysql).version_sql()
        );
        assert_eq!(
            of(ConnectionKind::Clickhouse).update_sql("t", "a = 1", "id = 2"),
            "alter table t update a = 1 where id = 2"
        );
    }

    /// 元数据补全语句按族分流：同族共享一份写法，缺概念的给 `Absent`，
    /// 还没写的给 `Unwritten`（这三态语义不同，混了就是「注释永远空」或「误报未实现」）。
    #[test]
    fn metadata_sql_splits_by_family() {
        fn sql_of(kind: ConnectionKind) -> Option<String> {
            match of(kind).table_comments_sql("'t'") {
                Meta::Sql(sql) => Some(sql),
                _ => None,
            }
        }

        // MySQL 协议族（含 TiDB —— 同源类型自动继承）走 information_schema
        for kind in [
            ConnectionKind::Mysql,
            ConnectionKind::Mariadb,
            ConnectionKind::Tidb,
            ConnectionKind::Doris,
        ] {
            let sql = sql_of(kind).unwrap_or_else(|| panic!("{kind:?} 应有列注释查询"));
            assert!(sql.contains("information_schema.columns"), "{kind:?}: {sql}");
            assert!(sql.contains("column_comment"), "{kind:?}: {sql}");
        }

        // ClickHouse 不在 information_schema 里
        assert!(sql_of(ConnectionKind::Clickhouse)
            .unwrap_or_default()
            .contains("system.columns"));

        // PG 走 pg_attribute + col_description
        let pg_comments = sql_of(ConnectionKind::Postgresql)
            .unwrap_or_else(|| panic!("Postgresql 应有列注释查询"));
        assert!(pg_comments.contains("pg_attribute"), "{pg_comments}");
        // 金仓的系统目录叫 sys_*：同一条 SQL 要**整条改写**过去，不能再带 pg_ 目录名
        let kb_comments = sql_of(ConnectionKind::Kingbase)
            .unwrap_or_else(|| panic!("Kingbase 应有列注释查询"));
        assert!(kb_comments.contains("sys_attribute"), "{kb_comments}");
        assert!(!kb_comments.contains("from pg_"), "{kb_comments}");

        // Oracle / 达梦：`comment` 是关键字，必须带双引号，否则 ORA-00923
        for kind in [ConnectionKind::Oracle, ConnectionKind::Dm] {
            let sql = sql_of(kind).unwrap_or_else(|| panic!("{kind:?} 应有列注释查询"));
            assert!(sql.contains(r#"as "comment""#), "{kind:?}: {sql}");
        }

        // SQL Server 用 isnull（sql_variant 上 coalesce 会报类型错）
        assert!(sql_of(ConnectionKind::Sqlserver)
            .unwrap_or_default()
            .contains("isnull("));

        // 真的没有这个概念 ≠ 还没写
        for kind in [ConnectionKind::Sqlite, ConnectionKind::Derby] {
            assert!(
                matches!(of(kind).table_comments_sql("'t'"), Meta::Absent),
                "{kind:?} 应判为 Absent"
            );
        }
        assert!(matches!(
            of(ConnectionKind::Duckdb).table_comments_sql("'t'"),
            Meta::Unwritten
        ));

        // 库级统计：文件型库拿不到占用空间，但**照样有语句**（给 `'—'`），
        // 不能写成「未实现」——那会让界面显示「未支持」而不是「无此概念」。
        for kind in [ConnectionKind::Sqlite, ConnectionKind::H2, ConnectionKind::Derby] {
            let sql = of(kind).database_stats_sql().unwrap_or_default();
            assert!(sql.contains("'—'"), "{kind:?} 应以 '—' 表示无占用空间: {sql}");
        }
        assert!(of(ConnectionKind::Duckdb).database_stats_sql().is_none());
    }

    /// 列类型文本的方言差异：下面每一条都对应过真机故障，别「顺手统一」。
    #[test]
    fn column_types_sql_keeps_dialect_traps() {
        fn sql_of(kind: ConnectionKind) -> String {
            match of(kind).column_types_sql("'t'") {
                Meta::Sql(sql) => sql,
                _ => String::new(),
            }
        }

        // MySQL 协议族共享 column_type（含 enum/unsigned 的完整文本）；TiDB 自动继承
        assert!(sql_of(ConnectionKind::Mysql).contains("column_type"));
        assert_eq!(sql_of(ConnectionKind::Tidb), sql_of(ConnectionKind::Mysql));
        assert_eq!(sql_of(ConnectionKind::Doris), sql_of(ConnectionKind::Mysql));
        // ClickHouse 没有 information_schema.columns
        assert!(sql_of(ConnectionKind::Clickhouse).contains("system.columns"));

        // SQL Server：text/ntext/image 的 character_maximum_length 是 2147483647 而不是 -1，
        // 拼成 `text(2147483647)` 会让建表报 2715 —— 必须先判裸类型名
        let mssql = sql_of(ConnectionKind::Sqlserver);
        assert!(mssql.contains("'text', 'ntext', 'image'"), "{mssql}");
        assert!(mssql.contains("'(max)'"), "{mssql}");

        // PG 系用 udt_name（短名），两种自增写法（identity / serial）都要认
        let pg = sql_of(ConnectionKind::Postgresql);
        assert!(pg.contains("udt_name"), "{pg}");
        assert!(pg.contains("is_identity") && pg.contains("nextval"), "{pg}");
        // 金仓只换目录名（``sys_class`` / ``sys_attribute``），类型文本与自增判定与 PG 一致
        let kb = sql_of(ConnectionKind::Kingbase);
        assert!(kb.contains("udt_name"), "{kb}");
        assert!(kb.contains("is_identity") && kb.contains("nextval"), "{kb}");
        assert!(!kb.contains("from pg_"), "{kb}");

        // Oracle 认 12c 标识列；达梦**故意不查** user_tab_identity_cols
        // （视图是否存在未验证，那句 SQL 一报错会连带丢掉类型与注释）
        assert!(sql_of(ConnectionKind::Oracle).contains("user_tab_identity_cols"));
        assert!(!sql_of(ConnectionKind::Dm).contains("user_tab_identity_cols"));

        // H2 只对 numeric/decimal 拼精度：否则 integer(32,0) 与界面基线对不上，
        // 一打开表就被判成「改 3」
        let h2 = sql_of(ConnectionKind::H2);
        assert!(h2.contains("'numeric','decimal','dec','number'"), "{h2}");

        // Derby 的类型文本带着 ` NOT NULL`，必须裁掉（否则会生成 `integer not null not null`）
        let derby = sql_of(ConnectionKind::Derby);
        assert!(
            derby.contains("length(cast(columndatatype as varchar(256))) - 9"),
            "{derby}"
        );

        // SQLite 的声明类型就在表结构里，自增靠「单列 INTEGER PRIMARY KEY」
        let sqlite = sql_of(ConnectionKind::Sqlite);
        assert!(sqlite.contains("pragma_table_info"), "{sqlite}");

        assert!(matches!(
            of(ConnectionKind::Duckdb).column_types_sql("'t'"),
            Meta::Unwritten
        ));
    }

    /// 对象级语句与引用规则的族差异。
    #[test]
    fn object_statements_split_by_family() {
        // `drop … if exists` 的覆盖范围按族不同：MySQL 连 event 都支持，
        // H2 / ClickHouse 只有 view，Oracle / DM / DB2 / Derby 根本没有
        assert!(of(ConnectionKind::Mysql).supports_drop_object("event"));
        assert!(of(ConnectionKind::Tidb).supports_drop_object("procedure"));
        assert!(of(ConnectionKind::Sqlite).supports_drop_object("trigger"));
        assert!(!of(ConnectionKind::Sqlite).supports_drop_object("procedure"));
        assert!(of(ConnectionKind::H2).supports_drop_object("view"));
        assert!(!of(ConnectionKind::H2).supports_drop_object("procedure"));
        assert!(of(ConnectionKind::Clickhouse).supports_drop_object("view"));
        assert!(!of(ConnectionKind::Clickhouse).supports_drop_object("trigger"));
        for kind in [
            ConnectionKind::Oracle,
            ConnectionKind::Dm,
            ConnectionKind::Db2,
            ConnectionKind::Derby,
        ] {
            assert!(!of(kind).supports_drop_object("view"), "{kind:?}");
        }

        // 改名只有两条路：SQL Server 的 sp_rename（必须带模式、参数是字面量）
        let mssql = of(ConnectionKind::Sqlserver)
            .rename_object_sql("procedure", "p1", "v2", Some("dbo"))
            .unwrap_or_default();
        assert_eq!(mssql, r"exec sp_rename N'dbo.p1', N'v2'");

        // PG 系是 alter … rename to；普通小写名按 quote 规则裸写
        let pg = of(ConnectionKind::Postgresql)
            .rename_object_sql("view", "v1", "v2", None)
            .unwrap_or_default();
        assert_eq!(pg, "alter view v1 rename to v2");
        // 触发器要在所属表上改，不支持
        assert!(of(ConnectionKind::Postgresql)
            .rename_object_sql("trigger", "t1", "t2", None)
            .is_none());
        // 其余方言没有改名语句
        assert!(of(ConnectionKind::Mysql)
            .rename_object_sql("view", "a", "b", None)
            .is_none());

        // 引用规则：MySQL 用反引号；下折方言里含大写的名字必须加引号
        assert_eq!(of(ConnectionKind::Mysql).quote("a b"), "`a b`");
        assert_eq!(of(ConnectionKind::Postgresql).quote("Name"), r#""Name""#);
        assert_eq!(of(ConnectionKind::Oracle).quote("a b"), r#""a b""#);
        // quote_exact 不动大小写、简单名也加引号（元数据里的真实大小写）
        assert_eq!(of(ConnectionKind::Oracle).quote_exact("day"), r#""day""#);
    }

    /// 原生 upsert 的族归属：少写 `when matched` / `on conflict` 不会报错，
    /// 只会把「更新」变成「插入」（行数翻倍），所以形态必须钉住。
    #[test]
    fn upsert_sql_splits_by_family() {
        fn upsert(kind: ConnectionKind) -> Option<String> {
            of(kind).upsert_sql(
                "t",
                &["id".to_string(), "name".to_string()],
                &[0],
                &[vec!["1".to_string(), "'a'".to_string()]],
            )
        }

        // 没有原生 upsert 的：Doris / ClickHouse 走两阶段，必须返回 None
        assert!(upsert(ConnectionKind::Doris).is_none());
        assert!(upsert(ConnectionKind::Clickhouse).is_none());

        // 同源类型自动继承
        assert_eq!(upsert(ConnectionKind::Tidb), upsert(ConnectionKind::Mysql));
        assert_eq!(
            upsert(ConnectionKind::Kingbase),
            upsert(ConnectionKind::Postgresql)
        );
        assert_eq!(upsert(ConnectionKind::Dm), upsert(ConnectionKind::Oracle));

        // H2 是 `merge … key (…)`，没有 `on conflict`
        let h2 = upsert(ConnectionKind::H2).unwrap_or_default();
        assert!(h2.contains("key (id)"), "{h2}");

        // SQLite 与 PG 同形（都认 `excluded.`）
        assert!(upsert(ConnectionKind::Sqlite)
            .unwrap_or_default()
            .contains("excluded.name"));

        // Derby 与 DB2 同形，且都不带尾分号（只有 T-SQL 要求）
        let derby = upsert(ConnectionKind::Derby).unwrap_or_default();
        assert!(derby.starts_with("merge into t as tgt using (values "), "{derby}");
        assert!(!derby.ends_with(';'), "{derby}");

        // Oracle/DM 的源是 `dual union all`，且别名不能带 AS
        let oracle = upsert(ConnectionKind::Oracle).unwrap_or_default();
        assert!(oracle.contains("from dual"), "{oracle}");
        assert!(!oracle.contains("merge into t as tgt"), "{oracle}");

        // 全是键时退化成「插入或忽略」，不能拼出空的 update set
        let one = |kind: ConnectionKind| {
            of(kind).upsert_sql("t", &["id".to_string()], &[0], &[vec!["1".to_string()]])
        };
        assert_eq!(
            one(ConnectionKind::Mysql).unwrap_or_default(),
            "insert into t (id) values (1) on duplicate key update id = id"
        );
        assert_eq!(
            one(ConnectionKind::Postgresql).unwrap_or_default(),
            "insert into t (id) values (1) on conflict (id) do nothing"
        );
    }

    /// 元数据页的四块：表清单过滤 / 估算行数 / 表选项 / 索引。
    /// 这些 SQL 一错就是「页签一片空白」，错误还被上层 `if let Ok(..)` 吞掉，所以钉住形状。
    #[test]
    fn metadata_pages_split_by_family() {
        // ---- 可见表：只有目录不按权限收窄的三家需要 ----
        assert!(of(ConnectionKind::H2)
            .visible_tables()
            .unwrap_or("")
            .contains("is_admin"));
        assert!(of(ConnectionKind::Db2)
            .visible_tables()
            .unwrap_or("")
            .contains("syscat.tabauth"));
        assert!(of(ConnectionKind::Derby)
            .visible_tables()
            .unwrap_or("")
            .contains("sqlAuthorization"));
        // MySQL 的清单本身按权限收窄，不需要第二道过滤
        assert!(of(ConnectionKind::Mysql).visible_tables().is_none());
        assert!(of(ConnectionKind::Postgresql).visible_tables().is_none());

        // ---- RisingWave：pg_class 没有 reltuples、也没有容量函数 ----
        // 这几条 SQL 里出现它们就是「打开表清单直接 Failed to bind expression」，
        // 而错误被上层吞成空白页 —— 只能靠这条单测钉住。
        let rw_tables = of(ConnectionKind::Risingwave)
            .tables_in_schema_sql("public")
            .unwrap_or_default();
        assert!(!rw_tables.contains("reltuples"), "{rw_tables}");
        assert!(rw_tables.contains("cast(null as bigint) as rows"), "{rw_tables}");
        // 权限清单只能是 RW 实现的四种：另三种是**执行期**报错，单测看不出来，
        // 但把 `Invalid parameter privilege` 挡在发版前。
        assert!(rw_tables.contains("'SELECT,INSERT,UPDATE,DELETE'"), "{rw_tables}");
        assert!(!rw_tables.contains("TRUNCATE"), "{rw_tables}");
        let rw_options = of(ConnectionKind::Risingwave)
            .table_options_sql("public")
            .unwrap_or_default();
        for word in ["reltuples", "pg_total_relation_size", "pg_size_pretty"] {
            assert!(!rw_options.contains(word), "RisingWave 没有 {word}: {rw_options}");
        }
        // 行数/占用留空，注释与属主照旧（真机探测这两样可用）
        assert!(rw_options.contains("cast(null as text) as rows"), "{rw_options}");
        assert!(rw_options.contains("obj_description"), "{rw_options}");
        assert!(of(ConnectionKind::Risingwave).database_stats_sql().is_none());
        // PG 不受影响：同样的开关对它是开的
        let pg_options = of(ConnectionKind::Postgresql)
            .table_options_sql("public")
            .unwrap_or_default();
        assert!(pg_options.contains("pg_total_relation_size"), "{pg_options}");
        assert!(pg_options.contains("greatest(c.reltuples, 0)"), "{pg_options}");
        let pg_tables = of(ConnectionKind::Postgresql)
            .tables_in_schema_sql("public")
            .unwrap_or_default();
        assert!(pg_tables.contains("nullif(c.reltuples, -1)"), "{pg_tables}");
        assert!(pg_tables.contains("TRUNCATE,REFERENCES,TRIGGER"), "{pg_tables}");

        // ---- 估算行数：只有 MySQL 族有可用统计；0 必须压成 null ----
        let rows = of(ConnectionKind::Mysql)
            .table_rows_sql("demo")
            .unwrap_or_default();
        assert!(rows.contains("then null else table_rows end"), "{rows}");
        assert!(rows.contains("'demo'"), "{rows}");
        assert_eq!(
            of(ConnectionKind::Tidb).table_rows_sql("demo"),
            of(ConnectionKind::Mysql).table_rows_sql("demo")
        );
        assert!(of(ConnectionKind::Postgresql).table_rows_sql("demo").is_none());
        assert!(of(ConnectionKind::Sqlite).table_rows_sql("demo").is_none());

        // ---- 表选项：Doris 的 `catalog.库` 必须剥成裸库名 ----
        let doris = of(ConnectionKind::Doris)
            .table_options_sql("clickhouse_catalog.demo")
            .unwrap_or_default();
        assert!(doris.contains("'demo'"), "{doris}");
        assert!(!doris.contains("clickhouse_catalog"), "{doris}");
        // 视图的 table_comment 是 'VIEW' 字面量，不能当注释带出去
        assert!(doris.contains("table_comment = 'VIEW'"), "{doris}");

        // Oracle / DM 的别名必须加引号（rows / comment 都是关键字）
        assert!(of(ConnectionKind::Oracle)
            .table_options_sql("SYSTEM")
            .unwrap_or_default()
            .contains("as \"rows\""));
        assert!(of(ConnectionKind::Dm)
            .table_options_sql("SYSTEM")
            .unwrap_or_default()
            .contains("as \"comment\""));
        // CH 不能查 create_time（system.tables 没这列，一写就整条报 Code 47）
        let ch = of(ConnectionKind::Clickhouse)
            .table_options_sql("default")
            .unwrap_or_default();
        assert!(ch.contains("system.tables"), "{ch}");
        assert!(!ch.contains("create_time"), "{ch}");
        // SQLite / Derby 没有表注释这个概念
        assert!(of(ConnectionKind::Sqlite).table_options_sql("main").is_none());
        assert!(of(ConnectionKind::Derby).table_options_sql("APP").is_none());
        assert!(of(ConnectionKind::H2)
            .table_options_sql("PUBLIC")
            .unwrap_or_default()
            .contains("remarks"));

        // ---- 索引 ----
        // MySQL 的 group_concat 带 `order by seq_in_index`；Doris 不认这个写法（会整条 500）
        let mysql_idx = of(ConnectionKind::Mysql).indexes_sql().unwrap_or("");
        assert!(mysql_idx.contains("order by seq_in_index"), "{mysql_idx}");
        let doris_idx = of(ConnectionKind::Doris).indexes_sql().unwrap_or("");
        assert!(!doris_idx.contains("order by seq_in_index"), "{doris_idx}");
        assert!(
            doris_idx.contains("group_concat(column_name, ',')"),
            "{doris_idx}"
        );
        // DB2 的索引名是 INDNAME；写成 INDEXNAME 整条 500
        let db2_idx = of(ConnectionKind::Db2).indexes_sql().unwrap_or("");
        assert!(db2_idx.contains("i.indname as"), "{db2_idx}");
        assert!(!db2_idx.contains("indexname"), "{db2_idx}");
        // Derby 没有 syscat schema，索引在 sys.sysconglomerates
        let derby_idx = of(ConnectionKind::Derby).indexes_sql().unwrap_or("");
        assert!(derby_idx.contains("sysconglomerates"), "{derby_idx}");
        // T-SQL 的 `schema` 别名必须带引号，否则语法错
        assert!(of(ConnectionKind::Sqlserver)
            .indexes_sql()
            .unwrap_or("")
            .contains("as \"schema\""));
        // CH 的跳数索引列在 expr 上，且不能加 database 过滤（加了就 0 行）
        let ch_idx = of(ConnectionKind::Clickhouse).indexes_sql().unwrap_or("");
        assert!(ch_idx.contains("expr as \"columns\""), "{ch_idx}");
        assert!(!ch_idx.contains("where"), "{ch_idx}");
        assert!(of(ConnectionKind::Duckdb).indexes_sql().is_none());
    }

    /// `literal` 是值转义的唯一出口：反斜杠要不要翻倍按族定。
    #[test]
    fn literal_escapes_by_family() {
        let win = "C:\\temp\\";
        // MySQL 把反斜杠当转义符，不翻倍会把收尾引号吃掉、语句被截断
        assert_eq!(of(ConnectionKind::Mysql).literal(win), "'C:\\\\temp\\\\'");
        assert_eq!(of(ConnectionKind::Postgresql).literal(win), "'C:\\temp\\'");
        // 单引号一律翻倍
        assert_eq!(of(ConnectionKind::Oracle).literal("O'Brien"), "'O''Brien'");
    }

    /// 对象清单五兄弟：schema / 表 / 例程 / 触发器 / 事件。
    ///
    /// 这里**逐个断言 `Absent` 与 `Unwritten`**：trait 给了默认实现，漏搬一个分支不会
    /// 编译失败 —— 上一轮 `DerbyOps::indexes_sql` 就是这么漏的（界面上只是页签空白）。
    #[test]
    fn object_lists_split_by_family() {
        fn sql_text(meta: Meta) -> String {
            match meta {
                Meta::Sql(sql) => sql,
                _ => String::new(),
            }
        }

        // ---- schema 清单 ----
        let schemas = |kind: ConnectionKind| of(kind).schemas_sql().unwrap_or("");
        assert!(schemas(ConnectionKind::Postgresql).contains("has_schema_privilege"));
        // 金仓的系统模式叫 `sys_catalog`，过滤条件也必须把 `sys_` 前缀滤掉
        // （PG 那份只滤 `pg_`，照抄会把金仓自己的系统模式全列进树里）
        let kb_schemas = schemas(ConnectionKind::Kingbase);
        assert!(kb_schemas.contains("sys_catalog.sys_namespace"), "{kb_schemas}");
        assert!(kb_schemas.contains("sys\\_%"), "{kb_schemas}");
        // 九个固定数据库角色的同名 schema 是噪音，必须滤掉
        assert!(schemas(ConnectionKind::Sqlserver).contains("db_denydatawriter"));
        assert!(schemas(ConnectionKind::Oracle).contains("all_objects"));
        assert!(schemas(ConnectionKind::H2).contains("information_schema.schemata"));
        // MySQL 的「库」就是 schema，树上不请求这一层
        assert!(of(ConnectionKind::Mysql).schemas_sql().is_none());
        assert!(of(ConnectionKind::Db2).schemas_sql().is_none());

        // ---- schema 下的表 ----
        let tis = |kind: ConnectionKind| of(kind).tables_in_schema_sql("public").unwrap_or_default();
        let pg = tis(ConnectionKind::Postgresql);
        assert!(pg.contains("has_table_privilege"), "{pg}");
        assert!(pg.contains("'public'"), "{pg}");
        // 金仓改走 sys_class / sys_namespace，权限函数名不变
        let kb_tables = tis(ConnectionKind::Kingbase);
        assert!(kb_tables.contains("sys_class"), "{kb_tables}");
        assert!(!kb_tables.contains("from pg_"), "{kb_tables}");
        assert!(kb_tables.contains("'public'"), "{kb_tables}");
        // SQL Server 的注释只能自己 join 扩展属性（mssql-jdbc 不填 REMARKS）
        let ms = tis(ConnectionKind::Sqlserver);
        assert!(ms.contains("MS_Description"), "{ms}");
        assert!(ms.contains("schema_id('public')"), "{ms}");
        // Oracle 必须带 type 列，否则视图会混进「表」里
        let ora = tis(ConnectionKind::Oracle);
        assert!(ora.contains("'VIEW' as \"type\""), "{ora}");
        // 这条 SQL 必须是一整行：续行拼接被搅坏过一次（ORA-03405）
        assert!(!ora.contains('\n'), "{ora}");
        assert!(of(ConnectionKind::Mysql).tables_in_schema_sql("x").is_none());

        // ---- 例程 ----
        assert!(sql_text(of(ConnectionKind::Mysql).procedures(Some("demo")))
            .contains("routine_schema = database()"));
        // 明确「这库没有例程」的三家是 Absent（空数组是事实），不是 Unwritten
        for kind in [
            ConnectionKind::Sqlite,
            ConnectionKind::Doris,
            ConnectionKind::Clickhouse,
        ] {
            assert!(matches!(of(kind).procedures(None), Meta::Absent), "{kind:?}");
        }
        // PG：带 schema 只看它，不带就排除系统模式
        let pg_some = sql_text(of(ConnectionKind::Postgresql).procedures(Some("app")));
        assert!(pg_some.contains("n.nspname = 'app'"), "{pg_some}");
        assert!(pg_some.contains("has_function_privilege"), "{pg_some}");
        let pg_none = sql_text(of(ConnectionKind::Postgresql).procedures(None));
        assert!(pg_none.contains("pg_catalog"), "{pg_none}");
        // Oracle：带 schema 走 all_objects + owner，不带退回 user_objects
        let ora_some = sql_text(of(ConnectionKind::Oracle).procedures(Some("system")));
        assert!(
            ora_some.contains("all_objects") && ora_some.contains("upper('system')"),
            "{ora_some}"
        );
        assert!(sql_text(of(ConnectionKind::Oracle).procedures(None)).contains("user_objects"));
        // DM 与 Oracle 同口径：带 schema 走 all_objects + owner，不带才退回 user_objects。
        // （2026-11-18 真机核对过 `all_objects` + `upper(owner)` 可用；不吃 schema 的后果是
        //  树上点 DEMO 列出 SYSDBA 名下的一堆系统过程，用户看到「这模式里怎么全是系统过程」）
        let dm_some = sql_text(of(ConnectionKind::Dm).procedures(Some("APP")));
        assert!(
            dm_some.contains("all_objects") && dm_some.contains("upper('APP')"),
            "{dm_some}"
        );
        assert!(sql_text(of(ConnectionKind::Dm).procedures(None)).contains("user_objects"));
        assert!(sql_text(of(ConnectionKind::Db2).procedures(None)).contains("syscat.routines"));
        assert!(sql_text(of(ConnectionKind::Derby).procedures(None)).contains("sys.sysaliases"));
        assert!(sql_text(of(ConnectionKind::Sqlserver).procedures(None)).contains("sys.procedures"));
        assert!(sql_text(of(ConnectionKind::H2).procedures(None)).contains("information_schema.routines"));
        assert!(matches!(
            of(ConnectionKind::Duckdb).procedures(None),
            Meta::Unwritten
        ));

        // ---- 触发器 ----
        assert!(sql_text(of(ConnectionKind::Sqlite).triggers(None)).contains("sqlite_master"));
        assert!(sql_text(of(ConnectionKind::Mysql).triggers(None)).contains("information_schema.triggers"));
        let pg_t = sql_text(of(ConnectionKind::Postgresql).triggers(Some("app")));
        assert!(
            pg_t.contains("not t.tgisinternal") && pg_t.contains("'app'"),
            "{pg_t}"
        );
        assert!(sql_text(of(ConnectionKind::Sqlserver).triggers(None)).contains("sys.triggers"));
        assert!(sql_text(of(ConnectionKind::Oracle).triggers(Some("system"))).contains("all_triggers"));
        // Derby 的系统表里没有所属表名，清单只给名字（少一列好过整个清单 42X05）
        let derby_t = sql_text(of(ConnectionKind::Derby).triggers(None));
        assert!(derby_t.contains("sys.systriggers"), "{derby_t}");
        assert!(!derby_t.contains("\"table\""), "{derby_t}");
        // H2 / ClickHouse / Doris 没有触发器
        for kind in [
            ConnectionKind::H2,
            ConnectionKind::Clickhouse,
            ConnectionKind::Doris,
        ] {
            assert!(matches!(of(kind).triggers(None), Meta::Absent), "{kind:?}");
        }

        // ---- 事件：只有 MySQL 有事件调度器 ----
        assert!(sql_text(of(ConnectionKind::Mysql).events()).contains("information_schema.events"));
        for kind in [
            ConnectionKind::Postgresql,
            ConnectionKind::Oracle,
            ConnectionKind::Sqlite,
            ConnectionKind::H2,
        ] {
            assert!(matches!(of(kind).events(), Meta::Absent), "{kind:?}");
        }
    }

    /// 用户与角色的七条只读清单。
    ///
    /// 重点是两件事：①**MariaDB 与 MySQL 必须真的分叉**（`is_role` / `roles_mapping`）；
    /// ②PG 那条详情不能再退回 SQL Server 的写法（`isnull` / `'a' + 'b'` 在 PG 上直接 42883）。
    #[test]
    fn user_management_lists_by_family() {
        fn sql_text(meta: Meta) -> String {
            match meta {
                Meta::Sql(sql) => sql,
                _ => String::new(),
            }
        }

        // ---- 用户清单 ----
        let mysql_users = sql_text(of(ConnectionKind::Mysql).users());
        assert!(mysql_users.contains("super_priv"), "{mysql_users}");
        // MySQL 8 的角色也是账号：靠「锁定 + 空认证串」排除
        assert!(mysql_users.contains("account_locked"), "{mysql_users}");
        let mariadb_users = sql_text(of(ConnectionKind::Mariadb).users());
        assert!(mariadb_users.contains("is_role"), "{mariadb_users}");
        assert_ne!(mysql_users, mariadb_users);
        // MariaDB 仍在 mysql 族（日志/分组口径不变），只是那几处真差异分叉
        assert_eq!(of(ConnectionKind::Mariadb).family(), "mysql");
        assert!(of(ConnectionKind::Mariadb).mariadb_flag());
        assert!(!of(ConnectionKind::Mysql).mariadb_flag());
        // Doris 的管理员标志是 Admin_priv（没有 Super_priv）
        assert!(sql_text(of(ConnectionKind::Doris).users()).contains("admin_priv"));
        assert!(sql_text(of(ConnectionKind::Postgresql).users()).contains("rolcanlogin"));
        assert!(sql_text(of(ConnectionKind::Sqlserver).users()).contains("sys.server_principals"));
        assert!(sql_text(of(ConnectionKind::Oracle).users()).contains("all_users"));
        assert!(sql_text(of(ConnectionKind::Clickhouse).users()).contains("system.users"));
        assert!(sql_text(of(ConnectionKind::H2).users()).contains("information_schema.users"));
        assert!(sql_text(of(ConnectionKind::Derby).users()).contains("sys.sysusers"));
        assert!(sql_text(of(ConnectionKind::Db2).users()).contains("syscat.dbauth"));
        // 没有账号体系的：空数组是事实，不是"没实现"
        for kind in [ConnectionKind::Sqlite, ConnectionKind::Duckdb] {
            assert!(matches!(of(kind).users(), Meta::Absent), "{kind:?}");
        }

        // ---- 用户详情 ----
        // MySQL 的账号是 (user, host)：按**最后一个** @ 切开再比两列
        let mysql_info = sql_text(of(ConnectionKind::Mysql).user_info("a@b@%"));
        assert!(mysql_info.contains("user = 'a@b'"), "{mysql_info}");
        assert!(mysql_info.contains("host = '%'"), "{mysql_info}");
        // 只给用户名时不限制 host（否则与 'x'@'localhost' 擦肩而过）
        let mysql_bare = sql_text(of(ConnectionKind::Mysql).user_info("demo"));
        assert!(!mysql_bare.contains("host ="), "{mysql_bare}");
        // PG：拼接用 ||、空值用 coalesce —— isnull / 'a' + 'b' 是 SQL Server 的写法，PG 上 42883
        let pg_info = sql_text(of(ConnectionKind::Postgresql).user_info("app"));
        assert!(pg_info.contains("coalesce"), "{pg_info}");
        assert!(!pg_info.contains("isnull"), "{pg_info}");
        assert!(pg_info.contains("quote_ident"), "{pg_info}");
        assert!(sql_text(of(ConnectionKind::Sqlserver).user_info("sa")).contains("type_desc"));
        assert!(sql_text(of(ConnectionKind::Oracle).user_info("system")).contains("dba_users"));
        assert!(sql_text(of(ConnectionKind::Dm).user_info("SYSDBA")).contains("dba_users"));
        assert!(sql_text(of(ConnectionKind::Clickhouse).user_info("default")).contains("system.users"));
        // CH 的 auth_type 是 Enum8，不能直接喂 arrayStringConcat
        assert!(sql_text(of(ConnectionKind::Clickhouse).user_info("default")).contains("toString(auth_type)"));
        assert!(sql_text(of(ConnectionKind::H2).user_info("\"\"")).contains("table_privileges"));
        assert!(matches!(
            of(ConnectionKind::Sqlite).user_info("x"),
            Meta::Unwritten
        ));

        // ---- 用户权限明细 ----
        let mysql_grants = sql_text(of(ConnectionKind::Mysql).user_grants("u", "%"));
        assert!(mysql_grants.contains("user_privileges"), "{mysql_grants}");
        // 例程级授权只在 mysql.procs_priv；Doris 没有这张表，查了必炸
        assert!(mysql_grants.contains("mysql.procs_priv"), "{mysql_grants}");
        let doris_grants = sql_text(of(ConnectionKind::Doris).user_grants("u", "%"));
        assert!(!doris_grants.contains("procs_priv"), "{doris_grants}");
        assert!(sql_text(of(ConnectionKind::Postgresql).user_grants("u", "")).contains("role_table_grants"));
        assert!(sql_text(of(ConnectionKind::Clickhouse).user_grants("u", "")).starts_with("show grants for"));
        assert!(sql_text(of(ConnectionKind::Oracle).user_grants("u", "")).contains("dba_sys_privs"));
        assert!(sql_text(of(ConnectionKind::H2).user_grants("\"\"", "")).contains("table_privileges"));

        // ---- 角色 ----
        assert!(sql_text(of(ConnectionKind::Mysql).user_roles("u", "%")).contains("role_edges"));
        assert!(sql_text(of(ConnectionKind::Mariadb).user_roles("u", "%")).contains("roles_mapping"));
        assert!(sql_text(of(ConnectionKind::Postgresql).user_roles("u", "")).contains("pg_auth_members"));
        assert!(sql_text(of(ConnectionKind::Clickhouse).user_roles("u", "")).contains("system.role_grants"));

        assert!(sql_text(of(ConnectionKind::Mysql).role_info("r")).contains("mysql.user"));
        assert!(sql_text(of(ConnectionKind::Oracle).role_info("r")).contains("dba_roles"));
        // Doris 这版不支持编辑角色权限，但仍给一行，别把表单卡在"没有找到"
        assert!(sql_text(of(ConnectionKind::Doris).role_info("r")).contains("as name"));

        // MySQL 的角色就是账号：角色权限直接复用用户权限那三条视图
        assert_eq!(
            sql_text(of(ConnectionKind::Mysql).role_grants("r", "%")),
            sql_text(of(ConnectionKind::Mysql).user_grants("r", "%"))
        );
        // MariaDB：information_schema 不列角色，只能 show grants
        assert!(sql_text(of(ConnectionKind::Mariadb).role_grants("r", "")).starts_with("show grants for"));
        assert!(sql_text(of(ConnectionKind::Postgresql).role_grants("r", "")).contains("aclexplode"));
        assert!(sql_text(of(ConnectionKind::Oracle).role_grants("r", "")).contains("role_sys_privs"));
        assert!(sql_text(of(ConnectionKind::Clickhouse).role_grants("r", "")).contains("system.grants"));
        assert!(matches!(
            of(ConnectionKind::Doris).role_grants("r", ""),
            Meta::Unwritten
        ));

        // ---- 角色清单 ----
        assert!(sql_text(of(ConnectionKind::Mysql).role_inventory()).contains("role_edges"));
        assert!(sql_text(of(ConnectionKind::Mariadb).role_inventory()).contains("roles_mapping"));
        // Oracle 只列用户自己建的角色（DBA 那 100+ 个自带角色是噪音，删掉是真事故）
        let ora_roles = sql_text(of(ConnectionKind::Oracle).role_inventory());
        assert!(ora_roles.contains("oracle_maintained"), "{ora_roles}");
        assert!(sql_text(of(ConnectionKind::Clickhouse).role_inventory()).contains("system.roles"));
        assert_eq!(sql_text(of(ConnectionKind::Doris).role_inventory()), "show roles");
        // PG 的纯角色 = 不能登录的，且排除 pg_* 系统预置角色
        let pg_roles = sql_text(of(ConnectionKind::Postgresql).role_inventory());
        assert!(pg_roles.contains("not rolcanlogin"), "{pg_roles}");
        assert!(pg_roles.contains("'pg_'"), "{pg_roles}");
        assert!(matches!(
            of(ConnectionKind::Sqlserver).role_inventory(),
            Meta::Unwritten
        ));
    }

    /// 用户增删改（`user_action_sql`）：钉住三态与拼接形状。
    ///
    /// 最重要的两条：①**口令缺失时新账号必须 Unwritten**（悄悄建出空口令账号是事故）；
    /// ②撤销是**差量点名**，不是「全删重授」。
    #[test]
    fn user_action_sql_splits_by_family() {
        fn sql(meta: Meta) -> String {
            match meta {
                Meta::Sql(sql) => sql,
                _ => String::new(),
            }
        }
        let run = |kind: ConnectionKind,
                   action: &str,
                   user: &str,
                   password: &str,
                   roles: &[String],
                   privileges: &UserPrivileges,
                   revokes: &[GrantRevoke],
                   attrs: &UserAttrs| {
            of(kind).user_action_sql(
                action, false, user, password, "%", roles, privileges, revokes, false, attrs,
            )
        };

        let no_roles: Vec<String> = Vec::new();
        let no_revokes: Vec<GrantRevoke> = Vec::new();
        let plain = UserPrivileges::default();
        let plain_attrs = UserAttrs::default();

        // ---- SQL Server：登录名是服务器级主体，必须带口令 ----
        let ms_create = sql(run(
            ConnectionKind::Sqlserver,
            "create",
            "app",
            "pw",
            &no_roles,
            &plain,
            &no_revokes,
            &plain_attrs,
        ));
        assert!(ms_create.contains("create login"), "{ms_create}");
        assert!(ms_create.contains("password = 'pw'"), "{ms_create}");
        assert!(matches!(
            run(
                ConnectionKind::Sqlserver,
                "create",
                "app",
                "",
                &no_roles,
                &plain,
                &no_revokes,
                &plain_attrs
            ),
            Meta::Unwritten
        ));
        // 「全局」在 SQL Server 就是服务器角色
        let mut ms_privs = UserPrivileges::default();
        ms_privs.global = vec!["sysadmin".to_string()];
        let ms_roles = sql(run(
            ConnectionKind::Sqlserver,
            "create",
            "app",
            "pw",
            &no_roles,
            &ms_privs,
            &no_revokes,
            &plain_attrs,
        ));
        assert!(ms_roles.contains("alter server role"), "{ms_roles}");
        assert_eq!(
            sql(run(
                ConnectionKind::Sqlserver,
                "drop",
                "app",
                "",
                &no_roles,
                &plain,
                &no_revokes,
                &plain_attrs
            )),
            "drop login app"
        );

        // ---- H2：空用户名必须写成空标识符 `""`（否则拼出 `grant … to ` 42001） ----
        let h2_create = sql(run(
            ConnectionKind::H2,
            "create",
            "\"\"",
            "pw",
            &no_roles,
            &plain,
            &no_revokes,
            &plain_attrs,
        ));
        assert!(h2_create.contains("create user \"\" password"), "{h2_create}");
        let mut h2_privs = UserPrivileges::default();
        h2_privs.tables = vec![(
            "PUBLIC".to_string(),
            "T".to_string(),
            vec!["select".to_string(), "delete".to_string()],
        )];
        let h2_grant = sql(run(
            ConnectionKind::H2,
            "alter",
            "app",
            "",
            &no_roles,
            &h2_privs,
            &no_revokes,
            &plain_attrs,
        ));
        assert!(
            h2_grant.contains("grant SELECT, DELETE on \"PUBLIC\".T"),
            "{h2_grant}"
        );
        // 白名单外的权限名不拼（H2 没有 grant all privileges 那套）
        let mut h2_bad = UserPrivileges::default();
        h2_bad.tables = vec![(
            "PUBLIC".to_string(),
            "T".to_string(),
            vec!["alter routine".to_string()],
        )];
        assert!(matches!(
            run(
                ConnectionKind::H2,
                "alter",
                "app",
                "",
                &no_roles,
                &h2_bad,
                &no_revokes,
                &plain_attrs
            ),
            Meta::Unwritten
        ));

        // ---- ClickHouse：没有 (user, host)；角色是独立主体 ----
        let ch_create = sql(run(
            ConnectionKind::Clickhouse,
            "create",
            "app",
            "pw",
            &no_roles,
            &plain,
            &no_revokes,
            &plain_attrs,
        ));
        assert!(ch_create.contains("identified by 'pw'"), "{ch_create}");
        assert!(!ch_create.contains('@'), "{ch_create}");
        let ch_revokes = vec![GrantRevoke {
            scope: "role".to_string(),
            privilege: "r1".to_string(),
            ..Default::default()
        }];
        let ch_revoke = sql(run(
            ConnectionKind::Clickhouse,
            "alter",
            "app",
            "",
            &no_roles,
            &plain,
            &ch_revokes,
            &plain_attrs,
        ));
        // 角色是独立一层：`revoke r1 from u`，没有 `on …`
        assert!(ch_revoke.contains("from"), "{ch_revoke}");
        assert!(!ch_revoke.contains(" on "), "{ch_revoke}");

        // ---- Oracle：必须带口令；建号顺手给 create session ----
        assert!(matches!(
            run(
                ConnectionKind::Oracle,
                "create",
                "app",
                "",
                &no_roles,
                &plain,
                &no_revokes,
                &plain_attrs
            ),
            Meta::Unwritten
        ));
        let ora_create = sql(run(
            ConnectionKind::Oracle,
            "create",
            "app",
            "pw@1",
            &no_roles,
            &plain,
            &no_revokes,
            &plain_attrs,
        ));
        // 口令要用**双引号标识符**：单引号字面量在 Oracle 上 ORA-00988
        assert!(ora_create.contains("identified by \"pw@1\""), "{ora_create}");
        assert!(ora_create.contains("grant create session"), "{ora_create}");
        // 自定义角色：普通名不加引号（服务端折大写），带特殊字符的原样加引号
        let ora_roles = vec!["my-role".to_string()];
        let ora_grant = sql(run(
            ConnectionKind::Oracle,
            "alter",
            "app",
            "",
            &ora_roles,
            &plain,
            &no_revokes,
            &plain_attrs,
        ));
        assert!(ora_grant.contains("grant \"my-role\" to app"), "{ora_grant}");
        let ora_revokes = vec![GrantRevoke {
            scope: "object".to_string(),
            privilege: "select".to_string(),
            owner: "hr".to_string(),
            object: "emp".to_string(),
            ..Default::default()
        }];
        let ora_revoke = sql(run(
            ConnectionKind::Oracle,
            "alter",
            "app",
            "",
            &no_roles,
            &plain,
            &ora_revokes,
            &plain_attrs,
        ));
        assert!(
            ora_revoke.contains("revoke SELECT on \"hr\".\"emp\""),
            "{ora_revoke}"
        );
        assert_eq!(
            sql(run(
                ConnectionKind::Oracle,
                "drop",
                "app",
                "",
                &no_roles,
                &plain,
                &no_revokes,
                &plain_attrs
            )),
            "drop user app"
        );

        // ---- 没实现 / 没验证过的一律 Unwritten（DM 的字典语法没真机验证） ----
        for kind in [ConnectionKind::Dm, ConnectionKind::Sqlite, ConnectionKind::Duckdb] {
            assert!(matches!(
                run(
                    kind,
                    "create",
                    "app",
                    "pw",
                    &no_roles,
                    &plain,
                    &no_revokes,
                    &plain_attrs
                ),
                Meta::Unwritten
            ));
        }
    }

    /// MySQL 系三方言与 PG 系两方言的用户增删改。
    ///
    /// 这一支的坑全在**角色怎么写**：MySQL 的角色是账号形状 `'r'@'h'`，
    /// MariaDB 的角色没有 host（多写 `@'%'` 是 1064，真机验证过），Doris 的主体
    /// 必须写 `TO ROLE 'r'` —— 共用一个拼装函数后，flavor 分叉就是唯一差异点。
    #[test]
    fn user_action_sql_mysql_pg_flavors() {
        fn sql(meta: Meta) -> String {
            match meta {
                Meta::Sql(sql) => sql,
                _ => String::new(),
            }
        }
        let run = |kind: ConnectionKind,
                   action: &str,
                   as_role: bool,
                   user: &str,
                   password: &str,
                   roles: &[String],
                   privileges: &UserPrivileges,
                   revokes: &[GrantRevoke]| {
            of(kind).user_action_sql(
                action,
                as_role,
                user,
                password,
                "%",
                roles,
                privileges,
                revokes,
                false,
                &UserAttrs::default(),
            )
        };

        let no_roles: Vec<String> = Vec::new();
        let no_revokes: Vec<GrantRevoke> = Vec::new();
        let plain = UserPrivileges::default();

        // ---- 建号：账号是 (user, host) 二元组，主体用单引号字符串写法 ----
        let create = sql(run(
            ConnectionKind::Mysql,
            "create",
            false,
            "app",
            "pw",
            &no_roles,
            &plain,
            &no_revokes,
        ));
        assert!(create.contains("create user app@'%'"), "{create}");
        assert!(create.contains("identified by 'pw'"), "{create}");
        assert_eq!(
            sql(run(
                ConnectionKind::Mysql,
                "drop",
                false,
                "app",
                "",
                &no_roles,
                &plain,
                &no_revokes
            )),
            "drop user app@'%'"
        );

        // ---- 建角色：三种口径必须分叉 ----
        assert_eq!(
            sql(run(
                ConnectionKind::Mysql,
                "create-role",
                false,
                "r",
                "",
                &no_roles,
                &plain,
                &no_revokes
            )),
            "create role 'r'@'%'"
        );
        assert_eq!(
            sql(run(
                ConnectionKind::Mariadb,
                "create-role",
                false,
                "r",
                "",
                &no_roles,
                &plain,
                &no_revokes
            )),
            "create role 'r'"
        );
        // Doris 的角色名是标识符（字符串写法 1064）
        assert_eq!(
            sql(run(
                ConnectionKind::Doris,
                "create-role",
                false,
                "r",
                "",
                &no_roles,
                &plain,
                &no_revokes
            )),
            "create role r"
        );

        // ---- 编辑角色的授权：主体也是三种口径 ----
        let mut db_privs = UserPrivileges::default();
        db_privs.databases = vec![("demo".to_string(), vec!["SELECT".to_string()])];
        let mysql_role = sql(run(
            ConnectionKind::Mysql,
            "alter",
            true,
            "app",
            "",
            &no_roles,
            &db_privs,
            &no_revokes,
        ));
        assert!(
            mysql_role.contains("grant SELECT on demo.* to 'app'@'%'"),
            "{mysql_role}"
        );
        let doris_role = sql(run(
            ConnectionKind::Doris,
            "alter",
            true,
            "app",
            "",
            &no_roles,
            &db_privs,
            &no_revokes,
        ));
        assert!(doris_role.contains("to role 'app'"), "{doris_role}");

        // ---- 授角色给用户：MySQL 带 host，MariaDB / Doris 不带 ----
        let roles = vec!["r1".to_string()];
        let mysql_grant = sql(run(
            ConnectionKind::Mysql,
            "alter",
            false,
            "app",
            "",
            &roles,
            &plain,
            &no_revokes,
        ));
        assert!(
            mysql_grant.contains("grant 'r1'@'%' to 'app'@'%'"),
            "{mysql_grant}"
        );
        let mariadb_grant = sql(run(
            ConnectionKind::Mariadb,
            "alter",
            false,
            "app",
            "",
            &roles,
            &plain,
            &no_revokes,
        ));
        assert!(
            mariadb_grant.contains("grant 'r1' to 'app'@'%'"),
            "{mariadb_grant}"
        );
        assert!(!mariadb_grant.contains("grant 'r1'@"), "{mariadb_grant}");

        // ---- 撤销角色：同口径分叉，且是差量点名 ----
        // 角色撤销的形状：privilege/owner 都是角色名（循环顶部按 privilege 挡空行），
        // object = 角色 host（与 role_edges 一致）
        let revokes = vec![GrantRevoke {
            scope: "role".to_string(),
            privilege: "r1".to_string(),
            owner: "r1".to_string(),
            object: "%".to_string(),
            ..Default::default()
        }];
        let mysql_revoke = sql(run(
            ConnectionKind::Mysql,
            "alter",
            false,
            "app",
            "",
            &no_roles,
            &plain,
            &revokes,
        ));
        assert!(
            mysql_revoke.contains("revoke 'r1'@'%' from 'app'@'%'"),
            "{mysql_revoke}"
        );
        let mariadb_revoke = sql(run(
            ConnectionKind::Mariadb,
            "alter",
            false,
            "app",
            "",
            &no_roles,
            &plain,
            &revokes,
        ));
        assert!(mariadb_revoke.contains("revoke 'r1' from"), "{mariadb_revoke}");
        assert!(!mariadb_revoke.contains("revoke 'r1'@"), "{mariadb_revoke}");

        // ---- 例程级：`on procedure `db`.`p`` 的类型字是语法的一部分 ----
        let mut rt_privs = UserPrivileges::default();
        rt_privs.routines = vec![(
            "demo".to_string(),
            "p".to_string(),
            "PROCEDURE".to_string(),
            vec!["EXECUTE".to_string()],
        )];
        let rt = sql(run(
            ConnectionKind::Mysql,
            "alter",
            false,
            "app",
            "",
            &no_roles,
            &rt_privs,
            &no_revokes,
        ));
        assert!(rt.contains("grant EXECUTE on procedure demo.p to"), "{rt}");

        // ---- PG 系：用户 = 带 login 的角色，两方言一字不差 ----
        let pg_create = sql(run(
            ConnectionKind::Postgresql,
            "create",
            false,
            "app",
            "pw",
            &no_roles,
            &plain,
            &no_revokes,
        ));
        assert!(
            pg_create.contains("create role app login password 'pw'"),
            "{pg_create}"
        );
        let mut pg_privs = UserPrivileges::default();
        pg_privs.tables = vec![(
            "public".to_string(),
            "orders".to_string(),
            vec!["SELECT".to_string(), "DELETE".to_string()],
        )];
        let pg_grant = sql(run(
            ConnectionKind::Postgresql,
            "alter",
            false,
            "app",
            "",
            &no_roles,
            &pg_privs,
            &no_revokes,
        ));
        // `public` 是 PG 保留字，quote 会加引号；普通名不加
        assert!(
            pg_grant.contains("grant select, delete on table \"public\".orders to app"),
            "{pg_grant}"
        );
        // 角色成员关系撤销：少了这条，取消勾选一个角色会静默无效
        let pg_revokes = vec![GrantRevoke {
            scope: "role".to_string(),
            privilege: "app_admin".to_string(),
            ..Default::default()
        }];
        let pg_revoke = sql(run(
            ConnectionKind::Postgresql,
            "alter",
            false,
            "app",
            "",
            &no_roles,
            &plain,
            &pg_revokes,
        ));
        assert!(pg_revoke.contains("revoke app_admin from app"), "{pg_revoke}");
        assert_eq!(
            sql(run(
                ConnectionKind::Kingbase,
                "create-role",
                false,
                "app",
                "",
                &no_roles,
                &plain,
                &no_revokes
            )),
            "create role app"
        );
        assert_eq!(
            sql(run(
                ConnectionKind::Kingbase,
                "drop-role",
                false,
                "app",
                "",
                &no_roles,
                &plain,
                &no_revokes
            )),
            "drop role app"
        );
    }
    /// 建表语句与表级动作：ddl 的"没实例可验证就不猜"（DB2/Derby/DuckDB → Unwritten）
    /// 与 table_action 的方言差异（CH 的 TRUNCATE、DB2 的 IMMEDIATE、SQLite 的退化
    /// DELETE、四家 RENAME 写法）。
    #[test]
    fn ddl_and_table_action_by_family() {
        fn sql(meta: Meta) -> String {
            match meta {
                Meta::Sql(sql) => sql,
                _ => String::new(),
            }
        }

        // ---- ddl：各家只算自己要的量；同族一字不差 ----
        for kind in [
            ConnectionKind::Mysql,
            ConnectionKind::Mariadb,
            ConnectionKind::Doris,
            ConnectionKind::Clickhouse,
        ] {
            let d = sql(of(kind).ddl("t1", None));
            assert!(d.contains("show create table t1"), "{kind:?}: {d}");
        }
        assert_eq!(sql(of(ConnectionKind::H2).ddl("t1", None)), "script nodata");
        let ms = sql(of(ConnectionKind::Sqlserver).ddl("t1", None));
        assert!(ms.contains("object_definition(object_id('t1'))"), "{ms}");
        assert!(ms.contains("from sys.tables t"), "{ms}");
        let pg_none = sql(of(ConnectionKind::Postgresql).ddl("t1", None));
        assert!(pg_none.contains("pg_get_viewdef"), "{pg_none}");
        assert!(pg_none.contains("nspname = current_schema()"), "{pg_none}");
        let pg_schema = sql(of(ConnectionKind::Postgresql).ddl("t1", Some("public")));
        assert!(pg_schema.contains("nspname = 'public'"), "{pg_schema}");
        // 金仓同一份 DDL 拼装，目录/函数换成 sys_*（`sys_get_viewdef` / `sys_class`）
        let kb_ddl = sql(of(ConnectionKind::Kingbase).ddl("t1", None));
        assert!(kb_ddl.contains("sys_get_viewdef"), "{kb_ddl}");
        assert!(kb_ddl.contains("from sys_class"), "{kb_ddl}");
        assert!(!kb_ddl.contains("from pg_"), "{kb_ddl}");
        let ora_schema = sql(of(ConnectionKind::Oracle).ddl("t1", Some("hr")));
        assert!(ora_schema.contains("upper('hr')"), "{ora_schema}");
        assert!(ora_schema.contains("all_views"), "{ora_schema}");
        let ora_self = sql(of(ConnectionKind::Oracle).ddl("t1", None));
        assert!(ora_self.contains("user_views"), "{ora_self}");
        assert!(!ora_self.contains("all_views"), "{ora_self}");
        // 达梦：`user_tab_columns` 只认会话当前模式，而达梦上 schema 切换不生效
        //（真机：database=DEMO 时 current_schema 仍是 SYSDBA）⇒ 必须走 all_* + owner 探测
        let dm = sql(of(ConnectionKind::Dm).ddl("t1", None));
        assert!(dm.contains("all_tab_columns"), "{dm}");
        assert!(dm.contains("sys_context('USERENV','CURRENT_SCHEMA')"), "{dm}");
        let dm_schema = sql(of(ConnectionKind::Dm).ddl("t1", Some("demo")));
        assert!(dm_schema.contains("upper('demo')"), "{dm_schema}");
        assert!(!dm_schema.contains("user_tab_columns"), "{dm_schema}");
        let sq = sql(of(ConnectionKind::Sqlite).ddl("t1", None));
        assert!(sq.contains("sqlite_master"), "{sq}");
        // 没实例可验证就不猜
        for kind in [ConnectionKind::Db2, ConnectionKind::Derby, ConnectionKind::Duckdb] {
            assert!(matches!(of(kind).ddl("t1", None), Meta::Unwritten), "{kind:?}");
        }

        // ---- table_action：通用拼法 + 各家例外 ----
        // MySQL / MariaDB / DB2 / Derby / CH 的改名都是独立 RENAME TABLE
        for kind in [
            ConnectionKind::Mysql,
            ConnectionKind::Mariadb,
            ConnectionKind::Db2,
            ConnectionKind::Derby,
            ConnectionKind::Clickhouse,
        ] {
            let r = sql(of(kind).table_action("t1", "rename", Some("t2")));
            assert!(r.contains("rename table t1 to t2"), "{kind:?}: {r}");
        }
        // Doris 的 ALTER … RENAME 不带 TO（真机实测）
        assert_eq!(
            sql(of(ConnectionKind::Doris).table_action("t1", "rename", Some("t2"))),
            "alter table t1 rename t2"
        );
        // SQL Server 只有 sp_rename，参数是裸名字面量
        assert_eq!(
            sql(of(ConnectionKind::Sqlserver).table_action("t1", "rename", Some("t2"))),
            "exec sp_rename 't1', 't2'"
        );
        // PG / SQLite / Oracle / H2 都是 ALTER … RENAME TO
        for kind in [
            ConnectionKind::Postgresql,
            ConnectionKind::Sqlite,
            ConnectionKind::Oracle,
            ConnectionKind::H2,
        ] {
            let r = sql(of(kind).table_action("t1", "rename", Some("t2")));
            assert!(r.contains("alter table t1 rename to t2"), "{kind:?}: {r}");
        }
        // CH 的清空走 TRUNCATE 正路（DELETE FROM 是"轻量删除"，老版本直接报错）
        assert_eq!(
            sql(of(ConnectionKind::Clickhouse).table_action("t1", "clear", None)),
            "truncate table t1"
        );
        // DB2 的 TRUNCATE 必须带 IMMEDIATE
        assert_eq!(
            sql(of(ConnectionKind::Db2).table_action("T1", "truncate", None)),
            "truncate table T1 immediate"
        );
        // SQLite 没有 TRUNCATE，退化成 DELETE
        assert_eq!(
            sql(of(ConnectionKind::Sqlite).table_action("t1", "truncate", None)),
            "delete from t1"
        );
        // 其余方言 truncate / drop 是通用拼法
        assert_eq!(
            sql(of(ConnectionKind::Mysql).table_action("t1", "truncate", None)),
            "truncate table t1"
        );
        assert_eq!(
            sql(of(ConnectionKind::Postgresql).table_action("t1", "drop", None)),
            "drop table t1"
        );
        // 边界：rename 没给新名、未知动作 → Unwritten
        for kind in [ConnectionKind::Mysql, ConnectionKind::Postgresql] {
            assert!(matches!(
                of(kind).table_action("t1", "rename", None),
                Meta::Unwritten
            ));
            assert!(matches!(
                of(kind).table_action("t1", "nope", None),
                Meta::Unwritten
            ));
        }
    }
    /// DDL 附加语句与补注释：默认为空（内联方言 / 没有注释概念），
    /// `comment on` 六家共用一份拼装件，SQL Server 是扩展属性。
    #[test]
    fn ddl_extras_and_post_comments_by_family() {
        // ---- ddl_extras：只有 SQL Server 有附加语句 ----
        let ms = of(ConnectionKind::Sqlserver).ddl_extras("t1");
        assert_eq!(ms.len(), 3);
        assert!(
            ms[0].0.contains("注释") && ms[0].1.contains("sp_addextendedproperty"),
            "{}",
            ms[0].1
        );
        // 注意 SQL 里 'create ' 与 'index ' 是字符串拼接，不能连着搜
        assert!(
            ms[1].0.contains("索引") && ms[1].1.contains("sys.indexes"),
            "{}",
            ms[1].1
        );
        assert!(
            ms[2].0.contains("外键") && ms[2].1.contains("foreign key"),
            "{}",
            ms[2].1
        );
        for kind in [
            ConnectionKind::Mysql,
            ConnectionKind::Sqlite,
            ConnectionKind::Oracle,
            ConnectionKind::H2,
        ] {
            assert!(of(kind).ddl_extras("t1").is_empty(), "{kind:?}");
        }

        // ---- post_create_comments ----
        let mut cols = std::collections::HashMap::new();
        cols.insert("c1".to_string(), "列注释".to_string());
        // 默认空：内联方言与没有注释概念的方言
        for kind in [
            ConnectionKind::Mysql,
            ConnectionKind::Mariadb,
            ConnectionKind::Doris,
            ConnectionKind::Clickhouse,
            ConnectionKind::Sqlite,
            ConnectionKind::Derby,
        ] {
            let out = of(kind).post_create_comments("t1", &cols, Some("表注释"), "");
            assert!(out.is_empty(), "{kind:?}: {out:?}");
        }
        // comment-on 六家：共用一份拼装件
        for kind in [
            ConnectionKind::Postgresql,
            ConnectionKind::Kingbase,
            ConnectionKind::Oracle,
            ConnectionKind::Dm,
            ConnectionKind::Db2,
            ConnectionKind::H2,
        ] {
            let out = of(kind).post_create_comments("t1", &cols, Some("表注释"), "s1");
            assert_eq!(out.len(), 2, "{kind:?}: {out:?}");
            assert!(
                out[0].contains("comment on table s1.t1 is '表注释'"),
                "{kind:?}: {}",
                out[0]
            );
            assert!(
                out[1].contains("comment on column s1.t1.c1 is '列注释'"),
                "{kind:?}: {}",
                out[1]
            );
        }
        // SQL Server：扩展属性，值与名都走 N'…' 字面量
        let out = of(ConnectionKind::Sqlserver).post_create_comments("t1", &cols, Some("表注释"), "dbo");
        assert!(out.len() >= 2, "{out:?}");
        assert!(
            out.iter().all(|s| s.contains("exec sp_addextendedproperty")),
            "{out:?}"
        );
        assert!(out.iter().any(|s| s.contains("N'表注释'")), "{out:?}");
        assert!(out.iter().any(|s| s.contains("@level2type=N'COLUMN'")), "{out:?}");
        // 空注释不发声
        let mut empty = std::collections::HashMap::new();
        empty.insert("c1".to_string(), String::new());
        let out = of(ConnectionKind::Postgresql).post_create_comments("t1", &empty, None, "");
        assert!(out.is_empty(), "{out:?}");
    }
    /// 对象定义查询按方言分家：MySQL 系 show create、PG 系 pg_get_*def、
    /// Oracle 三参 get_ddl、没有该对象类型的方言如实 Absent。
    #[test]
    fn object_source_splits_by_family() {
        fn sql(meta: Meta) -> String {
            match meta {
                Meta::Sql(s) => s,
                Meta::Absent => "<absent>".to_string(),
                Meta::Unwritten => "<unwritten>".to_string(),
            }
        }

        // MySQL / MariaDB：五种对象都是 show create <类型>
        for kind in [ConnectionKind::Mysql, ConnectionKind::Mariadb] {
            for t in ["view", "procedure", "function", "trigger", "event"] {
                let s = sql(of(kind).object_source(t, "v1", None));
                assert!(s.starts_with(&format!("show create {t} ")), "{kind:?}/{t}: {s}");
            }
        }
        // Doris：只有视图同源，例程/触发器/事件根本没有
        assert!(
            sql(of(ConnectionKind::Doris).object_source("view", "v1", None)).starts_with("show create view")
        );
        for t in ["procedure", "function", "trigger", "event"] {
            assert!(
                matches!(
                    of(ConnectionKind::Doris).object_source(t, "v1", None),
                    Meta::Absent
                ),
                "{t}"
            );
        }
        // ClickHouse：视图/表同源，例程/触发器/事件不存在
        assert!(
            sql(of(ConnectionKind::Clickhouse).object_source("table", "v1", None))
                .contains("show create table")
        );
        assert!(matches!(
            of(ConnectionKind::Clickhouse).object_source("procedure", "v1", None),
            Meta::Absent
        ));

        // PG：三支 pg_get_*def + event 不存在；模式筛进 pg_namespace 条件
        let pg = sql(of(ConnectionKind::Postgresql).object_source("view", "v1", None));
        assert!(
            pg.contains("pg_get_viewdef") && pg.contains("c.relname = 'v1'"),
            "{pg}"
        );
        assert!(
            pg.contains("nspname not in ('pg_catalog','information_schema')"),
            "{pg}"
        );
        let pg2 = sql(of(ConnectionKind::Postgresql).object_source("view", "v1", Some("public")));
        // 模式名走字符串字面量比较（literal），不是标识符引号
        assert!(pg2.contains("n.nspname = 'public'"), "{pg2}");
        assert!(
            sql(of(ConnectionKind::Postgresql).object_source("procedure", "v1", None))
                .contains("pg_get_functiondef")
        );
        assert!(
            sql(of(ConnectionKind::Postgresql).object_source("trigger", "v1", None))
                .contains("pg_get_triggerdef")
        );
        assert!(matches!(
            of(ConnectionKind::Postgresql).object_source("event", "v1", None),
            Meta::Absent
        ));
        // 金仓同一份实现，目录/函数换成 sys_*（含 `'pg_catalog'` 字面量 → `'sys_catalog'`）
        let kb_view = sql(of(ConnectionKind::Kingbase).object_source("view", "v1", None));
        assert!(kb_view.contains("sys_get_viewdef"), "{kb_view}");
        assert!(
            kb_view.contains("nspname not in ('sys_catalog','information_schema')"),
            "{kb_view}"
        );

        // Oracle：三参 get_ddl 带 owner；不带模式时走会话默认模式
        let ora = sql(of(ConnectionKind::Oracle).object_source("view", "v1", Some("s1")));
        assert!(ora.contains("get_ddl('VIEW', upper('v1'), upper('s1'))"), "{ora}");
        let ora2 = sql(of(ConnectionKind::Oracle).object_source("procedure", "v1", None));
        assert!(ora2.contains("get_ddl('PROCEDURE', upper('v1'))"), "{ora2}");
        assert!(!ora2.contains("upper('s1')"), "{ora2}");
        assert!(matches!(
            of(ConnectionKind::Oracle).object_source("event", "v1", None),
            Meta::Absent
        ));
        // DM：视图走两参 get_ddl（三参没真机验证过）；例程改走 all_source ——
        // get_ddl 在只读会话里内部要建临时表，直接报 -6506「试图在只读事务中修改数据」
        let dm_view = sql(of(ConnectionKind::Dm).object_source("view", "v1", None));
        assert!(dm_view.contains("get_ddl('VIEW', upper('v1'))"), "{dm_view}");
        let dm_fn = sql(of(ConnectionKind::Dm).object_source("function", "v1", None));
        assert!(dm_fn.contains("from all_source"), "{dm_fn}");
        assert!(matches!(
            of(ConnectionKind::Dm).object_source("event", "v1", None),
            Meta::Absent
        ));

        // SQL Server：object_definition 通吃；event 不存在
        let ms = sql(of(ConnectionKind::Sqlserver).object_source("view", "v1", None));
        assert!(ms.contains("object_definition(object_id('v1'))"), "{ms}");
        assert!(matches!(
            of(ConnectionKind::Sqlserver).object_source("event", "v1", None),
            Meta::Absent
        ));

        // H2：information_schema（比较用字符串字面量，不是标识符）
        let h2 = sql(of(ConnectionKind::H2).object_source("view", "v1", None));
        assert!(
            h2.contains("information_schema.views") && h2.contains("upper('v1')"),
            "{h2}"
        );
        assert!(sql(of(ConnectionKind::H2).object_source("procedure", "v1", None)).contains("routines"));
        assert!(matches!(
            of(ConnectionKind::H2).object_source("event", "v1", None),
            Meta::Absent
        ));

        // DB2：syscat 三表的 TEXT 列
        assert!(sql(of(ConnectionKind::Db2).object_source("view", "v1", None)).contains("syscat.views"));
        assert!(sql(of(ConnectionKind::Db2).object_source("trigger", "v1", None)).contains("syscat.triggers"));
        assert!(matches!(
            of(ConnectionKind::Db2).object_source("event", "v1", None),
            Meta::Absent
        ));

        // Derby：视图/触发器有目录，例程没有
        assert!(sql(of(ConnectionKind::Derby).object_source("view", "v1", None)).contains("sys.sysviews"));
        assert!(sql(of(ConnectionKind::Derby).object_source("trigger", "v1", None)).contains("systriggers"));
        assert!(matches!(
            of(ConnectionKind::Derby).object_source("procedure", "v1", None),
            Meta::Absent
        ));

        // SQLite：sqlite_master 通吃（表/视图/触发器都在里面）
        let sq = sql(of(ConnectionKind::Sqlite).object_source("view", "v1", None));
        assert!(sq.contains("sqlite_master") && sq.contains("'v1'"), "{sq}");
        // 没实现的方言不猜
        assert!(matches!(
            of(ConnectionKind::Duckdb).object_source("view", "v1", None),
            Meta::Unwritten
        ));
    }

    /// 默认值的时间写法：标准写法默认直通，MySQL 的 current_date 必须省略
    /// （DEFAULT CURRENT_DATE 语法错；MariaDB 认），SQL Server 翻 getdate()；
    /// CH 的「当前时间」改写成 now()/today()（裸写 current_timestamp 会被当列名）。
    #[test]
    fn time_defaults_by_family() {
        for kind in [
            ConnectionKind::Postgresql,
            ConnectionKind::Clickhouse,
            ConnectionKind::Oracle,
            ConnectionKind::Db2,
        ] {
            assert_eq!(
                of(kind).portable_time_literal("current_timestamp"),
                Some("current_timestamp"),
                "{kind:?}"
            );
            assert_eq!(
                of(kind).portable_time_literal("current_date"),
                Some("current_date"),
                "{kind:?}"
            );
        }
        assert_eq!(
            of(ConnectionKind::Mysql).portable_time_literal("current_date"),
            None
        );
        assert_eq!(
            of(ConnectionKind::Mariadb).portable_time_literal("current_date"),
            Some("current_date")
        );
        assert_eq!(
            of(ConnectionKind::Sqlserver).portable_time_literal("current_date"),
            Some("cast(getdate() as date)")
        );

        // 默认不改写
        for kind in [
            ConnectionKind::Mysql,
            ConnectionKind::Postgresql,
            ConnectionKind::Sqlserver,
        ] {
            assert_eq!(
                of(kind).rewrite_time_default("CURRENT_TIMESTAMP", false),
                None,
                "{kind:?}"
            );
        }
        // CH：跨类型时翻成自己的写法；文本列的默认值是内容，不改
        let ch = of(ConnectionKind::Clickhouse);
        assert_eq!(
            ch.rewrite_time_default("CURRENT_TIMESTAMP", false).as_deref(),
            Some("now()")
        );
        assert_eq!(ch.rewrite_time_default("NOW(3)", false).as_deref(), Some("now()"));
        assert_eq!(
            ch.rewrite_time_default("CURRENT_DATE", false).as_deref(),
            Some("today()")
        );
        assert_eq!(
            ch.rewrite_time_default("TODAY()", false).as_deref(),
            Some("today()")
        );
        assert_eq!(ch.rewrite_time_default("CURRENT_TIMESTAMP", true), None);
    }

    /// LIKE 转义尾巴按语法族分：默认转义符就是 `\` 的方言给空串，其余显式 escape。
    #[test]
    fn like_escape_suffix_splits_by_family() {
        for key in [
            "mysql",
            "mariadb",
            "doris",
            "clickhouse",
            "postgresql",
            "kingbase",
        ] {
            let kind = ConnectionKind::from_key(key).unwrap();
            assert_eq!(of(kind).like_escape_suffix(), "", "{key}");
        }
        for key in ["sqlite", "sqlserver", "oracle", "dm", "db2", "h2", "derby"] {
            let kind = ConnectionKind::from_key(key).unwrap();
            assert_eq!(of(kind).like_escape_suffix(), " escape '\\'", "{key}");
        }
    }

    /// 行编辑的时间字面量：Oracle/DM 才做 to_date/to_timestamp 翻译，其余退回普通字面量。
    #[test]
    fn temporal_row_literal_splits_by_family() {
        let oracle = of(ConnectionKind::Oracle);
        assert_eq!(
            oracle.temporal_row_literal("date", "2026-02-01").as_deref(),
            Some("to_date('2026-02-01', 'YYYY-MM-DD')")
        );
        assert_eq!(
            oracle
                .temporal_row_literal("timestamp(6)", "2026-02-01T13:45:59.123456")
                .as_deref(),
            Some("to_timestamp('2026-02-01 13:45:59.123456', 'YYYY-MM-DD HH24:MI:SS.FF')")
        );
        // 非日期类型 / 不认识的形状不猜
        assert_eq!(oracle.temporal_row_literal("varchar2(20)", "2026-02-01"), None);
        assert_eq!(oracle.temporal_row_literal("date", "02-01-2026"), None);
        // DM 与 Oracle 同语法
        assert_eq!(
            of(ConnectionKind::Dm)
                .temporal_row_literal("date", "2026-02-01")
                .as_deref(),
            Some("to_date('2026-02-01', 'YYYY-MM-DD')")
        );
        // 其它方言一律 None（ISO 串会话都认）
        for key in ["mysql", "postgresql", "sqlserver", "sqlite"] {
            let kind = ConnectionKind::from_key(key).unwrap();
            assert_eq!(of(kind).temporal_row_literal("date", "2026-02-01"), None, "{key}");
        }
    }
    /// 授权回显行 → GRANT 文本，按方言四档（MySQL 通用 / Oracle / PG 系 / H2）。
    #[test]
    fn grant_line_texts_by_family() {
        // MySQL 通用：全局 / 表级 / 例程（object 是 `类型:名字`） / 角色（账号形状）
        let my = of(ConnectionKind::Mysql);
        assert_eq!(
            my.grant_line_text("global", "SELECT", "", "", "u", "%")
                .as_deref(),
            Some("GRANT SELECT ON *.* TO 'u'@'%';")
        );
        // 全小写 plain 名裸写不加反引号（quote 的既有语义）；保留字才引起来
        assert_eq!(
            my.grant_line_text("table", "SELECT", "db", "t", "u", "%")
                .as_deref(),
            Some("GRANT SELECT ON db.t TO 'u'@'%';")
        );
        assert_eq!(
            my.grant_line_text("table", "SELECT", "my-db", "order", "u", "%")
                .as_deref(),
            Some("GRANT SELECT ON `my-db`.`order` TO 'u'@'%';")
        );
        assert_eq!(
            my.grant_line_text("routine", "EXECUTE", "db", "PROCEDURE:p", "u", "%")
                .as_deref(),
            Some("GRANT EXECUTE ON procedure db.p TO 'u'@'%';")
        );
        assert_eq!(
            my.grant_line_text("role", "r", "r", "%", "u", "%").as_deref(),
            Some("GRANT 'r'@'%' TO 'u'@'%';")
        );
        // Oracle：对象带 ON "模式"."对象"，账号没有 host
        let ora = of(ConnectionKind::Oracle);
        assert_eq!(
            ora.grant_line_text("object", "SELECT", "SCOTT", "EMP", "U", "")
                .as_deref(),
            Some("GRANT SELECT ON \"SCOTT\".\"EMP\" TO \"U\";")
        );
        assert_eq!(
            ora.grant_line_text("global", "CREATE SESSION", "", "", "U", "")
                .as_deref(),
            Some("GRANT CREATE SESSION TO \"U\";")
        );
        // PG 系：模式 / 表 / 例程都带 ON 子句，grantee 是标识符
        let pg = of(ConnectionKind::Postgresql);
        // `public` 是 PG 保留字被引起来；普通名裸写（quote 的既有语义）
        assert_eq!(
            pg.grant_line_text("schema", "USAGE", "public", "", "u", "")
                .as_deref(),
            Some("GRANT USAGE ON SCHEMA \"public\" TO u;")
        );
        assert_eq!(
            pg.grant_line_text("function", "EXECUTE", "public", "f", "u", "")
                .as_deref(),
            Some("GRANT EXECUTE ON FUNCTION \"public\".f TO u;")
        );
        // H2：表级一层，空用户名统一写成空标识符 `""`；普通名裸写（默认 quote）
        let h2 = of(ConnectionKind::H2);
        assert_eq!(
            h2.grant_line_text("table", "SELECT", "S", "T", "", "").as_deref(),
            Some("GRANT SELECT ON S.T TO \"\";")
        );
        // 认不出的 scope 一律 None（不硬拼）
        assert_eq!(my.grant_line_text("weird", "X", "a", "b", "u", "%"), None);
    }

    /// 授权入参矩阵 + 撤销校验按方言分：能力差异是方言的事，不在 handler 里列名单。
    #[test]
    fn grant_scopes_and_revocable_by_family() {
        let scopes = |k: ConnectionKind| of(k).grant_input_scopes();
        assert!(scopes(ConnectionKind::Mysql).contains(&"routinePrivileges"));
        // Doris 没有例程：不放行 routinePrivileges（拼 MySQL 语法必炸）
        assert!(!scopes(ConnectionKind::Doris).contains(&"routinePrivileges"));
        assert!(scopes(ConnectionKind::Doris).contains(&"tablePrivileges"));
        assert!(scopes(ConnectionKind::Oracle).contains(&"privileges"));
        assert!(!scopes(ConnectionKind::Oracle).contains(&"dbPrivileges"));
        assert!(scopes(ConnectionKind::Sqlserver).contains(&"serverRoles"));
        assert!(scopes(ConnectionKind::Postgresql).contains(&"dbPrivileges"));
        assert_eq!(scopes(ConnectionKind::H2), ["tablePrivileges", "revoke"]);
        // DB2/SQLite 等：没有已实现的授权层，全拒（动数据库前明确拒绝）
        assert!(scopes(ConnectionKind::Db2).is_empty());

        // 撤销校验：MySQL 的 db/table 要 owner，routine 还要 kind
        let my = of(ConnectionKind::Mysql);
        assert!(my.is_revocable("db", "SELECT", "db", "", ""));
        assert!(!my.is_revocable("db", "SELECT", "", "", ""));
        assert!(my.is_revocable("routine", "EXECUTE", "db", "PROCEDURE:p", "PROCEDURE"));
        assert!(!my.is_revocable("routine", "EXECUTE", "db", "PROCEDURE:p", "TRIGGER"));
        // 空 scope 缺省当 global（老调用方形状）
        assert!(my.is_revocable("", "SELECT", "", "", ""));
        // SQL Server 的 server-role 兼容带连字符写法；schema/object 要 owner
        let ms = of(ConnectionKind::Sqlserver);
        assert!(ms.is_revocable("server-role", "sysadmin", "", "", ""));
        assert!(!ms.is_revocable("schema", "SELECT", "", "", ""));
        assert!(ms.is_revocable("schema", "SELECT", "dbo", "", ""));
        // Oracle：role 允许自定义角色；PG 的 routine 要 kind
        let ora = of(ConnectionKind::Oracle);
        assert!(ora.is_revocable("role", "MY_ROLE", "", "", ""));
        // 兜底白名单（is_safe_name）放行「字母单词+空格」形状；注入形状必须拒
        assert!(ora.is_revocable("global", "ADMINISTER KEY MANAGEMENT", "", "", ""));
        assert!(!ora.is_revocable("global", "not;a' priv", "", "", ""));
        let pg = of(ConnectionKind::Postgresql);
        assert!(pg.is_revocable("database", "CONNECT", "db", "", ""));
        assert!(!pg.is_revocable("database", "CONNECT", "", "", ""));
        // 校验器选择：Oracle 两本白名单分开，H2 只有表级
        assert!(of(ConnectionKind::Oracle).grantable_privilege_check()(
            "CREATE SESSION"
        ));
        // 对象白名单不含系统权限（is_safe_name 兜底只放行安全形状，注入形状必须拒）
        assert!(!of(ConnectionKind::Oracle).object_privilege_check()(
            "CREATE;SESSION"
        ));
        assert!(of(ConnectionKind::H2).object_privilege_check()("SELECT"));
        // 文案：缺的名字类别按方言措辞
        assert_eq!(of(ConnectionKind::Mysql).revoke_missing_hint(), "库/表名");
        assert_eq!(of(ConnectionKind::Sqlserver).revoke_missing_hint(), "架构/对象名");
        assert_eq!(
            of(ConnectionKind::Postgresql).revoke_missing_hint(),
            "模式/对象名"
        );
    }
}
