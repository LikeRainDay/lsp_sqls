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

async fn assert_common_dml_completion(dialect: &dyn Dialect, database: &str) {
    let schema = schema(database);
    let qualified_table = format!("{database}.webhook");

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
