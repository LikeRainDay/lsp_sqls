use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{MysqlDialect, PostgresDialect};
use sql_lsp::schema::{Column, Schema, SchemaId, Table};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position};

fn schema(database: &str) -> Schema {
    Schema {
        id: SchemaId::new(),
        catalog: None,
        database: database.to_string(),
        server_version: None,
        tables: vec![Table {
            name: "webhook".to_string(),
            columns: vec![
                Column {
                    name: "owner".to_string(),
                    data_type: "varchar".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                Column {
                    name: "created_time".to_string(),
                    data_type: "timestamp".to_string(),
                    nullable: true,
                    ..Default::default()
                },
            ],
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

#[tokio::test]
async fn mysql_order_by_direction_completion_excludes_fields() {
    let schema = schema("shop");
    let items = complete(
        &MysqlDialect::new(),
        "SELECT * FROM shop.webhook ORDER BY owner ",
        &schema,
    )
    .await;

    assert!(has_label(&items, "ASC"));
    assert!(has_label(&items, "DESC"));
    assert!(!has_label(&items, "owner"));
    assert!(!has_kind(&items, CompletionItemKind::FIELD));

    let after_direction = complete(
        &MysqlDialect::new(),
        "SELECT * FROM shop.webhook ORDER BY owner ASC ",
        &schema,
    )
    .await;
    assert!(has_label(&after_direction, ","));
    assert!(has_label(&after_direction, "LIMIT"));
    assert!(has_label(&after_direction, "OFFSET"));
    assert!(!has_label(&after_direction, "ASC"));
    assert!(!has_label(&after_direction, "DESC"));
    assert!(!has_label(&after_direction, "owner"));
    assert!(!has_kind(&after_direction, CompletionItemKind::FIELD));
}

#[tokio::test]
async fn postgres_order_by_direction_completion_includes_nulls_options() {
    let schema = schema("public");
    let items = complete(
        &PostgresDialect::new(),
        "SELECT * FROM public.webhook ORDER BY created_time ",
        &schema,
    )
    .await;

    assert!(has_label(&items, "ASC"));
    assert!(has_label(&items, "DESC"));
    assert!(has_label(&items, "NULLS FIRST"));
    assert!(has_label(&items, "NULLS LAST"));
    assert!(!has_label(&items, "created_time"));
    assert!(!has_kind(&items, CompletionItemKind::FIELD));

    let prefixed = complete(
        &PostgresDialect::new(),
        "SELECT * FROM public.webhook ORDER BY created_time N",
        &schema,
    )
    .await;
    assert!(has_label(&prefixed, "NULLS FIRST"));
    assert!(has_label(&prefixed, "NULLS LAST"));
    assert!(!has_label(&prefixed, "ASC"));

    let after_direction = complete(
        &PostgresDialect::new(),
        "SELECT * FROM public.webhook ORDER BY created_time DESC ",
        &schema,
    )
    .await;
    assert!(has_label(&after_direction, "NULLS FIRST"));
    assert!(has_label(&after_direction, "NULLS LAST"));
    assert!(has_label(&after_direction, "LIMIT"));
    assert!(!has_label(&after_direction, "ASC"));
    assert!(!has_label(&after_direction, "DESC"));
    assert!(!has_label(&after_direction, "created_time"));
    assert!(!has_kind(&after_direction, CompletionItemKind::FIELD));

    let nulls_position = complete(
        &PostgresDialect::new(),
        "SELECT * FROM public.webhook ORDER BY created_time DESC NULLS ",
        &schema,
    )
    .await;
    assert!(has_label(&nulls_position, "FIRST"));
    assert!(has_label(&nulls_position, "LAST"));
    assert!(!has_label(&nulls_position, "NULLS FIRST"));
    assert!(!has_label(&nulls_position, "ASC"));

    let after_nulls_position = complete(
        &PostgresDialect::new(),
        "SELECT * FROM public.webhook ORDER BY created_time DESC NULLS FIRST ",
        &schema,
    )
    .await;
    assert!(has_label(&after_nulls_position, ","));
    assert!(has_label(&after_nulls_position, "LIMIT"));
    assert!(has_label(&after_nulls_position, "OFFSET"));
    assert!(!has_label(&after_nulls_position, "NULLS FIRST"));
    assert!(!has_label(&after_nulls_position, "ASC"));
    assert!(!has_label(&after_nulls_position, "created_time"));
    assert!(!has_kind(&after_nulls_position, CompletionItemKind::FIELD));
}
