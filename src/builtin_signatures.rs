use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BuiltinSignature {
    pub name: &'static str,
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
            name,
            parameter_groups: vec![parameters.to_vec()],
        }
    }

    fn grouped(name: &'static str, groups: &[&[&'static str]]) -> Self {
        Self {
            name,
            parameter_groups: groups.iter().map(|group| group.to_vec()).collect(),
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
    ("FIND_IN_SET", &["string", "string_list"]),
    ("RAND", &[]),
    ("MD5", &["string"]),
    ("SHA1", &["string"]),
    ("SHA2", &["string", "bit_length"]),
    ("JSON_EXTRACT", &["json", "path"]),
    ("JSON_UNQUOTE", &["json"]),
    ("GROUP_CONCAT", &["expression"]),
    ("UUID", &[]),
    ("NOW", &[]),
];

const SQLITE_SIGNATURES: &[(&str, &[&str])] = &[
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
    ("GEODIST", &["lat1", "lon1", "lat2", "lon2"]),
    ("KNN_DIST", &[]),
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

fn clickhouse_signatures(name: &str) -> Vec<BuiltinSignature> {
    if name.eq_ignore_ascii_case("toStartOfInterval") {
        return [
            &["value", "INTERVAL x unit"][..],
            &["value", "INTERVAL x unit", "time_zone"][..],
            &["value", "INTERVAL x unit", "origin", "time_zone?"][..],
        ]
        .into_iter()
        .map(|parameters| BuiltinSignature::single("toStartOfInterval", parameters))
        .collect();
    }
    if name.eq_ignore_ascii_case("quantilesTDigest") {
        return vec![BuiltinSignature::grouped(
            "quantilesTDigest",
            &[&["level", "...levels"], &["expression"]],
        )];
    }
    let signature = match name.to_ascii_lowercase().as_str() {
        "arrayjoin" => Some(("arrayJoin", &["array"][..])),
        "formatdatetime" => Some(("formatDateTime", &["time", "format", "time_zone?"][..])),
        "todate" => Some(("toDate", &["value"][..])),
        "todatetime" => Some(("toDateTime", &["value", "time_zone?"][..])),
        "tuple" => Some(("tuple", &["...values"][..])),
        "map" => Some(("map", &["key", "value", "...pairs"][..])),
        "multiif" => Some(("multiIf", &["condition", "then", "...branches", "else"][..])),
        _ => None,
    };
    signature
        .map(|(name, parameters)| vec![BuiltinSignature::single(name, parameters)])
        .unwrap_or_default()
}

fn clickhouse_signature_catalog() -> Vec<BuiltinSignature> {
    [
        "toStartOfInterval",
        "quantilesTDigest",
        "arrayJoin",
        "formatDateTime",
        "toDate",
        "toDateTime",
        "tuple",
        "map",
        "multiIf",
    ]
    .into_iter()
    .flat_map(clickhouse_signatures)
    .collect()
}

fn is_non_sql_dialect(dialect: &str) -> bool {
    matches!(
        dialect,
        "mongodb"
            | "mongo"
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
        "mysql" | "mariadb" => Some(MYSQL_SIGNATURES),
        "postgres" | "postgresql" | "pgsql" | "psql" => Some(POSTGRES_SIGNATURES),
        "sqlite" | "sqlite3" | "turso" | "cloudflare-d1" => Some(SQLITE_SIGNATURES),
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
        clickhouse_signature_catalog()
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
        let clickhouse = clickhouse_signatures(name);
        if !clickhouse.is_empty() {
            return clickhouse;
        }
    }

    let signature = dialect_signature_entries(&normalized_dialect)
        .and_then(|entries| lookup(entries, name))
        .or_else(|| lookup(COMMON_SIGNATURES, name));
    signature
        .filter(|signature| {
            version_allows(normalized_dialect.as_str(), signature.name, server_version)
        })
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{builtin_signature_catalog_for, builtin_signatures_for, builtin_value_catalog_for};

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
}
