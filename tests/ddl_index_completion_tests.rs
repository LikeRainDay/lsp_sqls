use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{MysqlDialect, PostgresDialect};
use sql_lsp::schema::{Column, Index, Schema, SchemaId, Table};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position};

fn column(name: &str) -> Column {
    Column {
        name: name.to_string(),
        data_type: "varchar".to_string(),
        nullable: false,
        ..Default::default()
    }
}

fn index(name: &str, columns: &[&str], unique: bool) -> Index {
    Index {
        name: name.to_string(),
        columns: columns.iter().map(|column| column.to_string()).collect(),
        is_unique: unique,
        ..Default::default()
    }
}

fn schema(database: &str) -> Schema {
    Schema {
        id: SchemaId::new(),
        database: database.to_string(),
        server_version: None,
        tables: vec![
            Table {
                name: "webhook".to_string(),
                columns: vec![column("owner"), column("name"), column("created_time")],
                indexes: vec![
                    index("webhook_owner_idx", &["owner"], false),
                    index("webhook_name_unique_idx", &["name"], true),
                ],
                ..Default::default()
            },
            Table {
                name: "form".to_string(),
                columns: vec![column("form_background_url"), column("form_css")],
                indexes: vec![index("form_css_idx", &["form_css"], false)],
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
async fn postgres_drop_index_completion_suggests_schema_indexes() {
    let schema = schema("public");

    let indexes = complete(&PostgresDialect::new(), "DROP INDEX ", &schema).await;
    assert!(has_label(&indexes, "public.webhook_owner_idx"));
    assert!(has_label(&indexes, "public.form_css_idx"));
    assert!(!has_label(&indexes, "owner"));
    assert!(!has_kind(&indexes, CompletionItemKind::CLASS));
    assert!(!has_kind(&indexes, CompletionItemKind::OPERATOR));

    let prefixed = complete(&PostgresDialect::new(), "DROP INDEX web", &schema).await;
    assert!(has_label(&prefixed, "public.webhook_owner_idx"));
    assert!(has_label(&prefixed, "public.webhook_name_unique_idx"));
    assert!(!has_label(&prefixed, "public.form_css_idx"));
}

#[tokio::test]
async fn mysql_alter_table_index_completion_stays_scoped_to_table() {
    let schema = schema("shop");

    for sql in [
        "ALTER TABLE shop.webhook DROP INDEX ",
        "ALTER TABLE shop.webhook RENAME INDEX ",
    ] {
        let indexes = complete(&MysqlDialect::new(), sql, &schema).await;
        assert!(has_label(&indexes, "webhook_owner_idx"));
        assert!(has_label(&indexes, "webhook_name_unique_idx"));
        assert!(!has_label(&indexes, "form_css_idx"));
        assert!(!has_label(&indexes, "owner"));
        assert!(!has_kind(&indexes, CompletionItemKind::CLASS));
        assert!(!has_kind(&indexes, CompletionItemKind::OPERATOR));
    }

    let prefixed = complete(
        &MysqlDialect::new(),
        "ALTER TABLE shop.webhook DROP INDEX webhook_o",
        &schema,
    )
    .await;
    assert!(has_label(&prefixed, "webhook_owner_idx"));
    assert!(!has_label(&prefixed, "webhook_name_unique_idx"));
}
