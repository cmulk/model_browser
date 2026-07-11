# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A single-binary Rust web server (axum) that serves a browser-based 3D viewer for local
`.3mf`/`.stl` model libraries. The frontend (vanilla JS + three.js) is embedded into the
binary at compile time via `rust-embed`, so the shipped artifact is one executable with
no external assets or runtime dependencies (other than the target directory of models).

## Commands

All common tasks are defined in the `justfile` (run with `just <task>`):

- `just dev` — run in debug mode against the default library path (`/mnt/d/inplace`)
- `just run` — run in release mode
- `just build` / `just release` — `cargo build` / `cargo build --release`
- `just check` — `cargo check` + `cargo clippy -- -D warnings`
- `just test` — `cargo test`
- `just fmt` / `just fmt-check` — `cargo fmt` / `cargo fmt -- --check`
- `just ci` — `fmt-check` + `check` + `test` (run this before considering work done)
- `just windows` — cross-compile release build for `x86_64-pc-windows-gnu`. Requires the
  `mingw-w64` system package (`sudo apt-get install -y mingw-w64` on Debian/Ubuntu) — this
  provides the `x86_64-w64-mingw32-gcc`/`dlltool` linker toolchain. `rustup target add` alone
  only installs Rust's std for the target and is not sufficient; without the system package the
  build fails at link time with `error: error calling dlltool 'x86_64-w64-mingw32-dlltool': No
  such file or directory`.

Run a single test with standard cargo filtering, e.g. `cargo test test_vertex_deduplication`.
Tests are colocated with implementation as `#[cfg(test)] mod tests` blocks in each `src/*.rs` file
(no separate `tests/` directory).

The binary itself takes `--dir <path>` (library root, defaults to `.`), `--port <port>`
(defaults to 8080), and `--no-open` (skip auto-opening a browser tab).

## Architecture

### Backend (`src/`)

- `main.rs` — axum app setup, route handlers, `AppState` (shared cache state), and the
  embedded-frontend fallback handler. This is the place to look first for how a request
  flows end to end.
- `paths.rs` — `validate_path()`, the single choke point all file-serving endpoints go
  through. It percent-decodes the requested path, rejects `..`/absolute paths, canonicalizes
  and verifies containment within the library root, and checks the extension against a
  per-endpoint allowlist. Any new endpoint that touches the filesystem from a client-supplied
  path must go through this.
- `tree.rs` — recursively scans the library root into a `TreeNode` tree (dirs + files),
  filtering to allowed extensions and pruning empty directories. This tree is what
  `/api/tree` returns as JSON and is cached in `AppState.tree_cache` (invalidated via
  `?refresh=1`).
- `mesh.rs` — the in-memory `Mesh` type (flat position/index buffers) shared by both STL and
  3MF parsing, binary STL parsing with vertex deduplication (quantized to 0.0001mm), and
  encoding to the custom binary wire format sent to the browser.
- `threemf.rs` — 3MF (zip + XML) parsing. Handles the Bambu Studio "production extension"
  where object geometry can live in external `3D/Objects/object_N.model` files referenced by
  `p:path`, resolved recursively through nested `<component>` transforms (`Transform::compose`).
  Also extracts embedded plate thumbnails from `Metadata/plate_*.png`.

### Request flow / caching

- `/api/tree` — returns the cached directory tree as JSON; `?refresh=1` forces a rescan.
- `/api/mesh?path=...` — validates the path, checks `AppState.mesh_cache` (an LRU keyed by
  `(path, mtime)`, capacity 3), otherwise parses the file (STL or 3MF, dispatched by
  extension) on a `spawn_blocking` task and caches + returns the result. Parsing is CPU-bound
  and kept off the async runtime.
- `/api/thumbnail`, `/api/image`, `/api/download` — serve embedded 3MF thumbnails, raw
  images, and streamed file downloads respectively, all behind `validate_path`.
- Everything else falls back to the embedded frontend (`FrontendAssets`, SPA-style: unknown
  paths serve `index.html`). HTML responses get CSP/security headers set inline in `main.rs`.

### Wire format (backend ↔ frontend contract)

Meshes are NOT sent as JSON — `Mesh::to_wire_format()` in `mesh.rs` packs them into a compact
binary format (`MESH_MAGIC` header, vertex/triangle counts, then flat f32 positions and u32
indices, all little-endian). The frontend decodes this manually via `DataView` in
`frontend/app.js` (`loadMesh`, around line 298-352). If you change the wire format, both sides
must change together, and `MESH_VERSION` should be bumped.

### Frontend (`frontend/`)

- `app.js` — single-file vanilla JS app: three.js scene/camera/controls setup, directory tree
  rendering and search/filtering, mesh loading + decoding, thumbnail/image preview, and
  download handling. No build step or framework — it's loaded directly as an ES module.
- `vendor/` — `three.module.js` and `OrbitControls.js`, pinned to a specific version and
  downloaded by `build.rs` (or `just vendor-js`) if not already present. These are `cargo:rerun-if-changed`-tracked but not committed to always re-fetch; don't hand-edit them.
- Coordinate convention: the scene is Z-up (matching 3MF/STL conventions), not three.js's
  default Y-up — see `camera.up.set(0, 0, 1)` and the grid rotation in `initThree()`.

### Security model

Since this serves an arbitrary local directory to a browser, path validation
(`paths::validate_path`) and the frontend CSP (`main.rs::serve_frontend`) are load-bearing,
not incidental — treat changes to either as security-sensitive. 3MF zip-path resolution has
its own traversal guard (`threemf::normalize_zip_path`) since component paths inside the
archive are also attacker-influenceable.
