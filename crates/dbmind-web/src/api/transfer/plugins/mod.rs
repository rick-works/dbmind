//! **读写插件**：一种数据源两个文件 —— `<kind>_reader.rs`（怎么取数）与
//! `<kind>_writer.rs`（怎么落库 + 这份库的搬运规格 `SPEC`）。
//!
//! ## 约定（新增一种库照抄这两个文件即可）
//!
//! ```text
//! plugins/mysql_reader.rs   plugin_reader!(MysqlReader);          // 读插件
//! plugins/mysql_writer.rs   static SPEC: Spec = …; plugin_writer!(MysqlWriter);  // 写插件
//! ```
//!
//! 注册表**不手写**：`build.rs` 扫这个目录，按文件名生成 `reader_plugin` /
//! `writer_plugin` / `spec_plugin` 三张表（产物在 `OUT_DIR/transfer_plugins.rs`）。
//! 所以新增一种库 = **加两个文件**，别处一行都不用改；文件名少一个角色会在启动日志与
//! `每种数据源都有读写两个插件` 这条测试里直接暴露。
//!
//! ## 引擎与插件的分工
//!
//! 引擎（`engine.rs`）只认 [`crate::api::transfer::Reader`] / [`crate::api::transfer::Writer`]
//! 两个 trait，所以「源 MySQL → 目标 Oracle」「源 PG → 目标 ClickHouse」都是同一个引擎 +
//! 两个插件实例，任意组合，不需要在引擎里出现任何数据库名字。
//!
//! 会话、超时、探活、重试、令牌、取消、计数**都不在插件里** —— 那些是「怎么搬」，
//! 与「搬的是什么库」无关，统一在 `engine.rs` / `sync.rs` 里做。

use std::sync::Arc;

use super::reader::SqlReader;
use super::spec::Spec;
use super::writer::SqlWriter;
use super::{Reader, TablePlan, Writer};

/// 读插件工厂：给一份表计划，产出取数实现。
pub(crate) type ReaderFactory = fn(&TablePlan) -> Box<dyn Reader>;

/// 写插件工厂：给一份表计划，产出落库实现（`Arc` 是因为写池会跨协程共享）。
pub(crate) type WriterFactory = fn(&TablePlan) -> Arc<dyn Writer>;

/// 生成「读插件」：`XxxReader` 组合通用 `SqlReader`，并把 `Reader` 全部方法委托过去。
///
/// 薄壳不是多余的一层：将来某个源要改成游标/分区消费，只改它自己那个文件，
/// 注册表与引擎都不用动。类型名会进日志（`Reader::name`），一眼能看出是谁在取数。
macro_rules! plugin_reader {
    ($ty:ident) => {
        #[doc = concat!(stringify!($ty), "：该数据源的读插件。")]
        pub struct $ty {
            inner: Box<dyn crate::api::transfer::Reader>,
        }

        impl $ty {
            pub fn new(plan: &crate::api::transfer::TablePlan) -> Self {
                Self {
                    inner: crate::api::transfer::reader::SqlReader::new(plan),
                }
            }
        }

        impl crate::api::transfer::Reader for $ty {
            fn name(&self) -> &'static str {
                stringify!($ty)
            }

            fn page_rows(&self) -> usize {
                self.inner.page_rows()
            }

            fn commit_rows(&self) -> u64 {
                self.inner.commit_rows()
            }

            fn max_parallel(&self) -> usize {
                self.inner.max_parallel()
            }

            fn parallel_ceiling(&self) -> usize {
                self.inner.parallel_ceiling()
            }

            fn split<'a>(
                &'a mut self,
                want: usize,
            ) -> crate::api::transfer::BF<'a, Result<Vec<crate::api::transfer::RowRange>, String>> {
                self.inner.split(want)
            }

            fn page<'a>(
                &'a self,
                range: &'a crate::api::transfer::RowRange,
                cursor: Option<crate::api::transfer::Cursor>,
            ) -> crate::api::transfer::BF<'a, Result<crate::api::transfer::Page, String>> {
                self.inner.page(range, cursor)
            }

            fn shrink_page(&self) {
                self.inner.shrink_page()
            }
        }

        pub(crate) fn reader(
            plan: &crate::api::transfer::TablePlan,
        ) -> Box<dyn crate::api::transfer::Reader> {
            Box::new($ty::new(plan))
        }
    };
}

/// 生成「写插件」：`XxxWriter` 组合通用 `SqlWriter`，并把 `Writer` 全部方法委托过去。
macro_rules! plugin_writer {
    ($ty:ident) => {
        #[doc = concat!(stringify!($ty), "：该数据源的写插件。")]
        pub struct $ty {
            inner: crate::api::transfer::writer::SqlWriter,
        }

        impl $ty {
            pub fn new(plan: &crate::api::transfer::TablePlan) -> Self {
                Self {
                    inner: crate::api::transfer::writer::SqlWriter::new(plan),
                }
            }
        }

        impl crate::api::transfer::Writer for $ty {
            fn name(&self) -> &'static str {
                stringify!($ty)
            }

            fn workers(&self) -> usize {
                self.inner.workers()
            }

            fn workers_ceiling(&self) -> usize {
                self.inner.workers_ceiling()
            }

            fn begin<'a>(&'a self, worker: usize) -> crate::api::transfer::BF<'a, ()> {
                self.inner.begin(worker)
            }

            fn write<'a>(
                &'a self,
                worker: usize,
                rows: &'a [Vec<dbmind_core::CellValue>],
            ) -> crate::api::transfer::BF<'a, Result<u64, String>> {
                self.inner.write(worker, rows)
            }

            fn update<'a>(
                &'a self,
                rows: &'a [Vec<dbmind_core::CellValue>],
            ) -> crate::api::transfer::BF<'a, Result<u64, String>> {
                self.inner.update(rows)
            }

            fn flush<'a>(&'a self) -> crate::api::transfer::BF<'a, Result<u64, String>> {
                self.inner.flush()
            }

            fn finish<'a>(&'a self, worker: usize) -> crate::api::transfer::BF<'a, Result<(), String>> {
                self.inner.finish(worker)
            }

            fn abort<'a>(&'a self, worker: usize) -> crate::api::transfer::BF<'a, ()> {
                self.inner.abort(worker)
            }

            fn renew<'a>(&'a self) -> crate::api::transfer::BF<'a, ()> {
                self.inner.renew()
            }

            fn kill_sessions<'a>(&'a self) -> crate::api::transfer::BF<'a, bool> {
                self.inner.kill_sessions()
            }

            fn shrink_stmt(&self, worker: usize) -> (usize, usize) {
                self.inner.shrink_stmt(worker)
            }
        }

        pub(crate) fn writer(
            plan: &crate::api::transfer::TablePlan,
        ) -> std::sync::Arc<dyn crate::api::transfer::Writer> {
            std::sync::Arc::new($ty::new(plan))
        }
    };
}

pub(crate) use plugin_reader;
pub(crate) use plugin_writer;

/// 没登记插件的类型走通用实现（能同步，只是用了最保守的参数）。
fn generic_reader(plan: &TablePlan) -> Box<dyn Reader> {
    SqlReader::new(plan)
}

fn generic_writer(plan: &TablePlan) -> Arc<dyn Writer> {
    Arc::new(SqlWriter::new(plan))
}

/// 没登记规格的类型：保守默认值（1000 行 / 4MB / 无事务语句）。
static GENERIC_SPEC: Spec = Spec::new("generic");

// 三张注册表由 build.rs 按目录里的文件生成 —— 加插件不用改这个文件。
include!(concat!(env!("OUT_DIR"), "/transfer_plugins.rs"));

/// 未登记插件的类型 → 它所属方言族的**代表 kind**。
///
/// `DialectOps::family()` 返回的名字本身就是族代表的 key（`postgresql` / `doris` /
/// `sqlserver` / `kingbase` …），所以不用再维护一张映射表。族内差异（Doris 没有原生
/// upsert、StarRocks 没有 `mysql` 库）在 `dialect_ops::of()` 里已经处理过，这里拿到的
/// 就是那个已经分好类的族。
///
/// 为什么要这层：同族品牌（`databend` / `risingwave` / `starrocks` / `tidb` /
/// `oceanbase` / `greatsql` …）的取数与批量参数跟族代表完全一致，此前因为「按 kind
/// 精确查表」直接落到 generic（1000 行 / 4MB 的保守参数），同步能跑但慢一截，
/// 而且界面上完全看不出来 —— 正是「插件化只做了一半」的那种静默降级。
fn family_key(kind: &str) -> Option<&'static str> {
    let typed = dbmind_core::ConnectionKind::from_key(kind)?;
    let family = crate::api::dialect_ops::of(typed).family();
    // generic / nosql 没有对应插件；族名与自身同名说明已经是代表，交给下层走 generic。
    if matches!(family, "generic" | "nosql") || family == kind {
        return None;
    }
    Some(family)
}

/// 按**源**类型取读插件工厂：精确登记优先，其次同族代表，最后通用实现。
pub(crate) fn reader_factory(kind: &str) -> ReaderFactory {
    reader_plugin(kind)
        .or_else(|| family_key(kind).and_then(reader_plugin))
        .unwrap_or(generic_reader)
}

/// 按**目标**类型取写插件工厂：精确登记优先，其次同族代表，最后通用实现。
pub(crate) fn writer_factory(kind: &str) -> WriterFactory {
    writer_plugin(kind)
        .or_else(|| family_key(kind).and_then(writer_plugin))
        .unwrap_or(generic_writer)
}

/// 按类型取搬运规格：精确登记优先，其次同族代表，最后保守默认值。
pub(crate) fn spec_data_for(kind: &str) -> &'static Spec {
    if let Some(spec) = spec_plugin(kind) {
        // 规格必须与文件名一致：复制粘贴写插件时最容易漏改 `Spec::new("…")`，
        // 那会变成「查 MySQL 却拿到 Oracle 的批量参数」这种只在真机上体现的静默错。
        debug_assert!(
            spec.key() == kind,
            "plugins/{kind}_writer.rs 的 SPEC.key 是 {}，与文件名不一致",
            spec.key()
        );
        return spec;
    }
    if let Some(spec) = family_key(kind).and_then(spec_plugin) {
        return spec;
    }
    &GENERIC_SPEC
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每种登记过的类型都必须**同时**有读插件与写插件。
    ///
    /// 只加一个文件是最容易犯的错（漏了写插件 ⇒ 同步能跑但用的是通用参数，
    /// 日志里插件名会暴露真相，却没人看）；build.rs 的注册表按文件名生成，
    /// 这个测试再把「成对」这件事钉死。
    #[test]
    fn 每种数据源都有读写两个插件() {
        for &kind in KINDS {
            assert!(
                reader_plugin(kind).is_some(),
                "{kind} 缺少读插件：应存在 plugins/{kind}_reader.rs"
            );
            assert!(
                writer_plugin(kind).is_some(),
                "{kind} 缺少写插件：应存在 plugins/{kind}_writer.rs"
            );
            // 规格必须与文件名一致（写错就是「查 A 拿到 B 的批量参数」这种静默错）
            assert_eq!(
                spec_data_for(kind).key(),
                kind,
                "{kind} 的 SPEC.key 与文件名不一致"
            );
        }
    }

    /// 关系型/分析型数据源一个都不能少：新增类型忘了加插件时，这条会红。
    #[test]
    fn 关系型与列式数据源都登记了插件() {
        for kind in [
            "mysql",
            "mariadb",
            "postgresql",
            "kingbase",
            "sqlserver",
            "oracle",
            "dm",
            "db2",
            "h2",
            "derby",
            "sqlite",
            "clickhouse",
            "doris",
            "duckdb",
        ] {
            assert!(KINDS.contains(&kind), "{kind} 没有登记读写插件");
        }
    }

    /// 没登记的类型必须落到通用实现，而不是 panic（NoSQL 之类走不到这里，但要能兜住）。
    #[test]
    fn 未登记类型走通用实现() {
        assert!(reader_plugin("not-a-db").is_none());
        assert!(writer_plugin("not-a-db").is_none());
        assert_eq!(spec_data_for("not-a-db").key(), "generic");
    }

    /// 同族品牌不写插件文件也要拿到**族代表**的搬运参数，不能退回 generic。
    ///
    /// 退回 generic 不会报错，只是每批 1000 行、4MB —— 同步能跑完，慢一截，
    /// 界面上完全看不出来。所以这条按「拿到的 SPEC 是谁」钉死。
    #[test]
    fn 同族品牌继承族代表参数() {
        use dbmind_core::ConnectionKind;

        let of = |kind: ConnectionKind| spec_data_for(kind.key()).key();
        // PostgreSQL 族
        assert_eq!(of(ConnectionKind::Risingwave), "postgresql");
        assert_eq!(of(ConnectionKind::Opengauss), "postgresql");
        assert_eq!(of(ConnectionKind::Highgo), "postgresql");
        // Databend 走 MySQL 协议（yaml `dialect: MySQL`，JDBC 也是 mysql-connector）
        assert_eq!(of(ConnectionKind::Databend), "mysql");
        // MySQL 族
        assert_eq!(of(ConnectionKind::Tidb), "mysql");
        assert_eq!(of(ConnectionKind::Oceanbase), "mysql");
        assert_eq!(of(ConnectionKind::Greatsql), "mysql");
        assert_eq!(of(ConnectionKind::Goldendb), "mysql");
        // Doris 族：DialectOps 里 StarRocks / SelectDB 与 Doris 同源
        assert_eq!(of(ConnectionKind::Starrocks), "doris");
        assert_eq!(of(ConnectionKind::Selectdb), "doris");

        // 已登记的类型**必须**用自己那份规格，不能被族兜底顶掉。
        assert_eq!(of(ConnectionKind::Mysql), "mysql");
        assert_eq!(of(ConnectionKind::Doris), "doris");
        assert_eq!(of(ConnectionKind::Kingbase), "kingbase");
        assert_eq!(of(ConnectionKind::Oracle), "oracle");
        assert_eq!(of(ConnectionKind::Dm), "dm");
    }

    /// 归族表本身：族代表必须能查到插件，否则「兜底」等于兜到 generic。
    #[test]
    fn 同族品牌归到族代表() {
        assert_eq!(family_key("databend"), Some("mysql"));
        assert_eq!(family_key("risingwave"), Some("postgresql"));
        assert_eq!(family_key("starrocks"), Some("doris"));
        assert_eq!(family_key("selectdb"), Some("doris"));
        assert_eq!(family_key("tidb"), Some("mysql"));
        assert_eq!(family_key("greatsql"), Some("mysql"));
        // 自己就是族代表 → 不进兜底（精确查表已经命中）
        assert_eq!(family_key("mysql"), None);
        assert_eq!(family_key("doris"), None);
        assert_eq!(family_key("kingbase"), None);
        // NoSQL / 未知类型没有插件
        assert_eq!(family_key("mongodb"), None);
        assert_eq!(family_key("not-a-db"), None);
    }

    /// 迁移不能改行为：这些数值/能力位全部对应改造前 `sync.rs` 里的 match 分支。
    ///
    /// 它们决定「一条语句多大、开不开事务、能不能掐会话」，写错了在界面上看不出来，
    /// 只在真机上体现为「写入中断」「永远同步中」——所以逐条钉死。
    #[test]
    fn 规格与改造前一致() {
        use super::super::spec::{ExprLimit, Savepoint};

        let rows = |kind: &str| spec_data_for(kind).rows();
        let cap = |kind: &str| spec_data_for(kind).stmt_cap();

        // 每批条数
        assert_eq!(rows("clickhouse"), 10_000);
        assert_eq!(rows("doris"), 2_000);
        assert_eq!(rows("postgresql"), 2_000);
        assert_eq!(rows("sqlserver"), 1_000, "T-SQL 多行 VALUES 1000 行硬限制");
        assert_eq!(rows("oracle"), 1_000);
        assert_eq!(rows("sqlite"), 5_000);
        assert_eq!(rows("h2"), 5_000);
        assert_eq!(rows("derby"), 5_000);
        assert_eq!(rows("mysql"), 1_000);
        assert_eq!(rows("mariadb"), 1_000);

        // 报文上限
        assert_eq!(cap("doris"), 512 * 1024, "FE 的 max_allowed_packet 默认 1MB");
        assert_eq!(cap("clickhouse"), 8 * 1024 * 1024);
        assert_eq!(cap("sqlite"), 8 * 1024 * 1024);
        assert_eq!(cap("mysql"), 1024 * 1024);
        assert_eq!(cap("mariadb"), 1024 * 1024);
        assert_eq!(cap("postgresql"), 4 * 1024 * 1024);
        assert_eq!(cap("oracle"), 4 * 1024 * 1024);

        // 事务语句
        for kind in [
            "mysql",
            "mariadb",
            "postgresql",
            "kingbase",
            "sqlite",
            "sqlserver",
        ] {
            assert!(spec_data_for(kind).txn().is_some(), "{kind} 应开事务");
        }
        for kind in ["oracle", "dm", "db2", "h2", "derby", "clickhouse", "doris"] {
            assert!(spec_data_for(kind).txn().is_none(), "{kind} 没有 SQL 级 begin");
        }

        // 保存点 / 预编译批
        assert_eq!(spec_data_for("postgresql").savepoint(), Savepoint::Pg);
        assert_eq!(spec_data_for("sqlserver").savepoint(), Savepoint::MsSql);
        assert_eq!(spec_data_for("mysql").savepoint(), Savepoint::None);
        for kind in ["sqlserver", "oracle", "dm", "h2"] {
            assert!(spec_data_for(kind).prepared_batch(), "{kind} 应启用预编译批");
        }
        assert!(
            !spec_data_for("db2").prepared_batch(),
            "DB2 的 executeBatch 会整批断连"
        );

        // 多语句批 / 表达式上限
        for kind in ["mysql", "mariadb", "doris", "sqlserver", "postgresql", "kingbase"] {
            assert!(spec_data_for(kind).multi_statement(), "{kind} 支持多语句批");
        }
        assert!(!spec_data_for("oracle").multi_statement());
        assert_eq!(spec_data_for("sqlserver").expr_limit(), ExprLimit::SqlServer);
        for kind in ["oracle", "dm", "db2"] {
            assert_eq!(spec_data_for(kind).expr_limit(), ExprLimit::OracleLike, "{kind}");
        }

        // 慢批自愈 / Stream Load 能力位
        for kind in ["mysql", "mariadb"] {
            assert!(spec_data_for(kind).has_kill(), "{kind} 能掐会话");
            assert!(spec_data_for(kind).activity_sql().is_some());
        }
        assert!(!spec_data_for("postgresql").has_kill());
        assert!(spec_data_for("doris").stream_load());
        assert!(!spec_data_for("mysql").stream_load());
    }
}
