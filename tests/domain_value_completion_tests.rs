use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{
    ClickHouseDialect, HiveDialect, MysqlDialect, PostgresDialect, SqliteDialect,
};
use sql_lsp::schema::{Column, Schema, SchemaId, Table};
use tower_lsp::lsp_types::{CompletionItemKind, InsertTextFormat, Position};

fn schema() -> Schema {
    Schema {
        id: SchemaId::new(),
        catalog: None,
        database: "app".to_string(),
        server_version: None,
        tables: vec![Table {
            name: "jobs".to_string(),
            columns: vec![
                Column {
                    name: "status".to_string(),
                    data_type: "enum('active','paused','can''t-run')".to_string(),
                    ..Default::default()
                },
                Column {
                    name: "title".to_string(),
                    data_type: "TEXT".to_string(),
                    ..Default::default()
                },
                Column {
                    name: "attempts".to_string(),
                    data_type: "INTEGER".to_string(),
                    ..Default::default()
                },
                Column {
                    name: "enabled".to_string(),
                    data_type: "BOOLEAN".to_string(),
                    ..Default::default()
                },
                Column {
                    name: "created_at".to_string(),
                    data_type: "TIMESTAMP".to_string(),
                    ..Default::default()
                },
                Column {
                    name: "payload".to_string(),
                    data_type: "JSON".to_string(),
                    ..Default::default()
                },
            ],
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

    for (column, label, insert_text) in [
        ("title", "''", "'${1:value}'"),
        ("attempts", "0", "${1:0}"),
        (
            "created_at",
            "'YYYY-MM-DD HH:MM:SS'",
            "'${1:YYYY-MM-DD HH:MM:SS}'",
        ),
        ("payload", "'{}'", r#"'{"${1:key}":"${2:value}"}'"#),
    ] {
        let sql = format!("SELECT * FROM jobs WHERE {column} = ");
        let items = dialect
            .completion(&sql, Position::new(0, sql.len() as u32), Some(&schema()))
            .await;
        let template = items
            .iter()
            .find(|item| item.label == label)
            .unwrap_or_else(|| panic!("{name} must suggest {label} for {column}: {items:#?}"));
        assert_eq!(template.kind, Some(CompletionItemKind::SNIPPET));
        assert_eq!(template.insert_text.as_deref(), Some(insert_text));
        assert_eq!(template.insert_text_format, Some(InsertTextFormat::SNIPPET));
        assert!(items.iter().any(|item| item.label == "NULL"));
    }

    let boolean_sql = "SELECT * FROM jobs WHERE enabled = ";
    let boolean_items = dialect
        .completion(
            boolean_sql,
            Position::new(0, boolean_sql.len() as u32),
            Some(&schema()),
        )
        .await;
    assert!(boolean_items.iter().any(|item| item.label == "TRUE"));
    assert!(boolean_items.iter().any(|item| item.label == "FALSE"));

    for sql in [
        "SELECT enabled, count(*) FROM jobs GROUP BY enabled HAVING enabled = ",
        "SELECT * FROM jobs j JOIN jobs k ON k.enabled = ",
        "UPDATE jobs SET enabled = ",
    ] {
        let items = dialect
            .completion(sql, Position::new(0, sql.len() as u32), Some(&schema()))
            .await;
        assert!(
            items.iter().any(|item| item.label == "TRUE"),
            "{name} must offer typed values in comparison contexts: {sql}: {items:#?}",
        );
    }

    let mut ambiguous_schema = schema();
    ambiguous_schema.tables[0].columns.push(Column {
        name: "shared_value".to_string(),
        data_type: "TEXT".to_string(),
        ..Default::default()
    });
    ambiguous_schema.tables.push(Table {
        name: "metrics".to_string(),
        columns: vec![Column {
            name: "shared_value".to_string(),
            data_type: "INTEGER".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    });
    let ambiguous_sql = "SELECT * FROM jobs CROSS JOIN metrics WHERE shared_value = ";
    let ambiguous_items = dialect
        .completion(
            ambiguous_sql,
            Position::new(0, ambiguous_sql.len() as u32),
            Some(&ambiguous_schema),
        )
        .await;
    assert!(ambiguous_items.iter().any(|item| item.label == "NULL"));
    assert!(!ambiguous_items.iter().any(|item| item.label == "''"));
    assert!(!ambiguous_items.iter().any(|item| item.label == "0"));
}

#[tokio::test]
async fn relational_dialects_offer_bounded_metadata_value_completions() {
    let dialects: Vec<(&str, Box<dyn Dialect>)> = vec![
        ("postgres", Box::new(PostgresDialect::new())),
        ("mysql", Box::new(MysqlDialect::new())),
        ("hive", Box::new(HiveDialect::new())),
        ("clickhouse", Box::new(ClickHouseDialect::new())),
        ("sqlite", Box::new(SqliteDialect::new())),
    ];
    for (name, dialect) in dialects {
        assert_domain_completion(name, dialect.as_ref()).await;
    }
}
