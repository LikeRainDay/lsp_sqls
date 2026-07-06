use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::*;
use sql_lsp::schema::{Column, Schema, SchemaId, Table};
use tower_lsp::lsp_types::CompletionItemKind;

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
        println!("  {}. [{}] {} - {:?}", i + 1, kind, item.label, item.detail);
    }
    if items.len() > 10 {
        println!("  ... and {} more", items.len() - 10);
    }
    println!("----------------------------------------");

    items
}

/// 综合测试：覆盖各种补全场景
/// 包括多表JOIN、ORDER BY、GROUP BY、HAVING等
#[tokio::test]
async fn test_comprehensive_completion_scenarios() {
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
                        ..Default::default()
                    },
                    Column {
                        name: "name".to_string(),
                        data_type: "VARCHAR".to_string(),
                        nullable: false,
                        comment: None,
                        source_location: None,
                        ..Default::default()
                    },
                    Column {
                        name: "email".to_string(),
                        data_type: "VARCHAR".to_string(),
                        nullable: false,
                        comment: None,
                        source_location: None,
                        ..Default::default()
                    },
                ],
                comment: Some("Users table".to_string()),
                source_location: None,
                ..Default::default()
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
                        ..Default::default()
                    },
                    Column {
                        name: "user_id".to_string(),
                        data_type: "INT".to_string(),
                        nullable: false,
                        comment: None,
                        source_location: None,
                        ..Default::default()
                    },
                    Column {
                        name: "total".to_string(),
                        data_type: "DECIMAL".to_string(),
                        nullable: false,
                        comment: None,
                        source_location: None,
                        ..Default::default()
                    },
                    Column {
                        name: "status".to_string(),
                        data_type: "VARCHAR".to_string(),
                        nullable: false,
                        comment: None,
                        source_location: None,
                        ..Default::default()
                    },
                ],
                comment: Some("Orders table".to_string()),
                source_location: None,
                ..Default::default()
            },
        ],
        functions: vec![],
        source_uri: None,
    };

    // ==================== 场景1: 多表 JOIN 的 WHERE 子句 ====================
    println!("\n=== Test 1: Multi-table JOIN - WHERE clause ===");
    let sql1 = "SELECT * FROM users u JOIN orders o ON u.id = o.user_id WHERE ";
    let items1 = test_completion_with_log(
        &dialect,
        "Multi-table JOIN - WHERE",
        sql1,
        0,
        sql1.len() as u32,
        Some(&schema),
    )
    .await;

    // 多表查询时，列名应该带表前缀避免歧义
    assert!(
        items1
            .iter()
            .any(|item| item.label.contains("users.") || item.label.contains("orders.")),
        "Multi-table query WHERE should include table-prefixed columns"
    );
    // 应该有列名（不管是否有前缀）
    assert!(
        items1
            .iter()
            .any(|item| item.label.contains("id") || item.label.contains("name")),
        "Should suggest columns"
    );
    // 列名应该在运算符之前
    let first_column_idx = items1
        .iter()
        .position(|item| item.kind == Some(tower_lsp::lsp_types::CompletionItemKind::FIELD));
    let first_operator_idx = items1
        .iter()
        .position(|item| item.kind == Some(tower_lsp::lsp_types::CompletionItemKind::OPERATOR));
    if let (Some(col_idx), Some(op_idx)) = (first_column_idx, first_operator_idx) {
        assert!(
            col_idx < op_idx,
            "Columns should appear before operators in WHERE clause"
        );
    }

    println!("\n=== Test 1b: WHERE clause without trailing space ===");
    let sql1b = "SELECT * FROM users WHERE";
    let items1b = test_completion_with_log(
        &dialect,
        "Single-table WHERE without trailing space",
        sql1b,
        0,
        sql1b.len() as u32,
        Some(&schema),
    )
    .await;

    assert!(
        items1b.iter().any(|item| item.label == "id"),
        "Should suggest column 'id' immediately after WHERE"
    );
    assert!(
        items1b.iter().any(|item| item.label == "name"),
        "Should suggest column 'name' immediately after WHERE"
    );

    // ==================== 场景2: 单表 ORDER BY 子句 ====================
    println!("\n=== Test 2: Single-table ORDER BY ===");
    let sql2 = "SELECT * FROM users ORDER BY ";
    let items2 = test_completion_with_log(
        &dialect,
        "Single-table ORDER BY",
        sql2,
        0,
        sql2.len() as u32,
        Some(&schema),
    )
    .await;

    // 单表查询，列名不应该有表前缀
    assert!(
        items2
            .iter()
            .any(|item| item.label == "id" || item.label == "name"),
        "Should suggest simple column names in single-table ORDER BY"
    );
    // 验证顺序：列名(Fields)应该在关键字(Keywords)之前
    // 找到第一个Field和第一个Keyword的位置
    let first_field_pos = items2
        .iter()
        .position(|item| item.kind == Some(CompletionItemKind::FIELD));
    let first_keyword_pos = items2
        .iter()
        .position(|item| item.kind == Some(CompletionItemKind::KEYWORD));

    // 如果两者都存在，确保Field在Keyword之前
    if let (Some(field_pos), Some(keyword_pos)) = (first_field_pos, first_keyword_pos) {
        assert!(
            field_pos < keyword_pos,
            "Columns should appear before keywords in ORDER BY clause"
        );
    }

    // ==================== 场景3: 多表 JOIN 的 ORDER BY 子句 ====================
    println!("\n=== Test 3: Multi-table JOIN - ORDER BY ===");
    let sql3 = "SELECT u.name, o.total FROM users u JOIN orders o ON u.id = o.user_id ORDER BY ";
    let items3 = test_completion_with_log(
        &dialect,
        "Multi-table ORDER BY",
        sql3,
        0,
        sql3.len() as u32,
        Some(&schema),
    )
    .await;

    // 多表查询，列名应该带表前缀
    assert!(
        items3
            .iter()
            .any(|item| item.label.contains("users.") || item.label.contains("orders.")),
        "Multi-table ORDER BY should suggest table-prefixed columns"
    );

    // ==================== 场景4: 单表 GROUP BY 子句 ====================
    println!("\n=== Test 4: Single-table GROUP BY ===");
    let sql4 = "SELECT name, COUNT(*) FROM users GROUP BY ";
    let items4 = test_completion_with_log(
        &dialect,
        "Single-table GROUP BY",
        sql4,
        0,
        sql4.len() as u32,
        Some(&schema),
    )
    .await;

    // 单表查询，列名不应该有表前缀
    assert!(
        items4
            .iter()
            .any(|item| item.label == "name" || item.label == "email"),
        "Should suggest simple column names in GROUP BY"
    );

    // ==================== 场景5: HAVING 子句 ====================
    println!("\n=== Test 5: HAVING clause ===");
    let sql5 = "SELECT user_id, COUNT(*) as cnt FROM orders GROUP BY user_id HAVING ";
    let items5 = test_completion_with_log(
        &dialect,
        "HAVING clause",
        sql5,
        0,
        sql5.len() as u32,
        Some(&schema),
    )
    .await;

    // 验证 HAVING 子句的顺序：列名 > 聚合函数 > 关键字
    let first_field_pos = items5
        .iter()
        .position(|item| item.kind == Some(CompletionItemKind::FIELD));
    let first_func_pos = items5
        .iter()
        .position(|item| item.kind == Some(CompletionItemKind::FUNCTION));

    if let (Some(field_pos), Some(func_pos)) = (first_field_pos, first_func_pos) {
        assert!(
            field_pos < func_pos,
            "Columns should appear before aggregate functions in HAVING clause"
        );
    }

    // ==================== 场景6: 表别名后的列补全 ====================
    println!("\n=== Test 6: Column completion after table alias ===");
    let sql6 = "SELECT u. FROM users u";
    let items6 = test_completion_with_log(
        &dialect,
        "Alias column completion",
        sql6,
        0,
        9, // 在 "u." 之后
        Some(&schema),
    )
    .await;

    // 表别名后应该只显示该表的列，不带表前缀
    assert!(
        items6.iter().any(|item| item.label == "id"),
        "Should suggest 'id' for users table"
    );
    assert!(
        items6.iter().any(|item| item.label == "name"),
        "Should suggest 'name' for users table"
    );
    // 不应该包含 orders 表的列
    assert!(
        !items6.iter().any(|item| item.label.contains("order_id")),
        "Should NOT suggest columns from other tables"
    );

    // ==================== 场景7: 子查询场景 ====================
    println!("\n=== Test 7: Subquery ===");
    let sql7 = "SELECT * FROM users WHERE id IN (SELECT user_id FROM orders WHERE ";
    let items7 = test_completion_with_log(
        &dialect,
        "Subquery WHERE clause",
        sql7,
        0,
        sql7.len() as u32,
        Some(&schema),
    )
    .await;

    // 子查询中应该只显示当前子查询表的列名，不应混入外层 users。
    assert!(
        items7
            .iter()
            .any(|item| item.label.contains("order_id") || item.label.contains("status")),
        "Subquery should suggest column names"
    );
    assert!(
        !items7
            .iter()
            .any(|item| item.label.contains("users.") || item.label == "email"),
        "Subquery WHERE should not suggest outer users columns"
    );

    println!("\n=== Test 7b: CTE main query scope ===");
    let sql7b = "WITH recent_orders AS (SELECT * FROM orders WHERE status = 'open') SELECT * FROM users WHERE ";
    let items7b = test_completion_with_log(
        &dialect,
        "CTE main query WHERE clause",
        sql7b,
        0,
        sql7b.len() as u32,
        Some(&schema),
    )
    .await;

    assert!(
        items7b.iter().any(|item| item.label == "id"),
        "CTE main query should suggest users.id"
    );
    assert!(
        !items7b.iter().any(|item| item.label.contains("orders.")),
        "CTE main query should not suggest CTE body table columns"
    );

    // ==================== 场景8: 无 Schema 的补全 ====================
    println!("\n=== Test 8: Completion without schema ===");
    let sql8 = "SELECT * FROM users WHERE ";
    let items8 = test_completion_with_log(
        &dialect,
        "No schema - WHERE clause",
        sql8,
        0,
        sql8.len() as u32,
        None, // 没有 schema
    )
    .await;

    // 没有 schema 且还没有左侧表达式时，不应该返回比较运算符或列名
    assert!(
        !items8
            .iter()
            .any(|item| item.kind == Some(tower_lsp::lsp_types::CompletionItemKind::OPERATOR)),
        "Should not suggest operators before a WHERE left-side expression without schema"
    );
    assert!(
        !items8
            .iter()
            .any(|item| item.kind == Some(tower_lsp::lsp_types::CompletionItemKind::FIELD)),
        "Should NOT suggest columns without schema"
    );

    let sql8_operator = "SELECT * FROM users WHERE id ";
    let items8_operator = test_completion_with_log(
        &dialect,
        "No schema - WHERE operator position",
        sql8_operator,
        0,
        sql8_operator.len() as u32,
        None,
    )
    .await;
    assert!(
        items8_operator.iter().any(|item| item.label == "LIKE"),
        "Should suggest keyword operators after a WHERE left-side expression even without schema"
    );

    // ==================== 场景9: FROM 子句表名补全 ====================
    println!("\n=== Test 9: FROM clause table completion ===");
    let sql9 = "SELECT * FROM ";
    let items9 = test_completion_with_log(
        &dialect,
        "FROM clause",
        sql9,
        0,
        sql9.len() as u32,
        Some(&schema),
    )
    .await;

    // FROM 子句应该只显示表名，不显示列名或关键字
    assert!(
        items9.iter().any(|item| item.label == "users"),
        "Should suggest table 'users'"
    );
    assert!(
        items9.iter().any(|item| item.label == "orders"),
        "Should suggest table 'orders'"
    );
    assert!(
        !items9
            .iter()
            .any(|item| item.label == "SELECT" || item.label == "WHERE"),
        "FROM clause should NOT suggest SQL keywords"
    );
    assert!(
        !items9
            .iter()
            .any(|item| item.kind == Some(tower_lsp::lsp_types::CompletionItemKind::FIELD)),
        "FROM clause should NOT suggest columns"
    );

    println!("\n=== Test 9b: FROM clause table completion without trailing space ===");
    let sql9b = "SELECT * FROM";
    let items9b = test_completion_with_log(
        &dialect,
        "FROM clause without trailing space",
        sql9b,
        0,
        sql9b.len() as u32,
        Some(&schema),
    )
    .await;

    assert!(
        items9b.iter().any(|item| item.label == "users"),
        "Should suggest table 'users' immediately after FROM"
    );
    assert!(
        items9b.iter().any(|item| item.label == "orders"),
        "Should suggest table 'orders' immediately after FROM"
    );
    assert!(
        !items9b
            .iter()
            .any(|item| item.kind == Some(tower_lsp::lsp_types::CompletionItemKind::FIELD)),
        "FROM keyword position should prefer table suggestions, not columns"
    );

    println!("\n=== Test 9c: DML table target completion ===");
    for (name, sql) in [
        ("INSERT INTO table target", "INSERT INTO "),
        ("UPDATE table target", "UPDATE "),
        ("DELETE FROM table target", "DELETE FROM "),
    ] {
        let items =
            test_completion_with_log(&dialect, name, sql, 0, sql.len() as u32, Some(&schema)).await;

        assert!(
            items.iter().any(|item| item.label == "users"),
            "{name} should suggest table 'users'"
        );
        assert!(
            items.iter().any(|item| item.label == "orders"),
            "{name} should suggest table 'orders'"
        );
        assert!(
            !items
                .iter()
                .any(|item| item.kind == Some(tower_lsp::lsp_types::CompletionItemKind::FIELD)),
            "{name} should prefer table suggestions, not columns"
        );
    }

    // ==================== 场景10: 多表 SELECT 子句 ====================
    println!("\n=== Test 10: Multi-table SELECT clause ===");
    let sql10 = "SELECT u.name, o. FROM users u JOIN orders o ON u.id = o.user_id";
    let items10 = test_completion_with_log(
        &dialect,
        "Multi-table SELECT - after o.",
        sql10,
        0,
        18, // 在 "o." 后面
        Some(&schema),
    )
    .await;

    // 应该只显示 orders 表的列，不带表前缀（已经有 o. 了）
    assert!(
        items10.iter().any(|item| item.label == "order_id"),
        "Should suggest 'order_id' from orders table"
    );
    assert!(
        items10.iter().any(|item| item.label == "total"),
        "Should suggest 'total' from orders table"
    );
    // 不应该显示 users 表的列
    assert!(
        !items10
            .iter()
            .any(|item| item.label.contains("name") || item.label.contains("email")),
        "Should NOT suggest columns from users table"
    );

    println!("\n=== All comprehensive tests passed! ===");
}
