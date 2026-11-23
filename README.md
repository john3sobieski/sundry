# Sundry

Offline-first desktop toolbox for everyday tasks: images, vectorization,
converters, text and files. Written in Rust.

**Status:** early development. The core library (`crates/core`) and the
command-line frontend (`crates/cli`) exist; the desktop shell (`crates/app`, Slint) is in progress. Tools so far: data size converter, image resize and image format conversion (batch, before/after preview, ZIP/7z export).

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

Resize all photos to 1920x1080 (padded) into a folder:

```
cargo run -p sundry-cli -- image.resize -i a.jpg -i b.png -p width=1920 -p height=1080 -o out/
```

Convert images to WebP:

```
cargo run -p sundry-cli -- image.convert -i a.png -i b.jpg -p format=webp -o out/
```

Example (data size converter):

```
cargo run -p sundry-cli -- convert.data -p value=15 -p from=kB -p to=Mbit
```

## License

[PolyForm Noncommercial 1.0.0](LICENSE).
