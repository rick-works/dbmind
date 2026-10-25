//! **MySQL 写插件**。
//!
//! 落库方式：多行 `INSERT`（复用通用 `SqlWriter`），批量与事务参数写在下面这份 [`SPEC`] 里。
//! 事务靠 `set autocommit = 0` / `commit` 开关；批量收到 1000 行 / 1MB 是链路考虑 ——
//! 一条几 MB 的 INSERT 在公网或 VPN 上一丢包就是整条重传（真机反复「写入中断」）。
//!
//! 批量/事务/自愈相关的差异全部写在下面这份 [`SPEC`] 里 —— 引擎与同步层只问规格，
//! 不再出现数据库名字（见 `transfer/spec.rs` 的模块文档）。

use crate::api::transfer::spec::Spec;

/// MySQL 的搬运规格。
pub(crate) static SPEC: Spec = Spec::new("mysql")
    .with_rows(1_000)
    .with_cap(1024 * 1024)
    .with_txn("set autocommit = 0", "commit", "set autocommit = 1")
    .with_multi_statement()
    .with_conn_id_sql("select connection_id()")
    .with_kill(kill_session)
    .with_activity_sql("select count(*) as n, coalesce(max(time), 0) as longest from information_schema.processlist where command = 'Query' and time >= 5")
    .with_session_setup(&[
        // MDL 默认能等一年 ⇒ 压到 60 秒，卡住时变成「锁等待超时」的报错，而不是永远「同步中」；
        // 跨表同步顺序不保证，外键校验只会制造失败。
        // `unique_checks = 0` 故意不做：唯一约束失效会静默写进重复数据。
        "set session lock_wait_timeout = 60, session innodb_lock_wait_timeout = 60",
        "set session foreign_key_checks = 0",
    ])
    // 断开目标端 binlog（省一半写放大），但会影响主从复制 ⇒ 只在用户显式开启时才做。
    .with_binlog_off("set session sql_log_bin = 0")
    // 纯插入时的 LOAD DATA LOCAL INFILE 快通道（见 transfer/loaddata.rs）：
    // 服务端只要开着 local_infile，同机实测比多值 INSERT 快 3~8 倍。
    // 连接侧开关由宿主建连时打开（MySQL 走 allowLoadLocalInfileInPath 白名单）。
    .with_load_data();

/// `KILL <id>`：终止该连接上正在跑的语句并断开，服务端随之回滚未提交事务、释放行锁。
fn kill_session(id: u64) -> String {
    format!("kill {id}")
}

crate::api::transfer::plugins::plugin_writer!(MysqlWriter);
