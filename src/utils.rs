use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::model::ActionRequest;

pub fn expand_tilde(input: &str) -> String {
    if let Some(stripped) = input.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped).to_string_lossy().to_string();
        }
    }

    if input == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().to_string();
        }
    }

    input.to_string()
}

pub fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn looks_like_glob(pattern: &str) -> bool {
    ["*", "?", "[", "]", "{"]
        .iter()
        .any(|needle| pattern.contains(needle))
}

pub fn is_subpath(path: &Path, parent: &Path) -> bool {
    let p = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let base = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    p.starts_with(base)
}

pub fn resolve_command_path(cmd: &str) -> Option<PathBuf> {
    let cmd_path = Path::new(cmd);
    if cmd_path.components().count() > 1 {
        return Some(
            cmd_path
                .canonicalize()
                .unwrap_or_else(|_| cmd_path.to_path_buf()),
        );
    }

    let path_var = env::var_os("PATH")?;
    for part in env::split_paths(&path_var) {
        let candidate = part.join(cmd);
        if candidate.is_file() {
            return Some(candidate.canonicalize().unwrap_or(candidate));
        }
    }
    None
}

pub fn make_scope_key(request: &ActionRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{:?}", request.kind));
    hasher.update(request.operation.as_bytes());
    if let Some(path) = &request.path {
        hasher.update(path.to_string_lossy().as_bytes());
    }
    if let Some(path) = &request.secondary_path {
        hasher.update(path.to_string_lossy().as_bytes());
    }
    if let Some(host) = &request.host {
        hasher.update(host.as_bytes());
    }

    let mut sorted_meta = BTreeMap::new();
    for (k, v) in &request.metadata {
        sorted_meta.insert(k, v);
    }
    for (k, v) in sorted_meta {
        hasher.update(k.as_bytes());
        hasher.update(v.as_bytes());
    }

    hex::encode(hasher.finalize())
}
