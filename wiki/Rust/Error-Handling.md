# Errors

Znicz uses a small enum `ZniczError` with `thiserror`. Each variant is a kind of failure (audio, decode, I/O, player).

```rust
pub type Result<T> = std::result::Result<T, ZniczError>;
```

**Rule:** return `Result`, do not `unwrap()` in library code unless a broken invariant means a programmer bug.

The binary uses `color-eyre` so panics and errors print a readable report.

MCP maps engine errors to MCP `internal_error` so the AI sees a message, not a crash.

## `?`

`?` means: if this is `Err`, return it now. If `Ok`, unwrap the value. It keeps decode loops short.

## Extra reading

- [Book: Error handling](https://doc.rust-lang.org/book/ch09-00-error-handling.html)
- [thiserror](https://docs.rs/thiserror/)
- [anyhow / eyre](https://docs.rs/eyre/) — good at the binary edge, not inside a library API
