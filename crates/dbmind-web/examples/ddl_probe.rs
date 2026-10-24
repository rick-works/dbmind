//! 临时探针：把「跨类型建表」真实生成的 DDL 打出来，交给 `_uitest/ddl-gen-verify.py`
//! 在真库上执行 —— 验证的是**代码产出**，不是手写的等价语句。
use dbmind_core::{ColumnDetail, ConnectionKind};
use dbmind_web::api::dialect::Dialect;

fn col(
    name: &str,
    type_name: &str,
    nullable: bool,
    primary_key: bool,
    auto_increment: bool,
    default_value: Option<&str>,
) -> ColumnDetail {
    ColumnDetail {
        name: name.to_string(),
        type_name: Some(type_name.to_string()),
        nullable,
        primary_key,
        default_value: default_value.map(str::to_string),
        auto_increment,
    }
}

fn main() {
    // 源都是 MySQL：`id bigint auto_increment primary key` + `status tinyint(1) default 0`
    let columns = vec![
        col("id", "bigint", false, true, true, None),
        col("status", "tinyint(1)", false, false, false, Some("0")),
    ];

    for (label, kind) in [
        ("ORACLE", ConnectionKind::Oracle),
        ("DORIS", ConnectionKind::Doris),
        ("SQLSERVER", ConnectionKind::Sqlserver),
    ] {
        let dialect = Dialect::new(kind);
        let ddl = dialect.create_table_from_columns("ddd_probe", &columns, Some(ConnectionKind::Mysql));
        println!("=== {label} ===");
        println!("{ddl}");
    }
}
