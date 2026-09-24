//! Redis 命令的只读判定。
//!
//! Redis 没有「语句」概念：**一次提交就是一条命令**，首 token 是命令名。
//! 于是判定读作两件事：
//!
//! 1. 命令名在只读表里（`GET` / `HGETALL` / `SCAN` / `INFO` …）；
//! 2. 少数命令的**子命令**决定性质（`CONFIG GET` 是读、`CONFIG SET` 是写；
//!    `SORT` 带 `STORE` 会把结果写回键 ⇒ 是写）。
//!
//! 与 SQL / Mongo 两侧同一原则：**判不出来一律按非只读处理**。
//!
//! 另一个刻意决定：`SUBSCRIBE` / `MONITOR` / `BLPOP` 这类**流式或阻塞**命令
//! 归为「无法归类」而不是「只读」—— 本协议的模型是一次请求一次响应，
//! 把它们当只读会让交互模型说谎。

use crate::sql::StatementKind;

/// 只读命令。
const READ_COMMANDS: &[&str] = &[
    // string / 通用
    "GET",
    "GETRANGE",
    "SUBSTR",
    "STRLEN",
    "MGET",
    "GETBIT",
    "BITCOUNT",
    "BITPOS",
    "DUMP",
    "EXISTS",
    "TYPE",
    "TTL",
    "PTTL",
    "EXPIRETIME",
    "PEXPIRETIME",
    "TOUCH",
    "RANDOMKEY",
    "KEYS",
    "SCAN",
    "DBSIZE",
    "SORT_RO",
    // 注意：这里只放「一定只读」的命令名；`CONFIG`/`SORT`/`ACL` 这类
    // 由子命令决定性质的走上面的 match，不在此表
    // hash
    "HGET",
    "HMGET",
    "HGETALL",
    "HKEYS",
    "HLEN",
    "HVALS",
    "HEXISTS",
    "HSTRLEN",
    "HRANDFIELD",
    "HSCAN",
    // list
    "LRANGE",
    "LLEN",
    "LINDEX",
    "LPOS",
    // set
    "SMEMBERS",
    "SCARD",
    "SISMEMBER",
    "SMISMEMBER",
    "SINTER",
    "SUNION",
    "SDIFF",
    "SRANDMEMBER",
    "SSCAN",
    // sorted set
    "ZRANGE",
    "ZREVRANGE",
    "ZRANGEBYSCORE",
    "ZREVRANGEBYSCORE",
    "ZRANGEBYLEX",
    "ZREVRANGEBYLEX",
    "ZCARD",
    "ZSCORE",
    "ZMSCORE",
    "ZRANK",
    "ZREVRANK",
    "ZCOUNT",
    "ZLEXCOUNT",
    "ZSCAN",
    "ZRANDMEMBER",
    "ZDIFF",
    "ZINTER",
    "ZUNION",
    "ZINTERCARD",
    // stream（读侧；XREAD 阻塞特性由宿主决定）
    "XLEN",
    "XRANGE",
    "XREVRANGE",
    "XINFO",
    // 地理 / 概率
    "GEODIST",
    "GEOPOS",
    "GEOHASH",
    "GEOSEARCH",
    "PFCOUNT",
    // 服务与元信息
    "PING",
    "ECHO",
    "INFO",
    "TIME",
    "LASTSAVE",
    "LOLWUT",
    "COMMAND",
    "SLOWLOG",
    "PUBSUB",
    "LATENCY",
    "MEMORY",
    "OBJECT",
];

/// 数据变更命令。
const WRITE_COMMANDS: &[&str] = &[
    // string
    "SET",
    "SETNX",
    "SETEX",
    "PSETEX",
    "GETSET",
    "GETDEL",
    "GETEX",
    "APPEND",
    "SETRANGE",
    "INCR",
    "DECR",
    "INCRBY",
    "DECRBY",
    "INCRBYFLOAT",
    "MSET",
    "MSETNX",
    "SETBIT",
    "BITOP",
    // 键
    "DEL",
    "UNLINK",
    "EXPIRE",
    "PEXPIRE",
    "EXPIREAT",
    "PEXPIREAT",
    "PERSIST",
    "RENAME",
    "RENAMENX",
    "MOVE",
    "COPY",
    "RESTORE",
    "SORT",
    "MIGRATE",
    "PUBLISH",
    "SPUBLISH",
    // hash
    "HSET",
    "HSETNX",
    "HMSET",
    "HDEL",
    "HINCRBY",
    "HINCRBYFLOAT",
    // list
    "LPUSH",
    "RPUSH",
    "LPUSHX",
    "RPUSHX",
    "LPOP",
    "RPOP",
    "LINSERT",
    "LSET",
    "LREM",
    "LTRIM",
    "RPOPLPUSH",
    "LMOVE",
    // set
    "SADD",
    "SREM",
    "SPOP",
    "SMOVE",
    "SINTERSTORE",
    "SUNIONSTORE",
    "SDIFFSTORE",
    // sorted set
    "ZADD",
    "ZINCRBY",
    "ZREM",
    "ZREMRANGEBYRANK",
    "ZREMRANGEBYSCORE",
    "ZREMRANGEBYLEX",
    "ZPOPMIN",
    "ZPOPMAX",
    "ZUNIONSTORE",
    "ZINTERSTORE",
    "ZDIFFSTORE",
    "ZRANGESTORE",
    // stream
    "XADD",
    "XTRIM",
    "XDEL",
    "XACK",
    "XCLAIM",
    "XAUTOCLAIM",
    "XSETID",
    "XGROUP",
    // 概率
    "PFADD",
    "PFMERGE",
    // 地理
    "GEOADD",
];

/// 结构与服务级变更。
const DDL_COMMANDS: &[&str] = &[
    "FLUSHALL",
    "FLUSHDB",
    "SWAPDB",
    "SELECT",
    "BGREWRITEAOF",
    "BGSAVE",
    "SAVE",
    "SHUTDOWN",
    "REPLICAOF",
    "SLAVEOF",
    "SCRIPT",
    "FUNCTION",
    "MODULE",
];

/// 流式/阻塞命令：本协议的请求-响应模型装不下，明确归入「无法归类」。
const STREAMING_COMMANDS: &[&str] = &[
    "SUBSCRIBE",
    "UNSUBSCRIBE",
    "PSUBSCRIBE",
    "PUNSUBSCRIBE",
    "SSUBSCRIBE",
    "SUNSUBSCRIBE",
    "MONITOR",
    "SYNC",
    "PSYNC",
    "BLPOP",
    "BRPOP",
    "BRPOPLPUSH",
    "BLMOVE",
    "BZPOPMIN",
    "BZPOPMAX",
    "XREAD",
    "XREADGROUP",
    "WAIT",
    "MULTI",
    "EXEC",
    "DISCARD",
    "WATCH",
    "UNWATCH",
];

/// 按行拆分：一条命令一行（Redis 没有语句分隔符）。
pub fn split_statements(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect()
}

/// 命令名（首 token，大写）。
pub fn command_name(command: &str) -> Option<String> {
    command
        .split_whitespace()
        .next()
        .map(|token| token.to_ascii_uppercase())
}

/// 子命令（第二个 token，大写）。`CONFIG GET maxmemory` → `GET`。
fn subcommand(command: &str) -> String {
    command
        .split_whitespace()
        .nth(1)
        .map(|token| token.to_ascii_uppercase())
        .unwrap_or_default()
}

/// 去掉引号与转义后的参数串（用于 `SORT ... STORE` 这类关键词检查）。
fn arguments_upper(command: &str) -> String {
    command
        .split_whitespace()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase()
}

pub fn classify(command: &str) -> StatementKind {
    let Some(name) = command_name(command) else {
        return StatementKind::Unknown;
    };
    let name = name.as_str();

    // 流式/阻塞：先判，避免被下面的读表误放
    if STREAMING_COMMANDS.contains(&name) {
        return StatementKind::Unknown;
    }

    match name {
        // 子命令决定性质：GET/RESETSTAT 是读，SET/REWRITE 是写
        "CONFIG" => match subcommand(command).as_str() {
            "GET" | "RESETSTAT" => StatementKind::Read,
            "SET" | "REWRITE" => StatementKind::Ddl,
            _ => StatementKind::Unknown,
        },
        // SORT 默认只读，但带 STORE 会把结果写回一个新键 ⇒ 是写
        "SORT" => {
            if arguments_upper(command).contains("STORE") {
                StatementKind::Write
            } else {
                StatementKind::Read
            }
        }
        "ACL" => match subcommand(command).as_str() {
            "WHOAMI" | "LIST" | "GETUSER" | "CAT" | "USERS" | "DRYRUN" | "GENPASS" => StatementKind::Read,
            "SETUSER" | "DELUSER" | "LOAD" | "SAVE" => StatementKind::Ddl,
            _ => StatementKind::Unknown,
        },
        "CLIENT" => match subcommand(command).as_str() {
            "LIST" | "INFO" | "GETNAME" | "ID" | "NO-EVICT" | "NO-TOUCH" => StatementKind::Read,
            "KILL" | "UNPAUSE" | "SETNAME" => StatementKind::Unknown,
            _ => StatementKind::Unknown,
        },
        "CLUSTER" => match subcommand(command).as_str() {
            "INFO" | "NODES" | "SLOTS" | "SHARDS" | "MYID" | "KEYSLOT" | "COUNTKEYSINSLOT"
            | "GETKEYSINSLOT" | "LINKS" => StatementKind::Read,
            _ => StatementKind::Unknown,
        },
        // OBJECT ENCODING / MEMORY USAGE 这类内省是读
        "OBJECT" | "MEMORY" => StatementKind::Read,
        _ => {
            if READ_COMMANDS.contains(&name) {
                StatementKind::Read
            } else if WRITE_COMMANDS.contains(&name) {
                StatementKind::Write
            } else if DDL_COMMANDS.contains(&name) {
                StatementKind::Ddl
            } else {
                StatementKind::Unknown
            }
        }
    }
}

/// 整段是否只读：必须是单条且为只读。
pub fn is_read_only(command: &str) -> bool {
    let statements = split_statements(command);
    !statements.is_empty() && statements.iter().all(|s| classify(s).is_read_only())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 读命令被认出来() {
        for command in [
            "GET user:1",
            "get user:1",
            "MGET a b c",
            "HGETALL user:1",
            "KEYS user:*",
            "SCAN 0 MATCH user:* COUNT 100",
            "TTL session:1",
            "TYPE k",
            "LRANGE list 0 -1",
            "SMEMBERS tags",
            "ZRANGE rank 0 -1 WITHSCORES",
            "INFO server",
            "DBSIZE",
            "PING",
        ] {
            assert_eq!(classify(command), StatementKind::Read, "{command}");
        }
    }

    #[test]
    fn 写命令被认出来() {
        for command in [
            "SET user:1 alice",
            "setex k 60 v",
            "DEL k1 k2",
            "EXPIRE k 60",
            "INCR counter",
            "HSET user:1 name alice",
            "LPUSH queue job",
            "SADD tags rust",
            "ZADD rank 10 alice",
            "XADD stream * field value",
            "RENAME a b",
            "PUBLISH channel hello",
        ] {
            assert_eq!(classify(command), StatementKind::Write, "{command}");
        }
    }

    #[test]
    fn 结构级命令被认出来() {
        for command in ["FLUSHALL", "FLUSHDB", "SELECT 1", "BGSAVE", "REPLICAOF host 6379"] {
            assert_eq!(classify(command), StatementKind::Ddl, "{command}");
        }
    }

    #[test]
    fn 子命令决定性质() {
        // CONFIG：GET 是读，SET 是结构级写
        assert_eq!(classify("CONFIG GET maxmemory"), StatementKind::Read);
        assert_eq!(classify("config resetstat"), StatementKind::Read);
        assert_eq!(classify("CONFIG SET maxmemory 100mb"), StatementKind::Ddl);
        assert_eq!(classify("CONFIG REWRITE"), StatementKind::Ddl);
        // SORT：默认只读，带 STORE 就是写（这条最容易漏）
        assert_eq!(classify("SORT mylist"), StatementKind::Read);
        assert_eq!(classify("SORT mylist LIMIT 0 10"), StatementKind::Read);
        assert_eq!(
            classify("SORT mylist STORE sorted:list"),
            StatementKind::Write,
            "SORT + STORE 会把结果写回键，必须按写处理"
        );
        // CLUSTER / ACL 内省是读
        assert_eq!(classify("CLUSTER INFO"), StatementKind::Read);
        assert_eq!(classify("ACL LIST"), StatementKind::Read);
        assert_eq!(classify("OBJECT ENCODING k"), StatementKind::Read);
        assert_eq!(classify("MEMORY USAGE k"), StatementKind::Read);
    }

    #[test]
    fn 流式与阻塞命令归入无法归类() {
        // 这些不能当只读：本协议的交互模型是一次请求一次响应
        for command in [
            "SUBSCRIBE news",
            "MONITOR",
            "BLPOP queue 0",
            "MULTI",
            "EXEC",
            "WATCH k",
            "XREAD COUNT 1 STREAMS s $",
        ] {
            assert_eq!(classify(command), StatementKind::Unknown, "{command}");
            assert!(!is_read_only(command), "{command} 不能被判为只读");
        }
    }

    #[test]
    fn 判不出来的一律当非只读() {
        assert_eq!(classify("SOMENEWCOMMAND a b"), StatementKind::Unknown);
        assert_eq!(classify(""), StatementKind::Unknown);
        assert!(!is_read_only("SOMENEWCOMMAND"));
    }

    #[test]
    fn 按行拆分且忽略空行与注释() {
        assert_eq!(
            split_statements("GET a\n\n  # 注释\nSET b 1"),
            vec!["GET a", "SET b 1"]
        );
        // 两条命令不能整体当只读
        assert!(!is_read_only("GET a\nDEL b"));
        assert!(is_read_only("GET a"));
    }
}
