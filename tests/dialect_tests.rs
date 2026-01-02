use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::*;
use sql_lsp::schema::{Column, Schema, SchemaId, Table};

#[tokio::test]
async fn test_mysql_dialect() {
    let dialect = MysqlDialect::new();
    assert_eq!(dialect.name(), "mysql");

    // 测试解析 - SELECT 没有 FROM（Tree-sitter 可能报告警告或错误）
    let diagnostics = dialect.parse("SELECT *", None).await;
    // Tree-sitter 应该能够处理，可能有诊断信息
    // diagnostics.len() 是 usize，总是 >= 0，所以这个断言总是为真
    let _ = diagnostics.len();

    // SELECT 有 FROM（Tree-sitter 可能仍然报告一些诊断，取决于 tree-sitter-sql 的实现）
    let diagnostics2 = dialect.parse("SELECT * FROM users", None).await;
    // 主要测试不会崩溃，Tree-sitter 可能对某些语法报告诊断
    // diagnostics2.len() 是 usize，总是 >= 0，所以这个断言总是为真
    let _ = diagnostics2.len();

    // 测试补全
    let items = dialect
        .completion(
            "SELECT ",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 7,
            },
            None,
        )
        .await;
    assert!(!items.is_empty());
    assert!(items.iter().any(|item| item.label == "FROM"));

    // 测试格式化
    let formatted = dialect.format("SELECT   *   FROM   users").await;
    assert_eq!(formatted, "SELECT * FROM users");
}

#[tokio::test]
async fn test_postgres_dialect() {
    let dialect = PostgresDialect::new();
    assert_eq!(dialect.name(), "postgres");

    // 在 SELECT 子句中，应该返回 SELECT 相关的关键字
    let items = dialect
        .completion(
            "SELECT ",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 7,
            },
            None,
        )
        .await;
    assert!(!items.is_empty());
    // SELECT 子句中应该包含 SELECT 相关关键字
    assert!(items
        .iter()
        .any(|item| item.label == "FROM" || item.label == "DISTINCT"));

    // 在默认上下文中，应该包含 ILIKE（WHERE 子句相关）
    let items_default = dialect
        .completion(
            "",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 0,
            },
            None,
        )
        .await;
    assert!(items_default.iter().any(|item| item.label == "ILIKE"));
}

#[tokio::test]
async fn test_hive_dialect() {
    let dialect = HiveDialect::new();
    assert_eq!(dialect.name(), "hive");

    // 在 SELECT 子句中，应该返回 SELECT 相关的关键字
    let items = dialect
        .completion(
            "SELECT ",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 7,
            },
            None,
        )
        .await;
    assert!(!items.is_empty());
    // SELECT 子句中应该包含 SELECT 相关关键字
    assert!(items
        .iter()
        .any(|item| item.label == "FROM" || item.label == "DISTINCT"));

    // 在默认上下文中，应该包含 PARTITION（CREATE TABLE 相关）
    let items_default = dialect
        .completion(
            "",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 0,
            },
            None,
        )
        .await;
    assert!(items_default.iter().any(|item| item.label == "PARTITION"));
}

#[tokio::test]
async fn test_elasticsearch_eql_dialect() {
    let dialect = ElasticsearchEqlDialect::new();
    assert_eq!(dialect.name(), "elasticsearch-eql");

    let items = dialect
        .completion(
            "sequence",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 8,
            },
            None,
        )
        .await;
    assert!(!items.is_empty());
    assert!(items.iter().any(|item| item.label == "sequence"));
    assert!(items.iter().any(|item| item.label == "where"));
}

#[tokio::test]
async fn test_elasticsearch_dsl_dialect() {
    let dialect = ElasticsearchDslDialect::new();
    assert_eq!(dialect.name(), "elasticsearch-dsl");

    let items = dialect
        .completion(
            "{",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 1,
            },
            None,
        )
        .await;
    assert!(!items.is_empty());
    assert!(items.iter().any(|item| item.label == "query"));
    assert!(items.iter().any(|item| item.label == "match"));
    assert!(items.iter().any(|item| item.label == "aggs"));
}

#[tokio::test]
async fn test_clickhouse_dialect() {
    let dialect = ClickHouseDialect::new();
    assert_eq!(dialect.name(), "clickhouse");

    // 在 SELECT 子句中，应该返回 SELECT 相关的关键字
    let items = dialect
        .completion(
            "SELECT ",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 7,
            },
            None,
        )
        .await;
    assert!(!items.is_empty());
    // SELECT 子句中应该包含 SELECT 相关关键字
    assert!(items
        .iter()
        .any(|item| item.label == "FROM" || item.label == "DISTINCT"));

    // 在默认上下文中，应该包含 MergeTree（CREATE TABLE 相关）
    let items_default = dialect
        .completion(
            "",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 0,
            },
            None,
        )
        .await;
    assert!(items_default.iter().any(|item| item.label == "MergeTree"));
}

#[tokio::test]
async fn test_redis_dialect() {
    let dialect = RedisDialect::new();
    assert_eq!(dialect.name(), "redis");

    let items = dialect
        .completion(
            "FT.",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 3,
            },
            None,
        )
        .await;
    assert!(!items.is_empty());
    assert!(items.iter().any(|item| item.label == "FT.SEARCH"));
}

#[tokio::test]
async fn test_dialect_with_schema() {
    let dialect = MysqlDialect::new();

    let schema = Schema {
        id: SchemaId::new(),
        database: "test_db".to_string(),
        tables: vec![Table {
            name: "users".to_string(),
            columns: vec![
                Column {
                    name: "id".to_string(),
                    data_type: "INT".to_string(),
                    nullable: false,
                    comment: Some("User ID".to_string()),
                },
                Column {
                    name: "name".to_string(),
                    data_type: "VARCHAR(255)".to_string(),
                    nullable: true,
                    comment: None,
                },
            ],
            comment: Some("Users table".to_string()),
        }],
        functions: vec![],
    };

    let items = dialect
        .completion(
            "SELECT ",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 7,
            },
            Some(&schema),
        )
        .await;

    // 应该包含表名和列名（在 SELECT 子句中，应该包含列名）
    // 注意：由于现在使用 AST 上下文分析，在 SELECT 后可能只返回列名和 SELECT 相关关键字
    // 检查是否有列名补全
    assert!(items
        .iter()
        .any(|item| item.label == "id" || item.label.contains("id")));
    assert!(items
        .iter()
        .any(|item| item.label == "name" || item.label.contains("name")));
    // 检查是否有表名（可能在 Default 上下文中）
    assert!(items
        .iter()
        .any(|item| item.label == "users" || item.label.contains("users")));
}
