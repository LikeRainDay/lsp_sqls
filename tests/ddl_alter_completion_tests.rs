use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{MysqlDialect, PostgresDialect};
use sql_lsp::schema::{Column, Constraint, Schema, SchemaId, Table};
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
        server_version: None,
        tables: vec![
            Table {
                name: "webhook".to_string(),
                columns: vec![column("owner"), column("name"), column("created_time")],
                constraints: vec![
                    Constraint {
                        name: "webhook_pkey".to_string(),
                        constraint_type: "PRIMARY KEY".to_string(),
                        columns: vec!["owner".to_string(), "name".to_string()],
                        ..Default::default()
                    },
                    Constraint {
                        name: "webhook_owner_check".to_string(),
                        constraint_type: "CHECK".to_string(),
                        columns: vec!["owner".to_string()],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            Table {
                name: "form".to_string(),
                columns: vec![column("form_background_url")],
                constraints: vec![Constraint {
                    name: "form_pkey".to_string(),
                    constraint_type: "PRIMARY KEY".to_string(),
                    columns: vec!["form_background_url".to_string()],
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

async fn assert_alter_table_completion(dialect: &dyn Dialect, database: &str) {
    let schema = schema(database);
    let table = format!("{database}.webhook");

    let actions = complete(dialect, &format!("ALTER TABLE {table} "), &schema).await;
    assert!(
        has_label(&actions, "ADD COLUMN"),
        "missing ADD COLUMN action: {actions:?}"
    );
    assert!(
        has_label(&actions, "DROP COLUMN"),
        "missing DROP COLUMN action: {actions:?}"
    );
    assert!(
        has_label(&actions, "ADD CONSTRAINT"),
        "missing ADD CONSTRAINT action: {actions:?}"
    );
    assert!(
        !has_label(&actions, "owner"),
        "ALTER TABLE action position should not suggest columns: {actions:?}"
    );
    assert!(
        !has_kind(&actions, CompletionItemKind::CLASS),
        "ALTER TABLE action position should not suggest relation targets: {actions:?}"
    );

    let prefixed_actions = complete(dialect, &format!("ALTER TABLE {table} DR"), &schema).await;
    assert!(has_label(&prefixed_actions, "DROP COLUMN"));
    assert!(has_label(&prefixed_actions, "DROP CONSTRAINT"));
    assert!(!has_label(&prefixed_actions, "ADD COLUMN"));

    for sql in [
        format!("ALTER TABLE {table} DROP COLUMN "),
        format!("ALTER TABLE {table} ALTER COLUMN "),
        format!("ALTER TABLE {table} RENAME COLUMN "),
    ] {
        let columns = complete(dialect, &sql, &schema).await;
        assert!(has_label(&columns, "owner"));
        assert!(has_label(&columns, "name"));
        assert!(!has_label(&columns, "form_background_url"));
        assert!(!has_kind(&columns, CompletionItemKind::CLASS));
        assert!(!has_kind(&columns, CompletionItemKind::OPERATOR));
    }

    let prefixed_column = complete(
        dialect,
        &format!("ALTER TABLE {table} DROP COLUMN ow"),
        &schema,
    )
    .await;
    assert!(has_label(&prefixed_column, "owner"));
    assert!(!has_label(&prefixed_column, "name"));

    for sql in [
        format!("ALTER TABLE {table} DROP CONSTRAINT "),
        format!("ALTER TABLE {table} RENAME CONSTRAINT "),
    ] {
        let constraints = complete(dialect, &sql, &schema).await;
        assert!(has_label(&constraints, "webhook_pkey"));
        assert!(has_label(&constraints, "webhook_owner_check"));
        assert!(!has_label(&constraints, "form_pkey"));
        assert!(!has_label(&constraints, "owner"));
        assert!(!has_kind(&constraints, CompletionItemKind::CLASS));
        assert!(!has_kind(&constraints, CompletionItemKind::OPERATOR));
    }
}

#[tokio::test]
async fn postgres_alter_table_completion_uses_table_scoped_targets() {
    assert_alter_table_completion(&PostgresDialect::new(), "public").await;
}

#[tokio::test]
async fn mysql_alter_table_completion_uses_table_scoped_targets() {
    assert_alter_table_completion(&MysqlDialect::new(), "shop").await;
}

#[tokio::test]
async fn mysql_modify_and_change_column_completion_uses_table_scoped_targets() {
    let schema = schema("shop");

    for sql in [
        "ALTER TABLE shop.webhook MODIFY COLUMN ",
        "ALTER TABLE shop.webhook MODIFY ",
        "ALTER TABLE shop.webhook CHANGE COLUMN ",
        "ALTER TABLE shop.webhook CHANGE ",
    ] {
        let columns = complete(&MysqlDialect::new(), sql, &schema).await;
        assert!(has_label(&columns, "owner"));
        assert!(has_label(&columns, "name"));
        assert!(!has_label(&columns, "form_background_url"));
        assert!(!has_kind(&columns, CompletionItemKind::CLASS));
        assert!(!has_kind(&columns, CompletionItemKind::OPERATOR));
    }

    let prefixed = complete(
        &MysqlDialect::new(),
        "ALTER TABLE shop.webhook CHANGE COLUMN ow",
        &schema,
    )
    .await;
    assert!(has_label(&prefixed, "owner"));
    assert!(!has_label(&prefixed, "name"));
}
