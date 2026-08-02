use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{ClickHouseDialect, HiveDialect, MysqlDialect, PostgresDialect};
use sql_lsp::schema::{Column, Schema, SchemaId, Table};
use tower_lsp::lsp_types::{CompletionItemKind, Position};

fn schema() -> Schema {
    Schema {
        id: SchemaId::new(),
        database: "app".to_string(),
        server_version: None,
        tables: vec![Table {
            name: "jobs".to_string(),
            columns: vec![Column {
                name: "status".to_string(),
                data_type: "enum('active','paused','can''t-run')".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        functions: vec![],
        source_uri: None,
    }
}

async fn assert_domain_completion(name: &str, dialect: &dyn Dialect) {
    let sql = "SELECT * FROM jobs WHERE status = ";
    let items = dialect
        .completion(sql, Position::new(0, sql.len() as u32), Some(&schema()))
        .await;
    let active = items
        .iter()
        .find(|item| item.label == "active")
        .unwrap_or_else(|| panic!("{name} must suggest enum values in predicates: {items:#?}"));
    assert_eq!(active.kind, Some(CompletionItemKind::ENUM_MEMBER));
    assert_eq!(active.insert_text.as_deref(), Some("'active'"));
    assert!(items.iter().any(|item| item.label == "paused"));

    let prefixed_sql = "SELECT * FROM jobs WHERE status = a";
    let prefixed_items = dialect
        .completion(
            prefixed_sql,
            Position::new(0, prefixed_sql.len() as u32),
            Some(&schema()),
        )
        .await;
    assert!(prefixed_items.iter().any(|item| item.label == "active"));
    assert!(!prefixed_items.iter().any(|item| item.label == "paused"));

    let in_sql = "SELECT * FROM jobs j WHERE j.status IN (";
    let in_items = dialect
        .completion(
            in_sql,
            Position::new(0, in_sql.len() as u32),
            Some(&schema()),
        )
        .await;
    assert!(in_items.iter().any(|item| {
        item.label == "can't-run" && item.insert_text.as_deref() == Some("'can''t-run'")
    }));
}

#[tokio::test]
async fn relational_dialects_offer_bounded_metadata_domain_values() {
    let dialects: Vec<(&str, Box<dyn Dialect>)> = vec![
        ("postgres", Box::new(PostgresDialect::new())),
        ("mysql", Box::new(MysqlDialect::new())),
        ("hive", Box::new(HiveDialect::new())),
        ("clickhouse", Box::new(ClickHouseDialect::new())),
    ];
    for (name, dialect) in dialects {
        assert_domain_completion(name, dialect.as_ref()).await;
    }
}
