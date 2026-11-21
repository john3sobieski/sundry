# Sundry

Offline-first desktop toolbox for everyday tasks: images, vectorization,
converters, text and files. Written in Rust.

**Status:** early development. The core library (`crates/core`) and the
command-line frontend (`crates/cli`) exist; the desktop shell (`crates/app`, Slint) is in progress; there are no real tools yet.

## Build

```
cargo test --workspace
```

## Desktop app

```
cargo run -p sundry-app --features demo
```

Requires Rust 1.92 or newer.

## CLI

```
sundry list [-c CATEGORY] [QUERY]
sundry info <tool>
sundry <tool> [-i FILE | -t TEXT]... [-p NAME=VALUE]... [-o PATH]
```

To try it before real tools exist, build with the demo tool:

```
cargo run -p sundry-cli --features demo -- text.repeat -t ab -p times=3
```

## License

[PolyForm Noncommercial 1.0.0](LICENSE).
