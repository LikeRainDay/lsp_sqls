use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::*;
use sql_lsp::schema::{Column, Function, FunctionParameter, Schema, SchemaId, Table};

fn utf16_position_inside(source: &str, needle: &str) -> u32 {
    let byte_index = source.find(needle).expect("needle should exist") + 1;
    source[..byte_index].encode_utf16().count() as u32
}

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
    let interactive_sql_samples = [
        "SELECT",
        "SELECT * FROM",
        "SELECT * FROM users WHERE",
        "SELECT * FROM users WHERE id =",
    ];

    for sql in interactive_sql_samples {
        let diagnostics = dialect.parse(sql, None).await;
        assert!(
            diagnostics.is_empty(),
            "interactive incomplete SQL should not publish diagnostics for {sql:?}: {diagnostics:?}"
        );
    }

    let diagnostics = dialect
        .parse("SELECT * FROM users WHERE id = )", None)
        .await;
    assert!(
        !diagnostics.is_empty(),
        "closed invalid SQL should still return diagnostics"
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
async fn test_postgres_schema_aware_completion_filters_referenced_tables() {
    let dialect = PostgresDialect::new();
    let schema = Schema {
        id: SchemaId::new(),
        database: "public".to_string(),
        tables: vec![
            Table {
                name: "users".to_string(),
                columns: vec![
                    Column {
                        name: "id".to_string(),
                        data_type: "integer".to_string(),
                        nullable: false,
                        source_location: None,
                        ..Default::default()
                    },
                    Column {
                        name: "name".to_string(),
                        data_type: "text".to_string(),
                        nullable: false,
                        source_location: None,
                        ..Default::default()
                    },
                ],
                source_location: None,
                ..Default::default()
            },
            Table {
                name: "orders".to_string(),
                columns: vec![
                    Column {
                        name: "order_id".to_string(),
                        data_type: "integer".to_string(),
                        nullable: false,
                        source_location: None,
                        ..Default::default()
                    },
                    Column {
                        name: "user_id".to_string(),
                        data_type: "integer".to_string(),
                        nullable: false,
                        source_location: None,
                        ..Default::default()
                    },
                ],
                source_location: None,
                ..Default::default()
            },
        ],
        functions: vec![],
        source_uri: None,
    };

    let single_table_sql = "SELECT * FROM users WHERE ";
    let single_table_items = dialect
        .completion(
            single_table_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: single_table_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;

    assert!(single_table_items.iter().any(|item| item.label == "id"));
    assert!(single_table_items.iter().any(|item| item.label == "name"));
    assert!(
        !single_table_items
            .iter()
            .any(|item| item.label.contains("order_id")),
        "single-table WHERE should not suggest columns from unrelated tables"
    );

    let join_sql = "SELECT * FROM users u JOIN orders o ON u.id = o.user_id WHERE ";
    let join_items = dialect
        .completion(
            join_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: join_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;

    assert!(
        join_items
            .iter()
            .any(|item| item.label == "users.id" || item.label == "orders.order_id"),
        "multi-table WHERE should qualify column labels"
    );

    let alias_sql = "SELECT u. FROM users u";
    let alias_items = dialect
        .completion(
            alias_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 9,
            },
            Some(&schema),
        )
        .await;

    assert!(alias_items.iter().any(|item| item.label == "id"));
    assert!(alias_items.iter().any(|item| item.label == "name"));
    assert!(
        !alias_items.iter().any(|item| item.label == "order_id"),
        "alias column completion should stay scoped to the aliased table"
    );

    let where_alias_sql = "SELECT * FROM users u JOIN orders o ON u.id = o.user_id WHERE o.";
    let where_alias_items = dialect
        .completion(
            where_alias_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: where_alias_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;

    assert!(where_alias_items
        .iter()
        .any(|item| item.label == "order_id"));
    assert!(where_alias_items.iter().any(|item| item.label == "user_id"));
    assert!(
        !where_alias_items.iter().any(|item| item.label == "name"),
        "WHERE alias member completion should stay scoped to the aliased table"
    );

    let outer_alias_after_subquery_sql =
        "SELECT * FROM users u WHERE EXISTS (SELECT 1 FROM orders u WHERE u.user_id = u.id) AND u.";
    let outer_alias_after_subquery_items = dialect
        .completion(
            outer_alias_after_subquery_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: outer_alias_after_subquery_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;

    assert!(
        outer_alias_after_subquery_items
            .iter()
            .any(|item| item.label == "name"),
        "outer alias member completion should use the outer alias table"
    );
    assert!(
        !outer_alias_after_subquery_items
            .iter()
            .any(|item| item.label == "order_id"),
        "outer alias member completion should not use same-named subquery aliases"
    );

    let subquery_sql = "SELECT * FROM users WHERE id IN (SELECT user_id FROM orders WHERE ";
    let subquery_items = dialect
        .completion(
            subquery_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: subquery_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;

    assert!(
        subquery_items
            .iter()
            .any(|item| item.label == "order_id" || item.label == "orders.order_id"),
        "subquery WHERE should suggest columns from the subquery table"
    );
    assert!(
        !subquery_items
            .iter()
            .any(|item| item.label == "users.name" || item.label == "name"),
        "subquery WHERE should not suggest outer query columns"
    );

    let cte_sql = "WITH recent_orders AS (SELECT * FROM orders) SELECT * FROM users WHERE ";
    let cte_items = dialect
        .completion(
            cte_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: cte_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;

    assert!(
        cte_items.iter().any(|item| item.label == "id"),
        "CTE main query should suggest users columns"
    );
    assert!(
        !cte_items
            .iter()
            .any(|item| item.label == "orders.order_id" || item.label == "order_id"),
        "CTE main query should not suggest columns from CTE body tables"
    );
}

#[tokio::test]
async fn test_postgres_completion_keeps_relation_targets_separate_from_column_targets() {
    let dialect = PostgresDialect::new();
    let schema = Schema {
        id: SchemaId::new(),
        database: "public".to_string(),
        tables: vec![
            Table {
                name: "form".to_string(),
                columns: vec![
                    Column {
                        name: "form_background_url".to_string(),
                        data_type: "text".to_string(),
                        nullable: true,
                        source_location: None,
                        ..Default::default()
                    },
                    Column {
                        name: "form_css".to_string(),
                        data_type: "text".to_string(),
                        nullable: true,
                        source_location: None,
                        ..Default::default()
                    },
                ],
                source_location: None,
                ..Default::default()
            },
            Table {
                name: "casbin_api_rule".to_string(),
                columns: vec![
                    Column {
                        name: "id".to_string(),
                        data_type: "integer".to_string(),
                        nullable: false,
                        source_location: None,
                        ..Default::default()
                    },
                    Column {
                        name: "owner".to_string(),
                        data_type: "varchar".to_string(),
                        nullable: true,
                        source_location: None,
                        ..Default::default()
                    },
                ],
                source_location: None,
                ..Default::default()
            },
        ],
        functions: vec![],
        source_uri: None,
    };

    let from_sql = "SELECT * from";
    let from_items = dialect
        .completion(
            from_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: from_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;

    assert!(
        from_items.iter().any(|item| item.label == "public.form"),
        "FROM completion should include relation targets: {from_items:?}"
    );
    assert!(
        !from_items
            .iter()
            .any(|item| item.label == "form_background_url" || item.label == "form_css"),
        "FROM completion must not leak similarly-prefixed column names: {from_items:?}"
    );

    let where_sql = "SELECT * FROM public.casbin_api_rule where";
    let where_items = dialect
        .completion(
            where_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: where_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;

    let id_column = where_items
        .iter()
        .find(|item| item.label == "id")
        .expect("WHERE completion should include columns from the selected table");
    assert_eq!(id_column.sort_text.as_deref(), Some("0:id"));
    let operator = where_items
        .iter()
        .find(|item| item.label == "!=")
        .expect("WHERE completion should still include operators");
    assert_eq!(operator.sort_text.as_deref(), Some("1:!="));
    assert!(
        !where_items
            .iter()
            .any(|item| item.label == "form_background_url"),
        "WHERE completion should stay scoped to public.casbin_api_rule: {where_items:?}"
    );
}

#[tokio::test]
async fn test_postgres_completion_uses_utf16_lsp_positions() {
    let dialect = PostgresDialect::new();
    let schema = Schema {
        id: SchemaId::new(),
        database: "public".to_string(),
        tables: vec![Table {
            name: "users".to_string(),
            columns: vec![
                Column {
                    name: "id".to_string(),
                    data_type: "integer".to_string(),
                    nullable: false,
                    source_location: None,
                    ..Default::default()
                },
                Column {
                    name: "name".to_string(),
                    data_type: "text".to_string(),
                    nullable: false,
                    source_location: None,
                    ..Default::default()
                },
            ],
            source_location: None,
            ..Default::default()
        }],
        functions: vec![],
        source_uri: None,
    };

    let sql = "SELECT '😀', u. FROM users u";
    let before_cursor = "SELECT '😀', u.";
    let items = dialect
        .completion(
            sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: before_cursor.encode_utf16().count() as u32,
            },
            Some(&schema),
        )
        .await;

    assert!(items.iter().any(|item| item.label == "id"));
    assert!(items.iter().any(|item| item.label == "name"));
    assert!(
        !items.iter().any(|item| item.label == "SELECT"),
        "table-column completion should not fall back to default keywords after non-ASCII text"
    );
}

#[tokio::test]
async fn test_mysql_schema_aware_select_completion_filters_referenced_tables() {
    let dialect = MysqlDialect::new();
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
                        source_location: None,
                        ..Default::default()
                    },
                    Column {
                        name: "name".to_string(),
                        data_type: "VARCHAR".to_string(),
                        nullable: false,
                        source_location: None,
                        ..Default::default()
                    },
                ],
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
                        source_location: None,
                        ..Default::default()
                    },
                    Column {
                        name: "user_id".to_string(),
                        data_type: "INT".to_string(),
                        nullable: false,
                        source_location: None,
                        ..Default::default()
                    },
                ],
                source_location: None,
                ..Default::default()
            },
        ],
        functions: vec![],
        source_uri: None,
    };

    let single_table_sql = "SELECT ord FROM users";
    let single_table_items = dialect
        .completion(
            single_table_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 10,
            },
            Some(&schema),
        )
        .await;

    assert!(
        !single_table_items
            .iter()
            .any(|item| item.label.contains("order_id")),
        "single-table SELECT should not suggest columns from unrelated tables"
    );

    let join_sql = "SELECT user FROM users u JOIN orders o ON u.id = o.user_id";
    let join_items = dialect
        .completion(
            join_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 11,
            },
            Some(&schema),
        )
        .await;

    assert!(
        join_items.iter().any(|item| item.label == "orders.user_id"),
        "multi-table SELECT should qualify and include referenced table columns"
    );

    let where_prefix_sql = "SELECT * FROM orders WHERE us";
    let where_prefix_items = dialect
        .completion(
            where_prefix_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: where_prefix_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;
    assert!(
        where_prefix_items
            .iter()
            .any(|item| item.label == "user_id"),
        "WHERE prefix should include matching MySQL columns: {where_prefix_items:?}"
    );
    assert!(
        !where_prefix_items
            .iter()
            .any(|item| item.label == "order_id"),
        "WHERE prefix should filter unrelated MySQL columns: {where_prefix_items:?}"
    );
    assert!(
        !where_prefix_items.iter().any(|item| item.label == "LIKE"),
        "WHERE prefix should filter unrelated MySQL operators: {where_prefix_items:?}"
    );

    let unqualified_table_items = dialect
        .completion(
            "SELECT * FROM us",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 16,
            },
            Some(&schema),
        )
        .await;
    let unqualified_table_item = unqualified_table_items
        .iter()
        .find(|item| item.label == "users")
        .expect("MySQL should suggest unqualified table names by default");
    assert_eq!(unqualified_table_item.insert_text.as_deref(), Some("users"));

    let qualified_table_items = dialect
        .completion(
            "SELECT * FROM shop.us",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 21,
            },
            Some(&schema),
        )
        .await;
    let qualified_table_item = qualified_table_items
        .iter()
        .find(|item| item.label == "shop.users")
        .expect("MySQL should preserve database-qualified table completion");
    assert_eq!(
        qualified_table_item.insert_text.as_deref(),
        Some("shop.users")
    );

    let outer_alias_after_subquery_sql =
        "SELECT * FROM users u WHERE EXISTS (SELECT 1 FROM orders u WHERE u.user_id = u.id) AND u.";
    let outer_alias_after_subquery_items = dialect
        .completion(
            outer_alias_after_subquery_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: outer_alias_after_subquery_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;
    assert!(
        outer_alias_after_subquery_items
            .iter()
            .any(|item| item.label == "name"),
        "outer MySQL alias should resolve to users after a same-named subquery alias"
    );
    assert!(
        !outer_alias_after_subquery_items
            .iter()
            .any(|item| item.label == "order_id"),
        "outer MySQL alias should not resolve to same-named subquery alias"
    );

    let on_alias_sql = "SELECT * FROM users u JOIN orders o ON u.";
    let on_alias_items = dialect
        .completion(
            on_alias_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: on_alias_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;

    assert!(on_alias_items.iter().any(|item| item.label == "id"));
    assert!(on_alias_items.iter().any(|item| item.label == "name"));
    assert!(
        !on_alias_items.iter().any(|item| item.label == "order_id"),
        "JOIN ON alias member completion should stay scoped to the aliased table"
    );
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
async fn test_elasticsearch_dsl_schema_aware_fields() {
    let dialect = ElasticsearchDslDialect::new();
    let schema = Schema {
        id: SchemaId::new(),
        database: "default".to_string(),
        tables: vec![Table {
            name: "users".to_string(),
            columns: vec![
                Column {
                    name: "email".to_string(),
                    data_type: "keyword".to_string(),
                    nullable: true,
                    source_location: None,
                    ..Default::default()
                },
                Column {
                    name: "profile.age".to_string(),
                    data_type: "integer".to_string(),
                    nullable: true,
                    source_location: None,
                    ..Default::default()
                },
            ],
            comment: Some("User search index".to_string()),
            source_location: None,
            ..Default::default()
        }],
        functions: vec![],
        source_uri: None,
    };

    let dsl = r#"{"query":{"term":{"#;
    let items = dialect
        .completion(
            dsl,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: dsl.len() as u32,
            },
            Some(&schema),
        )
        .await;
    assert!(items.iter().any(|item| item.label == "users"));
    assert!(items.iter().any(|item| item.label == "email"));
    assert!(items.iter().any(|item| item.label == "profile.age"));

    let field_prefix_dsl = r#"{"query":{"term":{"em"#;
    let field_prefix_items = dialect
        .completion(
            field_prefix_dsl,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: field_prefix_dsl.len() as u32,
            },
            Some(&schema),
        )
        .await;
    assert!(field_prefix_items.iter().any(|item| item.label == "email"));
    assert!(
        !field_prefix_items
            .iter()
            .any(|item| item.label == "profile.age"),
        "Elasticsearch field completion should respect the current prefix"
    );
    assert_eq!(
        field_prefix_items
            .iter()
            .find(|item| item.label == "email")
            .and_then(|item| item.filter_text.as_deref()),
        Some("email")
    );

    let index_prefix_dsl = r#"{"index":"us"#;
    let index_prefix_items = dialect
        .completion(
            index_prefix_dsl,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: index_prefix_dsl.len() as u32,
            },
            Some(&schema),
        )
        .await;
    assert!(index_prefix_items.iter().any(|item| item.label == "users"));

    let query = r#"{"index":"users","query":{"term":{"email":"ada@example.com"}}}"#;
    let index_position = query.find("users").unwrap() + 1;
    let field_position = query.find("email").unwrap() + 1;

    assert!(dialect
        .hover(
            query,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: index_position as u32,
            },
            Some(&schema),
        )
        .await
        .is_some());
    assert!(dialect
        .hover(
            query,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: field_position as u32,
            },
            Some(&schema),
        )
        .await
        .is_some());
    assert!(dialect
        .goto_definition(
            query,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: field_position as u32,
            },
            Some(&schema),
        )
        .await
        .is_some());

    let unicode_query =
        r#"{"note":"😀","index":"users","query":{"term":{"email":"ada@example.com"}}}"#;
    assert!(dialect
        .hover(
            unicode_query,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: utf16_position_inside(unicode_query, "users"),
            },
            Some(&schema),
        )
        .await
        .is_some());
    assert!(dialect
        .hover(
            unicode_query,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: utf16_position_inside(unicode_query, "email"),
            },
            Some(&schema),
        )
        .await
        .is_some());
    assert!(dialect
        .goto_definition(
            unicode_query,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: utf16_position_inside(unicode_query, "email"),
            },
            Some(&schema),
        )
        .await
        .is_some());
}

#[tokio::test]
async fn test_elasticsearch_dsl_http_style_requests() {
    let dialect = ElasticsearchDslDialect::new();
    let request = r#"GET /users/_search
{"query":{"match_all":{}}}

DELETE /users"#;

    let diagnostics = dialect.parse(request, None).await;
    assert!(
        diagnostics.iter().all(|diagnostic| diagnostic.severity
            != Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR)),
        "HTTP-style Elasticsearch requests should parse their JSON bodies without request-line errors"
    );

    let incomplete_body = r#"GET /users/_search
{"query":"#;
    let incomplete_body_diagnostics = dialect.parse(incomplete_body, None).await;
    assert!(
        incomplete_body_diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity
                != Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR)),
        "interactive incomplete Elasticsearch JSON should not publish syntax errors"
    );

    let incomplete_dsl = dialect.parse(r#"{"query":{"term":{"#, None).await;
    assert!(
        incomplete_dsl.iter().all(|diagnostic| diagnostic.severity
            != Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR)),
        "interactive incomplete Elasticsearch DSL should not publish syntax errors"
    );

    let invalid_body = r#"GET /users/_search
{"query":}"#;
    let invalid_body_diagnostics = dialect.parse(invalid_body, None).await;
    assert!(invalid_body_diagnostics.iter().any(|diagnostic| {
        diagnostic.range.start.line == 1 && diagnostic.message.contains("JSON")
    }));

    let invalid_path = dialect.parse("GET _search", None).await;
    assert!(invalid_path.iter().any(|diagnostic| {
        diagnostic.severity == Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR)
            && diagnostic.message.contains("must start with '/'")
    }));
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

    let filtered_items = dialect
        .completion(
            "FT.S",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 4,
            },
            None,
        )
        .await;
    assert!(filtered_items.iter().any(|item| item.label == "FT.SEARCH"));
    assert!(
        !filtered_items.iter().any(|item| item.label == "GET"),
        "Redis command completion should respect the current prefix"
    );
}

#[tokio::test]
async fn test_redis_schema_aware_key_completion_and_hover() {
    let dialect = RedisDialect::new();
    let schema = Schema {
        id: SchemaId::new(),
        database: "0".to_string(),
        tables: vec![Table {
            name: "user:1".to_string(),
            object_type: Some("hash".to_string()),
            columns: vec![],
            comment: Some("2 value item(s)".to_string()),
            source_location: None,
            ..Default::default()
        }],
        functions: vec![],
        source_uri: None,
    };

    let items = dialect
        .completion(
            "HGETALL ",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 8,
            },
            Some(&schema),
        )
        .await;
    let key = items
        .iter()
        .find(|item| item.label == "user:1")
        .expect("Redis completion should include schema keys");
    assert_eq!(key.insert_text.as_deref(), Some("user:1"));
    assert_eq!(key.filter_text.as_deref(), Some("user:1"));

    let filtered_items = dialect
        .completion(
            "HGETALL user",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 12,
            },
            Some(&schema),
        )
        .await;
    assert!(filtered_items.iter().any(|item| item.label == "user:1"));
    assert!(
        !filtered_items.iter().any(|item| item.label == "GET"),
        "Redis key completion should filter command noise after a key prefix"
    );

    assert!(dialect
        .hover(
            "HGETALL user:1",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 10,
            },
            Some(&schema),
        )
        .await
        .is_some());
}

#[tokio::test]
async fn test_mongodb_dialect() {
    let dialect = MongoDbDialect::new();
    assert_eq!(dialect.name(), "mongodb");

    let diagnostics = dialect
        .parse(r#"{"collection":"users","find":{},"limit":10}"#, None)
        .await;
    assert!(
        diagnostics.iter().all(|diagnostic| diagnostic.severity
            != Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR)),
        "valid MongoDB JSON should not produce syntax errors"
    );

    let incomplete_samples = [
        r#"{"collection":"users""#,
        r#"{"collection":"#,
        r#"{"find":{"email":"ada"#,
    ];
    for sample in incomplete_samples {
        let diagnostics = dialect.parse(sample, None).await;
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity
                    != Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR)),
            "interactive incomplete MongoDB JSON should not publish syntax errors for {sample:?}: {diagnostics:?}"
        );
    }

    let invalid = dialect.parse(r#"{"collection":"users",}"#, None).await;
    assert!(invalid
        .iter()
        .any(|diagnostic| diagnostic.severity
            == Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR)));

    let schema = Schema {
        id: SchemaId::new(),
        database: "app".to_string(),
        tables: vec![Table {
            name: "users".to_string(),
            columns: vec![Column {
                name: "email".to_string(),
                data_type: "string".to_string(),
                nullable: true,
                source_location: None,
                ..Default::default()
            }],
            comment: Some("Application users".to_string()),
            source_location: None,
            ..Default::default()
        }],
        functions: vec![],
        source_uri: None,
    };

    let items = dialect
        .completion(
            "{",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 1,
            },
            Some(&schema),
        )
        .await;
    assert!(items.iter().any(|item| item.label == "collection"));
    assert!(items.iter().any(|item| item.label == "find"));
    assert!(items.iter().any(|item| item.label == "create"));
    assert!(items.iter().any(|item| item.label == "drop"));
    assert!(items.iter().any(|item| item.label == "dropDatabase"));
    assert!(items.iter().any(|item| item.label == "users"));
    assert!(items.iter().any(|item| item.label == "email"));

    let collection_prefix_json = r#"{"collection":"us"#;
    let collection_prefix_items = dialect
        .completion(
            collection_prefix_json,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: collection_prefix_json.len() as u32,
            },
            Some(&schema),
        )
        .await;
    assert!(collection_prefix_items
        .iter()
        .any(|item| item.label == "users"));
    assert!(
        !collection_prefix_items
            .iter()
            .any(|item| item.label == "email"),
        "MongoDB collection completion should respect the current prefix"
    );

    let field_prefix_json = r#"{"find":{"em"#;
    let field_prefix_items = dialect
        .completion(
            field_prefix_json,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: field_prefix_json.len() as u32,
            },
            Some(&schema),
        )
        .await;
    assert!(field_prefix_items.iter().any(|item| item.label == "email"));
    assert!(
        !field_prefix_items.iter().any(|item| item.label == "users"),
        "MongoDB field completion should filter collection noise after a field prefix"
    );
    assert_eq!(
        field_prefix_items
            .iter()
            .find(|item| item.label == "email")
            .and_then(|item| item.filter_text.as_deref()),
        Some("email")
    );

    let query = r#"{"collection":"users","find":{"email":"ada@example.com"}}"#;
    let collection_position = query.find("users").unwrap() + 1;
    let field_position = query.find("email").unwrap() + 1;
    assert!(dialect
        .hover(
            query,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: collection_position as u32,
            },
            Some(&schema),
        )
        .await
        .is_some());
    assert!(dialect
        .hover(
            query,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: field_position as u32,
            },
            Some(&schema),
        )
        .await
        .is_some());
    assert!(dialect
        .goto_definition(
            query,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: field_position as u32,
            },
            Some(&schema),
        )
        .await
        .is_some());

    let unicode_query = r#"{"note":"😀","collection":"users","find":{"email":"ada@example.com"}}"#;
    assert!(dialect
        .hover(
            unicode_query,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: utf16_position_inside(unicode_query, "users"),
            },
            Some(&schema),
        )
        .await
        .is_some());
    assert!(dialect
        .hover(
            unicode_query,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: utf16_position_inside(unicode_query, "email"),
            },
            Some(&schema),
        )
        .await
        .is_some());
    assert!(dialect
        .goto_definition(
            unicode_query,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: utf16_position_inside(unicode_query, "email"),
            },
            Some(&schema),
        )
        .await
        .is_some());

    let formatted = dialect
        .format(r#"{"collection":"users","find":{"active":true}}"#)
        .await;
    assert!(formatted.contains("\n  \"collection\": \"users\""));
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
                    ..Default::default()
                },
                Column {
                    name: "name".to_string(),
                    data_type: "VARCHAR(255)".to_string(),
                    nullable: true,
                    comment: None,
                    source_location: None,
                    ..Default::default()
                },
            ],
            comment: Some("Users table".to_string()),
            source_location: None,
            ..Default::default()
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

#[tokio::test]
async fn test_postgres_completion_at_clause_keywords_matches_editor_flow() {
    let dialect = PostgresDialect::new();
    let schema = Schema {
        id: SchemaId::new(),
        database: "public".to_string(),
        tables: vec![
            Table {
                name: "casbin_api_rule".to_string(),
                columns: vec![
                    Column {
                        name: "id".to_string(),
                        data_type: "integer".to_string(),
                        nullable: false,
                        source_location: None,
                        ..Default::default()
                    },
                    Column {
                        name: "ptype".to_string(),
                        data_type: "text".to_string(),
                        nullable: true,
                        source_location: None,
                        ..Default::default()
                    },
                ],
                source_location: None,
                ..Default::default()
            },
            Table {
                name: "form".to_string(),
                columns: vec![Column {
                    name: "form_background_url".to_string(),
                    data_type: "text".to_string(),
                    nullable: true,
                    source_location: None,
                    ..Default::default()
                }],
                source_location: None,
                ..Default::default()
            },
        ],
        functions: vec![],
        source_uri: None,
    };

    let select_sql = "SELECT";
    let select_items = dialect
        .completion(
            select_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: select_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;
    assert!(
        select_items.iter().any(|item| item.label == "id"),
        "SELECT completion should move to expression candidates: {select_items:?}"
    );
    assert!(
        !select_items.iter().any(|item| item.label == "SELECT"),
        "SELECT completion should not suggest the completed SELECT keyword again: {select_items:?}"
    );

    let from_sql = "SELECT * from";
    let from_items = dialect
        .completion(
            from_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: from_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;
    assert!(
        from_items
            .iter()
            .any(|item| item.label == "public.casbin_api_rule"),
        "FROM completion should suggest relation names: {from_items:?}"
    );
    assert!(
        !from_items
            .iter()
            .any(|item| item.label == "form_background_url"),
        "FROM completion should not leak column suggestions: {from_items:?}"
    );

    let where_sql = "SELECT * from public.casbin_api_rule where";
    let mut where_items = dialect
        .completion(
            where_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: where_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;
    where_items.sort_by(|a, b| {
        let a_sort = a.sort_text.as_ref().unwrap_or(&a.label);
        let b_sort = b.sort_text.as_ref().unwrap_or(&b.label);
        a_sort.cmp(b_sort)
    });

    assert_eq!(
        where_items.first().map(|item| item.label.as_str()),
        Some("id")
    );
    assert!(
        where_items.iter().any(|item| item.label == "ptype"),
        "WHERE completion should suggest columns from the referenced table"
    );

    let where_space_sql = "SELECT * from public.casbin_api_rule where ";
    let where_space_items = dialect
        .completion(
            where_space_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: where_space_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;
    assert_eq!(
        where_space_items.first().map(|item| item.label.as_str()),
        Some("id"),
        "WHERE completion should return columns before operators without relying on client-side sorting: {where_space_items:?}"
    );

    let where_prefix_sql = "SELECT * from public.casbin_api_rule where pt";
    let where_prefix_items = dialect
        .completion(
            where_prefix_sql,
            tower_lsp::lsp_types::Position {
                line: 0,
                character: where_prefix_sql.len() as u32,
            },
            Some(&schema),
        )
        .await;
    assert!(
        where_prefix_items.iter().any(|item| item.label == "ptype"),
        "WHERE prefix should include matching columns: {where_prefix_items:?}"
    );
    assert!(
        !where_prefix_items.iter().any(|item| item.label == "id"),
        "WHERE prefix should filter unrelated columns: {where_prefix_items:?}"
    );
    assert!(
        !where_prefix_items.iter().any(|item| item.label == "!="),
        "WHERE prefix should filter unrelated operators: {where_prefix_items:?}"
    );
}

#[tokio::test]
async fn test_schema_function_completion() {
    let schema = Schema {
        id: SchemaId::new(),
        database: "test_db".to_string(),
        tables: vec![],
        functions: vec![Function {
            name: "calculate_score".to_string(),
            routine_type: Some("function".to_string()),
            parameters: vec![FunctionParameter {
                name: "user_id".to_string(),
                data_type: "integer".to_string(),
                optional: false,
            }],
            return_type: "integer".to_string(),
            description: Some("Calculate a user score".to_string()),
        }],
        source_uri: None,
    };

    let mysql = MysqlDialect::new();
    let mysql_items = mysql
        .completion(
            "SELECT calc",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 11,
            },
            Some(&schema),
        )
        .await;
    assert!(mysql_items.iter().any(|item| {
        item.label == "calculate_score"
            && item.kind == Some(tower_lsp::lsp_types::CompletionItemKind::FUNCTION)
    }));
    let mysql_qualified_items = mysql
        .completion(
            "SELECT test_db.calc",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 19,
            },
            Some(&schema),
        )
        .await;
    let mysql_qualified_item = mysql_qualified_items
        .iter()
        .find(|item| item.label == "test_db.calculate_score")
        .expect("MySQL should preserve database-qualified function completion");
    assert_eq!(
        mysql_qualified_item.insert_text.as_deref(),
        Some("test_db.calculate_score()")
    );

    let postgres = PostgresDialect::new();
    let postgres_items = postgres
        .completion(
            "SELECT calc",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 11,
            },
            Some(&schema),
        )
        .await;
    assert!(postgres_items.iter().any(|item| {
        item.label == "calculate_score"
            && item.kind == Some(tower_lsp::lsp_types::CompletionItemKind::FUNCTION)
    }));
    let postgres_qualified_items = postgres
        .completion(
            "SELECT test_db.calc",
            tower_lsp::lsp_types::Position {
                line: 0,
                character: 19,
            },
            Some(&schema),
        )
        .await;
    let postgres_qualified_item = postgres_qualified_items
        .iter()
        .find(|item| item.label == "test_db.calculate_score")
        .expect("Postgres should preserve schema-qualified function completion");
    assert_eq!(
        postgres_qualified_item.insert_text.as_deref(),
        Some("test_db.calculate_score()")
    );
}

fn hover_contents_to_string(contents: &tower_lsp::lsp_types::HoverContents) -> String {
    match contents {
        tower_lsp::lsp_types::HoverContents::Scalar(marked) => match marked {
            tower_lsp::lsp_types::MarkedString::String(value) => value.clone(),
            tower_lsp::lsp_types::MarkedString::LanguageString(value) => value.value.clone(),
        },
        tower_lsp::lsp_types::HoverContents::Array(values) => values
            .iter()
            .map(|value| match value {
                tower_lsp::lsp_types::MarkedString::String(text) => text.clone(),
                tower_lsp::lsp_types::MarkedString::LanguageString(text) => text.value.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        tower_lsp::lsp_types::HoverContents::Markup(markup) => markup.value.clone(),
    }
}

#[tokio::test]
async fn test_schema_function_hover() {
    let schema = Schema {
        id: SchemaId::new(),
        database: "test_db".to_string(),
        tables: vec![],
        functions: vec![Function {
            name: "calculate_score".to_string(),
            routine_type: Some("function".to_string()),
            parameters: vec![FunctionParameter {
                name: "user_id".to_string(),
                data_type: "integer".to_string(),
                optional: false,
            }],
            return_type: "integer".to_string(),
            description: Some("Calculate a user score".to_string()),
        }],
        source_uri: None,
    };

    let sql = "SELECT calculate_score(42)";
    let position = tower_lsp::lsp_types::Position {
        line: 0,
        character: 10,
    };

    let mysql = MysqlDialect::new();
    let mysql_hover = mysql
        .hover(sql, position, Some(&schema))
        .await
        .expect("MySQL should show schema function hover");
    let mysql_hover_text = hover_contents_to_string(&mysql_hover.contents);
    assert!(mysql_hover_text.contains("**Function**: `calculate_score(user_id integer)`"));
    assert!(mysql_hover_text.contains("Calculate a user score"));
    assert!(mysql_hover_text.contains("**Returns**: `integer`"));
    assert!(mysql_hover_text.contains("- `user_id`: `integer`"));

    let postgres = PostgresDialect::new();
    let postgres_hover = postgres
        .hover(sql, position, Some(&schema))
        .await
        .expect("Postgres should show schema function hover");
    let postgres_hover_text = hover_contents_to_string(&postgres_hover.contents);
    assert!(postgres_hover_text.contains("**Function**: `calculate_score(user_id integer)`"));
    assert!(postgres_hover_text.contains("Calculate a user score"));
    assert!(postgres_hover_text.contains("**Returns**: `integer`"));
    assert!(postgres_hover_text.contains("- `user_id`: `integer`"));
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
                        name: "created_at".to_string(),
                        data_type: "DATETIME".to_string(),
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
                        name: "total_amount".to_string(),
                        data_type: "DECIMAL".to_string(),
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
