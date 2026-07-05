use tower_lsp::lsp_types::Position;

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
