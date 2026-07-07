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
                    Column {
                        name: "created_time".to_string(),
                        data_type: "timestamp".to_string(),
                        nullable: true,
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

fn has_operator(items: &[CompletionItem], label: &str) -> bool {
    items
        .iter()
        .any(|item| item.label == label && item.kind == Some(CompletionItemKind::OPERATOR))
}

fn has_kind(items: &[CompletionItem], kind: CompletionItemKind) -> bool {
    items.iter().any(|item| item.kind == Some(kind))
}

async fn assert_common_dml_completion(dialect: &dyn Dialect, database: &str) {
    let schema = schema(database);
    let qualified_table = format!("{database}.webhook");

    let insert_actions =
        complete(dialect, &format!("INSERT INTO {qualified_table} "), &schema).await;
    assert!(has_label(&insert_actions, "VALUES"));
    assert!(has_label(&insert_actions, "SELECT"));
    assert!(
        !has_label(&insert_actions, "owner"),
        "INSERT action position should not suggest columns: {insert_actions:?}"
    );
    assert!(
        !has_kind(&insert_actions, CompletionItemKind::CLASS),
        "INSERT action position should not suggest relation targets: {insert_actions:?}"
    );

    let insert_prefixed = complete(
        dialect,
        &format!("INSERT INTO {qualified_table} VAL"),
        &schema,
    )
    .await;
    assert!(has_label(&insert_prefixed, "VALUES"));
    assert!(!has_label(&insert_prefixed, "SELECT"));

    let insert_columns = complete(
        dialect,
        &format!("INSERT INTO {qualified_table} ("),
        &schema,
    )
    .await;
    assert!(has_label(&insert_columns, "owner"));
    assert!(has_label(&insert_columns, "name"));
    assert!(
        !has_label(&insert_columns, "form_background_url"),
        "INSERT column list should stay scoped to the target table: {insert_columns:?}"
    );

    let update_actions = complete(dialect, &format!("UPDATE {qualified_table} "), &schema).await;
    assert!(has_label(&update_actions, "SET"));
    assert!(
        !has_label(&update_actions, "owner"),
        "UPDATE action position should not suggest columns: {update_actions:?}"
    );
    assert!(
        !has_kind(&update_actions, CompletionItemKind::CLASS),
        "UPDATE action position should not suggest relation targets: {update_actions:?}"
    );

    let update_prefixed = complete(dialect, &format!("UPDATE {qualified_table} S"), &schema).await;
    assert!(has_label(&update_prefixed, "SET"));
    assert!(!has_label(&update_prefixed, "owner"));

    let update_set = complete(dialect, &format!("UPDATE {qualified_table} SET "), &schema).await;
    assert!(has_label(&update_set, "owner"));
    assert!(has_label(&update_set, "name"));
    assert!(
        !has_operator(&update_set, "="),
        "UPDATE SET start should suggest columns before operators: {update_set:?}"
    );
    assert!(
        !has_label(&update_set, "form_background_url"),
        "UPDATE SET should stay scoped to the target table: {update_set:?}"
    );

    let update_operator = complete(
        dialect,
        &format!("UPDATE {qualified_table} SET owner "),
        &schema,
    )
    .await;
    assert!(has_operator(&update_operator, "="));
    assert!(
        !has_label(&update_operator, "owner"),
        "UPDATE SET operator position should not keep returning field candidates: {update_operator:?}"
    );

    let delete_actions =
        complete(dialect, &format!("DELETE FROM {qualified_table} "), &schema).await;
    assert!(has_label(&delete_actions, "WHERE"));
    assert!(
        !has_label(&delete_actions, "owner"),
        "DELETE action position should not suggest columns: {delete_actions:?}"
    );
    assert!(
        !has_kind(&delete_actions, CompletionItemKind::CLASS),
        "DELETE action position should not suggest relation targets: {delete_actions:?}"
    );

    let delete_prefixed = complete(
        dialect,
        &format!("DELETE FROM {qualified_table} WH"),
        &schema,
    )
    .await;
    assert!(has_label(&delete_prefixed, "WHERE"));
    assert!(!has_label(&delete_prefixed, "owner"));

    let update_where = complete(
        dialect,
        &format!("UPDATE {qualified_table} SET owner = 'app' WHERE "),
        &schema,
    )
    .await;
    assert!(has_label(&update_where, "owner"));
    assert!(has_label(&update_where, "created_time"));
    assert!(!has_label(&update_where, "form_background_url"));

    let delete_where = complete(
        dialect,
        &format!("DELETE FROM {qualified_table} WHERE "),
        &schema,
    )
    .await;
    assert!(has_label(&delete_where, "owner"));
    assert!(has_label(&delete_where, "name"));
    assert!(!has_label(&delete_where, "form_background_url"));

    let returning = complete(
        dialect,
        &format!("INSERT INTO {qualified_table} (owner) VALUES ('app') RETURNING "),
        &schema,
    )
    .await;
    assert!(has_label(&returning, "owner"));
    assert!(has_label(&returning, "name"));
    assert!(!has_label(&returning, "form_background_url"));
}

#[tokio::test]
async fn postgres_dml_completion_stays_scoped_to_target_relation() {
    assert_common_dml_completion(&PostgresDialect::new(), "public").await;
}

#[tokio::test]
async fn mysql_dml_completion_stays_scoped_to_target_relation() {
    assert_common_dml_completion(&MysqlDialect::new(), "shop").await;
}
