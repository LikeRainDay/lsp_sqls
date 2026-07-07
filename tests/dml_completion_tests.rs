use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::{MysqlDialect, PostgresDialect};
use sql_lsp::schema::{Column, Constraint, Schema, SchemaId, Table};
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
                constraints: vec![
                    Constraint {
                        name: "webhook_pkey".to_string(),
                        constraint_type: "PRIMARY KEY".to_string(),
                        columns: vec!["owner".to_string()],
                        ..Default::default()
                    },
                    Constraint {
                        name: "webhook_name_key".to_string(),
                        constraint_type: "UNIQUE".to_string(),
                        columns: vec!["name".to_string()],
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

    let insert_values = complete(
        dialect,
        &format!("INSERT INTO {qualified_table} (owner, name) VALUES ("),
        &schema,
    )
    .await;
    assert!(has_label(&insert_values, "DEFAULT"));
    assert!(has_label(&insert_values, "NULL"));
    assert!(
        !has_label(&insert_values, "owner"),
        "INSERT value position should not suggest columns: {insert_values:?}"
    );
    assert!(
        !has_kind(&insert_values, CompletionItemKind::CLASS),
        "INSERT value position should not suggest relation targets: {insert_values:?}"
    );

    let insert_values_prefixed = complete(
        dialect,
        &format!("INSERT INTO {qualified_table} (owner, name) VALUES (NU"),
        &schema,
    )
    .await;
    assert!(has_label(&insert_values_prefixed, "NULL"));
    assert!(!has_label(&insert_values_prefixed, "DEFAULT"));
    assert!(!has_label(&insert_values_prefixed, "owner"));

    let insert_continuation = complete(
        dialect,
        &format!("INSERT INTO {qualified_table} (owner) VALUES ('app') "),
        &schema,
    )
    .await;
    if database == "public" {
        assert!(has_label(&insert_continuation, "ON CONFLICT"));
        assert!(has_label(&insert_continuation, "RETURNING"));
    } else {
        assert!(has_label(&insert_continuation, "ON DUPLICATE KEY UPDATE"));
        assert!(!has_label(&insert_continuation, "RETURNING"));
    }
    assert!(!has_label(&insert_continuation, "owner"));
    assert!(!has_operator(&insert_continuation, "="));

    let insert_continuation_prefixed = complete(
        dialect,
        &format!("INSERT INTO {qualified_table} (owner) VALUES ('app') O"),
        &schema,
    )
    .await;
    if database == "public" {
        assert!(has_label(&insert_continuation_prefixed, "ON CONFLICT"));
        assert!(!has_label(&insert_continuation_prefixed, "RETURNING"));
    } else {
        assert!(has_label(
            &insert_continuation_prefixed,
            "ON DUPLICATE KEY UPDATE"
        ));
    }
    assert!(!has_label(&insert_continuation_prefixed, "owner"));

    if database == "public" {
        let conflict_target = complete(
            dialect,
            &format!("INSERT INTO {qualified_table} (owner) VALUES ('app') ON CONFLICT ("),
            &schema,
        )
        .await;
        assert!(has_label(&conflict_target, "owner"));
        assert!(has_label(&conflict_target, "name"));
        assert!(!has_label(&conflict_target, "form_background_url"));
        assert!(!has_label(&conflict_target, "DO NOTHING"));
        assert!(!has_operator(&conflict_target, "="));

        let conflict_action = complete(
            dialect,
            &format!("INSERT INTO {qualified_table} (owner) VALUES ('app') ON CONFLICT (owner) "),
            &schema,
        )
        .await;
        assert!(has_label(&conflict_action, "DO NOTHING"));
        assert!(has_label(&conflict_action, "DO UPDATE SET"));
        assert!(!has_label(&conflict_action, "owner"));
        assert!(!has_operator(&conflict_action, "="));

        let conflict_do_tail = complete(
            dialect,
            &format!("INSERT INTO {qualified_table} (owner) VALUES ('app') ON CONFLICT DO "),
            &schema,
        )
        .await;
        assert!(has_label(&conflict_do_tail, "NOTHING"));
        assert!(has_label(&conflict_do_tail, "UPDATE SET"));
        assert!(!has_label(&conflict_do_tail, "DO NOTHING"));
        assert!(!has_label(&conflict_do_tail, "owner"));

        let conflict_constraint = complete(
            dialect,
            &format!(
                "INSERT INTO {qualified_table} (owner) VALUES ('app') ON CONFLICT ON CONSTRAINT "
            ),
            &schema,
        )
        .await;
        assert!(has_label(&conflict_constraint, "webhook_pkey"));
        assert!(has_label(&conflict_constraint, "webhook_name_key"));
        assert!(!has_label(&conflict_constraint, "webhook_owner_check"));
        assert!(!has_label(&conflict_constraint, "form_pkey"));
        assert!(!has_label(&conflict_constraint, "DO NOTHING"));
        assert!(!has_operator(&conflict_constraint, "="));

        let conflict_constraint_action = complete(
            dialect,
            &format!(
                "INSERT INTO {qualified_table} (owner) VALUES ('app') ON CONFLICT ON CONSTRAINT webhook_pkey "
            ),
            &schema,
        )
        .await;
        assert!(has_label(&conflict_constraint_action, "DO NOTHING"));
        assert!(has_label(&conflict_constraint_action, "DO UPDATE SET"));
        assert!(!has_label(&conflict_constraint_action, "webhook_pkey"));
        assert!(!has_label(&conflict_constraint_action, "owner"));

        let conflict_update_set = complete(
            dialect,
            &format!(
                "INSERT INTO {qualified_table} (owner) VALUES ('app') ON CONFLICT (owner) DO UPDATE SET "
            ),
            &schema,
        )
        .await;
        assert!(
            has_label(&conflict_update_set, "owner"),
            "ON CONFLICT DO UPDATE SET should suggest target columns: {conflict_update_set:?}"
        );
        assert!(has_label(&conflict_update_set, "name"));
        assert!(!has_label(&conflict_update_set, "form_background_url"));
        assert!(!has_operator(&conflict_update_set, "="));

        let conflict_update_operator = complete(
            dialect,
            &format!(
                "INSERT INTO {qualified_table} (owner) VALUES ('app') ON CONFLICT (owner) DO UPDATE SET owner "
            ),
            &schema,
        )
        .await;
        assert!(has_operator(&conflict_update_operator, "="));
        assert!(!has_label(&conflict_update_operator, "owner"));

        let conflict_update_value = complete(
            dialect,
            &format!(
                "INSERT INTO {qualified_table} (owner) VALUES ('app') ON CONFLICT (owner) DO UPDATE SET owner = "
            ),
            &schema,
        )
        .await;
        assert!(has_label(&conflict_update_value, "DEFAULT"));
        assert!(has_label(&conflict_update_value, "NULL"));
        assert!(!has_label(&conflict_update_value, "owner"));
        assert!(!has_operator(&conflict_update_value, "="));

        let conflict_update_value_continuation = complete(
            dialect,
            &format!(
                "INSERT INTO {qualified_table} (owner) VALUES ('app') ON CONFLICT (owner) DO UPDATE SET owner = 'app' "
            ),
            &schema,
        )
        .await;
        assert!(has_label(&conflict_update_value_continuation, ","));
        assert!(has_label(&conflict_update_value_continuation, "WHERE"));
        assert!(has_label(&conflict_update_value_continuation, "RETURNING"));
        assert!(!has_label(&conflict_update_value_continuation, "owner"));
        assert!(!has_operator(&conflict_update_value_continuation, "="));
    } else {
        let insert_set = complete(
            dialect,
            &format!("INSERT INTO {qualified_table} SET "),
            &schema,
        )
        .await;
        assert!(has_label(&insert_set, "owner"));
        assert!(has_label(&insert_set, "name"));
        assert!(!has_label(&insert_set, "form_background_url"));
        assert!(!has_operator(&insert_set, "="));

        let insert_set_operator = complete(
            dialect,
            &format!("INSERT INTO {qualified_table} SET owner "),
            &schema,
        )
        .await;
        assert!(has_operator(&insert_set_operator, "="));
        assert!(!has_label(&insert_set_operator, "owner"));

        let insert_set_value = complete(
            dialect,
            &format!("INSERT INTO {qualified_table} SET owner = "),
            &schema,
        )
        .await;
        assert!(has_label(&insert_set_value, "DEFAULT"));
        assert!(has_label(&insert_set_value, "NULL"));
        assert!(!has_label(&insert_set_value, "owner"));
        assert!(!has_operator(&insert_set_value, "="));

        let insert_set_value_continuation = complete(
            dialect,
            &format!("INSERT INTO {qualified_table} SET owner = 'app' "),
            &schema,
        )
        .await;
        assert!(has_label(&insert_set_value_continuation, ","));
        assert!(has_label(
            &insert_set_value_continuation,
            "ON DUPLICATE KEY UPDATE"
        ));
        assert!(!has_label(&insert_set_value_continuation, "WHERE"));
        assert!(!has_label(&insert_set_value_continuation, "owner"));
        assert!(!has_operator(&insert_set_value_continuation, "="));

        let duplicate_update_set = complete(
            dialect,
            &format!(
                "INSERT INTO {qualified_table} (owner) VALUES ('app') ON DUPLICATE KEY UPDATE "
            ),
            &schema,
        )
        .await;
        assert!(has_label(&duplicate_update_set, "owner"));
        assert!(has_label(&duplicate_update_set, "name"));
        assert!(!has_label(&duplicate_update_set, "form_background_url"));
        assert!(!has_operator(&duplicate_update_set, "="));

        let duplicate_update_operator = complete(
            dialect,
            &format!(
                "INSERT INTO {qualified_table} (owner) VALUES ('app') ON DUPLICATE KEY UPDATE owner "
            ),
            &schema,
        )
        .await;
        assert!(has_operator(&duplicate_update_operator, "="));
        assert!(!has_label(&duplicate_update_operator, "owner"));

        let duplicate_update_value = complete(
            dialect,
            &format!(
                "INSERT INTO {qualified_table} (owner) VALUES ('app') ON DUPLICATE KEY UPDATE owner = "
            ),
            &schema,
        )
        .await;
        assert!(has_label(&duplicate_update_value, "DEFAULT"));
        assert!(has_label(&duplicate_update_value, "NULL"));
        assert!(!has_label(&duplicate_update_value, "owner"));
        assert!(!has_operator(&duplicate_update_value, "="));

        let duplicate_update_value_continuation = complete(
            dialect,
            &format!(
                "INSERT INTO {qualified_table} (owner) VALUES ('app') ON DUPLICATE KEY UPDATE owner = 'app' "
            ),
            &schema,
        )
        .await;
        assert!(has_label(&duplicate_update_value_continuation, ","));
        assert!(!has_label(&duplicate_update_value_continuation, "WHERE"));
        assert!(!has_label(&duplicate_update_value_continuation, "owner"));
        assert!(!has_operator(&duplicate_update_value_continuation, "="));
    }

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

    let update_value = complete(
        dialect,
        &format!("UPDATE {qualified_table} SET owner = "),
        &schema,
    )
    .await;
    assert!(has_label(&update_value, "DEFAULT"));
    assert!(has_label(&update_value, "NULL"));
    assert!(
        !has_label(&update_value, "owner"),
        "UPDATE SET value position should not return fields: {update_value:?}"
    );
    assert!(
        !has_kind(&update_value, CompletionItemKind::CLASS),
        "UPDATE SET value position should not return relation targets: {update_value:?}"
    );
    assert!(
        !has_operator(&update_value, "="),
        "UPDATE SET value position should not return operators: {update_value:?}"
    );

    let update_value_prefixed = complete(
        dialect,
        &format!("UPDATE {qualified_table} SET owner = NU"),
        &schema,
    )
    .await;
    assert!(has_label(&update_value_prefixed, "NULL"));
    assert!(!has_label(&update_value_prefixed, "DEFAULT"));
    assert!(!has_label(&update_value_prefixed, "owner"));

    let update_value_continuation = complete(
        dialect,
        &format!("UPDATE {qualified_table} SET owner = 'app' "),
        &schema,
    )
    .await;
    assert!(has_label(&update_value_continuation, ","));
    assert!(has_label(&update_value_continuation, "WHERE"));
    assert!(
        !has_label(&update_value_continuation, "owner"),
        "UPDATE SET completed value should suggest continuations, not fields: {update_value_continuation:?}"
    );
    assert!(
        !has_operator(&update_value_continuation, "="),
        "UPDATE SET completed value should not suggest operators: {update_value_continuation:?}"
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

    let where_value = complete(
        dialect,
        &format!("SELECT * FROM {qualified_table} WHERE owner = "),
        &schema,
    )
    .await;
    assert!(has_label(&where_value, "NULL"));
    assert!(has_label(&where_value, "TRUE"));
    assert!(
        !has_label(&where_value, "DEFAULT"),
        "WHERE value position should not suggest DEFAULT outside assignment values: {where_value:?}"
    );
    assert!(
        !has_label(&where_value, "owner"),
        "WHERE value position should not keep returning fields: {where_value:?}"
    );
    assert!(
        !has_kind(&where_value, CompletionItemKind::CLASS),
        "WHERE value position should not return relation targets: {where_value:?}"
    );
    assert!(
        !has_operator(&where_value, "="),
        "WHERE value position should not return operators: {where_value:?}"
    );

    let where_value_prefixed = complete(
        dialect,
        &format!("SELECT * FROM {qualified_table} WHERE owner = N"),
        &schema,
    )
    .await;
    assert!(has_label(&where_value_prefixed, "NULL"));
    assert!(
        !has_label(&where_value_prefixed, "TRUE"),
        "Value keyword prefix should filter unrelated keywords: {where_value_prefixed:?}"
    );
    assert!(!has_label(&where_value_prefixed, "owner"));

    let where_in_first_value = complete(
        dialect,
        &format!("SELECT * FROM {qualified_table} WHERE owner IN ("),
        &schema,
    )
    .await;
    assert!(has_label(&where_in_first_value, "NULL"));
    assert!(has_label(&where_in_first_value, "TRUE"));
    assert!(
        !has_label(&where_in_first_value, "owner"),
        "IN list first value should not return fields: {where_in_first_value:?}"
    );
    assert!(
        !has_operator(&where_in_first_value, "="),
        "IN list first value should not return operators: {where_in_first_value:?}"
    );

    let where_in_next_value = complete(
        dialect,
        &format!("SELECT * FROM {qualified_table} WHERE owner IN ('app', "),
        &schema,
    )
    .await;
    assert!(has_label(&where_in_next_value, "NULL"));
    assert!(has_label(&where_in_next_value, "TRUE"));
    assert!(
        !has_label(&where_in_next_value, "owner"),
        "IN list next value should not return fields: {where_in_next_value:?}"
    );
    assert!(
        !has_operator(&where_in_next_value, "="),
        "IN list next value should not return operators: {where_in_next_value:?}"
    );

    let where_in_next_value_prefixed = complete(
        dialect,
        &format!("SELECT * FROM {qualified_table} WHERE owner IN ('app', N"),
        &schema,
    )
    .await;
    assert!(has_label(&where_in_next_value_prefixed, "NULL"));
    assert!(!has_label(&where_in_next_value_prefixed, "TRUE"));
    assert!(!has_label(&where_in_next_value_prefixed, "owner"));

    let where_value_continuation = complete(
        dialect,
        &format!("SELECT * FROM {qualified_table} WHERE owner = 'app' "),
        &schema,
    )
    .await;
    assert!(has_label(&where_value_continuation, "AND"));
    assert!(has_label(&where_value_continuation, "OR"));
    assert!(has_label(&where_value_continuation, "ORDER BY"));
    assert!(
        !has_label(&where_value_continuation, "owner"),
        "WHERE completed value should suggest continuations, not fields: {where_value_continuation:?}"
    );
    assert!(
        !has_operator(&where_value_continuation, "="),
        "WHERE completed value should not suggest operators: {where_value_continuation:?}"
    );

    let where_continuation_prefixed = complete(
        dialect,
        &format!("SELECT * FROM {qualified_table} WHERE owner = 'app' O"),
        &schema,
    )
    .await;
    assert!(has_label(&where_continuation_prefixed, "OR"));
    assert!(has_label(&where_continuation_prefixed, "ORDER BY"));
    assert!(!has_label(&where_continuation_prefixed, "AND"));
    assert!(!has_label(&where_continuation_prefixed, "owner"));

    let where_between_first_value = complete(
        dialect,
        &format!("SELECT * FROM {qualified_table} WHERE owner BETWEEN 1 "),
        &schema,
    )
    .await;
    assert!(has_label(&where_between_first_value, "AND"));
    assert!(
        !has_label(&where_between_first_value, "OR"),
        "BETWEEN first bound should only advance to AND: {where_between_first_value:?}"
    );
    assert!(
        !has_label(&where_between_first_value, "ORDER BY"),
        "BETWEEN first bound should not suggest query continuations before AND: {where_between_first_value:?}"
    );
    assert!(
        !has_label(&where_between_first_value, "owner"),
        "BETWEEN first bound should not return fields: {where_between_first_value:?}"
    );
    assert!(
        !has_operator(&where_between_first_value, "="),
        "BETWEEN first bound should not return operators: {where_between_first_value:?}"
    );

    let where_between_first_value_prefixed = complete(
        dialect,
        &format!("SELECT * FROM {qualified_table} WHERE owner BETWEEN 1 A"),
        &schema,
    )
    .await;
    assert!(has_label(&where_between_first_value_prefixed, "AND"));
    assert!(!has_label(&where_between_first_value_prefixed, "OR"));
    assert!(!has_label(&where_between_first_value_prefixed, "owner"));

    let where_in_continuation = complete(
        dialect,
        &format!("SELECT * FROM {qualified_table} WHERE owner IN ('app') "),
        &schema,
    )
    .await;
    assert!(has_label(&where_in_continuation, "AND"));
    assert!(has_label(&where_in_continuation, "OR"));
    assert!(!has_label(&where_in_continuation, "owner"));
    assert!(!has_operator(&where_in_continuation, "="));

    let case_when_value = complete(dialect, "SELECT CASE WHEN owner = ", &schema).await;
    assert!(has_label(&case_when_value, "NULL"));
    assert!(has_label(&case_when_value, "TRUE"));
    assert!(
        !has_label(&case_when_value, "owner"),
        "CASE WHEN right-hand value should not return fields: {case_when_value:?}"
    );
    assert!(
        !has_operator(&case_when_value, "="),
        "CASE WHEN right-hand value should not return operators: {case_when_value:?}"
    );

    let case_when_continuation =
        complete(dialect, "SELECT CASE WHEN owner = 'app' ", &schema).await;
    assert!(has_label(&case_when_continuation, "THEN"));
    assert!(has_label(&case_when_continuation, "AND"));
    assert!(has_label(&case_when_continuation, "OR"));
    assert!(
        !has_label(&case_when_continuation, "ORDER BY"),
        "CASE WHEN completed predicate should suggest THEN or boolean continuations only: {case_when_continuation:?}"
    );
    assert!(!has_label(&case_when_continuation, "owner"));
    assert!(!has_operator(&case_when_continuation, "="));

    let case_when_then_prefix =
        complete(dialect, "SELECT CASE WHEN owner = 'app' T", &schema).await;
    assert!(has_label(&case_when_then_prefix, "THEN"));
    assert!(!has_label(&case_when_then_prefix, "AND"));
    assert!(!has_label(&case_when_then_prefix, "owner"));

    let case_when_between_first =
        complete(dialect, "SELECT CASE WHEN owner BETWEEN 1 ", &schema).await;
    assert!(has_label(&case_when_between_first, "AND"));
    assert!(
        !has_label(&case_when_between_first, "THEN"),
        "CASE WHEN BETWEEN first bound must require AND before THEN: {case_when_between_first:?}"
    );
    assert!(!has_label(&case_when_between_first, "OR"));
    assert!(!has_label(&case_when_between_first, "owner"));

    let simple_case_value = complete(dialect, "SELECT CASE owner WHEN ", &schema).await;
    assert!(has_label(&simple_case_value, "NULL"));
    assert!(has_label(&simple_case_value, "TRUE"));
    assert!(has_label(&simple_case_value, "owner"));
    assert!(
        !has_label(&simple_case_value, "FROM"),
        "Simple CASE WHEN value should not suggest SELECT continuations: {simple_case_value:?}"
    );
    assert!(
        !has_operator(&simple_case_value, "="),
        "Simple CASE WHEN value should not suggest predicate operators: {simple_case_value:?}"
    );

    let simple_case_value_continuation =
        complete(dialect, "SELECT CASE owner WHEN 'app' ", &schema).await;
    assert!(has_label(&simple_case_value_continuation, "THEN"));
    assert!(!has_label(&simple_case_value_continuation, "AND"));
    assert!(!has_label(&simple_case_value_continuation, "OR"));
    assert!(!has_label(&simple_case_value_continuation, "owner"));
    assert!(!has_operator(&simple_case_value_continuation, "="));

    let simple_case_then_prefix =
        complete(dialect, "SELECT CASE owner WHEN 'app' T", &schema).await;
    assert!(has_label(&simple_case_then_prefix, "THEN"));
    assert!(!has_label(&simple_case_then_prefix, "TRUE"));
    assert!(!has_label(&simple_case_then_prefix, "owner"));

    let simple_case_second_value = complete(
        dialect,
        "SELECT CASE owner WHEN 'app' THEN 'yes' WHEN ",
        &schema,
    )
    .await;
    assert!(has_label(&simple_case_second_value, "NULL"));
    assert!(has_label(&simple_case_second_value, "owner"));
    assert!(!has_label(&simple_case_second_value, "THEN"));
    assert!(!has_operator(&simple_case_second_value, "="));

    let case_then_result = complete(dialect, "SELECT CASE WHEN owner = 'app' THEN ", &schema).await;
    assert!(has_label(&case_then_result, "NULL"));
    assert!(has_label(&case_then_result, "TRUE"));
    assert!(has_label(&case_then_result, "owner"));
    assert!(
        !has_label(&case_then_result, "FROM"),
        "CASE THEN result expression should not suggest SELECT continuations: {case_then_result:?}"
    );
    assert!(
        !has_operator(&case_then_result, "="),
        "CASE THEN result expression should not suggest predicate operators: {case_then_result:?}"
    );

    let case_then_continuation = complete(
        dialect,
        "SELECT CASE WHEN owner = 'app' THEN 'yes' ",
        &schema,
    )
    .await;
    assert!(has_label(&case_then_continuation, "WHEN"));
    assert!(has_label(&case_then_continuation, "ELSE"));
    assert!(has_label(&case_then_continuation, "END"));
    assert!(!has_label(&case_then_continuation, "FROM"));
    assert!(!has_label(&case_then_continuation, "owner"));
    assert!(!has_operator(&case_then_continuation, "="));

    let case_then_end_literal = complete(
        dialect,
        "SELECT CASE WHEN owner = 'app' THEN 'end' ",
        &schema,
    )
    .await;
    assert!(has_label(&case_then_end_literal, "ELSE"));
    assert!(has_label(&case_then_end_literal, "END"));
    assert!(!has_label(&case_then_end_literal, "owner"));

    let case_else_result = complete(
        dialect,
        "SELECT CASE WHEN owner = 'app' THEN 'yes' ELSE ",
        &schema,
    )
    .await;
    assert!(has_label(&case_else_result, "NULL"));
    assert!(has_label(&case_else_result, "owner"));
    assert!(!has_label(&case_else_result, "END"));

    let case_else_continuation = complete(
        dialect,
        "SELECT CASE WHEN owner = 'app' THEN 'yes' ELSE 'no' ",
        &schema,
    )
    .await;
    assert!(has_label(&case_else_continuation, "END"));
    assert!(!has_label(&case_else_continuation, "ELSE"));
    assert!(!has_label(&case_else_continuation, "WHEN"));
    assert!(!has_label(&case_else_continuation, "owner"));

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
