#![allow(dead_code)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use tempfile::{Builder, TempDir};

pub struct TestRuntimeDir {
    path: PathBuf,
    _temp: Option<TempDir>,
}

impl TestRuntimeDir {
    pub fn new(label: &str) -> Self {
        if keep_artifacts() {
            let path = env::temp_dir().join(format!(
                "agent-ruler-test-{}-{}",
                sanitize_label(label),
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&path).expect("create retained test runtime dir");
            eprintln!(
                "AR_TEST_KEEP=1 retaining test runtime dir: {}",
                path.display()
            );
            return Self { path, _temp: None };
        }

        let temp = Builder::new()
            .prefix(&format!("agent-ruler-test-{}-", sanitize_label(label)))
            .tempdir()
            .expect("create temp runtime dir");
        let path = temp.path().to_path_buf();
        Self {
            path,
            _temp: Some(temp),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn keep_artifacts() -> bool {
    env::var("AR_TEST_KEEP")
        .map(|v| {
            let v = v.to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes"
        })
        .unwrap_or(false)
}

fn sanitize_label(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}
