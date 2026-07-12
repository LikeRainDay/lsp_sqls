use tower_lsp::lsp_types::{Diagnostic, Position};

/// Convert a byte-based position (as used by tree-sitter) to an LSP UTF-16
/// position. The byte column is interpreted against the original source line.
pub fn byte_position_to_lsp_position(source: &str, position: Position) -> Position {
    let line = source.split('\n').nth(position.line as usize).unwrap_or("");

    Position {
        line: position.line,
        character: byte_column_to_utf16_column(line, position.character as usize),
    }
}

pub fn byte_column_to_utf16_column(line: &str, byte_column: usize) -> u32 {
    let mut byte_column = byte_column.min(line.len());
    while !line.is_char_boundary(byte_column) {
        byte_column -= 1;
    }

    line[..byte_column].encode_utf16().count() as u32
}

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

pub fn lsp_position_at_end(source: &str) -> Position {
    byte_position_to_lsp_position(source, byte_position_at_end(source))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_byte_columns_to_utf16_on_the_requested_line() {
        let source = "SELECT 1;\nSELECT '😀中文', value";
        let second_line = source.lines().nth(1).expect("second line");
        let value_byte_column = second_line.find("value").expect("value column");

        assert_eq!(
            byte_position_to_lsp_position(
                source,
                Position {
                    line: 1,
                    character: value_byte_column as u32,
                },
            ),
            Position {
                line: 1,
                character: second_line[..value_byte_column].encode_utf16().count() as u32,
            }
        );
        assert_ne!(
            value_byte_column as u32,
            second_line[..value_byte_column].encode_utf16().count() as u32,
            "the fixture must distinguish UTF-8 byte columns from UTF-16 columns"
        );
    }

    #[test]
    fn converts_eof_to_utf16_after_multiline_non_ascii_text() {
        let source = "SELECT '中文';\nSELECT '😀'";

        assert_eq!(
            lsp_position_at_end(source),
            Position {
                line: 1,
                character: "SELECT '😀'".encode_utf16().count() as u32,
            }
        );
    }
}
