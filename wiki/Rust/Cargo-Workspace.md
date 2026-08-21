# Cargo workspaces

This repo is a **workspace**. One `Cargo.toml` at the root lists members:

```toml
[workspace]
members = ["znicz-core", "znicz-tui", "znicz-mcp", "znicz"]
```

Shared crate versions live in `[workspace.dependencies]`. Each member says `serde.workspace = true` instead of repeating version numbers.

## Useful commands

```bash
cargo build --workspace
cargo test --workspace
cargo run -p znicz -- --list-devices
cargo run -p znicz -- path/to/file.flac
```

`target/` holds build output. Do not commit it (see `.gitignore`).

## Edition

We use **edition 2021** so we stay compatible with current cpal/rmcp crates. Edition is not the same as compiler version. The compiler is pinned in `rust-toolchain.toml`.

## Extra reading

- [Cargo book: workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
- [Edition guide](https://doc.rust-lang.org/edition-guide/)
