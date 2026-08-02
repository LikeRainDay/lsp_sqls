use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{MysqlDialect, PostgresDialect};
use sql_lsp::schema::{Column, Schema, SchemaId, Table};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position};

fn schema(database: &str) -> Schema {
    Schema {
        id: SchemaId::new(),
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
async fn mysql_group_by_continuation_excludes_fields() {
    let schema = schema("shop");
    let items = complete(
        &MysqlDialect::new(),
        "SELECT owner, count(*) FROM shop.webhook GROUP BY owner ",
        &schema,
    )
    .await;

    assert!(has_label(&items, ","));
    assert!(has_label(&items, "HAVING"));
    assert!(has_label(&items, "ORDER BY"));
    assert!(has_label(&items, "LIMIT"));
    assert!(has_label(&items, "WITH ROLLUP"));
    assert!(!has_label(&items, "owner"));
    assert!(!has_label(&items, "created_time"));
    assert!(!has_label(&items, "ASC"));
    assert!(!has_kind(&items, CompletionItemKind::FIELD));

    let after_comma = complete(
        &MysqlDialect::new(),
        "SELECT owner, count(*) FROM shop.webhook GROUP BY owner, ",
        &schema,
    )
    .await;
    assert!(has_label(&after_comma, "owner"));
    assert!(has_label(&after_comma, "created_time"));
    assert!(has_kind(&after_comma, CompletionItemKind::FIELD));
}

#[tokio::test]
async fn postgres_group_by_continuation_uses_follow_up_clauses() {
    let schema = schema("public");
    let items = complete(
        &PostgresDialect::new(),
        "SELECT owner, count(*) FROM public.webhook GROUP BY owner ",
        &schema,
    )
    .await;

    assert!(has_label(&items, ","));
    assert!(has_label(&items, "HAVING"));
    assert!(has_label(&items, "ORDER BY"));
    assert!(has_label(&items, "LIMIT"));
    assert!(has_label(&items, "OFFSET"));
    assert!(has_label(&items, "FETCH"));
    assert!(!has_label(&items, "owner"));
    assert!(!has_label(&items, "created_time"));
    assert!(!has_label(&items, "ASC"));
    assert!(!has_label(&items, "DESC"));
    assert!(!has_kind(&items, CompletionItemKind::FIELD));

    let prefixed = complete(
        &PostgresDialect::new(),
        "SELECT owner, count(*) FROM public.webhook GROUP BY owner H",
        &schema,
    )
    .await;
    assert!(has_label(&prefixed, "HAVING"));
    assert!(!has_label(&prefixed, "ORDER BY"));
    assert!(!has_label(&prefixed, "owner"));
}
