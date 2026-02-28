use std::path::{Path, PathBuf};

use globset::{Glob, GlobMatcher};

use crate::config::Policy;
use crate::utils::{looks_like_glob, path_to_string};

#[derive(Debug)]
pub(super) struct CompiledPatterns {
    pub(super) workspace: Vec<PathPattern>,
    pub(super) user_data: Vec<PathPattern>,
    pub(super) shared: Vec<PathPattern>,
    pub(super) system_critical: Vec<PathPattern>,
    pub(super) secrets: Vec<PathPattern>,
    pub(super) persistence_deny: Vec<PathPattern>,
    pub(super) persistence_approval: Vec<PathPattern>,
    pub(super) allowed_exec_prefixes: Vec<PathPattern>,
}

#[derive(Debug)]
pub(super) struct PathPattern {
    kind: PatternKind,
}

#[derive(Debug)]
enum PatternKind {
    Prefix(PathBuf),
    Glob(GlobMatcher),
}

impl PathPattern {
    pub(super) fn new(value: &str) -> Option<Self> {
        if value.trim().is_empty() {
            return None;
        }
        if looks_like_glob(value) {
            if let Ok(glob) = Glob::new(value) {
                return Some(Self {
                    kind: PatternKind::Glob(glob.compile_matcher()),
                });
            }
            return None;
        }

        Some(Self {
            kind: PatternKind::Prefix(PathBuf::from(value)),
        })
    }

    pub(super) fn matches(&self, path: &Path) -> bool {
        match &self.kind {
            PatternKind::Prefix(prefix) => {
                let candidate = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
                let base = prefix
                    .canonicalize()
                    .unwrap_or_else(|_| prefix.to_path_buf());
                candidate.starts_with(base)
            }
            PatternKind::Glob(matcher) => matcher.is_match(path_to_string(path)),
        }
    }
}

impl CompiledPatterns {
    pub(super) fn from_policy(policy: &Policy) -> Self {
        Self {
            workspace: compile_patterns(&policy.zones.workspace_paths),
            user_data: compile_patterns(&policy.zones.user_data_paths),
            shared: compile_patterns(&policy.zones.shared_paths),
            system_critical: compile_patterns(&policy.zones.system_critical_paths),
            secrets: compile_patterns(&policy.zones.secrets_paths),
            persistence_deny: compile_patterns(&policy.rules.persistence.deny_paths),
            persistence_approval: compile_patterns(&policy.rules.persistence.approval_paths),
            allowed_exec_prefixes: compile_patterns(&policy.rules.execution.allowed_exec_prefixes),
        }
    }
}

fn compile_patterns(input: &[String]) -> Vec<PathPattern> {
    input
        .iter()
        .filter_map(|value| PathPattern::new(value))
        .collect()
}
