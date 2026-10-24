//! **一种数据库在数据搬运上的差异 = 一份数据**（批量大小、报文上限、事务语句、保存点、
//! 表达式上限、会话参数，以及慢批自愈要用的「查会话 id / 掐会话」能力）。
//!
//! ## 为什么单独一层
//!
//! 这些差异原来散在 `sync.rs` 的几个 `match kind_key` 与 `transfer/writer.rs` 的
//! `matches!(kind, "mysql" | "mariadb")` 里：新增一种库要在五六处加分支，漏一处就是**静默错**
//!（事务没开、批量撞 packet 上限、保存点语法不存在、慢批掐不掉）。
//!
//! 现在：**一种库 = 一条 [`Spec`]**，就写在它自己的写插件文件里
//!（`plugins/<kind>_writer.rs` 的 `SPEC`）。读插件、写插件、引擎、同步层都只问这份规格，
//! 代码里不再出现数据库名字。
//!
//! ## 新增一种数据库类型
//!
//! **只加两个文件**：`plugins/<kind>_reader.rs`（怎么取数）与 `plugins/<kind>_writer.rs`
//!（怎么落库 + 这份 `SPEC`）。注册表由 `build.rs` 扫目录自动生成，别处一行都不用改。

/// 事务的三条语句（`end` 为空串表示不需要复位）。
#[derive(Debug, Clone, Copy)]
pub struct Txn {
    pub begin: &'static str,
    pub commit: &'static str,
    /// 提交/回滚后把会话复位。MySQL 系必须把 autocommit 打开，
    /// 否则下一条语句会静默进一个新事务、再也提交不了。
    pub end: &'static str,
}

/// 保存点的语法家族（批量失败后回滚到本批之前再用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Savepoint {
    /// 没有保存点概念（失败只能整段回滚）
    None,
    /// `savepoint n` / `rollback to savepoint n` / `release savepoint n`
    Pg,
    /// `save transaction n` / `rollback transaction n`
    MsSql,
}

/// 派生表表达式总数的硬限制。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExprLimit {
    None,
    /// SQL Server 多行 `VALUES` 的解析器硬限制：1000 行
    SqlServer,
    /// `65_535 / 列数`，再夹在 [10, 10_000]（Oracle / DM / DB2 的 `(值1, 值2, …)`）
    OracleLike,
}

/// 一种库的搬运规格。**能写成数据的，就别写成分支**。
#[derive(Debug, Clone, Copy)]
pub struct Spec {
    key: &'static str,
    rows: usize,
    stmt_cap: usize,
    txn: Option<Txn>,
    savepoint: Savepoint,
    prepared_batch: bool,
    expr_limit: ExprLimit,
    multi_statement: bool,
    stream_load: bool,
    /// 取本连接的会话 id（目前只有 MySQL 系有 `connection_id()`）；`None` = 不支持慢批自愈
    conn_id_sql: Option<&'static str>,
    /// 从服务端掐断指定会话的语句
    kill: Option<fn(u64) -> String>,
    /// 「目标端此刻在忙什么」的诊断查询（`None` = 该方言问不到）
    activity_sql: Option<&'static str>,
    /// 开事务前要设的会话参数（锁等待上限、外键检查等），按顺序执行、失败忽略
    session_setup: &'static [&'static str],
    /// 显式开启 `DBMIND_SYNC_SKIP_BINLOG` 时才执行的会话参数（断开目标端 binlog）
    binlog_off: Option<&'static str>,
}

impl Spec {
    pub(crate) const fn new(key: &'static str) -> Self {
        Self {
            key,
            rows: 1_000,
            // 没有 packet 级限制的库：仍受链路限制（一条几 MB 的语句丢一个包就整条重传）
            stmt_cap: 4 * 1024 * 1024,
            txn: None,
            savepoint: Savepoint::None,
            prepared_batch: false,
            expr_limit: ExprLimit::None,
            multi_statement: false,
            stream_load: false,
            conn_id_sql: None,
            kill: None,
            activity_sql: None,
            session_setup: &[],
            binlog_off: None,
        }
    }

    // ---- 取值（引擎 / 同步层只看这些）----

    /// 注册表键（= 连接类型 YAML 的 `dbType`）
    pub(crate) const fn key(&self) -> &'static str {
        self.key
    }

    /// 一条 INSERT 里的**默认**条数（界面填了以界面为准）。
    pub(crate) const fn rows(&self) -> usize {
        self.rows
    }

    /// 一条语句的**字节**上限（协议/packet 上限，不是链路实测值）。
    pub(crate) const fn stmt_cap(&self) -> usize {
        self.stmt_cap
    }

    pub(crate) const fn txn(&self) -> Option<Txn> {
        self.txn
    }

    pub(crate) const fn savepoint(&self) -> Savepoint {
        self.savepoint
    }

    pub(crate) const fn prepared_batch(&self) -> bool {
        self.prepared_batch
    }

    pub(crate) const fn expr_limit(&self) -> ExprLimit {
        self.expr_limit
    }

    pub(crate) const fn multi_statement(&self) -> bool {
        self.multi_statement
    }

    pub(crate) const fn stream_load(&self) -> bool {
        self.stream_load
    }

    pub(crate) const fn conn_id_sql(&self) -> Option<&'static str> {
        self.conn_id_sql
    }

    /// 掐断一条会话的语句；`None` = 该方言掐不断（引擎据此不做慢批自愈）。
    pub(crate) fn kill_sql(&self, id: u64) -> Option<String> {
        self.kill.map(|build| build(id))
    }

    /// 能不能从服务端掐断本表的写会话（慢批自愈的前提）。
    pub(crate) const fn has_kill(&self) -> bool {
        self.kill.is_some()
    }

    /// 目标端「此刻在忙什么」的诊断 SQL（`information_schema.processlist`）。
    ///
    /// 界面日志不再解释「目标库里有没有语句在跑」（用户口径：只报失败原因本身），
    /// 所以暂时没有调用方；字段保留，排查时按需接回。
    #[allow(dead_code)]
    pub(crate) const fn activity_sql(&self) -> Option<&'static str> {
        self.activity_sql
    }

    pub(crate) const fn session_setup(&self) -> &'static [&'static str] {
        self.session_setup
    }

    pub(crate) const fn binlog_off(&self) -> Option<&'static str> {
        self.binlog_off
    }

    // ---- 构造器（都是 const，便于写成 `static SPEC`）----

    pub(crate) const fn with_rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }

    pub(crate) const fn with_cap(mut self, stmt_cap: usize) -> Self {
        self.stmt_cap = stmt_cap;
        self
    }

    pub(crate) const fn with_txn(
        mut self,
        begin: &'static str,
        commit: &'static str,
        end: &'static str,
    ) -> Self {
        self.txn = Some(Txn {
            begin,
            commit,
            end,
        });
        self
    }

    pub(crate) const fn with_savepoint(mut self, style: Savepoint) -> Self {
        self.savepoint = style;
        self
    }

    pub(crate) const fn with_prepared(mut self) -> Self {
        self.prepared_batch = true;
        self
    }

    pub(crate) const fn with_expr_limit(mut self, limit: ExprLimit) -> Self {
        self.expr_limit = limit;
        self
    }

    pub(crate) const fn with_multi_statement(mut self) -> Self {
        self.multi_statement = true;
        self
    }

    pub(crate) const fn with_stream_load(mut self) -> Self {
        self.stream_load = true;
        self
    }

    pub(crate) const fn with_conn_id_sql(mut self, sql: &'static str) -> Self {
        self.conn_id_sql = Some(sql);
        self
    }

    pub(crate) const fn with_kill(mut self, build: fn(u64) -> String) -> Self {
        self.kill = Some(build);
        self
    }

    pub(crate) const fn with_activity_sql(mut self, sql: &'static str) -> Self {
        self.activity_sql = Some(sql);
        self
    }

    pub(crate) const fn with_session_setup(mut self, sql: &'static [&'static str]) -> Self {
        self.session_setup = sql;
        self
    }

    pub(crate) const fn with_binlog_off(mut self, sql: &'static str) -> Self {
        self.binlog_off = Some(sql);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 默认规格就是「一种普通 JDBC 库」：不改任何行为。
    #[test]
    fn 默认规格保守() {
        let spec = Spec::new("duckdb");
        assert_eq!(spec.rows(), 1_000);
        assert_eq!(spec.stmt_cap(), 4 * 1024 * 1024);
        assert!(spec.txn().is_none());
        assert_eq!(spec.savepoint(), Savepoint::None);
        assert!(!spec.prepared_batch());
        assert_eq!(spec.expr_limit(), ExprLimit::None);
        assert!(spec.conn_id_sql().is_none());
        assert!(!spec.has_kill());
        assert!(spec.kill_sql(1).is_none());
        assert!(spec.session_setup().is_empty());
    }
}
