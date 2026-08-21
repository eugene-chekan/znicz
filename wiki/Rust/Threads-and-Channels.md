# Threads, channels, atomics

## Threads

`std::thread::spawn` starts a function on another OS thread. The player loop lives there so the TUI can stay smooth.

The audio callback is **another** thread created by cpal / the OS.

## Channels

A **channel** is a queue between threads.

```rust
command_tx.send(Command::Pause)?;
// other thread:
command_rx.try_recv()
```

Znicz uses `crossbeam-channel` (unbounded). The UI never waits for audio. It fires a command and paints the last known state.

## `Arc` and locks

`Arc<T>` lets several threads own the same `T`.

`RwLock` allows many readers or one writer. We use it for `PlayerState` snapshots. The audio callback does **not** take this lock.

## Atomics

`AtomicBool` / `AtomicU32` are single values you can read/write without a mutex. Good for pause and volume in the callback.

`Ordering::Relaxed` is enough for a flag that does not need to publish a whole data structure. Flush uses `Acquire`/`Release` so “please drop samples” is visible to the other thread.

## Extra reading

- [Book: Fearless concurrency](https://doc.rust-lang.org/book/ch16-00-concurrency.html)
- [Atkins: atomics](https://doc.rust-lang.org/nomicon/atomics.html) (advanced)
- [crossbeam-channel](https://docs.rs/crossbeam-channel/)
