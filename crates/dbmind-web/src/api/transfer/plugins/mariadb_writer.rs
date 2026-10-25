//! **MariaDB 写插件**。
//!
//! 与 MySQL 同源：多行 `INSERT` + `autocommit` 开关组事务，同一套批量与自愈参数。
//!
//! 批量/事务/自愈相关的差异全部写在下面这份 [`SPEC`] 里 —— 引擎与同步层只问规格，
//! 不再出现数据库名字（见 `transfer/spec.rs` 的模块文档）。

use crate::api::transfer::spec::Spec;

/// MariaDB 的搬运规格。
pub(crate) static SPEC: Spec = Spec::new("mariadb")
    .with_rows(1_000)
    .with_cap(1024 * 1024)
    .with_txn("set autocommit = 0", "commit", "set autocommit = 1")
    .with_multi_statement()
    .with_conn_id_sql("select connection_id()")
    .with_kill(kill_session)
    .with_activity_sql("select count(*) as n, coalesce(max(time), 0) as longest from information_schema.processlist where command = 'Query' and time >= 5")
    .with_session_setup(&[
        "set session lock_wait_timeout = 60, session innodb_lock_wait_timeout = 60",
        "set session foreign_key_checks = 0",
    ])
    .with_binlog_off("set session sql_log_bin = 0")
    // 纯插入时的 LOAD DATA LOCAL INFILE 快通道（见 transfer/loaddata.rs）。
    // 连接侧的开关由宿主在建连时打开（MariaDB 驱动是 `allowLocalInfile=true`）。
    .with_load_data();

/// `KILL <id>`：终止该连接上正在跑的语句并断开，服务端随之回滚未提交事务、释放行锁。
fn kill_session(id: u64) -> String {
    format!("kill {id}")
}

crate::api::transfer::plugins::plugin_writer!(MariadbWriter);
