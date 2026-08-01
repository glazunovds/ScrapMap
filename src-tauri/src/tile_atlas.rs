use std::{
    env,
    path::{Component, Path, PathBuf},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::Deserialize;
use serde_json::Value;

const ATLAS_DIRECTORY: &str = "atlas";
const ATLAS_MANIFEST: &str = "manifest.json";
const ATLAS_TILES_DIRECTORY: &str = "tiles";
const MAX_PREVIEW_BYTES: u64 = 4 * 1024 * 1024;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const JPEG_SIGNATURE: &[u8; 3] = b"\xff\xd8\xff";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasPreviewRequestV1 {
    pub relative_path: String,
}

fn atlas_root() -> Option<PathBuf> {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("ScrapMap").join(ATLAS_DIRECTORY))
}

fn safe_relative_path(relative_path: &str) -> Option<PathBuf> {
    if relative_path.is_empty()
        || relative_path.len() > 512
        || relative_path.contains('\0')
        || relative_path.contains('\\')
        || relative_path.contains(':')
        || !(relative_path.to_ascii_lowercase().ends_with(".png")
            || relative_path.to_ascii_lowercase().ends_with(".jpg")
            || relative_path.to_ascii_lowercase().ends_with(".jpeg"))
    {
        return None;
    }

    let path = Path::new(relative_path);
    if path.is_absolute() {
        return None;
    }

    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => return None,
        }
    }

    (!safe.as_os_str().is_empty()).then_some(safe)
}

fn is_atlas_manifest(value: &Value) -> bool {
    value
        .as_object()
        .and_then(|object| object.get("kind"))
        .and_then(Value::as_str)
        == Some("scrapmap.tile-atlas")
        && value.get("entries").and_then(Value::as_array).is_some()
}

#[tauri::command]
pub fn atlas_manifest() -> Result<Option<Value>, String> {
    let Some(root) = atlas_root() else {
        return Ok(None);
    };

    let path = root.join(ATLAS_MANIFEST);
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    let manifest = match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) if is_atlas_manifest(&value) => value,
        _ => return Ok(None),
    };
    Ok(Some(manifest))
}

#[tauri::command]
pub fn atlas_preview(request: AtlasPreviewRequestV1) -> Result<Option<String>, String> {
    let Some(relative_path) = safe_relative_path(&request.relative_path) else {
        return Ok(None);
    };
    let Some(root) = atlas_root() else {
        return Ok(None);
    };

    let path = root.join(ATLAS_TILES_DIRECTORY).join(relative_path);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_PREVIEW_BYTES => metadata,
        _ => return Ok(None),
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) if bytes.len() as u64 == metadata.len() => bytes,
        _ => return Ok(None),
    };
    let mime_type = if bytes.starts_with(PNG_SIGNATURE) {
        "image/png"
    } else if bytes.starts_with(JPEG_SIGNATURE) {
        "image/jpeg"
    } else {
        return Ok(None);
    };

    Ok(Some(format!(
        "data:{mime_type};base64,{}",
        BASE64.encode(bytes)
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_relative_path_accepts_png_subpaths() {
        assert_eq!(
            safe_relative_path("meadow/abc.png").unwrap(),
            PathBuf::from("meadow").join("abc.png")
        );
    }

    #[test]
    fn safe_relative_path_rejects_escape_and_absolute_forms() {
        for value in [
            "../abc.png",
            "meadow/../../abc.png",
            "C:/atlas/abc.png",
            "/atlas/abc.png",
            "meadow\\abc.png",
            "meadow/abc.txt",
            "",
        ] {
            assert!(safe_relative_path(value).is_none(), "accepted {value}");
        }
        assert_eq!(
            safe_relative_path("topdown/1000101.jpg").unwrap(),
            PathBuf::from("topdown").join("1000101.jpg")
        );
    }

    #[test]
    fn manifest_shape_is_checked_before_exposure() {
        assert!(is_atlas_manifest(&serde_json::json!({
            "kind": "scrapmap.tile-atlas",
            "entries": []
        })));
        assert!(!is_atlas_manifest(&serde_json::json!({
            "kind": "other",
            "entries": []
        })));
        assert!(!is_atlas_manifest(&serde_json::json!({
            "kind": "scrapmap.tile-atlas"
        })));
    }
}
