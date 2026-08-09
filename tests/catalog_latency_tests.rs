use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{ClickHouseDialect, HiveDialect, MysqlDialect, PostgresDialect};
use sql_lsp::schema::{Column, Schema, SchemaId, Table};
use std::time::{Duration, Instant};
use tower_lsp::lsp_types::Position;

const TABLE_COUNT: usize = 2_500;
const MAX_COLD_COMPLETION: Duration = Duration::from_secs(5);

fn large_schema() -> Schema {
    Schema {
        id: SchemaId::new(),
        catalog: None,
        database: "analytics".to_string(),
        server_version: None,
        tables: (0..TABLE_COUNT)
            .map(|index| Table {
                name: format!("event_partition_{index:04}"),
                columns: vec![
                    Column {
                        name: "event_id".to_string(),
                        data_type: "BIGINT".to_string(),
                        ..Default::default()
                    },
                    Column {
                        name: "created_at".to_string(),
                        data_type: "TIMESTAMP".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            })
            .collect(),
        functions: vec![],
        source_uri: None,
    }
}

async fn assert_large_catalog_completion(name: &str, dialect: &dyn Dialect, schema: &Schema) {
    let sql = "SELECT * FROM event_partition_2499";
    let started = Instant::now();
    let items = dialect
        .completion(sql, Position::new(0, sql.len() as u32), Some(schema))
        .await;
    let elapsed = started.elapsed();

    assert!(
        items.iter().any(|item| {
            item.label == "event_partition_2499" || item.label == "analytics.event_partition_2499"
        }),
        "{name} must find the target relation in a {TABLE_COUNT}-table catalog"
    );
    assert!(
        elapsed < MAX_COLD_COMPLETION,
        "{name} cold completion took {elapsed:?}, above the {MAX_COLD_COMPLETION:?} regression budget"
    );
    eprintln!("{name}: {TABLE_COUNT} tables completed in {elapsed:?}");
}

#[tokio::test]
async fn relational_dialects_keep_large_catalog_completion_bounded() {
    let schema = large_schema();
    let dialects: Vec<(&str, Box<dyn Dialect>)> = vec![
        ("postgres", Box::new(PostgresDialect::new())),
        ("mysql", Box::new(MysqlDialect::new())),
        ("hive", Box::new(HiveDialect::new())),
        ("clickhouse", Box::new(ClickHouseDialect::new())),
    ];

    for (name, dialect) in dialects {
        assert_large_catalog_completion(name, dialect.as_ref(), &schema).await;
    }
}
