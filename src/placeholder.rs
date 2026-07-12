//! SQL template/bind placeholder recognition.
//!
//! The parser consumes a same-byte-length normalized SQL string so tree-sitter
//! can build a useful tree without shifting LSP positions. Placeholders inside
//! literals, quoted identifiers, comments, and PostgreSQL dollar-quoted bodies
//! are intentionally left untouched.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlPlaceholderKind {
    DollarNumber,
    QuestionMark,
    AtName,
    ColonName,
    Mustache,
    DollarBrace,
    PythonNamed,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SqlPlaceholderDialect {
    #[default]
    Generic,
    Postgres,
    Mysql,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SqlPlaceholderSpan {
    pub start: usize,
    pub end: usize,
    pub kind: SqlPlaceholderKind,
}

pub const PLACEHOLDER_IDENTIFIER: &str = "__oxide_placeholder__";

pub fn normalize_sql_placeholders(source: &str) -> String {
    normalize_sql_placeholders_for_dialect(source, SqlPlaceholderDialect::Generic)
}

pub fn normalize_sql_placeholders_for_dialect(
    source: &str,
    dialect: SqlPlaceholderDialect,
) -> String {
    let spans = sql_placeholder_spans_for_dialect(source, dialect);
    if spans.is_empty() {
        return source.to_string();
    }

    let mut normalized = source.as_bytes().to_vec();
    for span in spans {
        normalized[span.start] = b'p';
        normalized[span.start + 1..span.end].fill(b'_');
    }

    // Every recognized placeholder is ASCII, so replacing it byte-for-byte
    // cannot invalidate the source's existing UTF-8 encoding.
    String::from_utf8(normalized).expect("placeholder normalization preserves UTF-8")
}

pub fn sql_placeholder_spans(source: &str) -> Vec<SqlPlaceholderSpan> {
    sql_placeholder_spans_for_dialect(source, SqlPlaceholderDialect::Generic)
}

pub fn sql_placeholder_spans_for_dialect(
    source: &str,
    dialect: SqlPlaceholderDialect,
) -> Vec<SqlPlaceholderSpan> {
    let mut spans = Vec::new();
    let mut index = 0;

    while index < source.len() {
        if source[index..].starts_with("--") || starts_hash_comment(source, index, dialect) {
            index = skip_line_comment(source, index);
            continue;
        }

        if source[index..].starts_with("/*") {
            index = skip_block_comment(source, index);
            continue;
        }

        let Some(ch) = source[index..].chars().next() else {
            break;
        };

        if ch == '$' {
            if let Some(end) = skip_dollar_quoted_region(source, index) {
                index = end;
                continue;
            }
        }

        if matches!(ch, '\'' | '"' | '`') {
            index = skip_quoted_region(source, index, ch);
            continue;
        }

        if ch == '[' {
            index = skip_bracketed_identifier(source, index);
            continue;
        }

        if let Some(span) = placeholder_at_for_dialect(source, index, dialect) {
            index = span.end;
            spans.push(span);
            continue;
        }

        index += ch.len_utf8();
    }

    spans
}

/// Recognize a placeholder beginning at a position already known to be SQL
/// code (rather than a quoted/comment region).
pub fn placeholder_at(source: &str, start: usize) -> Option<SqlPlaceholderSpan> {
    placeholder_at_for_dialect(source, start, SqlPlaceholderDialect::Generic)
}

pub fn placeholder_at_for_dialect(
    source: &str,
    start: usize,
    dialect: SqlPlaceholderDialect,
) -> Option<SqlPlaceholderSpan> {
    let bytes = source.as_bytes();
    let first = *bytes.get(start)?;
    let previous_is_identifier = start
        .checked_sub(1)
        .and_then(|index| bytes.get(index))
        .is_some_and(|byte| is_identifier_byte(*byte));

    let (end, kind) = match first {
        b'$' if !previous_is_identifier => {
            if bytes.get(start + 1).is_some_and(u8::is_ascii_digit) {
                (
                    take_while(bytes, start + 2, u8::is_ascii_digit),
                    SqlPlaceholderKind::DollarNumber,
                )
            } else if bytes.get(start + 1) == Some(&b'{') {
                (
                    delimited_name_end(bytes, start + 2, b'}', None)?,
                    SqlPlaceholderKind::DollarBrace,
                )
            } else {
                return None;
            }
        }
        b'?' => {
            if matches!(bytes.get(start + 1), Some(b'|' | b'&')) {
                return None;
            }
            if dialect == SqlPlaceholderDialect::Postgres
                && bytes
                    .get(start + 1)
                    .is_none_or(|byte| !byte.is_ascii_digit())
                && postgres_question_mark_is_operator(source, start)
            {
                return None;
            }
            (
                take_while(bytes, start + 1, u8::is_ascii_digit),
                SqlPlaceholderKind::QuestionMark,
            )
        }
        b'@' if !previous_is_identifier && bytes.get(start + 1) != Some(&b'@') => (
            named_parameter_end(bytes, start + 1)?,
            SqlPlaceholderKind::AtName,
        ),
        b':' if !previous_is_identifier && bytes.get(start + 1) != Some(&b':') => (
            named_parameter_end(bytes, start + 1)?,
            SqlPlaceholderKind::ColonName,
        ),
        b'{' if bytes.get(start + 1) == Some(&b'{') => (
            delimited_name_end(bytes, start + 2, b'}', Some(b'}'))?,
            SqlPlaceholderKind::Mustache,
        ),
        b'%' if !previous_is_identifier && bytes.get(start + 1) == Some(&b'(') => (
            delimited_name_end(bytes, start + 2, b')', Some(b's'))?,
            SqlPlaceholderKind::PythonNamed,
        ),
        _ => return None,
    };

    Some(SqlPlaceholderSpan { start, end, kind })
}

fn postgres_question_mark_is_operator(source: &str, start: usize) -> bool {
    let context = postgres_operator_context_prefix(source, start);
    let prefix = context.trim_end();
    let Some((last_index, last)) = prefix.char_indices().next_back() else {
        return false;
    };

    if matches!(last, ')' | ']' | '\'' | '"') || last.is_numeric() {
        return true;
    }
    if !(last.is_alphabetic() || matches!(last, '_' | '$')) {
        return false;
    }

    let word_start = prefix[..last_index]
        .char_indices()
        .rev()
        .find(|(_, ch)| !(ch.is_alphanumeric() || matches!(ch, '_' | '$')))
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0);
    let word = prefix[word_start..].to_ascii_uppercase();

    !matches!(
        word.as_str(),
        "ALL"
            | "AND"
            | "ANY"
            | "AS"
            | "BETWEEN"
            | "BY"
            | "CASE"
            | "DELETE"
            | "DISTINCT"
            | "DO"
            | "ELSE"
            | "EXISTS"
            | "FROM"
            | "GROUP"
            | "HAVING"
            | "ILIKE"
            | "IN"
            | "INSERT"
            | "INTO"
            | "IS"
            | "JOIN"
            | "LIKE"
            | "LIMIT"
            | "NOT"
            | "OFFSET"
            | "ON"
            | "OR"
            | "ORDER"
            | "RETURNING"
            | "SELECT"
            | "SET"
            | "SOME"
            | "THEN"
            | "UPDATE"
            | "USING"
            | "VALUE"
            | "VALUES"
            | "WHEN"
            | "WHERE"
    )
}

fn postgres_operator_context_prefix(source: &str, end: usize) -> String {
    let mut output = String::with_capacity(end);
    let mut index = 0;

    while index < end {
        if source[index..].starts_with("--") {
            let next = skip_line_comment(source, index).min(end);
            push_context_mask(source, &mut output, index, next, false);
            index = next;
            continue;
        }
        if source[index..].starts_with("/*") {
            let next = skip_block_comment(source, index).min(end);
            push_context_mask(source, &mut output, index, next, false);
            index = next;
            continue;
        }

        let Some(ch) = source[index..].chars().next() else {
            break;
        };
        if ch == '$' {
            if let Some(next) = skip_dollar_quoted_region(source, index) {
                let next = next.min(end);
                push_context_mask(source, &mut output, index, next, true);
                index = next;
                continue;
            }
        }
        if matches!(ch, '\'' | '"' | '`') {
            let next = skip_quoted_region(source, index, ch).min(end);
            push_context_mask(source, &mut output, index, next, true);
            index = next;
            continue;
        }
        if ch == '[' {
            let next = skip_bracketed_identifier(source, index).min(end);
            push_context_mask(source, &mut output, index, next, true);
            index = next;
            continue;
        }

        output.push(ch);
        index += ch.len_utf8();
    }

    output
}

fn push_context_mask(
    source: &str,
    output: &mut String,
    start: usize,
    end: usize,
    expression_marker: bool,
) {
    let mut first = expression_marker;
    for ch in source[start..end].chars() {
        if first {
            output.push('x');
            first = false;
        } else if matches!(ch, '\r' | '\n') {
            output.push(ch);
        } else {
            output.push(' ');
        }
    }
}

fn named_parameter_end(bytes: &[u8], name_start: usize) -> Option<usize> {
    let first = *bytes.get(name_start)?;
    if !is_name_start(first) {
        return None;
    }
    Some(take_while(bytes, name_start + 1, is_name_continue))
}

fn delimited_name_end(
    bytes: &[u8],
    name_start: usize,
    close: u8,
    suffix: Option<u8>,
) -> Option<usize> {
    let first = *bytes.get(name_start)?;
    if !is_name_start(first) {
        return None;
    }

    let name_end = take_while(bytes, name_start + 1, is_template_name_continue);
    if bytes.get(name_end) != Some(&close) {
        return None;
    }

    let mut end = name_end + 1;
    if let Some(suffix) = suffix {
        if bytes.get(end) != Some(&suffix) {
            return None;
        }
        end += 1;
    }
    Some(end)
}

fn take_while(bytes: &[u8], mut index: usize, predicate: fn(&u8) -> bool) -> usize {
    while bytes.get(index).is_some_and(predicate) {
        index += 1;
    }
    index
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$')
}

fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_name_continue(byte: &u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'$')
}

fn is_template_name_continue(byte: &u8) -> bool {
    is_name_continue(byte) || matches!(*byte, b'.' | b'-')
}

fn starts_hash_comment(source: &str, index: usize, dialect: SqlPlaceholderDialect) -> bool {
    dialect != SqlPlaceholderDialect::Postgres
        && source[index..].starts_with('#')
        && !source[index..].starts_with("#>")
        && !source[index..].starts_with("#-")
        && (index == 0
            || source[..index]
                .chars()
                .next_back()
                .is_none_or(char::is_whitespace))
}

fn skip_line_comment(source: &str, mut index: usize) -> usize {
    while index < source.len() {
        let Some(ch) = source[index..].chars().next() else {
            break;
        };
        index += ch.len_utf8();
        if ch == '\n' {
            break;
        }
    }
    index
}

fn skip_block_comment(source: &str, mut index: usize) -> usize {
    let mut depth = 0usize;
    while index < source.len() {
        if source[index..].starts_with("/*") {
            depth += 1;
            index += 2;
            continue;
        }
        if source[index..].starts_with("*/") {
            depth = depth.saturating_sub(1);
            index += 2;
            if depth == 0 {
                break;
            }
            continue;
        }
        let Some(ch) = source[index..].chars().next() else {
            break;
        };
        index += ch.len_utf8();
    }
    index
}

fn skip_quoted_region(source: &str, mut index: usize, quote: char) -> usize {
    index += quote.len_utf8();
    while index < source.len() {
        let Some(ch) = source[index..].chars().next() else {
            break;
        };
        index += ch.len_utf8();

        if ch == '\\' && quote == '\'' && index < source.len() {
            if let Some(escaped) = source[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        if ch == quote {
            if source[index..].starts_with(quote) {
                index += quote.len_utf8();
            } else {
                break;
            }
        }
    }
    index
}

fn skip_bracketed_identifier(source: &str, mut index: usize) -> usize {
    index += 1;
    while index < source.len() {
        let Some(ch) = source[index..].chars().next() else {
            break;
        };
        index += ch.len_utf8();
        if ch == ']' {
            if source[index..].starts_with(']') {
                index += 1;
            } else {
                break;
            }
        }
    }
    index
}

fn skip_dollar_quoted_region(source: &str, index: usize) -> Option<usize> {
    let tag = dollar_quote_tag_at(source, index)?;
    let body_start = index + tag.len();
    Some(
        source[body_start..]
            .find(tag)
            .map(|body_end| body_start + body_end + tag.len())
            .unwrap_or(source.len()),
    )
}

fn dollar_quote_tag_at(source: &str, index: usize) -> Option<&str> {
    let rest = source.get(index..)?;
    let bytes = rest.as_bytes();
    if bytes.first() != Some(&b'$') {
        return None;
    }

    let mut end = 1;
    while end < bytes.len() {
        let byte = bytes[end];
        if byte == b'$' {
            return Some(&rest[..=end]);
        }
        if end == 1 && !is_name_start(byte) {
            return None;
        }
        if end > 1 && !is_name_continue(&byte) {
            return None;
        }
        end += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_supported_placeholder_forms() {
        let sql = "SELECT $1, ?, ?1, @id, :id, {{id}}, ${id}, %(id)s";
        let spans = sql_placeholder_spans(sql);
        let values = spans
            .iter()
            .map(|span| &sql[span.start..span.end])
            .collect::<Vec<_>>();

        assert_eq!(
            values,
            ["$1", "?", "?1", "@id", ":id", "{{id}}", "${id}", "%(id)s"]
        );
        assert_eq!(normalize_sql_placeholders(sql).len(), sql.len());
    }

    #[test]
    fn postgres_distinguishes_json_existence_operators_from_question_bindings() {
        let sql = "SELECT payload ? 'key', payload ?| ARRAY['a'], payload ?& ARRAY['a'], payload /* lhs */ ? $1, payload ? ?, ? AS bind_value, 数据 ? '键' FROM events WHERE /* rhs */ id = ? AND tenant_id = ?1 AND -- bind follows comment\n ?";
        let spans = sql_placeholder_spans_for_dialect(sql, SqlPlaceholderDialect::Postgres);
        let values = spans
            .iter()
            .map(|span| &sql[span.start..span.end])
            .collect::<Vec<_>>();

        assert_eq!(values, ["$1", "?", "?", "?", "?1", "?"]);

        let normalized =
            normalize_sql_placeholders_for_dialect(sql, SqlPlaceholderDialect::Postgres);
        assert!(normalized.contains("payload ? 'key'"));
        assert!(normalized.contains("payload /* lhs */ ? p_"));
        assert!(normalized.contains("id = p"));
    }

    #[test]
    fn postgres_hash_operators_do_not_hide_following_placeholders() {
        let sql = "SELECT payload #> '{a}', payload #>> '{b}', payload #- '{c}' FROM events WHERE id = :id";
        let spans = sql_placeholder_spans_for_dialect(sql, SqlPlaceholderDialect::Postgres);
        let values = spans
            .iter()
            .map(|span| &sql[span.start..span.end])
            .collect::<Vec<_>>();

        assert_eq!(values, [":id"]);
    }

    #[test]
    fn generic_scanning_keeps_existing_question_mark_behavior() {
        let sql = "SELECT payload ? 'key', ? FROM events";
        let values = sql_placeholder_spans(sql)
            .iter()
            .map(|span| &sql[span.start..span.end])
            .collect::<Vec<_>>();

        assert_eq!(values, ["?", "?"]);
    }

    #[test]
    fn leaves_protected_regions_and_postgres_operators_untouched() {
        let sql = r#"
SELECT ':id', "{{column}}", `@column`, [${column}], payload ?| array['x'];
-- :comment
# @comment
/* {{comment}} /* ${nested} */ */
DO $body$ BEGIN RAISE NOTICE ':inside'; END $body$;
SELECT :outside;
"#;
        let spans = sql_placeholder_spans(sql);
        let values = spans
            .iter()
            .map(|span| &sql[span.start..span.end])
            .collect::<Vec<_>>();

        assert_eq!(values, [":outside"]);
    }

    #[test]
    fn normalization_preserves_utf8_bytes_and_line_breaks() {
        let sql = "SELECT '😀', {{value}}\r\nFROM {{schema}}.users WHERE id = :id";
        let normalized = normalize_sql_placeholders(sql);

        assert_eq!(normalized.len(), sql.len());
        assert_eq!(normalized.matches('\n').count(), sql.matches('\n').count());
        assert_eq!(&normalized[.."SELECT '😀', ".len()], "SELECT '😀', ");
    }
}
