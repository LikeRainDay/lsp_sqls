use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{MysqlDialect, PostgresDialect, SqliteDialect};
use sql_lsp::schema::{Column, Schema, SchemaId, Table};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position};

fn schema(database: &str) -> Schema {
    Schema {
        id: SchemaId::new(),
        catalog: None,
        database: database.to_string(),
        server_version: None,
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
async fn relation_target_completion_suggests_tables_not_columns() {
    let mysql_schema = schema("shop");
    let mysql_items = complete(&MysqlDialect::new(), "SELECT * from", &mysql_schema).await;

    assert!(has_label(&mysql_items, "webhook"));
    assert!(has_label(&mysql_items, "form"));
    assert!(has_kind(&mysql_items, CompletionItemKind::CLASS));
    assert!(!has_label(&mysql_items, "owner"));
    assert!(!has_label(&mysql_items, "form_css"));
    assert!(!has_kind(&mysql_items, CompletionItemKind::FIELD));

    let postgres_schema = schema("public");
    let postgres_items = complete(
        &PostgresDialect::new(),
        "SELECT * FROM public.",
        &postgres_schema,
    )
    .await;

    assert!(has_label(&postgres_items, "public.webhook"));
    assert!(has_label(&postgres_items, "public.form"));
    assert!(has_kind(&postgres_items, CompletionItemKind::CLASS));
    assert!(!has_label(&postgres_items, "owner"));
    assert!(!has_kind(&postgres_items, CompletionItemKind::FIELD));
}

#[tokio::test]
async fn sqlite_completion_uses_its_own_dialect_id_with_relational_scope() {
    let schema = schema("main");
    let from_items = complete(&SqliteDialect::new(), "SELECT * FROM ", &schema).await;
    assert!(has_label(&from_items, "webhook"));
    assert!(has_label(&from_items, "form"));
    assert!(has_kind(&from_items, CompletionItemKind::CLASS));

    let where_items = complete(
        &SqliteDialect::new(),
        "SELECT * FROM main.webhook WHERE ",
        &schema,
    )
    .await;
    assert!(has_label(&where_items, "owner"));
    assert!(!has_label(&where_items, "form_css"));
    assert!(has_kind(&where_items, CompletionItemKind::FIELD));
}

#[tokio::test]
async fn where_clause_start_suggests_scoped_columns_before_operators() {
    let schema = schema("public");
    let items = complete(
        &PostgresDialect::new(),
        "SELECT * FROM public.webhook WHERE ",
        &schema,
    )
    .await;

    assert!(has_label(&items, "owner"));
    assert!(!has_label(&items, "form_css"));
    assert!(has_kind(&items, CompletionItemKind::FIELD));
    assert!(!has_kind(&items, CompletionItemKind::CLASS));
    assert!(!has_kind(&items, CompletionItemKind::OPERATOR));

    let owner = items
        .iter()
        .find(|item| item.label == "owner")
        .and_then(|item| item.sort_text.as_deref())
        .unwrap_or_default();
    let truthy = items
        .iter()
        .find(|item| item.label == "TRUE")
        .and_then(|item| item.sort_text.as_deref())
        .unwrap_or_default();
    assert!(
        owner < truthy,
        "columns should sort before literal keywords"
    );
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
