use std::collections::HashSet;

use crate::clickhouse_signatures::{self, CatalogKind as ClickHouseCatalogKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BuiltinSignature {
    pub name: String,
    pub parameter_groups: Vec<Vec<&'static str>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BuiltinValue {
    pub name: &'static str,
    pub category: &'static str,
}

impl BuiltinSignature {
    fn single(name: &'static str, parameters: &'static [&'static str]) -> Self {
        Self {
            name: name.to_string(),
            parameter_groups: vec![parameters.to_vec()],
        }
    }

    pub fn label(&self) -> String {
        let groups = self
            .parameter_groups
            .iter()
            .map(|group| format!("({})", group.join(", ")))
            .collect::<String>();
        format!("{}{groups}", self.name)
    }
}

// DBX keeps a compact, dialect-aware signature fallback in
// apps/desktop/src/lib/sql/sqlCompletion.ts. Oxide uses live routine metadata
// first, then this catalog when a server does not expose built-ins through its
// schema APIs. Keep the common list deliberately portable across SQL families.
const COMMON_SIGNATURES: &[(&str, &[&str])] = &[
    ("COUNT", &["expression"]),
    ("SUM", &["expression"]),
    ("AVG", &["expression"]),
    ("MIN", &["expression"]),
    ("MAX", &["expression"]),
    ("CONCAT", &["value", "...values"]),
    ("SUBSTRING", &["string", "start", "length"]),
    ("SUBSTR", &["string", "start", "length"]),
    ("REPLACE", &["string", "old", "new"]),
    ("TRIM", &["string"]),
    ("LTRIM", &["string"]),
    ("RTRIM", &["string"]),
    ("UPPER", &["string"]),
    ("LOWER", &["string"]),
    ("LENGTH", &["string"]),
    ("EXTRACT", &["unit", "date"]),
    ("ROUND", &["number", "decimals"]),
    ("FLOOR", &["number"]),
    ("CEIL", &["number"]),
    ("CEILING", &["number"]),
    ("ABS", &["number"]),
    ("MOD", &["dividend", "divisor"]),
    ("POWER", &["base", "exponent"]),
    ("SQRT", &["number"]),
    ("SIGN", &["number"]),
    ("COALESCE", &["value", "...values"]),
    ("NULLIF", &["expression1", "expression2"]),
    ("CAST", &["expression AS type"]),
    ("GREATEST", &["...values"]),
    ("LEAST", &["...values"]),
];

const POSTGRES_SIGNATURES: &[(&str, &[&str])] = &[
    ("JSONB_BUILD_OBJECT", &["key", "value", "...pairs"]),
    ("JSONB_AGG", &["expression"]),
    ("TO_JSONB", &["value"]),
    ("JSONB_SET", &["target", "path", "new_value"]),
    ("ARRAY_AGG", &["expression"]),
    ("STRING_AGG", &["expression", "delimiter"]),
    ("GEN_RANDOM_UUID", &[]),
    ("NOW", &[]),
];

const MYSQL_SIGNATURES: &[(&str, &[&str])] = &[
    ("CONVERT", &["expression", "type"]),
    ("DATE_FORMAT", &["date", "format"]),
    ("FROM_UNIXTIME", &["unix_timestamp"]),
    ("UNIX_TIMESTAMP", &[]),
    ("VERSION", &[]),
    ("SYSDATE", &[]),
    ("CURRENT_DATE", &[]),
    ("CURRENT_TIME", &[]),
    ("CURRENT_TIMESTAMP", &[]),
    ("CURDATE", &[]),
    ("CURTIME", &[]),
    ("LOCALTIME", &[]),
    ("LOCALTIMESTAMP", &[]),
    ("UTC_DATE", &[]),
    ("UTC_TIME", &[]),
    ("UTC_TIMESTAMP", &[]),
    ("DATE", &["expression"]),
    ("TIME", &["expression"]),
    ("DATE_ADD", &["date", "INTERVAL expr unit"]),
    ("DATE_SUB", &["date", "INTERVAL expr unit"]),
    ("DATEDIFF", &["date1", "date2"]),
    (
        "TIMESTAMPDIFF",
        &["unit", "datetime_expr1", "datetime_expr2"],
    ),
    ("YEAR", &["date"]),
    ("MONTH", &["date"]),
    ("DAY", &["date"]),
    ("HOUR", &["datetime"]),
    ("MINUTE", &["datetime"]),
    ("SECOND", &["datetime"]),
    ("DAYOFWEEK", &["date"]),
    ("DAYOFYEAR", &["date"]),
    ("LAST_DAY", &["date"]),
    ("STR_TO_DATE", &["string", "format"]),
    ("MONTHNAME", &["date"]),
    ("DAYOFMONTH", &["date"]),
    ("WEEKDAY", &["date"]),
    ("WEEK", &["date", "mode"]),
    ("QUARTER", &["date"]),
    ("ADDDATE", &["date", "days"]),
    ("SUBDATE", &["date", "days"]),
    ("ADDTIME", &["datetime", "time"]),
    ("SUBTIME", &["datetime", "time"]),
    ("TIMEDIFF", &["datetime1", "datetime2"]),
    ("FROM_DAYS", &["day_number"]),
    ("TO_DAYS", &["date"]),
    ("MAKEDATE", &["year", "day_of_year"]),
    ("MAKETIME", &["hour", "minute", "second"]),
    ("IFNULL", &["expression", "fallback"]),
    ("IF", &["condition", "true_value", "false_value"]),
    ("CONCAT_WS", &["separator", "...values"]),
    ("LEFT", &["string", "length"]),
    ("RIGHT", &["string", "length"]),
    ("SUBSTRING_INDEX", &["string", "delimiter", "count"]),
    ("CHAR_LENGTH", &["string"]),
    ("INSTR", &["string", "substring"]),
    ("LOCATE", &["substring", "string"]),
    ("LPAD", &["string", "length", "pad"]),
    ("RPAD", &["string", "length", "pad"]),
    ("REVERSE", &["string"]),
    ("POSITION", &["substring", "string"]),
    ("REPEAT", &["string", "count"]),
    ("STRCMP", &["string1", "string2"]),
    ("FIND_IN_SET", &["string", "string_list"]),
    ("ELT", &["index", "string1", "...strings"]),
    ("FIELD", &["value", "value1", "...values"]),
    ("MAKE_SET", &["bits", "string1", "...strings"]),
    ("RAND", &[]),
    ("POW", &["base", "exponent"]),
    ("EXP", &["number"]),
    ("LN", &["number"]),
    ("LOG", &["base", "number"]),
    ("LOG10", &["number"]),
    ("LOG2", &["number"]),
    ("SIN", &["number"]),
    ("PI", &[]),
    ("COS", &["number"]),
    ("TAN", &["number"]),
    ("ASIN", &["number"]),
    ("ACOS", &["number"]),
    ("ATAN", &["number"]),
    ("ATAN2", &["y", "x"]),
    ("DEGREES", &["radians"]),
    ("RADIANS", &["degrees"]),
    ("BIN", &["number"]),
    ("HEX", &["value"]),
    ("UNHEX", &["string"]),
    ("OCT", &["number"]),
    ("CONV", &["number", "from_base", "to_base"]),
    ("TRUNCATE", &["number", "decimals"]),
    ("MD5", &["string"]),
    ("SHA1", &["string"]),
    ("SHA2", &["string", "bit_length"]),
    ("JSON_EXTRACT", &["json", "path"]),
    ("JSON_UNQUOTE", &["json"]),
    ("JSON_OBJECT", &["key", "value", "...pairs"]),
    ("JSON_ARRAY", &["...values"]),
    (
        "JSON_SET",
        &["json", "path", "value", "...path_value_pairs"],
    ),
    (
        "JSON_INSERT",
        &["json", "path", "value", "...path_value_pairs"],
    ),
    (
        "JSON_REPLACE",
        &["json", "path", "value", "...path_value_pairs"],
    ),
    ("JSON_REMOVE", &["json", "path", "...paths"]),
    ("JSON_CONTAINS", &["target", "candidate"]),
    ("JSON_LENGTH", &["json"]),
    ("GROUP_CONCAT", &["expression"]),
    ("PASSWORD", &["string"]),
    ("DATABASE", &[]),
    ("SCHEMA", &[]),
    ("USER", &[]),
    ("CURRENT_USER", &[]),
    ("COLLATION", &["string"]),
    ("FOUND_ROWS", &[]),
    ("LAST_INSERT_ID", &[]),
    ("BENCHMARK", &["count", "expression"]),
    ("SLEEP", &["seconds"]),
    ("UUID", &[]),
    ("UUID_SHORT", &[]),
    ("NOW", &[]),
];

const SQLITE_SIGNATURES: &[(&str, &[&str])] = &[
    ("JSON_EXTRACT", &["json", "path"]),
    ("JSON_SET", &["json", "path", "value"]),
    ("STRFTIME", &["format", "time"]),
    ("IFNULL", &["expression", "fallback"]),
    ("NOW", &[]),
];

// DBX deliberately removes NOW() from the SQLite-compatible Cloudflare D1
// fallback because D1 exposes SQLite date/time functions rather than a NOW
// routine. Keep the product boundary explicit instead of widening SQLite's
// convenience catalog to every compatibility alias.
const CLOUDFLARE_D1_SIGNATURES: &[(&str, &[&str])] = &[
    ("JSON_EXTRACT", &["json", "path"]),
    ("JSON_SET", &["json", "path", "value"]),
    ("STRFTIME", &["format", "time"]),
    ("IFNULL", &["expression", "fallback"]),
];

const SQLSERVER_SIGNATURES: &[(&str, &[&str])] = &[
    ("CONVERT", &["type", "expression"]),
    ("TRY_CAST", &["expression AS type"]),
    ("TRY_CONVERT", &["type", "expression"]),
    ("JSON_VALUE", &["expression", "path"]),
    ("JSON_QUERY", &["expression", "path"]),
    ("NEWID", &[]),
    ("GETDATE", &[]),
    ("GETUTCDATE", &[]),
    ("SYSDATETIME", &[]),
    ("SYSUTCDATETIME", &[]),
    ("DATEADD", &["datepart", "number", "date"]),
    ("DATEDIFF", &["datepart", "startdate", "enddate"]),
    ("DATEPART", &["datepart", "date"]),
    ("DATENAME", &["datepart", "date"]),
    ("EOMONTH", &["start_date"]),
    ("CHARINDEX", &["substring", "string"]),
    ("PATINDEX", &["pattern", "string"]),
    ("LEN", &["string"]),
    ("STUFF", &["string", "start", "length", "replace"]),
    ("ISNULL", &["expression", "replacement"]),
];

const MANTICORE_SIGNATURES: &[(&str, &[&str])] = &[
    ("MATCH", &["query"]),
    ("BM25F", &["field=weight", "...fields"]),
    ("EXIST", &["attribute", "default"]),
    ("IDF", &["keyword"]),
    ("PACKEDFACTORS", &[]),
    ("QUERY", &[]),
    ("REMAP", &["expression", "from_values", "to_values"]),
    ("SNIPPET", &["field", "query"]),
    ("WEIGHT", &[]),
    ("ZONESPANLIST", &[]),
    ("BIGINT", &["expression"]),
    ("DOUBLE", &["expression"]),
    ("INTEGER", &["expression"]),
    ("SINT", &["expression"]),
    ("TO_STRING", &["expression"]),
    ("UINT", &["expression"]),
    ("UINT64", &["expression"]),
    ("GEODIST", &["lat1", "lon1", "lat2", "lon2"]),
    ("CONTAINS", &["polygon", "point"]),
    ("POLY2D", &["...points"]),
    ("CRC32", &["expression"]),
    ("FIBONACCI", &["number"]),
    ("KNN_DIST", &[]),
    ("NOW", &[]),
    ("DATE_FORMAT", &["timestamp", "format"]),
    ("DAY", &["timestamp"]),
    ("MONTH", &["timestamp"]),
    ("YEAR", &["timestamp"]),
    ("HOUR", &["timestamp"]),
    ("MINUTE", &["timestamp"]),
    ("SECOND", &["timestamp"]),
];

// IBM Db2 11.5 built-in function catalog. These stable scalar forms are kept
// intentionally compact; live routine metadata still takes precedence.
const DB2_SIGNATURES: &[(&str, &[&str])] = &[
    ("ADD_DAYS", &["datetime", "days"]),
    ("ADD_HOURS", &["datetime", "hours"]),
    ("ADD_MINUTES", &["datetime", "minutes"]),
    ("ADD_MONTHS", &["datetime", "months"]),
    ("ADD_SECONDS", &["datetime", "seconds"]),
    ("ADD_YEARS", &["datetime", "years"]),
    ("DAYS", &["date"]),
    ("MONTHS_BETWEEN", &["date1", "date2"]),
    ("TIMESTAMP_FORMAT", &["string", "format"]),
    ("VARCHAR_FORMAT", &["timestamp", "format"]),
    ("GENERATE_UNIQUE", &[]),
];

// Dameng documents these date/time entries as functions (including the empty
// parentheses), unlike Oracle and Db2 bare system values.
const DAMENG_SIGNATURES: &[(&str, &[&str])] = &[
    ("SYSDATE", &[]),
    ("CURDATE", &[]),
    ("CURTIME", &[]),
    ("CURRENT_DATE", &[]),
    ("CURRENT_TIME", &[]),
    ("CURRENT_TIMESTAMP", &[]),
    ("LOCALTIMESTAMP", &[]),
    ("TO_CHAR", &["value", "format"]),
    ("TO_DATE", &["string", "format"]),
    ("DATEADD", &["datepart", "number", "date"]),
    ("DATEDIFF", &["datepart", "startdate", "enddate"]),
    ("DECODE", &["expression", "search", "result", "default"]),
    (
        "REGEXP_SUBSTR",
        &[
            "source",
            "pattern",
            "position",
            "occurrence",
            "match_parameter",
        ],
    ),
];

// DBX exposes these Oracle system values as expression completions without
// call parentheses. Keep them separate from function signatures so accepting
// SYSDATE never produces the invalid/needlessly transformed SYSDATE().
const ORACLE_SYSTEM_VALUES: &[&str] = &[
    "SYSDATE",
    "SYSTIMESTAMP",
    "CURRENT_DATE",
    "CURRENT_TIMESTAMP",
    "LOCALTIMESTAMP",
    "SESSIONTIMEZONE",
    "DBTIMEZONE",
    "USER",
    "UID",
];

// Db2 special registers are expressions, not routine calls. IBM documents
// both the traditional spaced form and SQL-standard underscore aliases.
const DB2_SPECIAL_REGISTERS: &[&str] = &[
    "CURRENT DATE",
    "CURRENT_DATE",
    "CURRENT TIME",
    "CURRENT_TIME",
    "CURRENT TIMESTAMP",
    "CURRENT_TIMESTAMP",
    "CURRENT TIMEZONE",
    "CURRENT_TIMEZONE",
    "CURRENT USER",
    "CURRENT_USER",
    "CURRENT SCHEMA",
    "CURRENT_SCHEMA",
    "CURRENT SERVER",
    "CURRENT_SERVER",
    "SESSION_USER",
    "SYSTEM_USER",
    "USER",
    "SYSDATE",
    "LOCALTIMESTAMP",
];

fn lookup(
    entries: &'static [(&'static str, &'static [&'static str])],
    name: &str,
) -> Option<BuiltinSignature> {
    entries
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(candidate, parameters)| BuiltinSignature::single(candidate, parameters))
}

fn version_allows(dialect: &str, name: &str, server_version: Option<(u32, u32)>) -> bool {
    let Some(version) = server_version else {
        return true;
    };
    let minimum = match (dialect, name) {
        ("mysql" | "mariadb", name) if name.starts_with("JSON_") => Some((5, 7)),
        ("postgres" | "postgresql" | "pgsql" | "psql", name)
            if name.starts_with("JSONB_") || name == "TO_JSONB" =>
        {
            Some((9, 4))
        }
        ("sqlite" | "sqlite3" | "turso" | "cloudflare-d1", name) if name.starts_with("JSON_") => {
            Some((3, 9))
        }
        ("sqlserver", "TRY_CAST" | "TRY_CONVERT" | "EOMONTH") => Some((11, 0)),
        ("sqlserver", "JSON_VALUE" | "JSON_QUERY") => Some((13, 0)),
        ("sqlserver", "STRING_AGG") => Some((14, 0)),
        _ => None,
    };
    minimum.is_none_or(|minimum| version >= minimum)
}

fn is_non_sql_dialect(dialect: &str) -> bool {
    matches!(
        dialect,
        "mongodb"
            | "mongo"
            | "mongodb-json"
            | "mongo-json"
            | "json"
            | "redis"
            | "elasticsearch"
            | "elastic"
            | "es"
            | "es-dsl"
            | "eql"
            | "es-eql"
    )
}

fn dialect_signature_entries(
    dialect: &str,
) -> Option<&'static [(&'static str, &'static [&'static str])]> {
    match dialect {
        "db2" => Some(DB2_SIGNATURES),
        "dameng" => Some(DAMENG_SIGNATURES),
        "mysql" | "mariadb" => Some(MYSQL_SIGNATURES),
        "postgres" | "postgresql" | "pgsql" | "psql" => Some(POSTGRES_SIGNATURES),
        "sqlite" | "sqlite3" | "turso" => Some(SQLITE_SIGNATURES),
        "cloudflare-d1" => Some(CLOUDFLARE_D1_SIGNATURES),
        "sqlserver" => Some(SQLSERVER_SIGNATURES),
        "manticoresearch" => Some(MANTICORE_SIGNATURES),
        _ => None,
    }
}

pub(crate) fn builtin_signature_catalog_for(
    dialect: &str,
    server_version: Option<(u32, u32)>,
) -> Vec<BuiltinSignature> {
    let normalized_dialect = dialect.to_ascii_lowercase();
    if is_non_sql_dialect(&normalized_dialect) {
        return Vec::new();
    }

    let mut signatures = if matches!(normalized_dialect.as_str(), "clickhouse" | "ch") {
        clickhouse_signatures::direct_expression_catalog()
    } else {
        Vec::new()
    };
    if let Some(entries) = dialect_signature_entries(&normalized_dialect) {
        signatures.extend(
            entries
                .iter()
                .filter(|(name, _)| version_allows(&normalized_dialect, name, server_version))
                .map(|(name, parameters)| BuiltinSignature::single(name, parameters)),
        );
    }
    let existing_names = signatures
        .iter()
        .map(|signature| signature.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    signatures.extend(
        COMMON_SIGNATURES
            .iter()
            .filter(|(name, _)| {
                !existing_names.contains(&name.to_ascii_lowercase())
                    && version_allows(&normalized_dialect, name, server_version)
            })
            .map(|(name, parameters)| BuiltinSignature::single(name, parameters)),
    );
    signatures
}

pub(crate) fn builtin_value_catalog_for(dialect: &str) -> Vec<BuiltinValue> {
    match dialect.to_ascii_lowercase().as_str() {
        "db2" => DB2_SPECIAL_REGISTERS
            .iter()
            .map(|name| BuiltinValue {
                name,
                category: "Db2 special register",
            })
            .collect(),
        "oracle" | "oceanbase-oracle" => ORACLE_SYSTEM_VALUES
            .iter()
            .map(|name| BuiltinValue {
                name,
                category: "Oracle system value",
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn builtin_signature_completion_catalog_for(
    dialect: &str,
    server_version: Option<(u32, u32)>,
    prefix: &str,
    limit: usize,
) -> Vec<BuiltinSignature> {
    let normalized_dialect = dialect.to_ascii_lowercase();
    if matches!(normalized_dialect.as_str(), "clickhouse" | "ch") {
        return clickhouse_signatures::completion_catalog(
            prefix,
            ClickHouseCatalogKind::Expression,
            limit,
        );
    }
    builtin_signature_catalog_for(&normalized_dialect, server_version)
        .into_iter()
        .filter(|signature| {
            prefix.is_empty()
                || signature
                    .name
                    .to_ascii_lowercase()
                    .starts_with(&prefix.to_ascii_lowercase())
        })
        .take(limit)
        .collect()
}

pub(crate) fn builtin_table_signature_completion_catalog_for(
    dialect: &str,
    prefix: &str,
    limit: usize,
) -> Vec<BuiltinSignature> {
    if matches!(dialect.to_ascii_lowercase().as_str(), "clickhouse" | "ch") {
        clickhouse_signatures::completion_catalog(prefix, ClickHouseCatalogKind::Table, limit)
    } else {
        Vec::new()
    }
}

pub(crate) fn builtin_function_is_known_for(
    dialect: &str,
    name: &str,
    server_version: Option<(u32, u32)>,
) -> bool {
    if matches!(dialect.to_ascii_lowercase().as_str(), "clickhouse" | "ch") {
        clickhouse_signatures::contains_expression(name)
    } else {
        !builtin_signatures_for(dialect, name, server_version).is_empty()
    }
}

pub(crate) fn builtin_function_catalog_is_available_for(dialect: &str) -> bool {
    !is_non_sql_dialect(&dialect.to_ascii_lowercase())
}

pub(crate) fn builtin_window_function_catalog_is_available_for(dialect: &str) -> bool {
    let normalized = dialect.to_ascii_lowercase();
    !is_non_sql_dialect(&normalized) && !matches!(normalized.as_str(), "clickhouse" | "ch")
}

pub(crate) fn builtin_signatures_for(
    dialect: &str,
    name: &str,
    server_version: Option<(u32, u32)>,
) -> Vec<BuiltinSignature> {
    let normalized_dialect = dialect.to_ascii_lowercase();
    if is_non_sql_dialect(&normalized_dialect) {
        return Vec::new();
    }

    if normalized_dialect == "clickhouse" || normalized_dialect == "ch" {
        let clickhouse = clickhouse_signatures::signatures_for(name);
        if !clickhouse.is_empty() {
            return clickhouse;
        }
    }

    let signature = dialect_signature_entries(&normalized_dialect)
        .and_then(|entries| lookup(entries, name))
        .or_else(|| lookup(COMMON_SIGNATURES, name));
    signature
        .filter(|signature| {
            version_allows(normalized_dialect.as_str(), &signature.name, server_version)
        })
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        builtin_signature_catalog_for, builtin_signature_completion_catalog_for,
        builtin_signatures_for, builtin_value_catalog_for,
    };

    #[test]
    fn dialect_override_wins_over_common_signature() {
        let signature = builtin_signatures_for("sqlserver", "convert", Some((16, 0)))
            .pop()
            .expect("SQL Server CONVERT signature");
        assert_eq!(signature.parameter_groups[0], ["type", "expression"]);
    }

    #[test]
    fn server_version_filters_unavailable_signatures() {
        assert!(builtin_signatures_for("sqlserver", "json_value", Some((12, 0))).is_empty());
        assert!(!builtin_signatures_for("sqlserver", "json_value", Some((13, 0))).is_empty());
    }

    #[test]
    fn clickhouse_parametric_signatures_retain_parameter_groups() {
        let signature = builtin_signatures_for("clickhouse", "quantilesTDigest", None)
            .pop()
            .expect("ClickHouse parametric aggregate");
        assert_eq!(signature.parameter_groups.len(), 2);
        assert_eq!(signature.parameter_groups[1], ["expression"]);
    }

    #[test]
    fn completion_catalog_deduplicates_dialect_overrides() {
        let catalog = builtin_signature_catalog_for("sqlserver", Some((16, 0)));
        let convert = catalog
            .iter()
            .filter(|signature| signature.name.eq_ignore_ascii_case("convert"))
            .collect::<Vec<_>>();
        assert_eq!(convert.len(), 1);
        assert_eq!(convert[0].parameter_groups[0], ["type", "expression"]);
    }

    #[test]
    fn completion_catalog_applies_version_gates_and_non_sql_boundaries() {
        let catalog = builtin_signature_catalog_for("sqlserver", Some((12, 0)));
        assert!(!catalog
            .iter()
            .any(|signature| signature.name == "JSON_VALUE"));
        assert!(builtin_signature_catalog_for("mongodb", None).is_empty());
    }

    #[test]
    fn oracle_system_values_are_not_function_signatures() {
        let values = builtin_value_catalog_for("oracle");
        assert!(values.iter().any(|value| value.name == "SYSDATE"));
        assert!(values.iter().any(|value| value.name == "SESSIONTIMEZONE"));
        assert_eq!(builtin_value_catalog_for("oceanbase-oracle"), values);
        assert!(builtin_value_catalog_for("postgres").is_empty());
        assert!(builtin_signatures_for("oracle", "SYSDATE", None).is_empty());
    }

    #[test]
    fn db2_registers_and_dameng_functions_keep_distinct_call_styles() {
        let db2_values = builtin_value_catalog_for("db2");
        assert!(db2_values.iter().any(|value| value.name == "CURRENT DATE"));
        assert!(db2_values.iter().any(|value| value.name == "SYSDATE"));
        assert!(builtin_signatures_for("db2", "SYSDATE", None).is_empty());
        assert_eq!(
            builtin_signatures_for("db2", "ADD_DAYS", None)[0].parameter_groups[0],
            ["datetime", "days"]
        );

        assert!(builtin_value_catalog_for("dameng").is_empty());
        assert_eq!(
            builtin_signatures_for("dameng", "SYSDATE", None)[0].parameter_groups[0],
            Vec::<&str>::new()
        );
        assert_eq!(
            builtin_signatures_for("dameng", "TO_DATE", None)[0].parameter_groups[0],
            ["string", "format"]
        );
    }

    #[test]
    fn manticore_and_sqlite_compatibility_catalogs_match_dbx_boundaries() {
        let manticore = builtin_signature_catalog_for("manticoresearch", None);
        assert_eq!(
            manticore
                .iter()
                .filter(|signature| {
                    ["POLY2D", "TO_STRING", "CRC32", "DATE_FORMAT"]
                        .contains(&signature.name.as_str())
                })
                .count(),
            4
        );
        assert_eq!(
            builtin_signatures_for("manticoresearch", "POLY2D", None)[0].parameter_groups[0],
            ["...points"]
        );

        assert_eq!(
            builtin_signatures_for("sqlite", "NOW", None)[0].parameter_groups[0],
            Vec::<&str>::new()
        );
        assert!(builtin_signatures_for("cloudflare-d1", "NOW", None).is_empty());
        assert!(!builtin_signatures_for("cloudflare-d1", "STRFTIME", None).is_empty());
    }

    #[test]
    fn mysql_catalog_includes_version_and_reverse_without_broadening_postgres() {
        for dialect in ["mysql", "mariadb"] {
            assert_eq!(
                builtin_signatures_for(dialect, "VERSION", None)[0].parameter_groups[0],
                Vec::<&str>::new(),
            );
            assert_eq!(
                builtin_signatures_for(dialect, "REVERSE", None)[0].parameter_groups[0],
                ["string"],
            );
            assert!(
                builtin_signature_completion_catalog_for(dialect, None, "ver", 10)
                    .iter()
                    .any(|signature| signature.name == "VERSION")
            );
            assert!(
                builtin_signature_completion_catalog_for(dialect, None, "reve", 10)
                    .iter()
                    .any(|signature| signature.name == "REVERSE")
            );
        }

        for dialect in ["postgres", "sqlserver"] {
            assert!(builtin_signatures_for(dialect, "VERSION", None).is_empty());
            assert!(builtin_signatures_for(dialect, "REVERSE", None).is_empty());
            assert!(
                builtin_signature_completion_catalog_for(dialect, None, "ver", 10)
                    .iter()
                    .all(|signature| signature.name != "VERSION")
            );
            assert!(
                builtin_signature_completion_catalog_for(dialect, None, "reve", 10)
                    .iter()
                    .all(|signature| signature.name != "REVERSE")
            );
        }
    }

    #[test]
    fn mysql_long_tail_catalog_matches_dbx_without_leaking_to_other_dialects() {
        let expected = [
            "MONTHNAME",
            "WEEK",
            "ADDTIME",
            "MAKEDATE",
            "POSITION",
            "MAKE_SET",
            "LOG2",
            "ATAN2",
            "CONV",
            "JSON_OBJECT",
            "JSON_INSERT",
            "JSON_REMOVE",
            "DATABASE",
            "LAST_INSERT_ID",
            "UUID_SHORT",
        ];
        for dialect in ["mysql", "mariadb"] {
            for name in expected {
                assert!(
                    !builtin_signatures_for(dialect, name, None).is_empty(),
                    "{dialect} should expose {name}"
                );
            }
            assert_eq!(
                builtin_signatures_for(dialect, "WEEK", None)[0].parameter_groups[0],
                ["date", "mode"]
            );
            assert_eq!(
                builtin_signatures_for(dialect, "JSON_INSERT", None)[0].parameter_groups[0],
                ["json", "path", "value", "...path_value_pairs"]
            );
            assert!(
                builtin_signature_completion_catalog_for(dialect, None, "uuid_s", 10)
                    .iter()
                    .any(|signature| signature.name == "UUID_SHORT")
            );
        }

        for dialect in ["postgres", "oracle", "db2", "dameng", "sqlserver"] {
            for name in expected {
                assert!(
                    builtin_signatures_for(dialect, name, None).is_empty(),
                    "{dialect} must not inherit MySQL-only {name}"
                );
            }
        }
    }
}
