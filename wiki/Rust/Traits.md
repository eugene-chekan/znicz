# Traits

A **trait** is a set of methods a type can implement. Like an interface, but you can implement it for types you do not own (with care).

Znicz has `AudioSource`:

```rust
pub trait AudioSource: Send {
    fn path(&self) -> Option<&Path>;
    fn url(&self) -> Option<&str>;
    fn title_hint(&self) -> &str;
    fn read_info(&self) -> Result<TrackInfo>;
    fn open_reader(&self) -> Result<Box<dyn MediaSource>>;
}
```

`LocalFileSource` is a file on disk (`path` is `Some`, `url` is `None`).
`HttpStreamSource` is a radio URL (`path` is `None`, `url` is `Some`). The
player talks to the trait, not to “only files”.

`open_reader` returns a Symphonia **`MediaSource`**: a file, or an HTTP body
that cannot seek. The GET and the later `Read` both run on the player thread.
`Box<dyn MediaSource>` is a **trait object**: we do not name the concrete
reader type.

`Send` means “this value can move to another thread”. Audio sources must be `Send`.

cpal’s `DeviceTrait` / `HostTrait` are the same idea: ALSA and WASAPI both look like “a device”.

## Extra reading

- [Book: Traits](https://doc.rust-lang.org/book/ch10-02-traits.html)
- [Book: Trait objects](https://doc.rust-lang.org/book/ch18-02-trait-objects.html)
