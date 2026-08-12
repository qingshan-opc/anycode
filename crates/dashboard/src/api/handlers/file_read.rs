use super::*;
use std::path::PathBuf;

/// 单文件读取上限(与前端 MAX_TEXT_FILE_BYTES 对齐)。
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// 一次最多读取的路径数(与前端 MAX_TEXT_FILES 对齐)。
const MAX_PATHS: usize = 10;

#[derive(Deserialize)]
pub struct ReadFilePathsBody {
    pub paths: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ReadFileResult {
    pub path: String,
    pub name: String,
    /// text | binary | missing | dir
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub size_bytes: u64,
    pub truncated: bool,
}

fn expand_tilde(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

fn read_one(raw: &str) -> ReadFileResult {
    let trimmed = raw.trim();
    let path = expand_tilde(trimmed);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| trimmed.to_string());
    let display = path.display().to_string();
    let base =
        |kind: &'static str, content: Option<String>, size: u64, truncated: bool| ReadFileResult {
            path: display.clone(),
            name: name.clone(),
            kind,
            content,
            size_bytes: size,
            truncated,
        };
    let Ok(meta) = std::fs::metadata(&path) else {
        return base("missing", None, 0, false);
    };
    if meta.is_dir() {
        return base("dir", None, 0, false);
    }
    let size = meta.len();
    let cap = usize::try_from(MAX_FILE_BYTES).unwrap_or(usize::MAX);
    let bytes = match std::fs::File::open(&path) {
        Ok(mut f) => {
            use std::io::Read;
            let mut buf = Vec::new();
            // 多读 1 字节用于判断是否截断,避免对大文件分两步。
            match f.by_ref().take(MAX_FILE_BYTES + 1).read_to_end(&mut buf) {
                Ok(_) => buf,
                Err(_) => return base("missing", None, 0, false),
            }
        }
        Err(_) => return base("missing", None, 0, false),
    };
    let truncated = bytes.len() > cap;
    let bytes = &bytes[..bytes.len().min(cap)];
    if looks_binary(bytes) {
        return base("binary", None, size, false);
    }
    base(
        "text",
        Some(String::from_utf8_lossy(bytes).into_owned()),
        size,
        truncated,
    )
}

/// POST /api/files/read-paths — 对话框粘贴本地文件路径时读取内容
/// (loopback 本地工作台,受 mutate_origin_guard / auth_middleware 保护)。
pub async fn read_file_paths(Json(body): Json<ReadFilePathsBody>) -> impl IntoResponse {
    if body.paths.is_empty() || body.paths.len() > MAX_PATHS {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("paths must be 1..={MAX_PATHS}") })),
        )
            .into_response();
    }
    let files: Vec<ReadFileResult> = body.paths.iter().map(|p| read_one(p)).collect();
    Json(json!({ "files": files })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_file_with_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.md");
        std::fs::write(&path, "hello 文件").unwrap();
        let r = read_one(path.to_str().unwrap());
        assert_eq!(r.kind, "text");
        assert_eq!(r.name, "notes.md");
        assert_eq!(r.content.as_deref(), Some("hello 文件"));
        assert!(!r.truncated);
    }

    #[test]
    fn missing_and_dir_and_binary_are_classified() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_one(dir.path().join("nope.txt").to_str().unwrap()).kind,
            "missing"
        );
        assert_eq!(read_one(dir.path().to_str().unwrap()).kind, "dir");
        let bin = dir.path().join("a.bin");
        std::fs::write(&bin, [0u8, 159, 146, 150]).unwrap();
        let r = read_one(bin.to_str().unwrap());
        assert_eq!(r.kind, "binary");
        assert!(r.content.is_none());
    }

    #[test]
    fn tilde_expands_to_home() {
        let home = dirs::home_dir().unwrap();
        let marker = home.join(".anycode-read-paths-test-marker");
        std::fs::write(&marker, "x").unwrap();
        let r = read_one("~/.anycode-read-paths-test-marker");
        let _ = std::fs::remove_file(&marker);
        assert_eq!(r.kind, "text");
        assert_eq!(r.path, marker.display().to_string());
    }
}
