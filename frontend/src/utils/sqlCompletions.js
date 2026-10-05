// SQL 编辑器补全用的**内置函数清单**：按格式化方言（connDialectOf 的返回值）给出。
//
// 覆盖面：各数据库官方文档里的常用内置函数基本收齐（聚合 / 数学 / 字符串 / 日期时间 /
// 控制 / 类型转换 / JSON / 加密 / 系统信息）， Oracle、SQL Server、PostgreSQL、MySQL
// 各自成表，SQLite 按其文档全量收录（它本来就只有几十个函数）。
// 没收录的函数照常手写，不受影响；分词器高亮用的是 allBuiltinFunctions() 的并集。

// 各方言通用的核心函数（所有 SQL 数据库都有）
const COMMON = [
  'COUNT', 'SUM', 'AVG', 'MIN', 'MAX',
  'CURRENT_DATE', 'CURRENT_TIME', 'CURRENT_TIMESTAMP',
  'ROUND', 'ABS', 'CEIL', 'FLOOR', 'MOD', 'SIGN', 'SQRT', 'POWER',
  'COALESCE', 'NULLIF', 'CAST',
  'UPPER', 'LOWER', 'LENGTH', 'CHAR_LENGTH', 'TRIM', 'LTRIM', 'RTRIM',
  'REPLACE', 'SUBSTRING', 'SUBSTR', 'CONCAT', 'INSTR', 'POSITION',
  'YEAR', 'MONTH', 'DAY', 'HOUR', 'MINUTE', 'SECOND', 'EXTRACT',
  'ROW_NUMBER', 'RANK', 'DENSE_RANK', 'LAG', 'LEAD', 'FIRST_VALUE', 'LAST_VALUE'
]

const DIALECT_EXTRA = {
  mysql: [
    // 控制 / 比较
    'IF', 'IFNULL', 'ISNULL', 'NOW',
    // 聚合
    'GROUP_CONCAT',
    // 字符串
    'ASCII', 'BIN', 'CHAR', 'CHAR_LENGTH', 'CHARACTER_LENGTH', 'CONCAT_WS',
    'ELT', 'EXPORT_SET', 'FIELD', 'FIND_IN_SET', 'FORMAT', 'HEX', 'INSERT',
    'LCASE', 'LEFT', 'LENGTH', 'LOAD_FILE', 'LOCATE', 'LPAD', 'LTRIM',
    'MAKE_SET', 'MID', 'OCT', 'ORD', 'POSITION', 'QUOTE', 'REPEAT',
    'REVERSE', 'RIGHT', 'RPAD', 'RTRIM', 'SOUNDEX', 'SPACE', 'STRCMP',
    'SUBSTRING_INDEX', 'UCASE',
    // 数学
    'ACOS', 'ASIN', 'ATAN', 'ATAN2', 'CEILING', 'CONV', 'COS', 'COT',
    'CRC32', 'DEGREES', 'EXP', 'LN', 'LOG', 'LOG2', 'LOG10', 'PI', 'POW',
    'RADIANS', 'RAND', 'SIGN', 'SQRT', 'TRUNCATE',
    // 日期时间
    'ADDDATE', 'ADDTIME', 'CURDATE', 'CURTIME', 'DATE', 'DATEDIFF',
    'DATE_ADD', 'DATE_FORMAT', 'DATE_SUB', 'DAYNAME', 'DAYOFMONTH',
    'DAYOFWEEK', 'DAYOFYEAR', 'EXTRACT', 'FROM_DAYS', 'FROM_UNIXTIME',
    'GET_FORMAT', 'LAST_DAY', 'LOCALTIME', 'LOCALTIMESTAMP', 'MAKEDATE',
    'MAKETIME', 'MICROSECOND', 'MONTHNAME', 'PERIOD_ADD', 'PERIOD_DIFF',
    'QUARTER', 'SEC_TO_TIME', 'STR_TO_DATE', 'SUBDATE', 'SUBTIME',
    'SYSDATE', 'TIME', 'TIMEDIFF', 'TIMESTAMP', 'TIMESTAMPADD',
    'TIMESTAMPDIFF', 'TIME_FORMAT', 'TIME_TO_SEC', 'TO_DAYS', 'TO_SECONDS',
    'UNIX_TIMESTAMP', 'UTC_DATE', 'UTC_TIME', 'UTC_TIMESTAMP', 'WEEK',
    'WEEKDAY', 'WEEKOFYEAR', 'YEARWEEK',
    // JSON
    'JSON_APPEND', 'JSON_ARRAY', 'JSON_ARRAY_APPEND', 'JSON_ARRAY_INSERT',
    'JSON_CONTAINS', 'JSON_CONTAINS_PATH', 'JSON_DEPTH', 'JSON_EXTRACT',
    'JSON_INSERT', 'JSON_KEYS', 'JSON_LENGTH', 'JSON_MERGE', 'JSON_OBJECT',
    'JSON_QUOTE', 'JSON_REMOVE', 'JSON_REPLACE', 'JSON_SEARCH', 'JSON_SET',
    'JSON_TYPE', 'JSON_UNQUOTE', 'JSON_VALID',
    // 加密 / 杂项
    'AES_DECRYPT', 'AES_ENCRYPT', 'COMPRESS', 'MD5', 'RANDOM_BYTES',
    'SHA1', 'SHA2', 'SHA', 'UNCOMPRESS', 'UNCOMPRESSED_LENGTH', 'UNHEX',
    'UUID', 'UUID_SHORT', 'BENCHMARK', 'CHAR', 'COERCIBILITY',
    'COLLATION', 'CONNECTION_ID', 'CURRENT_USER', 'DATABASE', 'FOUND_ROWS',
    'LAST_INSERT_ID', 'ROW_COUNT', 'SCHEMA', 'SESSION_USER', 'SYSTEM_USER',
    'USER', 'VERSION', 'DEFAULT', 'GROUPING', 'VARIANCE', 'VAR_POP', 'VAR_SAMP',
    'STDDEV', 'STDDEV_POP', 'STDDEV_SAMP', 'ANY_VALUE', 'INVERT'
  ],
  postgres: [
    'NOW',
    // 字符串
    'BIT_LENGTH', 'BTRIM', 'CHR', 'CONCAT_WS', 'CONVERT', 'DECODE',
    'ENCODE', 'INITCAP', 'LEFT', 'LPAD', 'MD5', 'OVERLAY', 'QUOTE_IDENT',
    'QUOTE_LITERAL', 'QUOTE_NULLABLE', 'REGEXP_MATCHES', 'REGEXP_REPLACE',
    'REGEXP_SPLIT_TO_ARRAY', 'REGEXP_SPLIT_TO_TABLE', 'REPEAT', 'RIGHT',
    'RPAD', 'SPLIT_PART', 'STARTS_WITH', 'STRING_AGG', 'STRING_TO_ARRAY',
    'STRIPOS', 'STRPOS', 'SUBSTR', 'TO_ASCII', 'TO_HEX', 'TRANSLATE',
    // 数学
    'CBRT', 'CEILING', 'DEGREES', 'DIV', 'EXP', 'LN', 'LOG', 'RADIANS',
    'RANDOM', 'SETSEED', 'TRUNC', 'WIDTH_BUCKET',
    // 日期时间
    'AGE', 'CLOCK_TIMESTAMP', 'DATE_TRUNC', 'ISFINITE', 'JUSTIFY_DAYS',
    'JUSTIFY_HOURS', 'JUSTIFY_INTERVAL', 'LOCALTIME', 'LOCALTIMESTAMP',
    'MAKE_DATE', 'MAKE_INTERVAL', 'MAKE_TIME', 'MAKE_TIMESTAMP',
    'STATEMENT_TIMESTAMP', 'TIMEOFDAY', 'TRANSACTION_TIMESTAMP',
    // JSON / 数组 / 聚合
    'ARRAY_AGG', 'ARRAY_APPEND', 'ARRAY_CAT', 'ARRAY_LENGTH', 'ARRAY_PREPEND',
    'ARRAY_TO_STRING', 'JSONB_BUILD_OBJECT', 'JSONB_AGG', 'JSON_AGG',
    'JSON_BUILD_OBJECT', 'JSON_EACH', 'JSON_OBJECT_KEYS', 'JSON_AGG',
    'ROW_TO_JSON', 'TO_JSON', 'TO_JSONB', 'XMLAGG',
    // 其他
    'CURRENT_CATALOG', 'CURRENT_SCHEMA', 'CURRENT_USER', 'GEN_RANDOM_UUID',
    'INET_CLIENT_ADDR', 'PG_BACKEND_PID', 'PG_ENCODING_TO_CHAR', 'SESSION_USER',
    'TO_CHAR', 'TO_DATE', 'TO_NUMBER', 'TO_TIMESTAMP', 'USER', 'VERSION',
    'GREATEST', 'LEAST'
  ],
  sqlite: [
    // SQLite 文档全量（内置函数本来就少）
    'NOW', 'ABS', 'CHANGES', 'CHAR', 'COALESCE', 'FORMAT', 'GLOB', 'HEX', 'IFNULL',
    'IIF', 'INSTR', 'JSON', 'JSON_ARRAY', 'JSON_ARRAY_LENGTH', 'JSON_EXTRACT',
    'JSON_INSERT', 'JSON_OBJECT', 'JSON_PATCH', 'JSON_QUOTE', 'JSON_REMOVE',
    'JSON_REPLACE', 'JSON_SET', 'JSON_TYPE', 'JSON_VALID', 'LAST_INSERT_ROWID',
    'LENGTH', 'LIKE', 'LOWER', 'LPAD', 'LTRIM', 'MAX', 'MIN', 'NULLIF',
    'PRINTF', 'QUOTE', 'RANDOM', 'REPLACE', 'RPAD', 'RTRIM', 'SIGN',
    'SOUNDEX', 'SQLITE_SOURCE_ID', 'SQLITE_VERSION', 'SUBSTR', 'SUBSTRING',
    'TOTAL', 'TRIM', 'TYPEOF', 'UNHEX', 'UNICODE', 'UPPER',
    'DATE', 'TIME', 'DATETIME', 'JULIANDAY', 'STRFTIME', 'UNIXEPOCH',
    'ICONV', 'GROUP_CONCAT'
  ],
  sqlserver: [
    // 字符串
    'ASCII', 'CHAR', 'CHARINDEX', 'CONCAT', 'CONCAT_WS', 'DATALENGTH',
    'DIFFERENCE', 'FORMAT', 'LEFT', 'LEN', 'LOWER', 'LTRIM', 'NCHAR',
    'PATINDEX', 'QUOTENAME', 'REPLACE', 'REPLICATE', 'REVERSE', 'RIGHT',
    'RTRIM', 'SOUNDEX', 'SPACE', 'STR', 'STRING_AGG', 'STRING_ESCAPE',
    'STRING_SPLIT', 'STUFF', 'SUBSTRING', 'TRANSLATE', 'TRIM', 'UNICODE',
    'UPPER',
    // 数学
    'ACOS', 'ASIN', 'ATAN', 'ATN2', 'CEILING', 'COS', 'COT', 'DEGREES',
    'EXP', 'FLOOR', 'LOG', 'LOG10', 'PI', 'POWER', 'RADIANS', 'RAND',
    'ROUND', 'SIGN', 'SIN', 'SQRT', 'SQUARE', 'TAN',
    // 日期时间
    'CURRENT_TIMESTAMP', 'DATEADD', 'DATEDIFF', 'DATEDIFF_BIG',
    'DATEFROMPARTS', 'DATENAME', 'DATEPART', 'DATETIME2FROMPARTS',
    'DATETIMEFROMPARTS', 'DAY', 'EOMONTH', 'GETDATE', 'GETUTCDATE',
    'ISDATE', 'MONTH', 'SMALLDATETIMEFROMPARTS', 'SWITCHOFFSET',
    'SYSDATETIME', 'SYSUTCDATETIME', 'TIMEFROMPARTS', 'TODATETIMEOFFSET',
    'YEAR',
    // 类型 / 逻辑 / 元数据
    'CAST', 'CONVERT', 'IIF', 'CHOOSE', 'TRY_CAST', 'TRY_CONVERT',
    'COALESCE', 'NULLIF', 'ISNULL', 'PARSE',
    'COL_LENGTH', 'COL_NAME', 'DB_ID', 'DB_NAME', 'OBJECT_ID', 'OBJECT_NAME',
    'SCHEMA_ID', 'SCHEMA_NAME', 'SERVERPROPERTY',
    // 聚合 / 排名 / 安全
    'CHECKSUM_AGG', 'GROUPING', 'GROUPING_ID', 'STDEV', 'STDEVP', 'VAR', 'VARP',
    'CUME_DIST', 'NTILE', 'PERCENT_RANK', 'PERCENTILE_CONT', 'PERCENTILE_DISC',
    'FIRST_VALUE', 'LAST_VALUE',
    'HAS_DBACCESS', 'IS_MEMBER', 'SUSER_SNAME', 'SYSTEM_USER', 'USER_NAME'
  ],
  oracle: [
    // 字符串
    'ASCII', 'CHR', 'COMPOSE', 'CONCAT', 'DECOMPOSE', 'INITCAP', 'INSTR',
    'LENGTH', 'LENGTHB', 'LOWER', 'LPAD', 'LTRIM', 'NLS_LOWER', 'NLS_UPPER',
    'REGEXP_INSTR', 'REGEXP_REPLACE', 'REGEXP_SUBSTR', 'REPLACE', 'RPAD',
    'RTRIM', 'SOUNDEX', 'SUBSTR', 'SUBSTRB', 'TRANSLATE', 'TRIM', 'UPPER',
    // 数学
    'ABS', 'ACOS', 'ASIN', 'ATAN', 'ATAN2', 'BITAND', 'CEIL', 'COS', 'COSH',
    'EXP', 'FLOOR', 'LN', 'LOG', 'MOD', 'POWER', 'REMAINDER', 'ROUND',
    'SIGN', 'SIN', 'SINH', 'SQRT', 'TAN', 'TANH', 'TRUNC',
    // 日期时间
    'ADD_MONTHS', 'CURRENT_DATE', 'CURRENT_TIMESTAMP', 'DBTIMEZONE',
    'EXTRACT', 'FROM_TZ', 'LAST_DAY', 'LOCALTIMESTAMP', 'MONTHS_BETWEEN',
    'NEW_TIME', 'NEXT_DAY', 'NUMTODSINTERVAL', 'NUMTOYMINTERVAL',
    'ORACLE_DATE', 'ROUND', 'SESSIONTIMEZONE', 'SYS_EXTRACT_UTC',
    'SYSDATE', 'SYSTIMESTAMP', 'TO_CHAR', 'TO_DATE', 'TO_DSINTERVAL',
    'TO_TIMESTAMP', 'TO_TIMESTAMP_TZ', 'TO_YMINTERVAL', 'TRUNC', 'TZ_OFFSET',
    // 转换 / 逻辑 / 聚合 / 分析
    'ASCISTR', 'CAST', 'CHARTOROWID', 'CONVERT', 'DECODE', 'DUMP', 'GV',
    'HEXTORAW', 'RAWTOHEX', 'ROWIDTOCHAR', 'TO_BLOB', 'TO_CLOB', 'TO_LOB',
    'TO_MULTI_BYTE', 'TO_NCLOB', 'TO_NUMBER', 'TO_SINGLE_BYTE', 'NVL',
    'NVL2', 'LNNVL', 'COALESCE', 'NULLIF', 'BFILENAME', 'EMPTY_BLOB',
    'EMPTY_CLOB', 'SYS_GUID', 'UID', 'USER', 'USERENV', 'VSIZE',
    'AVG', 'CORR', 'COUNT', 'COVAR_POP', 'LISTAGG', 'MAX', 'MEDIAN', 'MIN',
    'RATIO_TO_REPORT', 'STATS_MODE', 'STDDEV', 'SUM', 'VARIANCE',
    'CUME_DIST', 'DENSE_RANK', 'FIRST_VALUE', 'LAG', 'LAST_VALUE', 'LEAD',
    'NTILE', 'PERCENT_RANK', 'RANK', 'ROW_NUMBER'
  ]
}

// 方言别名归一：kingbase/derby 等落到相近方言（前者 PG 系，后者近似标准 SQL）
const ALIAS = {
  kingbase: 'postgres',
  dm: 'oracle',
  derby: 'sqlserver',
  doris: 'mysql',
  mariadb: 'mysql'
}

const dedupe = (list) => {
  const seen = new Set()
  return list.filter((f) => (seen.has(f) ? false : (seen.add(f), true)))
}

/** 给定格式化方言（或连接类型码），返回去重后的内置函数名数组。
 *  方言未知（还没选连接）时给**全量并集** —— 补全列表本来就会按已输入的字母过滤，
//  退化的空清单只会让用户以为「函数没有提示」 */
export const builtinFunctions = (dialect) => {
  const key = String(dialect || '').toLowerCase()
  if (!key || (!DIALECT_EXTRA[key] && !ALIAS[key])) return allBuiltinFunctions()
  return dedupe([...COMMON, ...DIALECT_EXTRA[key]])
}

/** 全方言并集：SQL 分词器给内置函数独立颜色时用（分词器按语言全局生效） */
export const allBuiltinFunctions = () => dedupe([...COMMON, ...Object.values(DIALECT_EXTRA).flat()])

// 常用函数的**参数占位说明**（补全插入代码片段时用）。没出现在表里的函数，
// 插入 `FN($0)`（光标落在括号内），不硬凑参数。
const FUNCTION_PARAMS = {
  // 聚合
  COUNT: 'expr', SUM: 'num', AVG: 'num', MIN: 'expr', MAX: 'expr',
  'GROUP_CONCAT': 'expr, sep', 'STRING_AGG': 'expr, sep', 'LISTAGG': 'expr, sep',
  VARIANCE: 'num', STDDEV: 'num', MEDIAN: 'num',
  // 数学
  ROUND: 'num, digits', ABS: 'num', CEIL: 'num', CEILING: 'num', FLOOR: 'num',
  MOD: 'a, b', SIGN: 'num', SQRT: 'num', POWER: 'base, exp', POW: 'base, exp',
  EXP: 'num', LN: 'num', LOG: 'num', LOG2: 'num', LOG10: 'num',
  TRUNCATE: 'num, digits', TRUNC: 'num, digits', GREATEST: 'val1, val2, …',
  LEAST: 'val1, val2, …', RAND: '', SIN: 'num', COS: 'num', TAN: 'num',
  // 字符串
  UPPER: 'str', LOWER: 'str', LENGTH: 'str', CHAR_LENGTH: 'str',
  TRIM: 'str', LTRIM: 'str', RTRIM: 'str', REPLACE: 'str, from, to',
  SUBSTRING: 'str, pos, len', SUBSTR: 'str, pos, len', MID: 'str, pos, len',
  CONCAT: 'str1, str2, …', 'CONCAT_WS': 'sep, str1, str2, …',
  INSTR: 'str, sub', LOCATE: 'sub, str', POSITION: 'sub, str',
  LEFT: 'str, n', RIGHT: 'str, n', LPAD: 'str, len, pad', RPAD: 'str, len, pad',
  REPEAT: 'str, n', REVERSE: 'str', SPACE: 'n', ASCII: 'str', SOUNDEX: 'str',
  'SUBSTRING_INDEX': 'str, delim, count', 'SPLIT_PART': 'str, sep, n',
  // 日期时间
  YEAR: 'date', MONTH: 'date', DAY: 'date', HOUR: 'time', MINUTE: 'time',
  SECOND: 'time', EXTRACT: 'part, date', DATEDIFF: 'date1, date2',
  'DATE_ADD': 'date, interval', 'DATE_SUB': 'date, interval', ADDDATE: 'date, interval',
  SUBDATE: 'date, interval', 'DATE_FORMAT': 'date, format',
  'STR_TO_DATE': 'str, format', 'FROM_UNIXTIME': 'unixtime',
  'UNIX_TIMESTAMP': 'date?', 'DATE_TRUNC': 'part, timestamp',
  'ADD_MONTHS': 'date, n', 'MONTHS_BETWEEN': 'date1, date2',
  'LAST_DAY': 'date', MAKEDATE: 'year, day', DATE: 'str', DATETIME: 'str',
  TIME: 'str', JULIANDAY: 'date', STRFTIME: 'format, date',
  'TO_CHAR': 'value, format', 'TO_DATE': 'str, format',
  'TO_TIMESTAMP': 'str, format', 'TO_NUMBER': 'str, format',
  // 控制 / 转换
  COALESCE: 'val1, val2, …', NULLIF: 'a, b', IFNULL: 'val, default',
  ISNULL: 'val', NVL: 'val, default', NVL2: 'val, not_null, null',
  IF: 'cond, then, else', IIF: 'cond, then, else',
  DECODE: 'expr, search, result, …', CAST: 'expr AS type',
  // JSON
  'JSON_EXTRACT': 'json, path', 'JSON_OBJECT': 'key, value, …',
  'JSON_ARRAY': 'value, …'
}

/** 无参函数：插入 `FN()` 且不再放占位符 */
const NO_PARAMS = new Set([
  'NOW', 'CURRENT_DATE', 'CURRENT_TIME', 'CURRENT_TIMESTAMP', 'LOCALTIME',
  'LOCALTIMESTAMP', 'SYSDATE', 'SYSTIMESTAMP', 'GETDATE', 'GETUTCDATE',
  'DATABASE', 'SCHEMA', 'USER', 'VERSION', 'CONNECTION_ID', 'LAST_INSERT_ID',
  'UUID', 'PI', 'ROW_NUMBER', 'RANK', 'DENSE_RANK', 'CURDATE', 'CURTIME'
])

/**
 * 函数补全的插入文本（Monaco 代码片段）。
 *
 * - 有参数说明的函数：`DATE_FORMAT(${1:date}, ${2:format})` —— Tab 逐个跳参数；
 * - 无参函数：`NOW()`；
 * - 其余：`FN($0)` —— 光标落在括号内。
 *
 * `typed` 是用户已敲的词：小写开头就按小写插入（跟随用户的输入习惯），
 * 大写/为空则按标准大写。
 */
export const functionInsertText = (name, typed) => {
  const fn = typed && /^[a-z]/.test(typed) ? name.toLowerCase() : name
  if (NO_PARAMS.has(name)) return fn + '()'
  const params = FUNCTION_PARAMS[name]
  if (params === undefined) return `${fn}($0)`
  if (params === '') return fn + '()'
  // 占位符转 snippet：`num, digits` → `${1:num}, ${2:digits}`
  const parts = params.split(',').map((p, i) => `\${${i + 1}:${p.trim()}}`)
  return `${fn}(${parts.join(', ')})`
}

/**
 * 关键字/函数补全的插入大小写：用户已敲小写就插小写，否则插标准大写。
 * 提示（过滤）本身大小写不敏感 —— 这里只决定**插入**的形态。
 */
export const smartCase = (name, typed) =>
  typed && /^[a-z]/.test(typed) ? name.toLowerCase() : name
