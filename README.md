# Sundry

Offline-first desktop toolbox for everyday tasks: images, vectorization,
converters, text and files. Written in Rust.

**Status:** early development. The core library (`crates/core`) and the
command-line frontend (`crates/cli`) exist; there are no real tools and no UI yet.

## Build

```
cargo test --workspace
```

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
