use crate::parser::SqlParser;
use crate::schema::{Column, Constraint, Function, Index, Schema, Table};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Documentation, Position};

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
        if end == 1 && !(byte == b'_' || byte.is_ascii_alphabetic()) {
            return None;
        }
        if end > 1 && !(byte == b'_' || byte.is_ascii_alphanumeric()) {
            return None;
        }
        end += 1;
    }

    None
}

fn flush_compact_space(output: &mut String, pending_space: &mut bool) {
    if *pending_space
        && !output.is_empty()
        && output.chars().last().is_some_and(|ch| !ch.is_whitespace())
    {
        output.push(' ');
    }
    *pending_space = false;
}

/// Compact whitespace without changing quoted values, identifiers, comments,
/// or PostgreSQL dollar-quoted bodies. This is intentionally conservative: it
/// is a safe fallback formatter, not a SQL rewriter.
pub(crate) fn compact_sql_whitespace(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut pending_space = false;
    let mut index = 0usize;

    while index < source.len() {
        if source[index..].starts_with("--") {
            flush_compact_space(&mut output, &mut pending_space);
            let end = source[index..]
                .find('\n')
                .map(|relative| index + relative + 1)
                .unwrap_or(source.len());
            output.push_str(&source[index..end]);
            index = end;
            continue;
        }

        if source[index..].starts_with("/*") {
            flush_compact_space(&mut output, &mut pending_space);
            let end = source[index + 2..]
                .find("*/")
                .map(|relative| index + relative + 4)
                .unwrap_or(source.len());
            output.push_str(&source[index..end]);
            index = end;
            pending_space = true;
            continue;
        }

        if let Some(tag) = dollar_quote_tag_at(source, index) {
            flush_compact_space(&mut output, &mut pending_space);
            let body_start = index + tag.len();
            let end = source[body_start..]
                .find(tag)
                .map(|relative| body_start + relative + tag.len())
                .unwrap_or(source.len());
            output.push_str(&source[index..end]);
            index = end;
            continue;
        }

        let Some(ch) = source[index..].chars().next() else {
            break;
        };
        if ch.is_whitespace() {
            pending_space = true;
            index += ch.len_utf8();
            continue;
        }

        if matches!(ch, '\'' | '"' | '`' | '[') {
            flush_compact_space(&mut output, &mut pending_space);
            let start = index;
            let closing = if ch == '[' { ']' } else { ch };
            index += ch.len_utf8();
            while index < source.len() {
                let Some(quoted) = source[index..].chars().next() else {
                    break;
                };
                index += quoted.len_utf8();

                if quoted == '\\' && ch != '[' && index < source.len() {
                    if let Some(escaped) = source[index..].chars().next() {
                        index += escaped.len_utf8();
                    }
                    continue;
                }

                if quoted == closing {
                    if source[index..].starts_with(closing) {
                        index += closing.len_utf8();
                        continue;
                    }
                    break;
                }
            }
            output.push_str(&source[start..index]);
            continue;
        }

        flush_compact_space(&mut output, &mut pending_space);
        output.push(ch);
        index += ch.len_utf8();
    }

    output.trim().to_string()
}

pub(crate) fn cursor_prefix(sql: &str, position: Position) -> String {
    let lines: Vec<&str> = sql.lines().collect();
    let line_text = lines.get(position.line as usize).unwrap_or(&"");
    let text_before = &line_text[..position.character.min(line_text.len() as u32) as usize];

    let token = cursor_identifier_token(text_before);

    if token.trim_end().ends_with('.') {
        return String::new();
    }

    SqlParser::identifier_last_part(token).to_lowercase()
}

pub(crate) fn cursor_prefix_excluding_keywords(
    sql: &str,
    position: Position,
    keywords: &[&str],
) -> String {
    let prefix = cursor_prefix(sql, position);
    if keywords
        .iter()
        .any(|keyword| prefix.eq_ignore_ascii_case(keyword))
    {
        String::new()
    } else {
        prefix
    }
}

pub(crate) fn cursor_has_identifier_qualifier(sql: &str, position: Position) -> bool {
    let lines: Vec<&str> = sql.lines().collect();
    let line_text = lines.get(position.line as usize).unwrap_or(&"");
    let text_before = &line_text[..position.character.min(line_text.len() as u32) as usize];

    cursor_identifier_token(text_before).contains('.')
}

pub(crate) fn predicate_operator_expected(sql: &str, position: Position) -> bool {
    let text_before = text_before_position(sql, position);
    if text_before.is_empty() {
        return false;
    }

    if !text_before
        .chars()
        .last()
        .is_some_and(|ch| ch.is_whitespace())
    {
        return false;
    }

    let trimmed = text_before.trim_end();
    let Some(last_char) = trimmed.chars().last() else {
        return false;
    };
    if matches!(
        last_char,
        '=' | '<' | '>' | '!' | '+' | '-' | '*' | '/' | '%' | '(' | ',' | '.'
    ) {
        return false;
    }

    let last_token = trimmed
        .split(|ch: char| ch.is_whitespace() || matches!(ch, '(' | ')' | ',' | ';'))
        .rfind(|token| !token.is_empty())
        .unwrap_or("")
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`' | '[' | ']'));
    let last_token_upper = last_token.to_ascii_uppercase();

    !matches!(
        last_token_upper.as_str(),
        "WHERE"
            | "ON"
            | "HAVING"
            | "AND"
            | "OR"
            | "NOT"
            | "SET"
            | "UPDATE"
            | "WHEN"
            | "THEN"
            | "ELSE"
            | "LIKE"
            | "ILIKE"
            | "IN"
            | "BETWEEN"
            | "IS"
    )
}

pub(crate) fn latest_predicate_clause(sql: &str, position: Position) -> Option<&'static str> {
    let text_before = text_before_position(sql, position).to_ascii_uppercase();
    let statement_start = text_before.rfind(';').map(|index| index + 1).unwrap_or(0);
    let statement = &text_before[statement_start..];

    ["WHERE", "HAVING", "ON", "WHEN", "SET", "UPDATE"]
        .into_iter()
        .filter_map(|clause| {
            previous_keyword_position(statement, clause).map(|position| (position, clause))
        })
        .max_by_key(|(position, _)| *position)
        .map(|(_, clause)| clause)
}

pub(crate) fn order_direction_keywords(
    sql: &str,
    position: Position,
    supports_nulls_ordering: bool,
    continuation_keywords: &[&'static str],
) -> Vec<&'static str> {
    let text_before = text_before_position(sql, position);
    let text_upper = text_before.to_ascii_uppercase();
    let statement_start = text_upper.rfind(';').map(|index| index + 1).unwrap_or(0);
    let statement_upper = &text_upper[statement_start..];

    let Some(order_position) = statement_upper.rfind("ORDER BY") else {
        return Vec::new();
    };

    let after_order_start = statement_start + order_position + "ORDER BY".len();
    let after_order = text_before.get(after_order_start..).unwrap_or("");
    let segment = after_order.rsplit(',').next().unwrap_or(after_order);
    let trimmed = segment.trim_end();
    if trimmed.is_empty() || trimmed.ends_with('.') || trimmed.ends_with('(') {
        return Vec::new();
    }

    let has_trailing_whitespace = segment.chars().last().is_some_and(|ch| ch.is_whitespace());
    let raw_tokens = trimmed
        .split(|ch: char| ch.is_whitespace())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if raw_tokens.is_empty() {
        return Vec::new();
    }

    let complete_tokens = if has_trailing_whitespace {
        raw_tokens.as_slice()
    } else {
        &raw_tokens[..raw_tokens.len().saturating_sub(1)]
    };
    let complete_upper = complete_tokens
        .iter()
        .map(|token| normalize_order_token(token))
        .collect::<Vec<_>>();
    let last_complete = complete_upper.last().map(String::as_str);
    let previous_complete = complete_upper.iter().rev().nth(1).map(String::as_str);

    let mut keywords = match (previous_complete, last_complete) {
        (_, Some("NULLS")) if supports_nulls_ordering => vec!["FIRST", "LAST"],
        (Some("NULLS"), Some("FIRST" | "LAST")) => continuation_keywords.to_vec(),
        (_, Some("ASC" | "DESC")) => {
            let mut options = Vec::new();
            if supports_nulls_ordering {
                options.extend(["NULLS FIRST", "NULLS LAST"]);
            }
            options.extend_from_slice(continuation_keywords);
            options
        }
        _ => {
            let mut options = vec!["ASC", "DESC"];
            if supports_nulls_ordering {
                options.extend(["NULLS FIRST", "NULLS LAST"]);
            }
            options.extend_from_slice(continuation_keywords);
            options
        }
    };
    keywords.dedup();
    keywords
}

pub(crate) fn order_direction_sort_prefix(keyword: &str) -> &'static str {
    match keyword {
        "ASC" | "DESC" | "NULLS FIRST" | "NULLS LAST" | "FIRST" | "LAST" => "0",
        "," => "1",
        _ => "2",
    }
}

pub(crate) fn group_by_continuation_sort_prefix(keyword: &str) -> &'static str {
    match keyword {
        "," => "0",
        "HAVING" => "1",
        "ORDER BY" => "2",
        _ => "3",
    }
}

pub(crate) fn select_continuation_keywords(sql: &str, position: Position) -> Vec<&'static str> {
    let text_before = text_before_position(sql, position);
    let text_upper = text_before.to_ascii_uppercase();
    let statement_start = text_upper.rfind(';').map(|index| index + 1).unwrap_or(0);
    let statement_upper = &text_upper[statement_start..];

    let Some(select_position) = statement_upper.rfind("SELECT") else {
        return vec![",", "AS", "FROM"];
    };

    let after_select_start = statement_start + select_position + "SELECT".len();
    let after_select = text_before.get(after_select_start..).unwrap_or("");
    let segment = after_select.rsplit(',').next().unwrap_or(after_select);
    let select_item = segment.trim();

    if select_item == "*" || select_item.ends_with(".*") {
        vec![",", "FROM"]
    } else {
        vec![",", "AS", "FROM"]
    }
}

pub(crate) fn expression_value_allows_default(sql: &str, position: Position) -> bool {
    let text_before = text_before_position(sql, position).to_ascii_uppercase();
    let statement_start = text_before.rfind(';').map(|index| index + 1).unwrap_or(0);
    let statement = &text_before[statement_start..];

    if let Some(set_position) = insert_set_position(statement) {
        return !statement[set_position + "SET".len()..].contains("ON DUPLICATE");
    }

    if let Some(duplicate_update_position) = statement.rfind("ON DUPLICATE KEY UPDATE") {
        let after_duplicate_update = duplicate_update_position + "ON DUPLICATE KEY UPDATE".len();
        return !statement[after_duplicate_update..].contains("WHERE");
    }

    let Some(update_position) = statement.rfind("UPDATE") else {
        return false;
    };
    let Some(set_position) = statement.rfind("SET") else {
        return false;
    };
    if set_position < update_position {
        return false;
    }

    !statement[set_position + "SET".len()..].contains("WHERE")
}

pub(crate) fn predicate_continuation_keywords(
    sql: &str,
    position: Position,
    supports_returning: bool,
) -> Vec<&'static str> {
    let text_before = text_before_position(sql, position).to_ascii_uppercase();
    let statement_start = text_before.rfind(';').map(|index| index + 1).unwrap_or(0);
    let statement = &text_before[statement_start..];

    let latest_clause = ["WHERE", "HAVING", "ON", "WHEN", "SET", "UPDATE"]
        .into_iter()
        .filter_map(|clause| {
            previous_keyword_position(statement, clause).map(|position| (position, clause))
        })
        .max_by_key(|(position, _)| *position);

    match latest_clause {
        Some((clause_position, clause))
            if between_first_value_needs_and(statement, clause_position, clause) =>
        {
            vec!["AND"]
        }
        Some((set_position, "SET")) if insert_set_position(statement) == Some(set_position) => {
            vec![",", "ON DUPLICATE KEY UPDATE"]
        }
        Some((_, "SET")) if supports_returning => vec![",", "WHERE", "RETURNING"],
        Some((_, "SET")) => vec![",", "WHERE"],
        Some((_, "UPDATE")) => vec![","],
        Some((_, "HAVING")) => vec!["AND", "OR", "ORDER BY", "LIMIT"],
        Some((_, "ON")) => vec!["AND", "OR", "WHERE", "GROUP BY", "ORDER BY", "LIMIT"],
        Some((_, "WHEN")) => vec!["AND", "OR", "THEN"],
        Some((_, "WHERE")) => vec!["AND", "OR", "GROUP BY", "HAVING", "ORDER BY", "LIMIT"],
        _ => vec!["AND", "OR"],
    }
}

fn insert_set_position(statement_upper: &str) -> Option<usize> {
    let insert_position = previous_keyword_position(statement_upper, "INSERT INTO")?;
    let set_position = previous_keyword_position(statement_upper, "SET")?;
    if set_position < insert_position {
        return None;
    }

    let after_insert = insert_position + "INSERT INTO".len();
    let between_insert_and_set = statement_upper.get(after_insert..set_position)?;
    if ["VALUES", "VALUE", "SELECT"]
        .into_iter()
        .any(|keyword| previous_keyword_position(between_insert_and_set, keyword).is_some())
    {
        return None;
    }

    Some(set_position)
}

pub(crate) fn case_continuation_keywords(sql: &str, position: Position) -> Vec<&'static str> {
    let text_before = text_before_position(sql, position);
    let raw_text_upper = text_before.to_ascii_uppercase();
    let searchable_text_upper = SqlParser::mask_sql_noise(&raw_text_upper);
    let statement_start = searchable_text_upper
        .rfind(';')
        .map(|index| index + 1)
        .unwrap_or(0);
    let searchable_statement = &searchable_text_upper[statement_start..];

    let Some(case_position) = previous_keyword_position(searchable_statement, "CASE") else {
        return vec!["WHEN", "ELSE", "END"];
    };
    let after_case = case_position + "CASE".len();
    if previous_keyword_position(&searchable_statement[after_case..], "END").is_some() {
        return Vec::new();
    }

    let latest_marker = ["THEN", "ELSE"]
        .into_iter()
        .filter_map(|marker| {
            previous_keyword_position(&searchable_statement[after_case..], marker)
                .map(|position| (position, marker))
        })
        .max_by_key(|(position, _)| *position)
        .map(|(_, marker)| marker);

    match latest_marker {
        Some("ELSE") => vec!["END"],
        Some("THEN") => vec!["WHEN", "ELSE", "END"],
        _ => vec!["WHEN", "ELSE", "END"],
    }
}

fn between_first_value_needs_and(
    statement_upper: &str,
    clause_position: usize,
    clause: &str,
) -> bool {
    if !matches!(clause, "WHERE" | "HAVING" | "ON" | "WHEN") {
        return false;
    }

    let after_clause = clause_position + clause.len();
    let Some(segment) = statement_upper.get(after_clause..) else {
        return false;
    };
    let Some(between_position) = previous_keyword_position(segment, "BETWEEN") else {
        return false;
    };
    let after_between = between_position + "BETWEEN".len();

    previous_keyword_position(&segment[after_between..], "AND").is_none()
}

fn previous_keyword_position(source_upper: &str, keyword: &str) -> Option<usize> {
    let mut search_pos = 0;
    let mut previous = None;

    while let Some(relative_position) = source_upper[search_pos..].find(keyword) {
        let position = search_pos + relative_position;
        if is_keyword_at(source_upper, position, keyword) {
            previous = Some(position);
        }
        search_pos = position + keyword.len();
    }

    previous
}

fn is_keyword_at(source_upper: &str, start: usize, keyword: &str) -> bool {
    let end = start + keyword.len();
    if end > source_upper.len() {
        return false;
    }

    let before_is_boundary = if start == 0 {
        true
    } else {
        source_upper[..start]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_identifier_char(ch))
    };
    let after_is_boundary = if end == source_upper.len() {
        true
    } else {
        source_upper[end..]
            .chars()
            .next()
            .is_none_or(|ch| !is_identifier_char(ch))
    };

    before_is_boundary && after_is_boundary
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$')
}

fn normalize_order_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | '[' | ']' | '(' | ')' | ';'))
        .to_ascii_uppercase()
}

pub(crate) fn cursor_identifier_token(text_before: &str) -> &str {
    let mut token_start = 0;
    let mut quote_end: Option<char> = None;
    let mut chars = text_before.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        if let Some(end_quote) = quote_end {
            if ch == end_quote {
                if chars.peek().is_some_and(|(_, next)| *next == end_quote) {
                    chars.next();
                    continue;
                }
                quote_end = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' | '`' => quote_end = Some(ch),
            '[' => quote_end = Some(']'),
            _ if ch.is_whitespace() || ch == ',' || ch == '(' => {
                token_start = index + ch.len_utf8();
            }
            _ => {}
        }
    }

    &text_before[token_start..]
}

pub(crate) fn text_before_position(sql: &str, position: Position) -> &str {
    let lines: Vec<&str> = sql.lines().collect();
    let mut offset = 0usize;

    for (line_index, line_text) in lines.iter().enumerate() {
        if line_index < position.line as usize {
            offset += line_text.len() + 1;
        } else if line_index == position.line as usize {
            offset += position.character.min(line_text.len() as u32) as usize;
            break;
        }
    }

    sql.get(..offset.min(sql.len())).unwrap_or(sql)
}

pub(crate) fn completion_sort_text(sort_prefix: &str, label: &str) -> String {
    format!("{}:{}", sort_prefix, label.to_ascii_lowercase())
}

pub(crate) fn set_completion_sort_text(item: &mut CompletionItem, sort_prefix: &str, label: &str) {
    item.sort_text = Some(completion_sort_text(sort_prefix, label));
}

pub(crate) fn create_keyword_item(dialect_label: &str, keyword: &str) -> CompletionItem {
    CompletionItem {
        label: keyword.to_string(),
        kind: Some(CompletionItemKind::KEYWORD),
        detail: Some(format!("{dialect_label} keyword: {keyword}")),
        documentation: None,
        deprecated: None,
        preselect: None,
        sort_text: Some(completion_sort_text("0", keyword)),
        filter_text: None,
        insert_text: Some(keyword.to_string()),
        insert_text_format: None,
        insert_text_mode: None,
        text_edit: None,
        additional_text_edits: None,
        commit_characters: None,
        command: None,
        data: None,
        tags: None,
        label_details: None,
    }
}

pub(crate) fn create_operator_item(operator: &str, sort_prefix: &str) -> CompletionItem {
    CompletionItem {
        label: operator.to_string(),
        kind: Some(CompletionItemKind::OPERATOR),
        detail: Some(format!("Operator: {}", operator)),
        documentation: None,
        deprecated: None,
        preselect: None,
        sort_text: Some(completion_sort_text(sort_prefix, operator)),
        filter_text: None,
        insert_text: Some(operator.to_string()),
        insert_text_format: None,
        insert_text_mode: None,
        text_edit: None,
        additional_text_edits: None,
        commit_characters: None,
        command: None,
        data: None,
        tags: None,
        label_details: None,
    }
}

pub(crate) fn add_operator_items(
    items: &mut Vec<CompletionItem>,
    operators: &[&str],
    prefix: &str,
    sort_prefix: &str,
) {
    for operator in operators {
        if !prefix.is_empty() && !operator.to_lowercase().starts_with(prefix) {
            continue;
        }
        items.push(create_operator_item(operator, sort_prefix));
    }
}

pub(crate) fn create_table_item(
    table: &Table,
    schema: &Schema,
    qualify_with_database: bool,
) -> CompletionItem {
    let label = if qualify_with_database && !schema.database.is_empty() {
        format!("{}.{}", schema.database, table.name)
    } else {
        table.name.clone()
    };
    let detail = if qualify_with_database && !schema.database.is_empty() {
        format!(
            "{}: {}.{}",
            table.object_kind(),
            schema.database,
            table.name
        )
    } else {
        format!("{}: {}", table.object_kind(), table.name)
    };

    CompletionItem {
        label: label.clone(),
        kind: Some(CompletionItemKind::CLASS),
        detail: Some(detail),
        documentation: table.documentation().map(Documentation::String),
        deprecated: None,
        preselect: None,
        sort_text: Some(completion_sort_text("1", &table.name)),
        filter_text: Some(table.name.clone()),
        insert_text: Some(label),
        insert_text_format: None,
        insert_text_mode: None,
        text_edit: None,
        additional_text_edits: None,
        commit_characters: None,
        command: None,
        data: None,
        tags: None,
        label_details: None,
    }
}

pub(crate) fn create_column_item(column: &Column, table_name: Option<&str>) -> CompletionItem {
    let label = if let Some(table) = table_name {
        format!("{}.{}", table, column.name)
    } else {
        column.name.clone()
    };

    let detail = if let Some(table) = table_name {
        format!("Column: {}.{} ({})", table, column.name, column.data_type)
    } else {
        format!("Column: {} ({})", column.name, column.data_type)
    };

    CompletionItem {
        label: label.clone(),
        kind: Some(CompletionItemKind::FIELD),
        detail: Some(detail),
        documentation: column.documentation().map(Documentation::String),
        deprecated: None,
        preselect: None,
        sort_text: Some(completion_sort_text("2", &column.name)),
        filter_text: Some(column.name.clone()),
        insert_text: Some(label),
        insert_text_format: None,
        insert_text_mode: None,
        text_edit: None,
        additional_text_edits: None,
        commit_characters: None,
        command: None,
        data: None,
        tags: None,
        label_details: None,
    }
}

pub(crate) fn create_function_item(
    function: &Function,
    schema: &Schema,
    qualify_with_database: bool,
) -> CompletionItem {
    let label = if qualify_with_database && !schema.database.is_empty() {
        format!("{}.{}", schema.database, function.name)
    } else {
        function.name.clone()
    };
    let signature = if qualify_with_database && !schema.database.is_empty() {
        format!("{}.{}", schema.database, function.signature())
    } else {
        function.signature()
    };
    let detail = if function.routine_kind() == "Procedure" {
        format!("Procedure: {signature}")
    } else {
        format!("Function: {signature} -> {}", function.return_type)
    };

    CompletionItem {
        label: label.clone(),
        kind: Some(CompletionItemKind::FUNCTION),
        detail: Some(detail),
        documentation: Some(Documentation::String(function.documentation())),
        deprecated: None,
        preselect: None,
        sort_text: Some(completion_sort_text("1", &function.name)),
        filter_text: Some(function.name.clone()),
        insert_text: Some(format!("{label}()")),
        insert_text_format: None,
        insert_text_mode: None,
        text_edit: None,
        additional_text_edits: None,
        commit_characters: None,
        command: None,
        data: None,
        tags: None,
        label_details: None,
    }
}

pub(crate) fn create_constraint_item(constraint: &Constraint) -> CompletionItem {
    CompletionItem {
        label: constraint.name.clone(),
        kind: Some(CompletionItemKind::REFERENCE),
        detail: Some(format!("Constraint: {}", constraint.constraint_type)),
        documentation: constraint
            .definition
            .as_ref()
            .map(|definition| Documentation::String(definition.clone())),
        deprecated: None,
        preselect: None,
        sort_text: Some(completion_sort_text("1", &constraint.name)),
        filter_text: Some(constraint.name.clone()),
        insert_text: Some(constraint.name.clone()),
        insert_text_format: None,
        insert_text_mode: None,
        text_edit: None,
        additional_text_edits: None,
        commit_characters: None,
        command: None,
        data: None,
        tags: None,
        label_details: None,
    }
}

pub(crate) fn create_index_item(
    index: &Index,
    schema: &Schema,
    qualify_with_database: bool,
) -> CompletionItem {
    let label = if qualify_with_database && !schema.database.is_empty() {
        format!("{}.{}", schema.database, index.name)
    } else {
        index.name.clone()
    };
    let index_kind = if index.is_primary {
        "Primary index"
    } else if index.is_unique {
        "Unique index"
    } else {
        "Index"
    };

    CompletionItem {
        label: label.clone(),
        kind: Some(CompletionItemKind::REFERENCE),
        detail: Some(format!("{}: {}", index_kind, index.name)),
        documentation: index
            .definition
            .as_ref()
            .map(|definition| Documentation::String(definition.clone())),
        deprecated: None,
        preselect: None,
        sort_text: Some(completion_sort_text("1", &index.name)),
        filter_text: Some(index.name.clone()),
        insert_text: Some(label),
        insert_text_format: None,
        insert_text_mode: None,
        text_edit: None,
        additional_text_edits: None,
        commit_characters: None,
        command: None,
        data: None,
        tags: None,
        label_details: None,
    }
}

pub(crate) fn add_schema_tables(
    items: &mut Vec<CompletionItem>,
    schema: &Schema,
    prefix: &str,
    qualify_with_database: bool,
) {
    for table in &schema.tables {
        if !prefix.is_empty() && !table.name.to_lowercase().starts_with(prefix) {
            continue;
        }
        items.push(create_table_item(table, schema, qualify_with_database));
    }
}

pub(crate) fn add_schema_functions(
    items: &mut Vec<CompletionItem>,
    schema: &Schema,
    prefix: &str,
    sort_prefix: &str,
    qualify_with_database: bool,
) {
    for function in &schema.functions {
        if !prefix.is_empty() && !function.name.to_lowercase().starts_with(prefix) {
            continue;
        }
        let mut item = create_function_item(function, schema, qualify_with_database);
        set_completion_sort_text(&mut item, sort_prefix, &function.name);
        items.push(item);
    }
}

pub(crate) fn add_schema_columns(
    items: &mut Vec<CompletionItem>,
    schema: &Schema,
    referenced_tables: &[String],
    use_table_prefix: bool,
    prefix: &str,
    sort_prefix: &str,
) {
    let candidate_tables = schema
        .tables
        .iter()
        .filter(|table| table_is_referenced(schema, table, referenced_tables))
        .collect::<Vec<_>>();
    let mut column_sources = HashMap::<String, usize>::new();
    for table in &candidate_tables {
        let mut table_columns = HashSet::new();
        for column in &table.columns {
            let normalized = column.name.to_ascii_lowercase();
            if table_columns.insert(normalized.clone()) {
                *column_sources.entry(normalized).or_default() += 1;
            }
        }
    }
    let mut seen_sources = HashSet::new();

    for (table_order, table) in candidate_tables.into_iter().enumerate() {
        let relation_order = referenced_tables
            .iter()
            .position(|reference| table_matches(schema, table, reference))
            .unwrap_or(table_order);

        for column in &table.columns {
            let Some(match_rank) = identifier_match_rank(&column.name, prefix) else {
                continue;
            };
            let source_key = format!(
                "{}\u{0}{}\u{0}{}",
                schema.database.to_ascii_lowercase(),
                table.name.to_ascii_lowercase(),
                column.name.to_ascii_lowercase()
            );
            if !seen_sources.insert(source_key) {
                continue;
            }

            let is_ambiguous = column_sources
                .get(&column.name.to_ascii_lowercase())
                .copied()
                .unwrap_or_default()
                > 1;
            let table_name = if use_table_prefix || (referenced_tables.is_empty() && is_ambiguous) {
                Some(table.name.as_str())
            } else {
                None
            };
            let mut item = create_column_item(column, table_name);
            item.detail = Some(format!(
                "Column: {}.{}.{} ({})",
                schema.database, table.name, column.name, column.data_type
            ));
            item.data = Some(json!({
                "oxide": {
                    "kind": "column",
                    "schema": schema.database,
                    "table": table.name,
                    "column": column.name,
                }
            }));
            item.sort_text = Some(format!(
                "{}:{:02}:{:04}:{}:{}",
                sort_prefix,
                match_rank,
                relation_order,
                column.name.to_ascii_lowercase(),
                table.name.to_ascii_lowercase()
            ));
            items.push(item);
        }
    }
}

fn identifier_match_rank(candidate: &str, prefix: &str) -> Option<u8> {
    if prefix.is_empty() {
        return Some(0);
    }

    let candidate_lower = candidate.to_ascii_lowercase();
    let prefix_lower = prefix.to_ascii_lowercase();
    if candidate_lower == prefix_lower {
        return Some(0);
    }
    if candidate_lower.starts_with(&prefix_lower) {
        return Some(1);
    }

    let abbreviation = identifier_abbreviation(candidate);
    if abbreviation.starts_with(&prefix_lower) {
        return Some(2);
    }
    is_subsequence(&prefix_lower, &candidate_lower).then_some(3)
}

fn identifier_abbreviation(candidate: &str) -> String {
    let mut abbreviation = String::new();
    let mut previous_is_separator = true;
    let mut previous_is_lowercase = false;
    for ch in candidate.chars() {
        if !ch.is_ascii_alphanumeric() {
            previous_is_separator = true;
            previous_is_lowercase = false;
            continue;
        }
        if previous_is_separator || (previous_is_lowercase && ch.is_ascii_uppercase()) {
            abbreviation.push(ch.to_ascii_lowercase());
        }
        previous_is_separator = false;
        previous_is_lowercase = ch.is_ascii_lowercase();
    }
    abbreviation
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut needle_chars = needle.chars();
    let mut next = needle_chars.next();
    for ch in haystack.chars() {
        if next == Some(ch) {
            next = needle_chars.next();
            if next.is_none() {
                return true;
            }
        }
    }
    next.is_none()
}

pub(crate) fn apply_column_aliases(
    items: &mut Vec<CompletionItem>,
    schema: &Schema,
    aliases: &HashMap<String, String>,
) {
    if aliases.is_empty() {
        return;
    }

    let mut expanded = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        let Some(source) = completion_column_source(&item) else {
            expanded.push(item);
            continue;
        };
        let matching_aliases = aliases
            .iter()
            .filter(|(_, reference)| {
                SqlParser::table_name_matches(reference, &source.schema, &source.table)
                    && source.schema.eq_ignore_ascii_case(&schema.database)
            })
            .map(|(alias, _)| alias.clone())
            .collect::<Vec<_>>();
        let mut matching_aliases = matching_aliases;
        matching_aliases.sort_by_key(|alias| alias.to_ascii_lowercase());

        let is_qualified = item.label != source.column;
        if matching_aliases.is_empty() || (!is_qualified && matching_aliases.len() == 1) {
            expanded.push(item);
            continue;
        }

        for alias in matching_aliases {
            let mut aliased = item.clone();
            let label = format!("{}.{}", alias, source.column);
            aliased.label = label.clone();
            aliased.insert_text = Some(label);
            aliased.sort_text = aliased
                .sort_text
                .as_ref()
                .map(|sort_text| format!("{sort_text}:{}", alias.to_ascii_lowercase()));
            expanded.push(aliased);
        }
    }
    *items = expanded;
}

struct CompletionColumnSource {
    schema: String,
    table: String,
    column: String,
}

fn completion_column_source(item: &CompletionItem) -> Option<CompletionColumnSource> {
    let source = item.data.as_ref()?.get("oxide")?;
    if source.get("kind")?.as_str()? != "column" {
        return None;
    }
    Some(CompletionColumnSource {
        schema: source.get("schema")?.as_str()?.to_string(),
        table: source.get("table")?.as_str()?.to_string(),
        column: source.get("column")?.as_str()?.to_string(),
    })
}

pub(crate) fn add_schema_using_columns(
    items: &mut Vec<CompletionItem>,
    schema: &Schema,
    referenced_tables: &[String],
    prefix: &str,
    sort_prefix: &str,
) {
    let referenced_schema_tables = referenced_tables
        .iter()
        .filter_map(|reference| find_table_by_reference(schema, reference))
        .collect::<Vec<_>>();

    let Some((right_table, left_tables)) = referenced_schema_tables.split_last() else {
        return;
    };

    if left_tables.is_empty() {
        add_schema_columns(items, schema, referenced_tables, false, prefix, sort_prefix);
        return;
    }

    let left_column_names = left_tables
        .iter()
        .flat_map(|table| table.columns.iter())
        .map(|column| column.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();

    let mut seen = HashSet::new();
    for column in &right_table.columns {
        let column_key = column.name.to_ascii_lowercase();
        if !left_column_names.contains(&column_key) || !seen.insert(column_key) {
            continue;
        }
        if !prefix.is_empty() && !column.name.to_lowercase().starts_with(prefix) {
            continue;
        }

        let mut item = create_column_item(column, None);
        set_completion_sort_text(&mut item, sort_prefix, &column.name);
        items.push(item);
    }
}

pub(crate) fn add_reference_table_columns(
    items: &mut Vec<CompletionItem>,
    schema: &Schema,
    table_reference: &str,
    prefix: &str,
    sort_prefix: &str,
) {
    let Some(table) = find_table_by_reference(schema, table_reference) else {
        return;
    };

    for column in &table.columns {
        if !prefix.is_empty() && !column.name.to_lowercase().starts_with(prefix) {
            continue;
        }

        let mut item = create_column_item(column, None);
        set_completion_sort_text(&mut item, sort_prefix, &column.name);
        items.push(item);
    }
}

pub(crate) fn add_schema_constraints(
    items: &mut Vec<CompletionItem>,
    schema: &Schema,
    referenced_tables: &[String],
    prefix: &str,
    sort_prefix: &str,
) {
    for table in &schema.tables {
        if !table_is_referenced(schema, table, referenced_tables) {
            continue;
        }

        for constraint in &table.constraints {
            if !prefix.is_empty() && !constraint.name.to_lowercase().starts_with(prefix) {
                continue;
            }

            let mut item = create_constraint_item(constraint);
            set_completion_sort_text(&mut item, sort_prefix, &constraint.name);
            items.push(item);
        }
    }
}

pub(crate) fn add_schema_conflict_constraints(
    items: &mut Vec<CompletionItem>,
    schema: &Schema,
    referenced_tables: &[String],
    prefix: &str,
    sort_prefix: &str,
) {
    for table in &schema.tables {
        if !table_is_referenced(schema, table, referenced_tables) {
            continue;
        }

        for constraint in &table.constraints {
            let constraint_type = constraint.constraint_type.to_ascii_uppercase();
            if !constraint_type.contains("PRIMARY")
                && !constraint_type.contains("UNIQUE")
                && !constraint_type.contains("EXCLUSION")
            {
                continue;
            }
            if !prefix.is_empty() && !constraint.name.to_lowercase().starts_with(prefix) {
                continue;
            }

            let mut item = create_constraint_item(constraint);
            set_completion_sort_text(&mut item, sort_prefix, &constraint.name);
            items.push(item);
        }
    }
}

pub(crate) fn add_schema_indexes(
    items: &mut Vec<CompletionItem>,
    schema: &Schema,
    referenced_tables: &[String],
    prefix: &str,
    sort_prefix: &str,
    qualify_with_database: bool,
) {
    let mut seen = HashSet::new();
    for table in &schema.tables {
        if !table_is_referenced(schema, table, referenced_tables) {
            continue;
        }

        for index in &table.indexes {
            if !prefix.is_empty() && !index.name.to_lowercase().starts_with(prefix) {
                continue;
            }
            if !seen.insert(index.name.to_ascii_lowercase()) {
                continue;
            }

            let mut item = create_index_item(index, schema, qualify_with_database);
            set_completion_sort_text(&mut item, sort_prefix, &index.name);
            items.push(item);
        }
    }
}

pub(crate) fn table_matches(schema: &Schema, table: &Table, reference: &str) -> bool {
    SqlParser::table_name_matches(reference, &schema.database, &table.name)
}

pub(crate) fn table_is_referenced(schema: &Schema, table: &Table, references: &[String]) -> bool {
    references.is_empty()
        || references
            .iter()
            .any(|reference| table_matches(schema, table, reference))
}

pub(crate) fn find_table_by_reference<'a>(
    schema: &'a Schema,
    reference: &str,
) -> Option<&'a Table> {
    schema
        .tables
        .iter()
        .find(|table| table_matches(schema, table, reference))
}

pub(crate) fn table_column_reference_at_position(
    parser: &SqlParser,
    tree: &tree_sitter::Tree,
    sql: &str,
    position: Position,
) -> Option<String> {
    SqlParser::column_qualifier_before_position(sql, position).or_else(|| {
        let node = parser.get_node_at_position(tree, position)?;
        parser.get_table_name_for_column(node, sql)
    })
}

#[cfg(test)]
pub(crate) fn referenced_table_names(
    parser: &SqlParser,
    tree: &tree_sitter::Tree,
    sql: &str,
) -> Vec<String> {
    let referenced_tables = parser.extract_referenced_tables(tree, sql);
    let aliases = parser.extract_aliases(tree, sql);
    let mut seen = HashSet::new();
    referenced_tables
        .iter()
        .map(|table| aliases.get(table).unwrap_or(table).clone())
        .filter(|table| seen.insert(table.to_ascii_lowercase()))
        .collect()
}

pub(crate) fn referenced_table_names_at_position(
    parser: &SqlParser,
    tree: &tree_sitter::Tree,
    sql: &str,
    position: Position,
) -> Vec<String> {
    let referenced_tables = parser.extract_referenced_tables_at_position(tree, sql, position);
    let aliases = parser.extract_aliases_at_position(tree, sql, position);
    let mut seen = HashSet::new();
    referenced_tables
        .iter()
        .map(|table| aliases.get(table).unwrap_or(table).clone())
        .filter(|table| seen.insert(table.to_ascii_lowercase()))
        .collect()
}

pub(crate) fn function_name_from_reference(reference: &str) -> String {
    let before_args = reference.split('(').next().unwrap_or(reference);
    SqlParser::identifier_last_part(before_args)
}

pub(crate) fn find_function_by_reference<'a>(
    schema: &'a Schema,
    reference: &str,
) -> Option<&'a Function> {
    let function_name = function_name_from_reference(reference);
    schema.functions.iter().find(|function| {
        function.name == function_name || function.name.eq_ignore_ascii_case(&function_name)
    })
}

pub(crate) fn is_function_reference(node: tree_sitter::Node, sql: &str) -> bool {
    let node_kind = node.kind();
    if node_kind == "function_name" || node_kind.contains("function") {
        return true;
    }

    let end_byte = node.end_byte().min(sql.len());
    sql[end_byte..].trim_start().starts_with('(')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Column, FunctionParameter, SchemaId};

    fn test_schema() -> Schema {
        Schema {
            id: SchemaId::new(),
            database: "app".to_string(),
            tables: vec![
                Table {
                    name: "users".to_string(),
                    columns: vec![
                        Column {
                            name: "id".to_string(),
                            data_type: "integer".to_string(),
                            nullable: false,
                            ..Default::default()
                        },
                        Column {
                            name: "name".to_string(),
                            data_type: "text".to_string(),
                            nullable: true,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                Table {
                    name: "orders".to_string(),
                    columns: vec![
                        Column {
                            name: "id".to_string(),
                            data_type: "integer".to_string(),
                            nullable: false,
                            ..Default::default()
                        },
                        Column {
                            name: "total".to_string(),
                            data_type: "numeric".to_string(),
                            nullable: false,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
            ],
            functions: vec![Function {
                name: "calculate_score".to_string(),
                routine_type: Some("function".to_string()),
                parameters: vec![FunctionParameter {
                    name: "user_id".to_string(),
                    data_type: "integer".to_string(),
                    optional: false,
                }],
                return_type: "integer".to_string(),
                description: None,
            }],
            source_uri: None,
        }
    }

    #[test]
    fn table_completion_respects_qualification_mode() {
        let schema = test_schema();
        let unqualified = create_table_item(&schema.tables[0], &schema, false);
        let qualified = create_table_item(&schema.tables[0], &schema, true);

        assert_eq!(unqualified.label, "users");
        assert_eq!(unqualified.insert_text.as_deref(), Some("users"));
        assert_eq!(unqualified.filter_text.as_deref(), Some("users"));
        assert_eq!(qualified.label, "app.users");
        assert_eq!(qualified.insert_text.as_deref(), Some("app.users"));
        assert_eq!(qualified.filter_text.as_deref(), Some("users"));
    }

    #[test]
    fn schema_columns_filter_referenced_tables_and_prefixes() {
        let schema = test_schema();
        let mut items = Vec::new();
        add_schema_columns(
            &mut items,
            &schema,
            &["app.users".to_string()],
            false,
            "na",
            "0",
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "name");
        assert_eq!(items[0].filter_text.as_deref(), Some("name"));

        let mut prefixed_items = Vec::new();
        add_schema_columns(
            &mut prefixed_items,
            &schema,
            &["users".to_string(), "orders".to_string()],
            true,
            "",
            "0",
        );
        let labels = prefixed_items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            labels,
            vec!["users.id", "users.name", "orders.id", "orders.total"]
        );
        assert_eq!(
            prefixed_items
                .iter()
                .map(|item| item.insert_text.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("users.id"),
                Some("users.name"),
                Some("orders.id"),
                Some("orders.total")
            ]
        );
    }

    #[test]
    fn schema_columns_disambiguate_duplicate_names_before_from() {
        let schema = test_schema();
        let mut items = Vec::new();
        add_schema_columns(&mut items, &schema, &[], false, "id", "0");

        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            vec!["users.id", "orders.id"]
        );
        assert!(items
            .iter()
            .all(|item| item.insert_text.as_deref() == Some(item.label.as_str())));
        assert_ne!(items[0].sort_text, items[1].sort_text);
        assert!(items.iter().all(|item| item.data.is_some()));
    }

    #[test]
    fn schema_columns_rank_abbreviation_and_fuzzy_matches_after_prefixes() {
        assert_eq!(identifier_match_rank("user_id", "ui"), Some(2));
        assert_eq!(identifier_match_rank("created_at", "cat"), Some(3));
        assert_eq!(identifier_match_rank("account_id", "zzz"), None);
    }

    #[test]
    fn aliased_join_columns_use_aliases_for_label_and_insertion() {
        let schema = test_schema();
        let mut items = Vec::new();
        add_schema_columns(
            &mut items,
            &schema,
            &["users".to_string(), "orders".to_string()],
            true,
            "id",
            "0",
        );
        let aliases = HashMap::from([
            ("u".to_string(), "users".to_string()),
            ("o".to_string(), "orders".to_string()),
        ]);
        apply_column_aliases(&mut items, &schema, &aliases);

        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            vec!["u.id", "o.id"]
        );
        assert_eq!(
            items
                .iter()
                .map(|item| item.insert_text.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("u.id"), Some("o.id")]
        );
    }

    #[test]
    fn functions_match_schema_qualified_references() {
        let schema = test_schema();
        let function = find_function_by_reference(&schema, "app.calculate_score(")
            .expect("schema-qualified function reference should match");

        assert_eq!(function.name, "calculate_score");
        let item = create_function_item(function, &schema, true);
        assert_eq!(item.label, "app.calculate_score");
        assert_eq!(item.filter_text.as_deref(), Some("calculate_score"));
    }

    #[test]
    fn referenced_table_names_deduplicate_aliases_preserving_order() {
        let sql = "SELECT * FROM app.users u JOIN app.orders o ON o.user_id = u.id JOIN app.users managers ON managers.id = u.manager_id";
        let mut parser = SqlParser::new();
        let result = parser.parse(sql);
        let tree = result.tree.as_ref().expect("SQL should parse");

        assert_eq!(
            referenced_table_names(&parser, tree, sql),
            vec!["app.users".to_string(), "app.orders".to_string()]
        );
    }

    #[test]
    fn compact_formatting_preserves_literals_comments_and_dollar_quotes() {
        let sql = "  SELECT   'a   b',  \"spaced  name\"  -- keep   comment\n  FROM   users  WHERE body = $tag$line  one\nline   two$tag$  /* block   comment */  ";

        assert_eq!(
            compact_sql_whitespace(sql),
            "SELECT 'a   b', \"spaced  name\" -- keep   comment\nFROM users WHERE body = $tag$line  one\nline   two$tag$ /* block   comment */"
        );
    }

    #[test]
    fn compact_formatting_preserves_escaped_and_bracketed_content() {
        assert_eq!(
            compact_sql_whitespace(
                "SELECT  'it''s  spaced',  `my  column`,  [also  spaced]  FROM  t"
            ),
            "SELECT 'it''s  spaced', `my  column`, [also  spaced] FROM t"
        );
    }
}
