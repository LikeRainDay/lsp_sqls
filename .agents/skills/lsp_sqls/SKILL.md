```markdown
# lsp_sqls Development Patterns

> Auto-generated skill from repository analysis

## Overview
This skill teaches you the core development patterns and conventions used in the `lsp_sqls` Rust codebase. You'll learn about file naming, import/export styles, commit message tendencies, and testing patterns. This guide is designed to help you quickly understand and contribute effectively to the repository.

## Coding Conventions

### File Naming
- Use **snake_case** for all file and module names.
  - Example: `sql_parser.rs`, `query_executor.rs`

### Import Style
- Use **relative imports** within the crate.
  - Example:
    ```rust
    mod utils;
    use crate::utils::parse_query;
    ```

### Export Style
- Use **named exports** for functions, structs, and modules.
  - Example:
    ```rust
    pub fn execute_query(...) { ... }
    pub struct SqlResult { ... }
    ```

### Commit Message Patterns
- Commit messages are freeform and do not follow a strict prefixing convention.
- Average commit message length is about 36 characters.

## Workflows

### Adding a New Feature
**Trigger:** When you need to implement a new capability in the codebase  
**Command:** `/add-feature`

1. Create a new file in `snake_case` if needed.
2. Implement the feature using relative imports for dependencies.
3. Export new functions or structs using `pub`.
4. Write or update tests as appropriate.
5. Commit changes with a clear, concise message.

### Fixing a Bug
**Trigger:** When you identify and need to fix a bug  
**Command:** `/fix-bug`

1. Locate the relevant module using snake_case naming.
2. Apply the fix, maintaining code style conventions.
3. Add or update tests to cover the bug fix.
4. Commit with a descriptive message.

### Running Tests
**Trigger:** To verify code correctness after changes  
**Command:** `/run-tests`

1. Identify test files (see Testing Patterns).
2. Run the test suite using the appropriate Rust test command:
    ```sh
    cargo test
    ```
3. Review results and address any failures.

## Testing Patterns

- Test files may follow the `*.test.ts` pattern, though the main language is Rust. (This suggests some TypeScript testing or legacy/test integration.)
- For Rust, tests are typically written in `mod tests` blocks within the same file or in a `tests/` directory.
    ```rust
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_query_execution() {
            // test implementation
        }
    }
    ```

## Commands
| Command      | Purpose                                    |
|--------------|--------------------------------------------|
| /add-feature | Start the process of adding a new feature  |
| /fix-bug     | Guide for fixing a bug                     |
| /run-tests   | Steps to run and verify tests              |
```
