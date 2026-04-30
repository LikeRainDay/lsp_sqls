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
    // sqlformat 返回的是带格式化的多行输出
    assert_eq!(formatted, "SELECT\n  *\nFROM\n  users");
}

#[tokio::test]
async fn test_mysql_diagnostics() {
    let dialect = MysqlDialect::new();
    // 语法错误：SELECT * FROM 后面没有表名
    let diagnostics = dialect.parse("SELECT * FROM", None).await;
    // 应该返回语法错误诊断
    assert!(
        !diagnostics.is_empty(),
        "Should return diagnostics for incomplete SQL"
    );
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
async fn test_bigquery_dialect() {
    let dialect = BigQueryDialect::new();
    assert_eq!(dialect.name(), "bigquery");

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
    assert!(items
        .iter()
        .any(|item| item.label == "FROM" || item.label == "DISTINCT"));

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
    assert!(items_default.iter().any(|item| item.label == "QUALIFY"));
}

#[tokio::test]
async fn test_bigquery_dynamic_completion() {
    let dialect = BigQueryDialect::new();

    let table = Table {
        name: "users".to_string(),
        columns: vec![
            Column {
                name: "id".to_string(),
                data_type: "STRING".to_string(),
                nullable: false,
                comment: None,
                source_location: None,
            },
            Column {
                name: "email".to_string(),
                data_type: "STRING".to_string(),
                nullable: true,
                comment: None,
                source_location: None,
            },
        ],
        comment: None,
        source_location: None,
    };

    dialect.add_to_cache("users".to_string(), table);

    let items = dialect
        .completion(
            "SELECT  FROM users",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 7,
            },
            None,
        )
        .await;

    assert!(!items.is_empty());
    assert!(items.iter().any(|item| item.label == "users.id"));
    assert!(items.iter().any(|item| item.label == "users.email"));
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
                    source_location: None,
                },
                Column {
                    name: "name".to_string(),
                    data_type: "VARCHAR(255)".to_string(),
                    nullable: true,
                    comment: None,
                    source_location: None,
                },
            ],
            comment: Some("Users table".to_string()),
            source_location: None,
        }],
        functions: vec![],
        source_uri: None,
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

    // 应该包含列名（在 SELECT 子句中，应该包含列名）
    // 注意：由于现在使用 AST 上下文分析，在 SELECT 后只返回列名和 SELECT 相关关键字
    // 检查是否有列名补全（单表查询，不带表前缀）
    assert!(items.iter().any(|item| item.label == "id"));
    assert!(items.iter().any(|item| item.label == "name"));
}

/// 辅助函数：测试补全并打印详细日志
/// 用于展示智能推断的输入输出
async fn test_completion_with_log(
    dialect: &dyn Dialect,
    name: &str,
    input_text: &str,
    line: u32,
    character: u32,
    schema: Option<&Schema>,
) -> Vec<tower_lsp::lsp_types::CompletionItem> {
    println!("\n[{}] Testing Completion...", name);
    println!("----------------------------------------");
    println!("Input Text:");
    for (i, l) in input_text.lines().enumerate() {
        println!("{:3} | {}", i, l);
        if i == line as usize {
            // Prefix length matches "{:3} | " (4 chars for number+padding, 3 chars for " | ")
            // Actually {:3} produces 3 chars. " | " is 3 chars. Total 6 chars.
            let prefix_len = 6;
            let indent = " ".repeat(prefix_len + character as usize);
            println!("{}^", indent);
        }
    }

    let position = tower_lsp::lsp_types::Position { line, character };
    let mut items = dialect.completion(input_text, position, schema).await;

    // Sort by sort_text (LSP standard behavior)
    items.sort_by(|a, b| {
        let a_sort = a.sort_text.as_ref().unwrap_or(&a.label);
        let b_sort = b.sort_text.as_ref().unwrap_or(&b.label);
        a_sort.cmp(b_sort)
    });

    println!("----------------------------------------");
    println!("Inference Result ({} items found):", items.len());
    for (i, item) in items.iter().take(10).enumerate() {
        let kind = match item.kind {
            Some(k) => format!("{:?}", k),
            None => "Unknown".to_string(),
        };
        println!(
            "  {}. [{}] {} - {:?}",
            i + 1,
            kind,
            item.label,
            item.detail /* .as_deref().unwrap_or("") */
        );
    }
    if items.len() > 10 {
        println!("  ... and {} more", items.len() - 10);
    }
    println!("----------------------------------------");

    items
}

#[tokio::test]
async fn test_intelligent_completion_logging() {
    let dialect = MysqlDialect::new();

    // Shared Schema for tests
    let schema = Schema {
        id: SchemaId::new(),
        database: "shop".to_string(),
        tables: vec![
            Table {
                name: "users".to_string(),
                columns: vec![
                    Column {
                        name: "id".to_string(),
                        data_type: "INT".to_string(),
                        nullable: false,
                        comment: None,
                        source_location: None,
                    },
                    Column {
                        name: "name".to_string(),
                        data_type: "VARCHAR".to_string(),
                        nullable: false,
                        comment: None,
                        source_location: None,
                    },
                    Column {
                        name: "created_at".to_string(),
                        data_type: "DATETIME".to_string(),
                        nullable: false,
                        comment: None,
                        source_location: None,
                    },
                ],
                comment: Some("Users table".to_string()),
                source_location: None,
            },
            Table {
                name: "orders".to_string(),
                columns: vec![
                    Column {
                        name: "order_id".to_string(),
                        data_type: "INT".to_string(),
                        nullable: false,
                        comment: None,
                        source_location: None,
                    },
                    Column {
                        name: "user_id".to_string(),
                        data_type: "INT".to_string(),
                        nullable: false,
                        comment: None,
                        source_location: None,
                    },
                    Column {
                        name: "total_amount".to_string(),
                        data_type: "DECIMAL".to_string(),
                        nullable: false,
                        comment: None,
                        source_location: None,
                    },
                ],
                comment: Some("Orders table".to_string()),
                source_location: None,
            },
        ],
        functions: vec![],
        source_uri: None,
    };

    // 场景 1: WHERE 子句上下文推断
    let sql1 = "SELECT * FROM users WHERE ";
    let items1 = test_completion_with_log(
        &dialect,
        "MySQL - Where Clause (with Schema)",
        sql1,
        0,
        26, // "SELECT * FROM users WHERE " 的长度
        Some(&schema),
    )
    .await;

    // WHERE clause should suggest keyword operators and columns, NOT symbol operators or general keywords
    assert!(
        items1.iter().any(|item| item.label == "LIKE"),
        "Should suggest operator 'LIKE'"
    );
    // Single-table query: expect simple column names without table prefix
    assert!(
        items1.iter().any(|item| item.label == "id"),
        "Should suggest 'id' column"
    );
    assert!(
        items1.iter().any(|item| item.label == "name"),
        "Should suggest 'name' column"
    );
    // Should NOT suggest general keywords
    assert!(
        !items1
            .iter()
            .any(|item| item.label == "SELECT" || item.label == "INSERT"),
        "Should NOT suggest general SQL keywords in WHERE clause"
    );

    // 场景 2: Column Completion (SELECT Context)
    let sql_cols = "SELECT id, na";
    let items_cols = test_completion_with_log(
        &dialect,
        "MySQL - Column Completion",
        sql_cols,
        0,
        13, // "SELECT id, na"
        Some(&schema),
    )
    .await;

    // After adding prefix filtering: only columns matching 'na' should be suggested
    assert!(
        items_cols.iter().any(|item| item.label == "name"),
        "Should suggest 'name' column (matches prefix 'na')"
    );
    // created_at should NOT be suggested since it doesn't match prefix 'na'
    assert_eq!(
        items_cols.len(),
        1,
        "Should only suggest 1 column matching prefix 'na'"
    );

    // 场景 3: Schema 感知补全 (Alias)
    let sql2 = "SELECT o. FROM orders o";
    // 模拟在 "o." 后面输入
    let items2 = test_completion_with_log(
        &dialect,
        "MySQL - Schema Aware & Alias",
        sql2,
        0,
        9,
        Some(&schema),
    )
    .await;

    // 验证是否包含 schema 中的列名
    assert!(items2.len() > 0);
    // Should suggest columns from orders table (e.g., order_id)
    assert!(
        items2
            .iter()
            .any(|item| item.label == "order_id" || item.label == "orders.order_id"),
        "Should suggest 'order_id' column for alias 'o'"
    );
    // Should NOT suggest keywords like "JOIN"
    assert!(
        !items2.iter().any(|item| item.label == "JOIN"),
        "Should NOT suggest keywords like 'JOIN' in TableColumn context"
    );

    // 场景 4: Join 子句
    let sql3 = "SELECT * FROM orders JOIN ";
    let items3 =
        test_completion_with_log(&dialect, "MySQL - JOIN Clause", sql3, 0, 26, Some(&schema)).await;

    // JOIN clause should suggest ONLY table names
    assert!(
        items3.iter().any(|item| item.label == "users"),
        "Should suggest table 'users' for JOIN"
    );
    assert!(
        items3.iter().any(|item| item.label == "orders"),
        "Should suggest table 'orders' for JOIN"
    );
    // Should NOT suggest keywords
    assert!(
        !items3
            .iter()
            .any(|item| item.label == "SELECT" || item.label == "INSERT"),
        "Should NOT suggest general SQL keywords in JOIN clause"
    );
}
