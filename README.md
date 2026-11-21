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
cargo run -p sundry-app
```

Requires Rust 1.92 or newer.

## CLI

```
sundry list [-c CATEGORY] [QUERY]
sundry info <tool>
sundry <tool> [-i FILE | -t TEXT]... [-p NAME=VALUE]... [-o PATH]
```

Example (data size converter):

```
cargo run -p sundry-cli -- convert.data -p value=15 -p from=kB -p to=Mbit
```

## License

[PolyForm Noncommercial 1.0.0](LICENSE).
