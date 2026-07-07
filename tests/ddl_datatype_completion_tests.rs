use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{ClickHouseDialect, HiveDialect, MysqlDialect, PostgresDialect};
use sql_lsp::schema::{Column, Schema, SchemaId, Table};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position};

fn column(name: &str) -> Column {
    Column {
        name: name.to_string(),
        data_type: "varchar".to_string(),
        nullable: false,
        ..Default::default()
    }
}

fn schema(database: &str) -> Schema {
    Schema {
        id: SchemaId::new(),
        database: database.to_string(),
        tables: vec![Table {
            name: "webhook".to_string(),
            columns: vec![column("owner"), column("name"), column("created_time")],
            ..Default::default()
        }],
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

async fn assert_relational_datatypes(dialect: &dyn Dialect, database: &str, expected_text: &str) {
    let schema = schema(database);

    let create_type = complete(
        dialect,
        &format!("CREATE TABLE {database}.audit_log (owner "),
        &schema,
    )
    .await;
    assert!(has_label(&create_type, "VARCHAR"));
    assert!(has_label(&create_type, expected_text));
    assert!(
        !has_label(&create_type, "webhook"),
        "data type position should not suggest tables: {create_type:?}"
    );
    assert!(
        !has_label(&create_type, "owner"),
        "data type position should not suggest columns: {create_type:?}"
    );
    assert!(
        !has_kind(&create_type, CompletionItemKind::OPERATOR),
        "data type position should not suggest operators: {create_type:?}"
    );

    let create_prefixed = complete(
        dialect,
        &format!("CREATE TABLE {database}.audit_log (owner var"),
        &schema,
    )
    .await;
    assert!(has_label(&create_prefixed, "VARCHAR"));
    assert!(!has_label(&create_prefixed, expected_text));

    let alter_type = complete(
        dialect,
        &format!("ALTER TABLE {database}.webhook ADD COLUMN status "),
        &schema,
    )
    .await;
    assert!(has_label(&alter_type, "VARCHAR"));
    assert!(has_label(&alter_type, expected_text));
    assert!(!has_label(&alter_type, "owner"));
    assert!(!has_kind(&alter_type, CompletionItemKind::CLASS));

    let alter_prefixed = complete(
        dialect,
        &format!("ALTER TABLE {database}.webhook ADD COLUMN status var"),
        &schema,
    )
    .await;
    assert!(has_label(&alter_prefixed, "VARCHAR"));
    assert!(!has_label(&alter_prefixed, expected_text));
}

#[tokio::test]
async fn postgres_column_definition_completion_suggests_data_types() {
    assert_relational_datatypes(&PostgresDialect::new(), "public", "TEXT").await;
}

#[tokio::test]
async fn mysql_column_definition_completion_suggests_data_types() {
    assert_relational_datatypes(&MysqlDialect::new(), "shop", "TEXT").await;
}

#[tokio::test]
async fn hive_column_definition_completion_suggests_hive_data_types() {
    let schema = schema("warehouse");
    let items = complete(
        &HiveDialect::new(),
        "CREATE TABLE warehouse.audit_log (payload ",
        &schema,
    )
    .await;

    assert!(has_label(&items, "STRING"));
    assert!(has_label(&items, "ARRAY<>"));
    assert!(!has_label(&items, "webhook"));
    assert!(!has_label(&items, "owner"));
}

#[tokio::test]
async fn clickhouse_column_definition_completion_suggests_clickhouse_data_types() {
    let schema = schema("analytics");
    let items = complete(
        &ClickHouseDialect::new(),
        "ALTER TABLE analytics.webhook ADD COLUMN status ",
        &schema,
    )
    .await;

    assert!(has_label(&items, "String"));
    assert!(has_label(&items, "DateTime"));
    assert!(!has_label(&items, "webhook"));
    assert!(!has_label(&items, "owner"));
}
