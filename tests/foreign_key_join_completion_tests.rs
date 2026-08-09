use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{
    clickhouse::ClickHouseDialect, hive::HiveDialect, mysql::MysqlDialect,
    postgres::PostgresDialect,
};
use sql_lsp::schema::{Column, Constraint, Schema, SchemaId, Table};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position};

fn schema(database: &str) -> Schema {
    Schema {
        id: SchemaId::new(),
        catalog: None,
        database: database.to_string(),
        server_version: None,
        tables: vec![
            Table {
                name: "users".to_string(),
                columns: vec![Column {
                    name: "id".to_string(),
                    data_type: "BIGINT".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Table {
                name: "orders".to_string(),
                columns: vec![
                    Column {
                        name: "id".to_string(),
                        data_type: "BIGINT".to_string(),
                        ..Default::default()
                    },
                    Column {
                        name: "user_id".to_string(),
                        data_type: "BIGINT".to_string(),
                        ..Default::default()
                    },
                ],
                constraints: vec![Constraint {
                    name: "orders_user_fk".to_string(),
                    constraint_type: "FOREIGN KEY".to_string(),
                    columns: vec!["user_id".to_string()],
                    referenced_schema: Some(database.to_string()),
                    referenced_table: Some("users".to_string()),
                    referenced_columns: vec!["id".to_string()],
                    definition: Some("FOREIGN KEY (user_id) REFERENCES users(id)".to_string()),
                }],
                ..Default::default()
            },
        ],
        functions: vec![],
        source_uri: None,
    }
}

async fn complete(dialect: &dyn Dialect, database: &str) -> Vec<CompletionItem> {
    let sql = "SELECT * FROM users u JOIN ";
    dialect
        .completion(
            sql,
            Position {
                line: 0,
                character: sql.len() as u32,
            },
            Some(&schema(database)),
        )
        .await
}

fn assert_fk_join(items: &[CompletionItem], expected: &str) {
    let item = items
        .iter()
        .find(|item| item.insert_text.as_deref() == Some(expected))
        .unwrap_or_else(|| panic!("missing FK JOIN completion {expected}: {items:#?}"));
    assert_eq!(item.kind, Some(CompletionItemKind::SNIPPET));
    assert_eq!(
        item.sort_text.as_deref().map(|value| &value[..1]),
        Some("0")
    );
    assert!(item
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("orders_user_fk")));
}

#[tokio::test]
async fn postgres_generates_qualified_fk_join_clause() {
    let items = complete(&PostgresDialect::new(), "public").await;
    assert_fk_join(&items, "public.orders o ON o.user_id = u.id");
}

#[tokio::test]
async fn mysql_generates_unqualified_fk_join_clause() {
    let items = complete(&MysqlDialect::new(), "shop").await;
    assert_fk_join(&items, "orders o ON o.user_id = u.id");
}

#[tokio::test]
async fn hive_generates_fk_join_clause() {
    let items = complete(&HiveDialect::new(), "warehouse").await;
    assert_fk_join(&items, "orders o ON o.user_id = u.id");
}

#[tokio::test]
async fn clickhouse_generates_fk_join_clause() {
    let items = complete(&ClickHouseDialect::new(), "analytics").await;
    assert_fk_join(&items, "orders o ON o.user_id = u.id");
}
