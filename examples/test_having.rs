use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::*;
use sql_lsp::schema::{Column, Schema, SchemaId, Table};
use tower_lsp::lsp_types::Position;

#[tokio::main]
async fn main() {
    let dialect = MysqlDialect::new();

    let schema = Schema {
        id: SchemaId::new(),
        database: "shop".to_string(),
        tables: vec![Table {
            name: "orders".to_string(),
            columns: vec![
                Column {
                    name: "order_id".to_string(),
                    data_type: "INT".to_string(),
                    nullable: false,
                    comment: None,
                    source_location: None,
                },
                Column {
                    name: "user_id".to_string(),
                    data_type: "INT".to_string(),
                    nullable: false,
                    comment: None,
                    source_location: None,
                },
                Column {
                    name: "total".to_string(),
                    data_type: "DECIMAL".to_string(),
                    nullable: false,
                    comment: None,
                    source_location: None,
                },
                Column {
                    name: "status".to_string(),
                    data_type: "VARCHAR".to_string(),
                    nullable: false,
                    comment: None,
                    source_location: None,
                },
            ],
            comment: Some("Orders table".to_string()),
            source_location: None,
        }],
        functions: vec![],
        source_uri: None,
    };

    let sql = "SELECT user_id, COUNT(*) as cnt FROM orders GROUP BY user_id HAVING ";
    let position = Position {
        line: 0,
        character: sql.len() as u32,
    };

    // First check what context is detected
    use sql_lsp::parser::SqlParser;
    let mut parser = SqlParser::new();
    let parse_result = parser.parse(sql);

    if let Some(tree) = &parse_result.tree {
        if let Some(node) = parser.get_node_at_position(tree, position) {
            let context = parser.analyze_completion_context(node, sql, position);
            println!("Detected context: {:?}", context);
            println!("Node kind: {}", node.kind());
            println!("Node text: {:?}", parser.node_text(node, sql));
        } else {
            println!("No node found at position");
        }
    } else {
        println!("No parse tree");
    }

    let mut items = dialect.completion(sql, position, Some(&schema)).await;

    // Sort by sort_text like in the test
    items.sort_by(|a, b| {
        let a_sort = a.sort_text.as_ref().unwrap_or(&a.label);
        let b_sort = b.sort_text.as_ref().unwrap_or(&b.label);
        a_sort.cmp(b_sort)
    });

    println!("\nHAVING clause completion items ({} total):", items.len());
    for (i, item) in items.iter().enumerate() {
        let kind = match item.kind {
            Some(k) => format!("{:?}", k),
            None => "Unknown".to_string(),
        };
        let sort = item.sort_text.as_ref().unwrap_or(&item.label);
        println!(
            "  {}. [{}] {} (sort: {}) - {:?}",
            i + 1,
            kind,
            item.label,
            sort,
            item.detail
        );
    }
}
