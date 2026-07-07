use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{MysqlDialect, PostgresDialect};
use sql_lsp::schema::{Column, Schema, SchemaId, Table};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position};

fn schema(database: &str) -> Schema {
    Schema {
        id: SchemaId::new(),
        database: database.to_string(),
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
async fn mysql_select_item_continuation_excludes_fields() {
    let schema = schema("shop");
    let items = complete(&MysqlDialect::new(), "SELECT owner ", &schema).await;

    assert!(has_label(&items, ","));
    assert!(has_label(&items, "AS"));
    assert!(has_label(&items, "FROM"));
    assert!(!has_label(&items, "owner"));
    assert!(!has_label(&items, "created_time"));
    assert!(!has_kind(&items, CompletionItemKind::FIELD));

    let after_comma = complete(&MysqlDialect::new(), "SELECT owner, ", &schema).await;
    assert!(has_label(&after_comma, "owner"));
    assert!(has_label(&after_comma, "created_time"));
    assert!(has_kind(&after_comma, CompletionItemKind::FIELD));
}

#[tokio::test]
async fn postgres_select_wildcard_continuation_suggests_from_without_alias() {
    let schema = schema("public");
    let items = complete(&PostgresDialect::new(), "SELECT * ", &schema).await;

    assert!(has_label(&items, ","));
    assert!(has_label(&items, "FROM"));
    assert!(!has_label(&items, "AS"));
    assert!(!has_label(&items, "owner"));
    assert!(!has_kind(&items, CompletionItemKind::FIELD));

    let prefixed = complete(&PostgresDialect::new(), "SELECT owner F", &schema).await;
    assert!(has_label(&prefixed, "FROM"));
    assert!(!has_label(&prefixed, "AS"));
    assert!(!has_label(&prefixed, "owner"));
}

#[tokio::test]
async fn select_wildcard_from_prefix_excludes_field_matches() {
    let schema = schema("public");

    for sql in ["SELECT * f", "SELECT * fr", "SELECT * fro"] {
        let items = complete(&PostgresDialect::new(), sql, &schema).await;

        assert!(has_label(&items, "FROM"), "{sql:?} should suggest FROM");
        assert!(!has_label(&items, "owner"), "{sql:?} should not suggest fields");
        assert!(
            !has_label(&items, "created_time"),
            "{sql:?} should not suggest fields"
        );
        assert!(!has_kind(&items, CompletionItemKind::FIELD));
    }

    let from_target = complete(&PostgresDialect::new(), "SELECT * from", &schema).await;
    assert!(has_label(&from_target, "public.webhook"));
    assert!(has_kind(&from_target, CompletionItemKind::CLASS));
    assert!(!has_kind(&from_target, CompletionItemKind::FIELD));
}
