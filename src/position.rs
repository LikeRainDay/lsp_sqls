use tower_lsp::lsp_types::{Diagnostic, Position};

/// Convert an LSP UTF-16 position to the byte-based column expected by tree-sitter.
pub fn lsp_position_to_byte_position(source: &str, position: Position) -> Position {
    let mut last_line = "";
    let mut last_line_index = 0u32;

    for (line_index, line) in source.split('\n').enumerate() {
        last_line = line;
        last_line_index = line_index as u32;
        if line_index == position.line as usize {
            return Position {
                line: position.line,
                character: utf16_column_to_byte_offset(line, position.character) as u32,
            };
        }
    }

    Position {
        line: last_line_index,
        character: last_line.len() as u32,
    }
}

pub fn utf16_column_to_byte_offset(line: &str, character: u32) -> usize {
    let target_units = character as usize;
    let mut current_units = 0usize;

    for (byte_index, ch) in line.char_indices() {
        if current_units >= target_units {
            return byte_index;
        }

        let next_units = current_units + ch.len_utf16();
        if next_units > target_units {
            return byte_index;
        }

        current_units = next_units;
    }

    line.len()
}

pub fn byte_position_at_end(source: &str) -> Position {
    let mut line = 0;
    let mut character = 0;

    for ch in source.chars() {
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf8() as u32;
        }
    }

    Position { line, character }
}

pub fn diagnostic_reaches_position(diagnostic: &Diagnostic, position: Position) -> bool {
    diagnostic.range.end.line > position.line
        || (diagnostic.range.end.line == position.line
            && diagnostic.range.end.character >= position.character.saturating_sub(1))
        || (diagnostic.range.start.line == position.line
            && diagnostic.range.start.character >= position.character.saturating_sub(1))
}

pub fn cursor_token_prefix(
    source: &str,
    position: Position,
    is_token_char: impl Fn(char) -> bool,
) -> String {
    let position = lsp_position_to_byte_position(source, position);
    let line = source.split('\n').nth(position.line as usize).unwrap_or("");
    let byte_index = position.character.min(line.len() as u32) as usize;
    let bytes = line.as_bytes();
    let mut start = byte_index.min(bytes.len());

    while start > 0 && is_token_char(bytes[start - 1] as char) {
        start -= 1;
    }

    line[start..byte_index.min(bytes.len())]
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase()
}
