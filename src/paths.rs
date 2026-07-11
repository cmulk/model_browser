use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use percent_encoding::percent_decode_str;
use serde_json::json;
use std::path::{Component, Path, PathBuf};

/// Error type for path validation failures.
#[derive(Debug)]
pub enum PathError {
    InvalidEncoding,
    TraversalAttempt,
    AbsolutePath,
    OutsideRoot,
    DisallowedExtension,
    NotFound,
}

impl IntoResponse for PathError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            PathError::InvalidEncoding => (StatusCode::BAD_REQUEST, "Invalid path encoding"),
            PathError::TraversalAttempt => (StatusCode::BAD_REQUEST, "Path traversal not allowed"),
            PathError::AbsolutePath => (StatusCode::BAD_REQUEST, "Absolute paths not allowed"),
            PathError::OutsideRoot => (StatusCode::BAD_REQUEST, "Path outside library root"),
            PathError::DisallowedExtension => (StatusCode::BAD_REQUEST, "File type not allowed"),
            PathError::NotFound => (StatusCode::NOT_FOUND, "File not found"),
        };
        let body = serde_json::to_string(&json!({"error": msg})).unwrap_or_default();
        (status, [("content-type", "application/json")], body).into_response()
    }
}

/// Validate and resolve a user-provided relative path against the library root.
///
/// 1. Percent-decode the path
/// 2. Reject absolute paths and `..` components
/// 3. Join with root and canonicalize
/// 4. Verify the result starts with the canonicalized root (with trailing separator)
/// 5. Check extension against the allowlist
pub fn validate_path(
    root: &Path,
    raw_path: &str,
    allowed_extensions: &[&str],
) -> Result<PathBuf, PathError> {
    // Percent-decode
    let decoded = percent_decode_str(raw_path)
        .decode_utf8()
        .map_err(|_| PathError::InvalidEncoding)?;
    let decoded = decoded.as_ref();

    // Reject empty path
    if decoded.is_empty() {
        return Err(PathError::NotFound);
    }

    let rel = Path::new(decoded);

    // Reject absolute paths
    if rel.is_absolute() {
        return Err(PathError::AbsolutePath);
    }

    // Reject any `..` components
    for component in rel.components() {
        match component {
            Component::ParentDir => return Err(PathError::TraversalAttempt),
            Component::RootDir | Component::Prefix(_) => return Err(PathError::AbsolutePath),
            _ => {}
        }
    }

    // Check extension against allowlist (if allowlist is non-empty)
    if !allowed_extensions.is_empty() {
        let ext = rel
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        if !allowed_extensions
            .iter()
            .any(|a| a.eq_ignore_ascii_case(&ext))
        {
            return Err(PathError::DisallowedExtension);
        }
    }

    // Join with root
    let joined = root.join(rel);

    // Canonicalize (this also verifies the file exists)
    let canonical = joined.canonicalize().map_err(|_| PathError::NotFound)?;

    // Get canonical root with trailing separator for prefix check
    let canonical_root = root.canonicalize().map_err(|_| PathError::NotFound)?;
    let root_str = canonical_root.to_string_lossy().to_string();
    let canonical_str = canonical.to_string_lossy().to_string();

    // Verify result is within root (with separator to prevent partial-match bypasses)
    let root_prefix = if root_str.ends_with(std::path::MAIN_SEPARATOR) {
        root_str
    } else {
        format!("{}{}", root_str, std::path::MAIN_SEPARATOR)
    };

    if !canonical_str.starts_with(&root_prefix) {
        return Err(PathError::OutsideRoot);
    }

    Ok(canonical)
}

/// Allowed extensions for mesh endpoints
pub const MESH_EXTENSIONS: &[&str] = &["3mf", "stl"];

/// Allowed extensions for image endpoints
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

/// Allowed extensions for download (all model + image types)
pub const DOWNLOAD_EXTENSIONS: &[&str] = &["3mf", "stl", "jpg", "jpeg", "png"];

/// Allowed extensions for thumbnail (3mf only)
pub const THUMBNAIL_EXTENSIONS: &[&str] = &["3mf"];

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_rejects_parent_dir() {
        let root = std::env::temp_dir();
        let result = validate_path(&root, "../etc/passwd", &[]);
        assert!(matches!(result, Err(PathError::TraversalAttempt)));
    }

    #[test]
    fn test_rejects_absolute_path() {
        let root = std::env::temp_dir();
        let result = validate_path(&root, "/etc/passwd", &[]);
        assert!(matches!(result, Err(PathError::AbsolutePath)));
    }

    #[test]
    fn test_rejects_encoded_traversal() {
        let root = std::env::temp_dir();
        // %2e%2e = ..
        let result = validate_path(&root, "%2e%2e/etc/passwd", &[]);
        assert!(matches!(result, Err(PathError::TraversalAttempt)));
    }

    #[test]
    fn test_rejects_disallowed_extension() {
        let tmp = std::env::temp_dir().join("model_browser_test_ext");
        fs::create_dir_all(&tmp).ok();
        let test_file = tmp.join("test.txt");
        fs::write(&test_file, "test").ok();
        let result = validate_path(&tmp, "test.txt", &["stl", "3mf"]);
        assert!(matches!(result, Err(PathError::DisallowedExtension)));
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_accepts_valid_path() {
        let tmp = std::env::temp_dir().join("model_browser_test_valid");
        fs::create_dir_all(&tmp).ok();
        let test_file = tmp.join("cube.stl");
        fs::write(&test_file, "fake stl data").ok();
        let result = validate_path(&tmp, "cube.stl", &["stl", "3mf"]);
        assert!(result.is_ok());
        fs::remove_dir_all(&tmp).ok();
    }
}
