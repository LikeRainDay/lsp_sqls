# CONTRIBUTING

Thank you for your interest in contributing to SQL LSP! This document provides guidelines for contributing to the project.

## Code of Conduct

Be respectful and constructive in all interactions.

## How to Contribute

### Reporting Issues

- Check existing issues before creating a new one
- Use issue templates when available
- Provide clear reproduction steps
- Include version information and environment details

### Submitting Pull Requests

1. **Fork the repository**
2. **Create a feature branch**: `git checkout -b feature/your-feature-name`
3. **Make your changes**
4. **Run tests**: `cargo test`
5. **Run linter**: `cargo clippy`
6. **Commit with clear messages**: Follow [Conventional Commits](https://www.conventionalcommits.org/)
7. **Push and create PR**

### Commit Message Format

```
<type>(<scope>): <subject>

<body>

<footer>
```

**Types**: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`

**Example**:

```
feat(hover): add rich markdown formatting for table info

- Display column count and types
- Show first 10 columns to avoid overflow
- Include table comments if available

Closes #123
```

## Development Setup

### Prerequisites

- Rust 1.70+ (latest stable recommended)
- `cargo` and `rustc`

### Building

```bash
# Clone the repository
git clone https://github.com/your-org/lsp_sqls.git
cd lsp_sqls

# Build debug version
cargo build

# Build release version
cargo build --release

# Run tests
cargo test

# Check code
cargo check
```

### Project Structure

```
lsp_sqls/
├── src/
│   ├── main.rs           # Entry point
│   ├── server.rs         # LSP server implementation
│   ├── dialect.rs        # Dialect trait
│   ├── dialects/         # Dialect implementations
│   ├── parser/           # SQL parsers
│   ├── schema.rs         # Schema management
│   └── token.rs          # Token definitions
├── tests/                # Integration tests
├── docs/                 # Documentation
└── scripts/              # Helper scripts
```

## Coding Guidelines

### Style

- Follow Rust standard style guide
- Run `cargo fmt` before committing
- Use meaningful variable and function names
- Add documentation comments for public APIs

### Testing

- Write unit tests for new features
- Add integration tests for LSP methods
- Ensure all tests pass before submitting PR

### Documentation

- Update relevant documentation for new features
- Add examples where appropriate
- Keep README.md up to date

## Adding a New SQL Dialect

1. Create a new file in `src/dialects/your_dialect.rs`
2. Implement the `Dialect` trait
3. Register in `src/dialects/mod.rs`
4. Add tests in `tests/`
5. Update documentation

Example:

```rust
use crate::dialect::Dialect;
use async_trait::async_trait;

pub struct YourDialect {
    // fields
}

#[async_trait]
impl Dialect for YourDialect {
    fn name(&self) -> &str {
        "your_dialect"
    }

    // implement other methods...
}
```

## Adding New LSP Features

1. Update `ServerCapabilities` in `src/server.rs`
2. Implement the handler method
3. Add to relevant dialect implementations
4. Write tests
5. Update documentation

## Performance Considerations

- Profile before optimizing
- Use `cargo bench` for benchmarks
- Consider caching for expensive operations
- Document performance characteristics

## Release Process

(Maintainers only)

1. Update `CHANGELOG.md`
2. Bump version in `Cargo.toml`
3. Create git tag: `git tag -a v0.x.0 -m "Release v0.x.0"`
4. Push tag: `git push origin v0.x.0`
5. GitHub Actions will handle the release

## Getting Help

- GitHub Issues: Bug reports and feature requests
- Discussions: Questions and general discussion
- Documentation: See `docs/` directory

## License

By contributing, you agree that your contributions will be licensed under the same license as the project (see LICENSE file).
