use sql_lsp::schema::{
    Column, Function, FunctionParameter, Schema, SchemaId, SchemaManager, Table,
};

#[test]
fn test_schema_id() {
    let id1 = SchemaId::new();
    let id2 = SchemaId::new();
    assert_ne!(id1, id2);

    let id_str = id1.0.to_string();
    let id3: SchemaId = id_str.parse().unwrap();
    assert_eq!(id1, id3);
}

#[test]
fn test_table_documentation_includes_object_type() {
    let view = Table {
        name: "active_users".to_string(),
        object_type: Some("VIEW".to_string()),
        columns: vec![Column {
            name: "id".to_string(),
            data_type: "integer".to_string(),
            nullable: false,
            ..Default::default()
        }],
        ..Default::default()
    };

    let documentation = view.documentation().unwrap();
    assert!(documentation.contains("Object: View"));
    assert!(documentation.contains("Columns: id integer"));

    let materialized_view = Table {
        name: "active_users_mv".to_string(),
        object_type: Some("MATERIALIZED VIEW".to_string()),
        ..Default::default()
    };
    assert_eq!(materialized_view.object_kind(), "Materialized View");
}

#[test]
fn test_schema_manager_basic() {
    let manager = SchemaManager::new();

    let schema = Schema {
        id: SchemaId::new(),
        catalog: None,
        database: "test_db".to_string(),
        server_version: None,
        tables: vec![],
        functions: vec![],
        source_uri: None,
    };

    let id = manager.register(schema.clone());
    assert_eq!(id, schema.id);

    let retrieved = manager.get(id).unwrap();
    assert_eq!(retrieved.database, "test_db");

    assert!(manager.update(id, schema.clone()));
    assert!(manager.remove(id));
    assert!(manager.get(id).is_none());
}

#[test]
fn test_schema_manager_multiple_schemas() {
    let manager = SchemaManager::new();

    let schema1 = Schema {
        id: SchemaId::new(),
        catalog: None,
        database: "db1".to_string(),
        server_version: None,
        tables: vec![Table {
            name: "table1".to_string(),
            columns: vec![],
            comment: None,
            source_location: None,
            ..Default::default()
        }],
        functions: vec![],
        source_uri: None,
    };

    let schema2 = Schema {
        id: SchemaId::new(),
        catalog: None,
        database: "db2".to_string(),
        server_version: None,
        tables: vec![Table {
            name: "table2".to_string(),
            columns: vec![],
            comment: None,
            source_location: None,
            ..Default::default()
        }],
        functions: vec![],
        source_uri: None,
    };

    let id1 = manager.register(schema1);
    let id2 = manager.register(schema2);

    assert_ne!(id1, id2);
    assert_eq!(manager.get(id1).unwrap().database, "db1");
    assert_eq!(manager.get(id2).unwrap().database, "db2");
    assert_eq!(manager.list_ids().len(), 2);
}

#[tokio::test]
async fn test_schema_manager_concurrent() {
    use std::sync::Arc;
    use tokio::task;

    let manager = Arc::new(SchemaManager::new());

    let mut handles = vec![];

    for i in 0..10 {
        let manager_clone = manager.clone();
        let handle = task::spawn(async move {
            let schema = Schema {
                id: SchemaId::new(),
                catalog: None,
                database: format!("db_{}", i),
                server_version: None,
                tables: vec![],
                functions: vec![],
                source_uri: None,
            };
            let id = manager_clone.register(schema);
            manager_clone.get(id)
        });
        handles.push(handle);
    }

    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result.is_some());
    }

    assert_eq!(manager.list_ids().len(), 10);
}

#[test]
fn test_schema_with_tables_and_columns() {
    let schema = Schema {
        id: SchemaId::new(),
        catalog: None,
        database: "test_db".to_string(),
        server_version: None,
        tables: vec![Table {
            name: "users".to_string(),
            columns: vec![
                Column {
                    name: "id".to_string(),
                    data_type: "INT".to_string(),
                    nullable: false,
                    comment: Some("Primary key".to_string()),
                    source_location: None,
                    ..Default::default()
                },
                Column {
                    name: "email".to_string(),
                    data_type: "VARCHAR(255)".to_string(),
                    nullable: false,
                    comment: None,
                    source_location: None,
                    ..Default::default()
                },
            ],
            comment: Some("User table".to_string()),
            source_location: None,
            ..Default::default()
        }],
        functions: vec![],
        source_uri: None,
    };

    assert_eq!(schema.tables.len(), 1);
    assert_eq!(schema.tables[0].columns.len(), 2);
    assert_eq!(schema.tables[0].name, "users");
}

#[test]
fn test_schema_with_functions() {
    let schema = Schema {
        id: SchemaId::new(),
        catalog: None,
        database: "test_db".to_string(),
        server_version: None,
        tables: vec![],
        functions: vec![Function {
            name: "my_function".to_string(),
            routine_type: Some("function".to_string()),
            parameters: vec![FunctionParameter {
                name: "param1".to_string(),
                data_type: "INT".to_string(),
                optional: false,
            }],
            return_type: "VARCHAR".to_string(),
            description: Some("Test function".to_string()),
        }],
        source_uri: None,
    };

    assert_eq!(schema.functions.len(), 1);
    assert_eq!(schema.functions[0].name, "my_function");
    assert_eq!(schema.functions[0].parameters.len(), 1);
}

#[test]
fn test_procedure_documentation_uses_routine_type() {
    let procedure = Function {
        name: "rebuild_cache".to_string(),
        routine_type: Some("procedure".to_string()),
        parameters: vec![FunctionParameter {
            name: "tenant_id".to_string(),
            data_type: "integer".to_string(),
            optional: false,
        }],
        return_type: "void".to_string(),
        description: None,
    };

    let documentation = procedure.markdown_documentation();
    assert!(documentation.contains("**Procedure**: `rebuild_cache(tenant_id integer)`"));
    assert!(documentation.contains("**Routine type**: `procedure`"));
    assert!(!documentation.contains("**Returns**"));
}
