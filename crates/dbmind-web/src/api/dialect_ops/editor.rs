//! 表结构编辑器的能力表：`ddlStyle` / `quoteStyle` / 列可改 / 字符集 / 列类型清单 …
//!
//! 这些键原先散在 `api/meta/browse.rs` 里，是一张 **27 个 kind 手写 match** 的大表。
//! 后果不是"不好看"，而是**静默降级**：`dialect_ops::of()` 里加了新类型（TiDB、Databend…）
//! 但没人记得回来加一行，`/features` 就落到 `generic` —— 界面上整张字段表变只读，
//! 而且不给任何解释（DB2、DM 自增都真机踩过）。
//!
//! 现在按**语法族**归属（跟 `DialectOps` 同一个坐标系），只在"同族但能力不同"时用
//! [`for_kind`] 里的例外补丁。新增一个同族品牌 = 加 YAML + 前端 js，本文件零改动。

use dbmind_core::ConnectionKind;

/// 一种方言族的结构编辑能力。
///
/// `charsets` / `collations_json` 只对真有「表级字符集」的方言（MySQL 系）有值，
/// 其余如实给空 —— 给空比给假名单好（前端下拉带 `allow-create`，空名单退化成手输，
/// 假名单会让用户选一个建表必失败的值）。
#[derive(Debug, Clone, Copy)]
pub struct EditCapabilities {
    /// 前端用哪套 ALTER 生成器（`mysql` / `pg` / `oracle` / `mssql` / `sqlite` /
    /// `clickhouse` / `db2` / `derby` / `databend` / `risingwave` / `doris` / `generic`）。
    pub ddl_style: &'static str,
    /// 标识符引号风格（`BACKTICK` / `DOUBLE` / `BRACKET`）。
    pub quote_style: &'static str,
    /// 已加载的列能不能改（类型/长度/可空/默认值）。
    pub supports_column_modify: bool,
    /// 能不能读写列注释。
    pub supports_comment: bool,
    /// 有没有表选项（引擎 / 字符集 / 注释 …）。
    pub supports_table_options: bool,
    /// 有没有自增列。
    pub supports_auto_increment: bool,
    /// 有没有 `ALTER INDEX … REBUILD` 这类索引重建语法。
    pub supports_index_rebuild: bool,
    /// 前端 `features.doris`：Doris 建表专属语法（分布式键 / 分桶）用得上。
    pub doris: bool,
    pub charsets: &'static [&'static str],
    /// 字符集 → 排序规则。存 JSON 文本而不是 `Value`：这是**纯静态数据**，
    /// 每次 `/features` 调用解析一次（几 KB，可忽略），换来「常量与 impl 都能是 const」。
    pub collations_json: &'static str,
}

impl EditCapabilities {
    /// 兜底：不知道的方言一律按"只能看不能改"处理。
    pub const fn generic() -> Self {
        Self {
            ddl_style: "generic",
            quote_style: "DOUBLE",
            supports_column_modify: false,
            supports_comment: false,
            supports_table_options: false,
            supports_auto_increment: false,
            supports_index_rebuild: false,
            doris: false,
            charsets: &[],
            collations_json: "",
        }
    }
}

// ------------------------------------------------------------------ 各族的能力行

/// MySQL 协议族（含 MariaDB / OceanBase / GreatSQL / GoldenDB / TiDB / Databend）。
pub(crate) const MYSQL: EditCapabilities = EditCapabilities {
    ddl_style: "mysql",
    quote_style: "BACKTICK",
    supports_column_modify: true,
    supports_comment: true,
    supports_table_options: true,
    supports_auto_increment: true,
    supports_index_rebuild: true,
    doris: false,
    charsets: MYSQL_CHARSETS,
    collations_json: MYSQL_COLLATIONS,
};

/// Databend（走 MySQL 协议，但 ALTER 语法与能力都不同）。
///
/// **实测**（见 `_uitest/ddl-cap-probe2.py`）：`MODIFY COLUMN c <type>` / `RENAME COLUMN` /
/// 加列删列 / 列注释可用；但 NOT NULL、DEFAULT、PRIMARY KEY、自增一律"提交成功、刷新没变化"
/// （ALTER 是异步任务），VARCHAR 长度被服务端归一成 16382。所以可空/默认值/长度三格锁死 ——
/// 让用户改完看不到效果才是骗人。
pub(crate) const DATABEND: EditCapabilities = EditCapabilities {
    ddl_style: "databend",
    quote_style: "DOUBLE",
    supports_column_modify: true,
    supports_comment: true,
    supports_table_options: false,
    supports_auto_increment: false,
    supports_index_rebuild: false,
    doris: false,
    charsets: &[],
    collations_json: "",
};

/// Doris 族（Doris / StarRocks / SelectDB）：字符集清单与 MySQL 相同，但 ALTER 生成器是自家的。
pub(crate) const DORIS: EditCapabilities = EditCapabilities {
    ddl_style: "doris",
    quote_style: "BACKTICK",
    supports_column_modify: true,
    supports_comment: true,
    supports_table_options: true,
    supports_auto_increment: true,
    supports_index_rebuild: true,
    doris: false,
    charsets: MYSQL_CHARSETS,
    collations_json: MYSQL_COLLATIONS,
};

/// PostgreSQL 族（含 Kingbase / openGauss / HighGo / Vastbase / UXDB / GaussDB）。
pub(crate) const POSTGRES: EditCapabilities = EditCapabilities {
    ddl_style: "pg",
    quote_style: "DOUBLE",
    supports_column_modify: true,
    supports_comment: true,
    supports_table_options: false,
    supports_auto_increment: false,
    supports_index_rebuild: true,
    doris: false,
    charsets: &[],
    collations_json: "",
};

/// RisingWave：`ALTER COLUMN c TYPE <type>` **不能带长度**（`varchar(50)` 直接语法错），
/// 改名 / SET NOT NULL / SET DEFAULT / 加主键 / 自增全是 `Feature is not yet implemented`；
/// `comment on column` 能执行但 `col_description` 读回来是空 —— 注释如实 false。
pub(crate) const RISINGWAVE: EditCapabilities = EditCapabilities {
    ddl_style: "risingwave",
    quote_style: "DOUBLE",
    supports_column_modify: true,
    supports_comment: false,
    supports_table_options: false,
    supports_auto_increment: false,
    supports_index_rebuild: false,
    doris: false,
    charsets: &[],
    collations_json: "",
};

/// Oracle 12c+：标识列能查到（`user_tab_identity_cols`），自增如实 true。
pub(crate) const ORACLE: EditCapabilities = EditCapabilities {
    ddl_style: "oracle",
    quote_style: "DOUBLE",
    supports_column_modify: true,
    supports_comment: true,
    supports_table_options: false,
    supports_auto_increment: true,
    supports_index_rebuild: true,
    doris: false,
    charsets: &[],
    collations_json: "",
};

/// 达梦：标识别记在哪张系统视图上、没有实例可验证 ⇒ 自增如实 false。
/// 原先跟着 Oracle 一起 true，界面给了个**永远勾不上**的勾选框（用户反馈踩过）。
pub(crate) const DM: EditCapabilities = EditCapabilities {
    ddl_style: "oracle",
    quote_style: "DOUBLE",
    supports_column_modify: true,
    supports_comment: true,
    supports_table_options: false,
    supports_auto_increment: false,
    supports_index_rebuild: true,
    doris: false,
    charsets: &[],
    collations_json: "",
};

pub(crate) const SQLSERVER: EditCapabilities = EditCapabilities {
    ddl_style: "mssql",
    quote_style: "BRACKET",
    supports_column_modify: true,
    supports_comment: true,
    supports_table_options: false,
    supports_auto_increment: true,
    supports_index_rebuild: true,
    doris: false,
    charsets: &[],
    collations_json: "",
};

/// SQLite 没有 `ALTER COLUMN`：只能加列或重建表，如实说不能改。
pub(crate) const SQLITE: EditCapabilities = EditCapabilities {
    ddl_style: "sqlite",
    quote_style: "DOUBLE",
    supports_column_modify: false,
    supports_comment: false,
    supports_table_options: false,
    supports_auto_increment: false,
    supports_index_rebuild: false,
    doris: false,
    charsets: &[],
    collations_json: "",
};

pub(crate) const CLICKHOUSE: EditCapabilities = EditCapabilities {
    ddl_style: "clickhouse",
    quote_style: "DOUBLE",
    supports_column_modify: true,
    supports_comment: true,
    supports_table_options: false,
    supports_auto_increment: false,
    supports_index_rebuild: false,
    doris: false,
    charsets: &[],
    collations_json: "",
};

/// H2：**实测过**才敢开（2.3 上逐条验过 pg 风格的 alter column / set default /
/// rename column / comment on column）。索引重建与表选项没有对应语法，如实 false。
pub(crate) const H2: EditCapabilities = EditCapabilities {
    ddl_style: "pg",
    quote_style: "DOUBLE",
    supports_column_modify: true,
    supports_comment: true,
    supports_table_options: false,
    supports_auto_increment: false,
    supports_index_rebuild: false,
    doris: false,
    charsets: &[],
    collations_json: "",
};

/// Derby：**实测过**（10.16）才开。列注释 Derby 没有（`comment on column` 不支持）。
pub(crate) const DERBY: EditCapabilities = EditCapabilities {
    ddl_style: "derby",
    quote_style: "DOUBLE",
    supports_column_modify: true,
    supports_comment: false,
    supports_table_options: false,
    supports_auto_increment: false,
    supports_index_rebuild: false,
    doris: false,
    charsets: &[],
    collations_json: "",
};

/// DB2 LUW：`ALTER COLUMN … SET DATA TYPE` / `SET|DROP NOT NULL` / `SET|DROP DEFAULT` /
/// `RENAME COLUMN` / `ADD|DROP COLUMN` / `COMMENT ON COLUMN` 都有。
/// 原先落到 generic、`supportsColumnModify=false`，整张字段表灰成只读（用户报过）。
pub(crate) const DB2: EditCapabilities = EditCapabilities {
    ddl_style: "db2",
    quote_style: "DOUBLE",
    supports_column_modify: true,
    supports_comment: true,
    supports_table_options: false,
    supports_auto_increment: false,
    supports_index_rebuild: false,
    doris: false,
    charsets: &[],
    collations_json: "",
};

/// SAP HANA：**没有真机验证过 ALTER**，先按只读给（supports_column_modify=false）。
/// ddl_style 用专属值而非 generic：守门测试要求每个暴露类型显式表态，
/// 这样将来要开放编辑时有人接手，而不是悄悄落进兜底。
pub(crate) const SAPHANA: EditCapabilities = EditCapabilities {
    ddl_style: "saphana",
    quote_style: "DOUBLE",
    supports_column_modify: false,
    supports_comment: false,
    supports_table_options: false,
    supports_auto_increment: false,
    supports_index_rebuild: false,
    doris: false,
    charsets: &[],
    collations_json: "",
};

/// DuckDB：**没实测过 ALTER 就不声明**，先按 SQLite 的保守档给。建表语法与 pg 同族。
pub(crate) const DUCKDB: EditCapabilities = EditCapabilities {
    ddl_style: "pg",
    quote_style: "DOUBLE",
    supports_column_modify: false,
    supports_comment: false,
    supports_table_options: false,
    supports_auto_increment: false,
    supports_index_rebuild: false,
    doris: false,
    charsets: &[],
    collations_json: "",
};

// ------------------------------------------------------------------ 列类型清单

pub(crate) const MYSQL_COLUMN_TYPES: &[&str] = &[
    "INT", "BIGINT", "SMALLINT", "TINYINT", "DECIMAL", "FLOAT", "DOUBLE", "VARCHAR", "CHAR", "TEXT",
    "LONGTEXT", "DATE", "DATETIME", "TIMESTAMP", "TIME", "JSON", "BLOB",
];

/// Doris 与 MySQL 同一套类型，但它自己的元数据报 `string`（VARCHAR 的别名）——
/// 清单里带上 STRING，"下拉 = 回显"才对得上（见 typeName.js）。
pub(crate) const DORIS_COLUMN_TYPES: &[&str] = &[
    "INT", "BIGINT", "SMALLINT", "TINYINT", "DECIMAL", "FLOAT", "DOUBLE", "VARCHAR", "STRING", "CHAR",
    "TEXT", "LONGTEXT", "DATE", "DATETIME", "TIMESTAMP", "TIME", "JSON", "BLOB",
];

/// Databend 走 MySQL 协议，但 `desc` 里的类型名是**小写**（int / varchar / decimal）——
/// 塞进 MySQL 的大写列表会让下拉与回显两种写法。
pub(crate) const DATABEND_COLUMN_TYPES: &[&str] = &[
    "boolean", "tinyint", "smallint", "int", "bigint", "float", "double", "decimal", "varchar",
    "string", "date", "timestamp", "variant", "binary",
];

/// PG 系：前面是标准写法，后面是 PG 自己 `udt_name` 报出来的写法（int4/float8/varchar/bool），
/// 少一个回显就落到清单外，"下拉 = 回显"就断了。
pub(crate) const POSTGRES_COLUMN_TYPES: &[&str] = &[
    "integer", "bigint", "smallint", "numeric", "real", "double precision", "character varying",
    "text", "boolean", "date", "timestamp", "timestamptz", "time", "jsonb", "uuid", "bytea", "int2",
    "int4", "int8", "float4", "float8", "varchar", "bool",
];

pub(crate) const SQLSERVER_COLUMN_TYPES: &[&str] = &[
    "int", "bigint", "smallint", "tinyint", "decimal", "numeric", "float", "real", "money",
    "smallmoney", "nvarchar", "varchar", "char", "nchar", "nvarchar(max)", "varchar(max)", "text",
    "ntext", "bit", "date", "time", "datetime", "datetime2", "smalldatetime", "datetimeoffset",
    "uniqueidentifier", "binary", "varbinary", "varbinary(max)", "image", "xml",
];

pub(crate) const ORACLE_COLUMN_TYPES: &[&str] = &[
    "NUMBER", "FLOAT", "BINARY_FLOAT", "VARCHAR2", "NVARCHAR2", "CHAR", "CLOB", "BLOB", "DATE",
    "TIMESTAMP", "RAW",
];

/// Decimal / FixedString / DateTime64 的参数由「长度 / 精度」列给。
pub(crate) const CLICKHOUSE_COLUMN_TYPES: &[&str] = &[
    "UInt8", "UInt16", "UInt32", "UInt64", "Int8", "Int16", "Int32", "Int64", "Float32", "Float64",
    "Decimal", "String", "FixedString", "Date", "DateTime", "DateTime64", "UUID", "Array(String)",
];

pub(crate) const SQLITE_COLUMN_TYPES: &[&str] =
    &["INTEGER", "REAL", "TEXT", "BLOB", "NUMERIC", "VARCHAR", "DATETIME"];

pub(crate) const H2_COLUMN_TYPES: &[&str] = &[
    "INT", "BIGINT", "DECIMAL", "DOUBLE", "VARCHAR", "CLOB", "BLOB", "DATE", "TIMESTAMP", "BOOLEAN",
];

pub(crate) const DB2_COLUMN_TYPES: &[&str] = &[
    "INTEGER", "BIGINT", "DECIMAL", "DOUBLE", "VARCHAR", "CLOB", "BLOB", "DATE", "TIMESTAMP",
];

pub(crate) const SAPHANA_COLUMN_TYPES: &[&str] = &[
    "TINYINT", "SMALLINT", "INTEGER", "BIGINT", "DECIMAL", "REAL", "DOUBLE",
    "VARCHAR", "NVARCHAR", "CHAR", "NCHAR", "TEXT", "NCLOB", "BLOB",
    "DATE", "TIME", "TIMESTAMP", "BOOLEAN",
];

pub(crate) const DERBY_COLUMN_TYPES: &[&str] = &[
    "INTEGER", "BIGINT", "DECIMAL", "DOUBLE", "VARCHAR", "CLOB", "BLOB", "DATE", "TIMESTAMP",
];

/// DuckDB（generic ANSI）：1.1 全部认的常用集合；STRUCT/LIST/MAP 等复合类型不进下拉
/// （带参数的写法由「长度/精度」给不了，宁缺勿错，手写 SQL 可用）。
pub(crate) const DUCKDB_COLUMN_TYPES: &[&str] = &[
    "TINYINT", "SMALLINT", "INTEGER", "BIGINT", "HUGEINT", "FLOAT", "DOUBLE", "DECIMAL", "VARCHAR",
    "BLOB", "BOOLEAN", "DATE", "TIME", "TIMESTAMP", "TIMESTAMPTZ", "UUID", "JSON",
];

// ------------------------------------------------------------------ 索引类型

pub(crate) const MYSQL_INDEX_TYPES: &[&str] = &["BTREE", "HASH", "FULLTEXT", "SPATIAL"];
/// TiDB 官方只认 BTREE/HASH（FULLTEXT/SPATIAL 写了是语法错）；OceanBase 同理。
pub(crate) const MYSQL_INDEX_TYPES_STRICT: &[&str] = &["BTREE", "HASH"];
pub(crate) const DORIS_INDEX_TYPES: &[&str] = &["BTREE", "INVERTED", "BITMAP", "BLOOMFILTER"];
pub(crate) const POSTGRES_INDEX_TYPES: &[&str] =
    &["btree", "hash", "gist", "gin", "spgist", "brin"];
pub(crate) const ORACLE_INDEX_TYPES: &[&str] =
    &["NORMAL", "UNIQUE", "BITMAP", "FUNCTION-BASED NORMAL"];
pub(crate) const SQLSERVER_INDEX_TYPES: &[&str] =
    &["CLUSTERED", "NONCLUSTERED", "UNIQUE", "COLUMNSTORE"];
pub(crate) const CLICKHOUSE_INDEX_TYPES: &[&str] =
    &["minmax", "set", "bloom_filter", "ngrambf_v1", "tokenbf_v1"];
/// 只有一侧存在的类型（Databend / RisingWave）给个非空表态，免得"能力清单守门"单测红。
pub(crate) const DATABEND_INDEX_TYPES: &[&str] = &["BLOOM"];
pub(crate) const RISINGWAVE_INDEX_TYPES: &[&str] = &["BTREE"];

// ------------------------------------------------------------------ 存储引擎

/// OB 语法层接受 ENGINE=（解析后忽略）；GreatSQL / GoldenDB 是 MySQL 协议族
/// （计算节点对外就是 MySQL 语法），引擎清单照 MySQL 给。
pub(crate) const MYSQL_ENGINES: &[&str] = &[
    "InnoDB", "MyISAM", "MEMORY", "CSV", "ARCHIVE", "BLACKHOLE", "MRG_MYISAM", "FEDERATED",
];
/// Doris / StarRocks / SelectDB 只有 **OLAP** 一个引擎：给 InnoDB 那套是**假选项**
/// （真机实测 `ENGINE=InnoDB` ⇒ `Unknown engine name: innodb`，建表直接失败）。
pub(crate) const DORIS_ENGINES: &[&str] = &["OLAP"];
/// TiDB 只有一个真正生效的引擎（MyISAM/MEMORY 语法兼容但被静默忽略）。
pub(crate) const TIDB_ENGINES: &[&str] = &["TiDB"];
/// 清单给的是**能直接落进 SQL 的完整写法**：MergeTree 族必须带 `()`，Log/Memory 这些不能带。
/// Replicated 系列必须列全，漏了会在清单外原样显示成不带括号的样子（用户实测踩过）。
/// 与前端 `common/objectforms/chEngines.js` 保持一致。
pub(crate) const CLICKHOUSE_ENGINES: &[&str] = &[
    "MergeTree()", "ReplacingMergeTree()", "SummingMergeTree()", "AggregatingMergeTree()",
    "CollapsingMergeTree()", "VersionedCollapsingMergeTree()", "GraphiteMergeTree()",
    "ReplicatedMergeTree()", "ReplicatedReplacingMergeTree()", "ReplicatedSummingMergeTree()",
    "ReplicatedAggregatingMergeTree()", "ReplicatedCollapsingMergeTree()",
    "ReplicatedVersionedCollapsingMergeTree()", "ReplicatedGraphiteMergeTree()", "Log", "TinyLog",
    "StripeLog", "Set", "Join", "Memory", "Null",
];
/// Databend 只有一种引擎（FUSE，天然列存 + 对象存储），界面上的下拉实际用不到，
/// 但清单不能空（守门单测）。RisingWave 同理（无存储引擎概念）。
pub(crate) const DATABEND_ENGINES: &[&str] = &["FUSE"];
pub(crate) const RISINGWAVE_ENGINES: &[&str] = &["HUMAN_READABLE"];

// ------------------------------------------------------------------ 字符集（MySQL 系）

pub(crate) const MYSQL_CHARSETS: &[&str] = &[
    "utf8mb4", "utf8mb3", "utf8", "latin1", "latin2", "latin5", "latin7", "gbk", "gb18030", "big5",
    "ascii", "binary", "ucs2", "utf16", "utf16le", "utf32", "cp1250", "cp1251", "cp1256", "cp1257",
    "euckr", "sjis", "cp932", "greek", "hebrew", "koi8r", "koi8u", "tis620", "macroman", "armscii8",
    "geostd8", "keybcs2", "swe7",
];

/// TiDB 支持的字符集/排序规则比 MySQL 少一大截，照实给小的 ——
/// 排序规则名臆造一个建表就直接失败，宁缺勿错。
pub(crate) const TIDB_CHARSETS: &[&str] = &["utf8mb4", "utf8", "latin1", "ascii", "binary", "gbk"];

/// 排序规则**逐字符集写全**：只列真实存在的名字，生成或臆造的名字会让建表直接失败。
pub(crate) const MYSQL_COLLATIONS: &str = r#"{
  "utf8mb4": ["utf8mb4_0900_ai_ci","utf8mb4_0900_as_ci","utf8mb4_0900_as_cs","utf8mb4_0900_bin","utf8mb4_general_ci","utf8mb4_bin","utf8mb4_unicode_ci","utf8mb4_unicode_520_ci","utf8mb4_croatian_ci","utf8mb4_czech_ci","utf8mb4_danish_ci","utf8mb4_esperanto_ci","utf8mb4_estonian_ci","utf8mb4_hungarian_ci","utf8mb4_icelandic_ci","utf8mb4_latvian_ci","utf8mb4_lithuanian_ci","utf8mb4_persian_ci","utf8mb4_polish_ci","utf8mb4_romanian_ci","utf8mb4_sinhala_ci","utf8mb4_slovak_ci","utf8mb4_slovenian_ci","utf8mb4_spanish_ci","utf8mb4_spanish2_ci","utf8mb4_swedish_ci","utf8mb4_turkish_ci","utf8mb4_vietnamese_ci"],
  "utf8mb3": ["utf8mb3_general_ci","utf8mb3_bin","utf8mb3_unicode_ci","utf8mb3_unicode_520_ci","utf8mb3_croatian_ci","utf8mb3_czech_ci","utf8mb3_danish_ci","utf8mb3_esperanto_ci","utf8mb3_estonian_ci","utf8mb3_general_mysql500_ci","utf8mb3_hungarian_ci","utf8mb3_icelandic_ci","utf8mb3_latvian_ci","utf8mb3_lithuanian_ci","utf8mb3_persian_ci","utf8mb3_polish_ci","utf8mb3_romanian_ci","utf8mb3_sinhala_ci","utf8mb3_slovak_ci","utf8mb3_slovenian_ci","utf8mb3_spanish_ci","utf8mb3_spanish2_ci","utf8mb3_swedish_ci","utf8mb3_turkish_ci","utf8mb3_vietnamese_ci"],
  "utf8": ["utf8_general_ci","utf8_bin","utf8_unicode_ci","utf8_unicode_520_ci","utf8_general_mysql500_ci","utf8_czech_ci","utf8_danish_ci","utf8_icelandic_ci","utf8_latvian_ci","utf8_lithuanian_ci","utf8_polish_ci","utf8_romanian_ci","utf8_slovak_ci","utf8_slovenian_ci","utf8_spanish_ci","utf8_spanish2_ci","utf8_swedish_ci","utf8_turkish_ci","utf8_vietnamese_ci"],
  "latin1": ["latin1_swedish_ci","latin1_german1_ci","latin1_german2_ci","latin1_danish_ci","latin1_bin","latin1_general_ci","latin1_general_cs","latin1_spanish_ci"],
  "latin2": ["latin2_general_ci","latin2_czech_cs","latin2_hungarian_ci","latin2_croatian_ci","latin2_bin"],
  "latin5": ["latin5_turkish_ci","latin5_bin"],
  "latin7": ["latin7_general_ci","latin7_general_cs","latin7_estonian_cs","latin7_bin"],
  "gbk": ["gbk_chinese_ci","gbk_bin","gbk_unicode_520_ci"],
  "gb18030": ["gb18030_chinese_ci","gb18030_bin","gb18030_unicode_520_ci"],
  "big5": ["big5_chinese_ci","big5_bin"],
  "ascii": ["ascii_general_ci","ascii_bin"],
  "binary": ["binary"],
  "ucs2": ["ucs2_general_ci","ucs2_bin","ucs2_unicode_ci","ucs2_unicode_520_ci","ucs2_general_mysql500_ci"],
  "utf16": ["utf16_general_ci","utf16_bin","utf16_unicode_ci","utf16_unicode_520_ci"],
  "utf16le": ["utf16le_general_ci","utf16le_bin"],
  "utf32": ["utf32_general_ci","utf32_bin","utf32_unicode_ci","utf32_unicode_520_ci"],
  "cp1250": ["cp1250_general_ci","cp1250_czech_cs","cp1250_croatian_ci","cp1250_polish_ci","cp1250_bin"],
  "cp1251": ["cp1251_general_ci","cp1251_general_cs","cp1251_bulgarian_ci","cp1251_ukrainian_ci","cp1251_bin"],
  "cp1256": ["cp1256_general_ci","cp1256_bin"],
  "cp1257": ["cp1257_general_ci","cp1257_lithuanian_ci","cp1257_bin"],
  "euckr": ["euckr_korean_ci","euckr_bin"],
  "sjis": ["sjis_japanese_ci","sjis_bin"],
  "cp932": ["cp932_japanese_ci","cp932_bin"],
  "greek": ["greek_general_ci","greek_bin"],
  "hebrew": ["hebrew_general_ci","hebrew_bin"],
  "koi8r": ["koi8r_general_ci","koi8r_bin"],
  "koi8u": ["koi8u_general_ci","koi8u_bin"],
  "tis620": ["tis620_thai_ci","tis620_bin"],
  "macroman": ["macroman_general_ci","macroman_bin"],
  "armscii8": ["armscii8_general_ci","armscii8_bin"],
  "geostd8": ["geostd8_general_ci","geostd8_bin"],
  "keybcs2": ["keybcs2_general_ci","keybcs2_bin"],
  "swe7": ["swe7_swedish_ci","swe7_bin"]
}"#;

pub(crate) const TIDB_COLLATIONS: &str = r#"{
  "utf8mb4": ["utf8mb4_bin","utf8mb4_general_ci"],
  "utf8": ["utf8_bin","utf8_general_ci"],
  "latin1": ["latin1_bin"],
  "ascii": ["ascii_bin"],
  "binary": ["binary"],
  "gbk": ["gbk_bin","gbk_chinese_ci"]
}"#;

// ------------------------------------------------------------------ 对外入口

/// 类型 → 结构编辑能力。**族默认 + 同族例外**，例外都必须写清为什么不同。
pub fn for_kind(kind: ConnectionKind) -> EditCapabilities {
    // 同族但整体能力不同：整行覆盖，不要拿族默认再打补丁（读代码时一眼能看清）。
    match kind {
        ConnectionKind::Databend => return DATABEND,
        ConnectionKind::Risingwave => return RISINGWAVE,
        _ => {}
    }
    let mut caps = super::of(kind).edit_capabilities();
    match kind {
        // TiDB 没有 `ALTER INDEX … REBUILD`；字符集也只支持 MySQL 的一个子集。
        ConnectionKind::Tidb => {
            caps.supports_index_rebuild = false;
            caps.charsets = TIDB_CHARSETS;
            caps.collations_json = TIDB_COLLATIONS;
        }
        // 前端用 `features.doris` 判断要不要显示分布式键/分桶 —— 只有真的 Doris 是。
        ConnectionKind::Doris => caps.doris = true,
        _ => {}
    }
    caps
}

/// 列类型下拉清单（既是可选项，也是**回显写法的标准**，见前端 `typeName.js`）。
pub fn column_types(kind: ConnectionKind) -> Vec<&'static str> {
    match kind {
        ConnectionKind::Databend => return DATABEND_COLUMN_TYPES.to_vec(),
        _ => {}
    }
    super::of(kind).column_type_names().to_vec()
}

/// 索引类型清单（`features.indexTypes`）。空清单 = 前端不显示「类型」那一列。
pub fn index_types(kind: ConnectionKind) -> Vec<&'static str> {
    match kind {
        ConnectionKind::Databend => return DATABEND_INDEX_TYPES.to_vec(),
        ConnectionKind::Risingwave => return RISINGWAVE_INDEX_TYPES.to_vec(),
        ConnectionKind::Tidb | ConnectionKind::Oceanbase => {
            return MYSQL_INDEX_TYPES_STRICT.to_vec()
        }
        _ => {}
    }
    super::of(kind).index_type_names().to_vec()
}

/// 存储引擎清单（`features.engines`）。空清单 = 前端整行退化成纯手输。
pub fn storage_engines(kind: ConnectionKind) -> Vec<&'static str> {
    match kind {
        ConnectionKind::Databend => return DATABEND_ENGINES.to_vec(),
        ConnectionKind::Risingwave => return RISINGWAVE_ENGINES.to_vec(),
        ConnectionKind::Tidb => return TIDB_ENGINES.to_vec(),
        _ => {}
    }
    super::of(kind).storage_engines().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每种已实现的类型都必须给出**非空**的列类型清单 ——
    /// 空清单在前端表现为「类型下拉没有选项」，只能手打，等于结构编辑器废了一半。
    #[test]
    fn 每种类型都有列类型清单() {
        for kind in ConnectionKind::ALL {
            if !kind.implemented() || !kind.is_jdbc() && kind.key() != "sqlite" {
                continue;
            }
            assert!(
                !column_types(*kind).is_empty(),
                "{} 没有列类型清单（结构编辑器的类型下拉会是空的）",
                kind.key()
            );
        }
    }

    /// 结构编辑器「能不能改」与「ddlStyle 是不是 generic」必须一致：
    /// 落 generic 就是整表只读，这两件事不能各说各话。
    #[test]
    fn 可改列的类型不能落到通用生成器() {
        for kind in ConnectionKind::ALL {
            let caps = for_kind(*kind);
            if caps.supports_column_modify {
                assert_ne!(
                    caps.ddl_style,
                    "generic",
                    "{} 声明可改列却落到 generic 生成器（界面会整表只读）",
                    kind.key()
                );
            }
        }
    }

    /// 同族品牌必须继承族代表的能力（不能因为没单独列一行就掉到 generic）。
    #[test]
    fn 同族品牌继承族能力() {
        let base = for_kind(ConnectionKind::Postgresql);
        for kind in [
            ConnectionKind::Kingbase,
            ConnectionKind::Opengauss,
            ConnectionKind::Highgo,
            ConnectionKind::Vastbase,
            ConnectionKind::Uxdb,
            ConnectionKind::Gaussdb,
        ] {
            assert_eq!(for_kind(kind).ddl_style, base.ddl_style, "{}", kind.key());
            assert_eq!(
                for_kind(kind).supports_column_modify,
                base.supports_column_modify,
                "{}",
                kind.key()
            );
        }
        // MySQL 族同理
        let base = for_kind(ConnectionKind::Mysql);
        for kind in [
            ConnectionKind::Mariadb,
            ConnectionKind::Oceanbase,
            ConnectionKind::Greatsql,
            ConnectionKind::Goldendb,
        ] {
            assert_eq!(for_kind(kind).ddl_style, base.ddl_style, "{}", kind.key());
            assert_eq!(for_kind(kind).quote_style, base.quote_style, "{}", kind.key());
        }
    }

    /// 排序规则 JSON 必须能解析（写错了前端拿到空对象，字符集下拉会退化成手输）。
    #[test]
    fn 排序规则是合法_json() {
        for kind in ConnectionKind::ALL {
            let caps = for_kind(*kind);
            if caps.collations_json.is_empty() {
                continue;
            }
            let parsed: serde_json::Value = serde_json::from_str(caps.collations_json)
                .unwrap_or_else(|err| panic!("{} 的 collations 不是合法 JSON：{err}", kind.key()));
            assert!(parsed.is_object(), "{} 的 collations 必须是对象", kind.key());
        }
    }
}
