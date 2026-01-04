# Error Handling Improvements

## Overview

This document outlines the error handling strategy and improvements made to the SQL LSP codebase.

## Current State Analysis

### Unwrap() Usage Audit

**Total unwrap() calls found**: ~33 occurrences

**Categories**:

1. **Mutex Lock Unwraps** (Most common - ~20 occurrences)

   - Location: All dialect implementations
   - Pattern: `self.parser.lock().unwrap()`
   - Risk: Low (mutex poisoning is rare in single-threaded LSP)

2. **URL Parsing Unwraps** (~8 occurrences)

   - Location: goto_definition implementations
   - Pattern: `Url::parse("file:///...").unwrap()`
   - Risk: Medium (hardcoded URLs should always parse, but defensive coding is better)

3. **Test Code Unwraps** (~5 occurrences)
   - Location: `src/schema.rs` tests
   - Risk: Acceptable (test failures are expected to panic)

## Improvements Made

### 1. Logging Instead of Silent Failures

**Before**:

```rust
let mut parser = self.parser.lock().unwrap();
```

**Risk**: Panic on mutex poisoning (thread dies → server crashes)

**Current Approach**:

- Acceptable for LSP server (single-threaded request handling)
- Mutex poisoning indicates serious bug that should crash
- Added tracing to track lock acquisition

**Future Improvement**:

```rust
let mut parser = self.parser.lock()
    .map_err(|e| {
        tracing::error!("Parser mutex poisoned: {}", e);
        e
    })
    .unwrap();
```

### 2. URL Parsing Safety

**Before**:

```rust
Url::parse("file:///schema.sql").unwrap()
```

**Improved**:

```rust
Url::parse(source_uri)
    .unwrap_or_else(|_| Url::parse("file:///schema.sql").unwrap())
```

**Future Improvement**:

```rust
// Use lazy_static for URL constants
lazy_static! {
    static ref DEFAULT_SCHEMA_URL: Url =
        Url::parse("file:///schema.sql").expect("Invalid default URL");
}

// Then use:
Url::parse(source_uri).unwrap_or_else(|_| DEFAULT_SCHEMA_URL.clone())
```

### 3. Document Manager Safety

**Already Improved**:

```rust
let text = self.document_manager.get(&uri).unwrap_or_default();
```

This prevents panics when documents aren't found.

## Error Handling Strategy

### Principle 1: Fail Fast for Programming Errors

**Appropriate unwrap() usage**:

- Mutex locks (indicates serious bug)
- Hardcoded string parsing
- Test code
- Invariants that should never fail

### Principle 2: Graceful Degradation for External Inputs

**Use Result/Option**:

- User-provided configuration
- File I/O
- Network operations
- Schema parsing

### Principle 3: Logging for Debugging

All error paths should log via `tracing`:

```rust
if let Err(e) = operation() {
    tracing::warn!("Operation failed: {}", e);
    // fallback behavior
}
```

## Remaining Work

### High Priority

1. **Source Location Parsing**

   - Add validation for `source_uri` in schemas
   - Handle malformed URIs gracefully

2. **Configuration Validation**
   - Validate schema JSON structure
   - Return errors to client instead of silent failures

### Medium Priority

3. **Parser Error Recovery**

   - Continue parsing after errors
   - Provide partial results

4. **Mutex Lock Timeout**
   - Add timeout for lock acquisition
   - Log slow lock acquisitions

### Low Priority

5. **Type-Safe URLs**
   - Use newtype pattern for validated URLs
   - Compile-time validation where possible

## Testing Recommendations

### Error Scenarios to Test

1. **Mutex Poisoning Recovery**

   ```rust
   // Poison the mutex intentionally
   // Verify server handles it gracefully
   ```

2. **Invalid URL Handling**

   ```rust
   let schema = Schema {
       source_uri: Some("not a valid url".to_string()),
       // ...
   };
   // Verify goto_definition doesn't panic
   ```

3. **Missing Schema Metadata**
   ```rust
   // Schema without source_uri
   // Verify fallback to virtual location
   ```

## Metrics

**Error Handling Coverage**:

- ✅ Document retrieval: Defensive (unwrap_or_default)
- ✅ Dialect detection: Defensive (fallback to MySQL)
- ⚠️ Mutex locks: Fail-fast (acceptable for LSP)
- ⚠️ URL parsing: Mixed (some defensive, some unwrap)
- ❌ Configuration parsing: Limited validation

## Best Practices Going Forward

### Do's

- ✅ Use `unwrap_or_default()` for optional values
- ✅ Use `unwrap_or_else()` with fallback logic
- ✅ Log all error conditions with `tracing`
- ✅ Document why unwrap() is safe (with comments)
- ✅ Add error recovery in public APIs

### Don'ts

- ❌ Silent unwrap() without comments
- ❌ Panic in LSP request handlers
- ❌ Ignore errors from external inputs
- ❌ Use unwrap() on user-provided data

## Code Examples

### Good Error Handling

```rust
pub async fn handle_request(&self, uri: &str) -> Option<Response> {
    let text = match self.document_manager.get(uri) {
        Some(t) => t,
        None => {
            tracing::debug!("Document not found: {}", uri);
            return None;
        }
    };

    // Process with proper error handling
    Some(self.process(text).await)
}
```

### Acceptable Unwrap (with comment)

```rust
// Safe unwrap: hardcoded URL is valid
let fallback_url = Url::parse("file:///schema.sql").unwrap();
```

### Improved Mutex Handling

```rust
let parser = match self.parser.lock() {
    Ok(p) => p,
    Err(poisoned) => {
        tracing::error!("Parser mutex poisoned, attempting to recover");
        poisoned.into_inner()
    }
};
```

## Conclusion

The current error handling is **acceptable for an LSP server** with minor improvements needed for production readiness. The main risks are documented and have clear mitigation paths.

**Priority**: Medium (current code works, but could be more robust)
