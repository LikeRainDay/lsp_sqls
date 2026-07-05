use sql_lsp::dialects::DialectRegistry;

#[test]
fn test_dialect_registry() {
    let registry = DialectRegistry::new();

    let names = registry.list_names();
    assert!(names.contains(&"mysql".to_string()));
    assert!(names.contains(&"postgres".to_string()));
    assert!(names.contains(&"hive".to_string()));
    assert!(names.contains(&"elasticsearch-eql".to_string()));
    assert!(names.contains(&"elasticsearch-dsl".to_string()));
    assert!(names.contains(&"clickhouse".to_string()));
    assert!(names.contains(&"redis".to_string()));
    assert!(names.contains(&"mongodb".to_string()));
}

#[test]
fn test_dialect_registry_get_by_name() {
    let registry = DialectRegistry::new();

    assert!(registry.get_by_name("mysql").is_some());
    assert!(registry.get_by_name("mariadb").is_some());
    assert!(registry.get_by_name("postgres").is_some());
    assert!(registry.get_by_name("postgresql").is_some());
    assert!(registry.get_by_name("pgsql").is_some());
    assert!(registry.get_by_name("psql").is_some());
    assert!(registry.get_by_name("hive").is_some());
    assert!(registry.get_by_name("hql").is_some());
    assert!(registry.get_by_name("elasticsearch-eql").is_some());
    assert!(registry.get_by_name("eql").is_some());
    assert!(registry.get_by_name("es-eql").is_some());
    assert!(registry.get_by_name("elasticsearch-dsl").is_some());
    assert!(registry.get_by_name("elasticsearch").is_some());
    assert!(registry.get_by_name("elastic").is_some());
    assert!(registry.get_by_name("es").is_some());
    assert!(registry.get_by_name("es-dsl").is_some());
    assert!(registry.get_by_name("clickhouse").is_some());
    assert!(registry.get_by_name("ch").is_some());
    assert!(registry.get_by_name("redis").is_some());
    assert!(registry.get_by_name("mongodb").is_some());
    assert!(registry.get_by_name("mongo").is_some());
    assert!(registry.get_by_name("json").is_some());
    assert!(registry.get_by_name("nonexistent").is_none());
}

#[test]
fn test_dialect_registry_case_insensitive() {
    let registry = DialectRegistry::new();

    assert!(registry.get_by_name("MySQL").is_some());
    assert!(registry.get_by_name("POSTGRES").is_some());
    assert!(registry.get_by_name("HiVe").is_some());
}
