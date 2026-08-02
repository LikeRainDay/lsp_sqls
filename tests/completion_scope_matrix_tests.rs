use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{ClickHouseDialect, HiveDialect, MysqlDialect, PostgresDialect};
use sql_lsp::schema::{Column, Schema, SchemaId, Table};
use tower_lsp::lsp_types::Position;

fn completion_schema() -> Schema {
    Schema {
        id: SchemaId::new(),
        database: "app".to_string(),
        server_version: None,
        tables: vec![
            Table {
                name: "users".to_string(),
                columns: vec![
                    Column {
                        name: "id".to_string(),
                        data_type: "integer".to_string(),
                        nullable: false,
                        ..Default::default()
                    },
                    Column {
                        name: "name".to_string(),
                        data_type: "text".to_string(),
                        nullable: true,
                        ..Default::default()
                    },
                    Column {
                        name: "customerAccountId".to_string(),
                        data_type: "text".to_string(),
                        nullable: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            Table {
                name: "orders".to_string(),
                columns: vec![
                    Column {
                        name: "id".to_string(),
                        data_type: "integer".to_string(),
                        nullable: false,
                        ..Default::default()
                    },
                    Column {
                        name: "total".to_string(),
                        data_type: "decimal".to_string(),
                        nullable: false,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        ],
        functions: vec![],
        source_uri: None,
    }
}

async fn complete<D: Dialect>(
    dialect: &D,
    sql: &str,
    character: usize,
    schema: &Schema,
) -> Vec<tower_lsp::lsp_types::CompletionItem> {
    dialect
        .completion(
            sql,
            Position {
                line: 0,
                character: character as u32,
            },
            Some(schema),
        )
        .await
}

async fn assert_scope_matrix<D: Dialect>(name: &str, dialect: D) {
    let schema = completion_schema();

    let without_from = complete(&dialect, "SELECT ", 7, &schema).await;
    let labels = without_from
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();
    assert!(labels.contains(&"users.name"), "{name}: {labels:?}");
    assert!(labels.contains(&"orders.total"), "{name}: {labels:?}");
    assert!(
        !labels.contains(&"name") && !labels.contains(&"total"),
        "{name} must keep source qualification before FROM binds a relation: {labels:?}"
    );

    let one_table_sql = "SELECT na FROM users";
    let one_table = complete(&dialect, one_table_sql, "SELECT na".len(), &schema).await;
    assert!(
        one_table.iter().any(|item| item.label == "name"),
        "{name} should use a short field after FROM binds users: {one_table:?}"
    );
    assert!(
        !one_table.iter().any(|item| item.label.contains("total")),
        "{name} should not leak orders fields into a users SELECT: {one_table:?}"
    );

    let predicate_sql = "SELECT * FROM users WHERE ";
    let predicate = complete(&dialect, predicate_sql, predicate_sql.len(), &schema).await;
    assert!(predicate.iter().any(|item| item.label == "id"));
    assert!(predicate.iter().any(|item| item.label == "name"));
    assert!(!predicate.iter().any(|item| item.label.contains("total")));

    let abbreviation_sql = "SELECT cai";
    let abbreviation = complete(&dialect, abbreviation_sql, abbreviation_sql.len(), &schema).await;
    assert!(
        abbreviation
            .iter()
            .any(|item| item.label == "users.customerAccountId"),
        "{name} should support camelCase abbreviation matching: {abbreviation:?}"
    );

    let aliased_predicate_sql = "SELECT * FROM users u WHERE ";
    let aliased_predicate = complete(
        &dialect,
        aliased_predicate_sql,
        aliased_predicate_sql.len(),
        &schema,
    )
    .await;
    assert!(aliased_predicate.iter().any(|item| item.label == "name"));
    assert!(!aliased_predicate
        .iter()
        .any(|item| item.label.contains("total")));

    for sql in [
        "SELECT * FROM users u JOIN orders o ON ",
        "SELECT * FROM users u JOIN orders o ON u.id = o.id WHERE ",
    ] {
        let items = complete(&dialect, sql, sql.len(), &schema).await;
        assert!(
            items.iter().any(|item| item.label == "u.name"),
            "{name} should qualify users columns with its alias in {sql}: {items:?}"
        );
        assert!(
            items.iter().any(|item| item.label == "o.total"),
            "{name} should qualify orders columns with its alias in {sql}: {items:?}"
        );
        assert!(
            !items
                .iter()
                .any(|item| item.label == "name" || item.label == "total"),
            "{name} must keep multi-table columns unambiguous in {sql}: {items:?}"
        );
    }
}

#[tokio::test]
async fn relational_dialects_share_datagrip_style_completion_scoping() {
    assert_scope_matrix("PostgreSQL", PostgresDialect::new()).await;
    assert_scope_matrix("MySQL", MysqlDialect::new()).await;
    assert_scope_matrix("Hive", HiveDialect::new()).await;
    assert_scope_matrix("ClickHouse", ClickHouseDialect::new()).await;
}
