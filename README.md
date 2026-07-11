# Model Browser

A single-binary web app for browsing and previewing local `.3mf` / `.stl` model
libraries in 3D, right in your browser. Point it at a directory, open the URL,
and get a searchable tree with an interactive three.js viewer — no install,
no external assets, no database.

The frontend (vanilla JS + three.js) is embedded into the binary at compile
time, so the shipped artifact is a single executable plus whatever directory
of models you point it at.

## Features

- Recursive directory tree of your model library, with search/filter
- Interactive 3D preview (orbit/pan/zoom) for `.3mf` and `.stl` files
- Multicolor rendering of Bambu Studio 3MF files (per-part filament colors)
- Vertex-deduplicated STL meshes with smooth shading
- 3MF plate thumbnail and embedded image preview
- File download
- Fast: buffered parsing with a fast non-cryptographic hasher for STL vertex
  welding, and allocation-free XML attribute parsing for 3MF

## Usage

```
model-browser --dir /path/to/models [--port 8080] [--no-open]
```

- `--dir` — path to the model library root (defaults to the current directory)
- `--port` — port to listen on (defaults to `8080`)
- `--no-open` — don't automatically open a browser tab on launch

Then open `http://127.0.0.1:<port>` (opened automatically unless `--no-open`
is passed).

## Building

Requires a Rust toolchain (edition 2024).

```
cargo build --release
```

The three.js vendor files are downloaded automatically by `build.rs` on
first build.

Common tasks are defined in the `justfile` (run with `just <task>`):

- `just dev` — run in debug mode
- `just build` / `just release` — `cargo build` / `cargo build --release`
- `just check` — `cargo check` + `cargo clippy -- -D warnings`
- `just test` — `cargo test`
- `just fmt` / `just fmt-check` — format / check formatting
- `just ci` — fmt-check + check + test
- `just windows` — cross-compile a release build for `x86_64-pc-windows-gnu`
  (requires the `mingw-w64` system package)

See [CLAUDE.md](CLAUDE.md) for architecture notes and more detail on the
build/test workflow.

## Security

This serves an arbitrary local directory to a browser. All filesystem access
goes through a single path-validation choke point that rejects traversal and
enforces a per-endpoint file-extension allowlist; see [CLAUDE.md](CLAUDE.md#security-model)
for details.

## License

No license has been chosen yet — all rights reserved by default.
