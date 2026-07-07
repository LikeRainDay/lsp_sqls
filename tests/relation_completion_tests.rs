use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{MysqlDialect, PostgresDialect};
use sql_lsp::schema::{Column, Schema, SchemaId, Table};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position};

fn schema(database: &str) -> Schema {
    Schema {
        id: SchemaId::new(),
        database: database.to_string(),
        tables: vec![
            Table {
                name: "webhook".to_string(),
                columns: vec![Column {
                    name: "owner".to_string(),
                    data_type: "varchar".to_string(),
                    nullable: false,
                    ..Default::default()
                }],
                ..Default::default()
            },
            Table {
                name: "form".to_string(),
                columns: vec![Column {
                    name: "form_css".to_string(),
                    data_type: "text".to_string(),
                    nullable: true,
                    ..Default::default()
                }],
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

#[tokio::test]
async fn mysql_from_continuation_excludes_relation_targets() {
    let schema = schema("shop");
    let items = complete(&MysqlDialect::new(), "SELECT * FROM shop.webhook ", &schema).await;

    assert!(has_label(&items, "JOIN"));
    assert!(has_label(&items, "WHERE"));
    assert!(has_label(&items, "GROUP BY"));
    assert!(has_label(&items, "ORDER BY"));
    assert!(has_label(&items, "LIMIT"));
    assert!(!has_label(&items, "webhook"));
    assert!(!has_label(&items, "form"));
    assert!(!has_label(&items, "owner"));
    assert!(!has_kind(&items, CompletionItemKind::CLASS));
    assert!(!has_kind(&items, CompletionItemKind::FIELD));

    let after_comma = complete(
        &MysqlDialect::new(),
        "SELECT * FROM shop.webhook, ",
        &schema,
    )
    .await;
    assert!(has_label(&after_comma, "webhook"));
    assert!(has_label(&after_comma, "form"));
    assert!(has_kind(&after_comma, CompletionItemKind::CLASS));
}

#[tokio::test]
async fn postgres_join_condition_completion_excludes_relation_targets() {
    let schema = schema("public");
    let items = complete(
        &PostgresDialect::new(),
        "SELECT * FROM public.webhook JOIN public.form ",
        &schema,
    )
    .await;

    assert!(has_label(&items, "ON"));
    assert!(has_label(&items, "USING"));
    assert!(has_label(&items, "AS"));
    assert!(!has_label(&items, "public.webhook"));
    assert!(!has_label(&items, "public.form"));
    assert!(!has_label(&items, "owner"));
    assert!(!has_kind(&items, CompletionItemKind::CLASS));
    assert!(!has_kind(&items, CompletionItemKind::FIELD));

    let prefixed = complete(
        &PostgresDialect::new(),
        "SELECT * FROM public.webhook JOIN public.form O",
        &schema,
    )
    .await;
    assert!(has_label(&prefixed, "ON"));
    assert!(!has_label(&prefixed, "USING"));
    assert!(!has_label(&prefixed, "public.form"));
}
