use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{ClickHouseDialect, HiveDialect, MysqlDialect, PostgresDialect};
use sql_lsp::schema::{Column, Schema, SchemaId, Table};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position};

fn column(name: &str) -> Column {
    Column {
        name: name.to_string(),
        data_type: "integer".to_string(),
        nullable: false,
        ..Default::default()
    }
}

fn schema(database: &str) -> Schema {
    Schema {
        id: SchemaId::new(),
        catalog: None,
        database: database.to_string(),
        server_version: None,
        tables: vec![
            Table {
                name: "users".to_string(),
                columns: vec![column("id"), column("tenant_id"), column("email")],
                ..Default::default()
            },
            Table {
                name: "orders".to_string(),
                columns: vec![
                    column("id"),
                    column("tenant_id"),
                    column("user_id"),
                    column("order_id"),
                    column("total"),
                ],
                ..Default::default()
            },
            Table {
                name: "payments".to_string(),
                columns: vec![
                    column("payment_id"),
                    column("order_id"),
                    column("tenant_id"),
                ],
                ..Default::default()
            },
        ],
        functions: vec![],
        source_uri: None,
    }
}

async fn complete(dialect: &dyn Dialect, sql: &str, schema: &Schema) -> Vec<CompletionItem> {
    dialect
        .completion(
            sql,
            Position {
                line: 0,
                character: sql.len() as u32,
            },
            Some(schema),
        )
        .await
}

fn has_label(items: &[CompletionItem], label: &str) -> bool {
    items.iter().any(|item| item.label == label)
}

fn has_kind(items: &[CompletionItem], kind: CompletionItemKind) -> bool {
    items.iter().any(|item| item.kind == Some(kind))
}

async fn assert_join_using_completion(dialect: &dyn Dialect, database: &str) {
    let schema = schema(database);

    let using_columns = complete(
        dialect,
        &format!("SELECT * FROM {database}.users u JOIN {database}.orders o USING ("),
        &schema,
    )
    .await;
    assert!(has_label(&using_columns, "id"));
    assert!(has_label(&using_columns, "tenant_id"));
    assert!(!has_label(&using_columns, "email"));
    assert!(!has_label(&using_columns, "total"));
    assert!(
        !has_kind(&using_columns, CompletionItemKind::OPERATOR),
        "JOIN USING should not suggest predicate operators: {using_columns:?}"
    );
    assert!(
        !has_kind(&using_columns, CompletionItemKind::CLASS),
        "JOIN USING should not suggest relation targets: {using_columns:?}"
    );

    let prefixed_columns = complete(
        dialect,
        &format!("SELECT * FROM {database}.users u JOIN {database}.orders o USING (ten"),
        &schema,
    )
    .await;
    assert!(has_label(&prefixed_columns, "tenant_id"));
    assert!(!has_label(&prefixed_columns, "id"));

    let chained_join_columns = complete(
        dialect,
        &format!(
            "SELECT * FROM {database}.users u JOIN {database}.orders o USING (id) JOIN {database}.payments p USING ("
        ),
        &schema,
    )
    .await;
    assert!(has_label(&chained_join_columns, "order_id"));
    assert!(has_label(&chained_join_columns, "tenant_id"));
    assert!(!has_label(&chained_join_columns, "payment_id"));
    assert!(!has_label(&chained_join_columns, "id"));
}

#[tokio::test]
async fn postgres_join_using_completion_suggests_shared_columns() {
    assert_join_using_completion(&PostgresDialect::new(), "public").await;
}

#[tokio::test]
async fn mysql_join_using_completion_suggests_shared_columns() {
    assert_join_using_completion(&MysqlDialect::new(), "shop").await;
}

#[tokio::test]
async fn hive_join_using_completion_suggests_shared_columns() {
    assert_join_using_completion(&HiveDialect::new(), "warehouse").await;
}

#[tokio::test]
async fn clickhouse_join_using_completion_suggests_shared_columns() {
    assert_join_using_completion(&ClickHouseDialect::new(), "analytics").await;
}
