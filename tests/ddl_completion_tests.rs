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
                columns: vec![
                    Column {
                        name: "owner".to_string(),
                        data_type: "varchar".to_string(),
                        nullable: false,
                        ..Default::default()
                    },
                    Column {
                        name: "name".to_string(),
                        data_type: "varchar".to_string(),
                        nullable: false,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            Table {
                name: "form".to_string(),
                columns: vec![Column {
                    name: "form_background_url".to_string(),
                    data_type: "varchar".to_string(),
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

async fn assert_create_index_completion(
    dialect: &dyn Dialect,
    database: &str,
    qualified_table_label: &str,
) {
    let schema = schema(database);

    for sql in [
        "CREATE INDEX webhook_owner_idx ON",
        "CREATE INDEX webhook_owner_idx ON ",
    ] {
        let on_target = complete(dialect, sql, &schema).await;
        assert!(
            has_label(&on_target, qualified_table_label),
            "CREATE INDEX ON should suggest relation targets for {sql:?}: {on_target:?}"
        );
        assert!(
            !on_target
                .iter()
                .any(|item| item.kind == Some(CompletionItemKind::FIELD)),
            "CREATE INDEX ON should not suggest fields before the table target for {sql:?}: {on_target:?}"
        );
    }

    let column_sql = format!("CREATE INDEX webhook_owner_idx ON {database}.webhook (");
    let columns = complete(dialect, &column_sql, &schema).await;
    assert!(has_label(&columns, "owner"));
    assert!(has_label(&columns, "name"));
    assert!(
        !has_label(&columns, "form_background_url"),
        "CREATE INDEX column list should stay scoped to the ON table: {columns:?}"
    );
}

#[tokio::test]
async fn postgres_create_index_completion_distinguishes_ddl_on_from_join_on() {
    assert_create_index_completion(&PostgresDialect::new(), "public", "public.webhook").await;
}

#[tokio::test]
async fn mysql_create_index_completion_distinguishes_ddl_on_from_join_on() {
    assert_create_index_completion(&MysqlDialect::new(), "shop", "webhook").await;
}
