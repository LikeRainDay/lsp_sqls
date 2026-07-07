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

fn has_kind(items: &[CompletionItem], kind: CompletionItemKind) -> bool {
    items.iter().any(|item| item.kind == Some(kind))
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

async fn assert_references_completion(
    dialect: &dyn Dialect,
    database: &str,
    qualified_table_label: &str,
) {
    let schema = schema(database);

    let table_target = complete(
        dialect,
        &format!("CREATE TABLE {database}.child (owner_id INT REFERENCES "),
        &schema,
    )
    .await;
    assert!(
        has_label(&table_target, qualified_table_label),
        "REFERENCES table target should suggest relation names: {table_target:?}"
    );
    assert!(
        !has_kind(&table_target, CompletionItemKind::FIELD),
        "REFERENCES table target should not suggest fields: {table_target:?}"
    );

    let create_columns = complete(
        dialect,
        &format!("CREATE TABLE {database}.child (owner_id INT REFERENCES {database}.webhook ("),
        &schema,
    )
    .await;
    assert!(has_label(&create_columns, "owner"));
    assert!(has_label(&create_columns, "name"));
    assert!(
        !has_label(&create_columns, "form_background_url"),
        "REFERENCES column list should stay scoped to the referenced table: {create_columns:?}"
    );
    assert!(!has_kind(&create_columns, CompletionItemKind::CLASS));
    assert!(!has_kind(&create_columns, CompletionItemKind::OPERATOR));

    let prefixed_column = complete(
        dialect,
        &format!("CREATE TABLE {database}.child (owner_id INT REFERENCES {database}.webhook (ow"),
        &schema,
    )
    .await;
    assert!(has_label(&prefixed_column, "owner"));
    assert!(!has_label(&prefixed_column, "name"));

    let reference_actions = complete(
        dialect,
        &format!("CREATE TABLE {database}.child (owner_id INT REFERENCES {database}.webhook "),
        &schema,
    )
    .await;
    assert!(has_label(&reference_actions, "("));
    assert!(has_label(&reference_actions, "ON DELETE"));
    assert!(has_label(&reference_actions, "ON UPDATE"));
    assert!(
        !has_label(&reference_actions, qualified_table_label),
        "REFERENCES action position should not continue suggesting relation names: {reference_actions:?}"
    );
    assert!(
        !has_label(&reference_actions, "owner"),
        "REFERENCES action position should not suggest columns: {reference_actions:?}"
    );
    assert!(!has_kind(&reference_actions, CompletionItemKind::CLASS));
    assert!(!has_kind(&reference_actions, CompletionItemKind::FIELD));

    let prefixed_action = complete(
        dialect,
        &format!("CREATE TABLE {database}.child (owner_id INT REFERENCES {database}.webhook O"),
        &schema,
    )
    .await;
    assert!(has_label(&prefixed_action, "ON DELETE"));
    assert!(has_label(&prefixed_action, "ON UPDATE"));

    let alter_columns = complete(
        dialect,
        &format!(
            "ALTER TABLE {database}.form ADD CONSTRAINT fk_owner FOREIGN KEY (owner) REFERENCES {database}.webhook ("
        ),
        &schema,
    )
    .await;
    assert!(has_label(&alter_columns, "owner"));
    assert!(has_label(&alter_columns, "name"));
    assert!(
        !has_label(&alter_columns, "form_background_url"),
        "ALTER TABLE REFERENCES column list should use the referenced table, not the altered table: {alter_columns:?}"
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

#[tokio::test]
async fn postgres_references_completion_suggests_target_table_and_columns() {
    assert_references_completion(&PostgresDialect::new(), "public", "public.webhook").await;
}

#[tokio::test]
async fn mysql_references_completion_suggests_target_table_and_columns() {
    assert_references_completion(&MysqlDialect::new(), "shop", "webhook").await;
}
