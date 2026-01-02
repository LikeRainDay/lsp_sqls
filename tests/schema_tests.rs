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
fn test_schema_manager_basic() {
    let manager = SchemaManager::new();

    let schema = Schema {
        id: SchemaId::new(),
        database: "test_db".to_string(),
        tables: vec![],
        functions: vec![],
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
        database: "db1".to_string(),
        tables: vec![Table {
            name: "table1".to_string(),
            columns: vec![],
            comment: None,
        }],
        functions: vec![],
    };

    let schema2 = Schema {
        id: SchemaId::new(),
        database: "db2".to_string(),
        tables: vec![Table {
            name: "table2".to_string(),
            columns: vec![],
            comment: None,
        }],
        functions: vec![],
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
                database: format!("db_{}", i),
                tables: vec![],
                functions: vec![],
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
        database: "test_db".to_string(),
        tables: vec![Table {
            name: "users".to_string(),
            columns: vec![
                Column {
                    name: "id".to_string(),
                    data_type: "INT".to_string(),
                    nullable: false,
                    comment: Some("Primary key".to_string()),
                },
                Column {
                    name: "email".to_string(),
                    data_type: "VARCHAR(255)".to_string(),
                    nullable: false,
                    comment: None,
                },
            ],
            comment: Some("User table".to_string()),
        }],
        functions: vec![],
    };

    assert_eq!(schema.tables.len(), 1);
    assert_eq!(schema.tables[0].columns.len(), 2);
    assert_eq!(schema.tables[0].name, "users");
}

#[test]
fn test_schema_with_functions() {
    let schema = Schema {
        id: SchemaId::new(),
        database: "test_db".to_string(),
        tables: vec![],
        functions: vec![Function {
            name: "my_function".to_string(),
            parameters: vec![FunctionParameter {
                name: "param1".to_string(),
                data_type: "INT".to_string(),
                optional: false,
            }],
            return_type: "VARCHAR".to_string(),
            description: Some("Test function".to_string()),
        }],
    };

    assert_eq!(schema.functions.len(), 1);
    assert_eq!(schema.functions[0].name, "my_function");
    assert_eq!(schema.functions[0].parameters.len(), 1);
}
