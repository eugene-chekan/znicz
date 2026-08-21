# Traits

A **trait** is a set of methods a type can implement. Like an interface, but you can implement it for types you do not own (with care).

Znicz has `AudioSource`:

```rust
pub trait AudioSource: Send {
    fn path(&self) -> &Path;
    fn read_info(&self) -> Result<TrackInfo>;
    fn open_reader(&self) -> Result<Box<dyn Read + Send>>;
}
```

Phase 1: `LocalFileSource`. Later: HTTP radio can be another type that implements the same trait. The player talks to the trait, not to “only files”.

`Send` means “this value can move to another thread”. Audio sources must be `Send`.

`Box<dyn Read + Send>` is a **trait object**: we do not name the concrete reader type, only that it can be read from another thread.

cpal’s `DeviceTrait` / `HostTrait` are the same idea: ALSA and WASAPI both look like “a device”.

## Extra reading

- [Book: Traits](https://doc.rust-lang.org/book/ch10-02-traits.html)
- [Book: Trait objects](https://doc.rust-lang.org/book/ch18-02-trait-objects.html)
