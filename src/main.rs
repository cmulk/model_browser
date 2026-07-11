mod mesh;
mod paths;
mod threemf;
mod tree;

use axum::{
    Router,
    extract::{Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use clap::Parser;
use lru::LruCache;
use rust_embed::Embed;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;
use tower_http::compression::CompressionLayer;

/// 3D Printed Model Browser — browse and preview .3mf and .stl files.
#[derive(Parser, Debug)]
#[command(name = "model-browser", about = "Browse and preview 3D model files")]
struct Cli {
    /// Path to the model library directory
    #[arg(long, default_value = ".")]
    dir: PathBuf,

    /// Port to listen on
    #[arg(long, default_value = "8080")]
    port: u16,

    /// Don't auto-open browser on launch
    #[arg(long)]
    no_open: bool,
}

/// Embedded frontend assets.
#[derive(Embed)]
#[folder = "frontend/"]
struct FrontendAssets;

/// Shared application state.
struct AppState {
    /// Canonicalized library root path.
    root: PathBuf,
    /// Cached directory tree.
    tree_cache: RwLock<Option<tree::TreeNode>>,
    /// LRU cache for parsed meshes: key = (path, mtime).
    mesh_cache: RwLock<LruCache<(String, u64), Vec<u8>>>,
}

impl AppState {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            tree_cache: RwLock::new(None),
            mesh_cache: RwLock::new(LruCache::new(NonZeroUsize::new(3).unwrap())),
        }
    }

    fn get_or_build_tree(&self) -> tree::TreeNode {
        {
            let cache = self.tree_cache.read().unwrap();
            if let Some(ref t) = *cache {
                return t.clone();
            }
        }
        let t = tree::build_tree(&self.root);
        {
            let mut cache = self.tree_cache.write().unwrap();
            *cache = Some(t.clone());
        }
        t
    }

    fn invalidate_tree(&self) {
        let mut cache = self.tree_cache.write().unwrap();
        *cache = None;
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // Validate and canonicalize the library directory
    let root = match cli.dir.canonicalize() {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "Error: cannot access library directory {:?}: {}",
                cli.dir, e
            );
            std::process::exit(1);
        }
    };

    if !root.is_dir() {
        eprintln!("Error: {:?} is not a directory", root);
        std::process::exit(1);
    }

    let state = Arc::new(AppState::new(root.clone()));

    // Pre-build tree on startup
    let tree = state.get_or_build_tree();
    let dir_count = tree.dirs.len();
    tracing::info!("Scanned library: {} top-level folders", dir_count);

    // Build router
    let app = Router::new()
        // API routes
        .route("/api/tree", get(api_tree))
        .route("/api/mesh", get(api_mesh))
        .route("/api/thumbnail", get(api_thumbnail))
        .route("/api/image", get(api_image))
        .route("/api/download", get(api_download))
        // Static frontend (catch-all)
        .fallback(get(serve_frontend))
        .with_state(state)
        // Compression for everything except /api/mesh (handled per-route)
        .layer(CompressionLayer::new().no_br().no_zstd());

    let addr = format!("127.0.0.1:{}", cli.port);
    let url = format!("http://{}", addr);

    println!("╔══════════════════════════════════════════╗");
    println!("║   Model Browser                          ║");
    println!("║   Library: {:?}", root);
    println!("║   URL: {}", url);
    println!("╚══════════════════════════════════════════╝");

    // Auto-open browser
    if !cli.no_open
        && let Err(e) = open::that(&url)
    {
        tracing::warn!("Failed to open browser: {}", e);
    }

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("Error: cannot bind to {}: {}", addr, e);
            std::process::exit(1);
        });

    tracing::info!("Listening on {}", addr);
    axum::serve(listener, app).await.unwrap();
}

// ─── API Handlers ──────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct TreeQuery {
    refresh: Option<String>,
}

async fn api_tree(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TreeQuery>,
) -> impl IntoResponse {
    if query.refresh.as_deref() == Some("1") {
        state.invalidate_tree();
    }
    let tree = state.get_or_build_tree();
    let json = serde_json::to_string(&tree).unwrap_or_else(|_| "{}".to_string());

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        json,
    )
}

#[derive(serde::Deserialize)]
struct PathQuery {
    path: String,
}

async fn api_mesh(State(state): State<Arc<AppState>>, Query(query): Query<PathQuery>) -> Response {
    let validated = match paths::validate_path(&state.root, &query.path, paths::MESH_EXTENSIONS) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };

    // Check mesh cache
    let mtime = std::fs::metadata(&validated)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let cache_key = (query.path.clone(), mtime);

    // Try cache first
    {
        let mut cache = state.mesh_cache.write().unwrap();
        if let Some(data) = cache.get(&cache_key) {
            let data = data.clone();
            return mesh_response(data);
        }
    }

    // Parse in a blocking task
    let path = validated.clone();
    let result = tokio::task::spawn_blocking(move || {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let mesh_result = match ext.as_str() {
            "stl" => mesh::read_stl_file(&path),
            "3mf" => threemf::read_3mf_file(&path),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Unsupported file type",
            )),
        };

        mesh_result.map(|m| m.to_wire_format())
    })
    .await;

    match result {
        Ok(Ok(data)) => {
            // Cache it
            {
                let mut cache = state.mesh_cache.write().unwrap();
                cache.put(cache_key, data.clone());
            }
            mesh_response(data)
        }
        Ok(Err(e)) => {
            let body = serde_json::to_string(
                &serde_json::json!({"error": format!("Failed to parse mesh: {}", e)}),
            )
            .unwrap_or_default();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                )],
                body,
            )
                .into_response()
        }
        Err(e) => {
            let body =
                serde_json::to_string(&serde_json::json!({"error": format!("Task failed: {}", e)}))
                    .unwrap_or_default();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                )],
                body,
            )
                .into_response()
        }
    }
}

fn mesh_response(data: Vec<u8>) -> Response {
    let len = data.len().to_string();
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            ),
            (header::CONTENT_LENGTH, HeaderValue::from_str(&len).unwrap()),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
            // Disable compression for mesh data (breaks Content-Length progress tracking)
            (
                header::CONTENT_ENCODING,
                HeaderValue::from_static("identity"),
            ),
        ],
        data,
    )
        .into_response()
}

async fn api_thumbnail(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PathQuery>,
) -> Response {
    let validated =
        match paths::validate_path(&state.root, &query.path, paths::THUMBNAIL_EXTENSIONS) {
            Ok(p) => p,
            Err(e) => return e.into_response(),
        };

    let result = tokio::task::spawn_blocking(move || threemf::extract_thumbnail(&validated)).await;

    match result {
        Ok(Ok(data)) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, HeaderValue::from_static("image/png")),
                (
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("max-age=3600"),
                ),
            ],
            data,
        )
            .into_response(),
        Ok(Err(_)) => {
            let body = serde_json::to_string(&serde_json::json!({"error": "No thumbnail found"}))
                .unwrap_or_default();
            (
                StatusCode::NOT_FOUND,
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                )],
                body,
            )
                .into_response()
        }
        Err(e) => {
            let body =
                serde_json::to_string(&serde_json::json!({"error": format!("Task failed: {}", e)}))
                    .unwrap_or_default();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                )],
                body,
            )
                .into_response()
        }
    }
}

async fn api_image(State(state): State<Arc<AppState>>, Query(query): Query<PathQuery>) -> Response {
    let validated = match paths::validate_path(&state.root, &query.path, paths::IMAGE_EXTENSIONS) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };

    let content_type = mime_guess::from_path(&validated)
        .first_raw()
        .unwrap_or("application/octet-stream");

    match tokio::fs::read(&validated).await {
        Ok(data) => (
            StatusCode::OK,
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_str(content_type)
                        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
                ),
                (
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("max-age=3600"),
                ),
            ],
            data,
        )
            .into_response(),
        Err(_) => {
            let body = serde_json::to_string(&serde_json::json!({"error": "File not found"}))
                .unwrap_or_default();
            (
                StatusCode::NOT_FOUND,
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                )],
                body,
            )
                .into_response()
        }
    }
}

async fn api_download(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PathQuery>,
) -> Response {
    let validated = match paths::validate_path(&state.root, &query.path, paths::DOWNLOAD_EXTENSIONS)
    {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };

    let filename = validated
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");

    // Stream the file to avoid loading huge files into RAM
    let file = match tokio::fs::File::open(&validated).await {
        Ok(f) => f,
        Err(_) => {
            let body = serde_json::to_string(&serde_json::json!({"error": "File not found"}))
                .unwrap_or_default();
            return (
                StatusCode::NOT_FOUND,
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                )],
                body,
            )
                .into_response();
        }
    };

    let file_size = file.metadata().await.map(|m| m.len()).unwrap_or(0);

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = axum::body::Body::from_stream(stream);

    let disposition = format!("attachment; filename=\"{}\"", filename);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CONTENT_LENGTH, file_size.to_string())
        .header(header::CACHE_CONTROL, "no-cache")
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

// ─── Frontend Serving ──────────────────────────────────────────────────────────

async fn serve_frontend(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match FrontendAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path)
                .first_raw()
                .unwrap_or("application/octet-stream");

            let mut builder = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime);

            // Security headers on HTML responses
            if mime.starts_with("text/html") {
                builder = builder
                    .header(
                        "content-security-policy",
                        "default-src 'self'; script-src 'self' 'sha256-asCMfIyncyzWAxTmZ4etc2+gdnNNOB2Oh7f+B40MD0U='; style-src 'self' 'unsafe-inline'; \
                         img-src 'self' blob: data:; object-src 'none'; frame-ancestors 'none'; \
                         base-uri 'self'; form-action 'none'",
                    )
                    .header("x-frame-options", "DENY")
                    .header("x-content-type-options", "nosniff")
                    .header(
                        "permissions-policy",
                        "camera=(), microphone=(), geolocation=()",
                    );
            }

            builder
                .body(axum::body::Body::from(content.data.to_vec()))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        None => {
            // Fall back to index.html for SPA-style routing
            if let Some(content) = FrontendAssets::get("index.html") {
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .header(
                        "content-security-policy",
                        "default-src 'self'; script-src 'self' 'sha256-asCMfIyncyzWAxTmZ4etc2+gdnNNOB2Oh7f+B40MD0U='; style-src 'self' 'unsafe-inline'; \
                         img-src 'self' blob: data:; object-src 'none'; frame-ancestors 'none'; \
                         base-uri 'self'; form-action 'none'",
                    )
                    .header("x-frame-options", "DENY")
                    .header("x-content-type-options", "nosniff")
                    .header(
                        "permissions-policy",
                        "camera=(), microphone=(), geolocation=()",
                    )
                    .body(axum::body::Body::from(content.data.to_vec()))
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
            } else {
                StatusCode::NOT_FOUND.into_response()
            }
        }
    }
}
