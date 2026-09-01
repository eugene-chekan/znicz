# Ownership and borrowing

Rust’s main idea: **every value has one owner**. When the owner goes away, the value is dropped (freed). You can **borrow** it instead of copying.

## Why this shows up in Znicz

The ring buffer has two ends. Only **one** thread may own the producer. Only **one** thread may own the consumer. We **move** the consumer into the cpal callback. The player thread keeps the producer. That is ownership, not a lock.

`PlayerHandle` is `Clone` because it holds:

- a channel **sender** (cheap to clone)
- `Arc<RwLock<PlayerState>>` (`Arc` = shared ownership of the lock)

The player process holds `PlayerHandle`. The TUI and MCP each hold an
`IpcClient` to that one process.

## Borrow examples

```rust
fn title(state: &PlayerState) -> &str {
    // borrow, do not take
}
```

`&` is a shared borrow (many readers). `&mut` is an exclusive borrow (one writer).

If the compiler says “cannot move out of borrowed content”, you probably need `.clone()`, `.as_ref()`, or a reference instead of taking the value.

## Extra reading

- [Book: Ownership](https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html)
- [Book: References](https://doc.rust-lang.org/book/ch04-02-references-and-borrowing.html)
