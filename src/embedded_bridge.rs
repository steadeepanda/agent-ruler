use std::fs;
use std::path::{Component, Path};

use anyhow::{anyhow, Context, Result};
use include_dir::{include_dir, Dir, DirEntry};
use sha2::{Digest, Sha256};

use crate::runners::RunnerKind;

static EMBEDDED_BRIDGE_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/bridge");

const EMBEDDED_BRIDGE_VERSION: &str = env!("CARGO_PKG_VERSION");
const EMBEDDED_BRIDGE_STAMP_FILE: &str = ".embedded-bridge-version";
const REQUIRED_BRIDGE_PATHS: &[&str] = &[
    "openclaw/channel_bridge.py",
    "channels/telegram/channel_bridge.py",
    "claudecode/channels/telegram/channel_bridge.py",
    "claudecode/skills/agent-ruler-safe-runtime.md",
    "claudecode/claudecode-agent-ruler-tools/.claude-plugin/plugin.json",
    "claudecode/claudecode-agent-ruler-tools/.mcp.json",
    "claudecode/claudecode-agent-ruler-tools/hooks/hooks.json",
    "claudecode/claudecode-agent-ruler-tools/hooks/pretooluse.py",
    "claudecode/claudecode-agent-ruler-tools/skills/agent-ruler-safe-runtime/SKILL.md",
    "opencode/channels/telegram/channel_bridge.py",
    "opencode/skills/agent-ruler-safe-runtime.md",
    "opencode/opencode-agent-ruler-tools/index.mjs",
    "shared/agent_ruler_mcp_server.py",
    "openclaw/approvals-hook/HOOK.md",
    "openclaw/openclaw-agent-ruler-tools/index.ts",
    "openclaw/openclaw-agent-ruler-tools/openclaw.plugin.json",
    "openclaw/openclaw-agent-ruler-tools/sanity-check.mjs",
    "openclaw/openclaw-agent-ruler-tools/skills/agent-ruler-safe-runtime.md",
];
const OPENCLAW_REQUIRED_BRIDGE_PATHS: &[&str] = &[
    "openclaw/channel_bridge.py",
    "openclaw/approvals-hook/HOOK.md",
    "openclaw/openclaw-agent-ruler-tools/index.ts",
    "openclaw/openclaw-agent-ruler-tools/openclaw.plugin.json",
    "openclaw/openclaw-agent-ruler-tools/sanity-check.mjs",
    "openclaw/openclaw-agent-ruler-tools/skills/agent-ruler-safe-runtime.md",
];
const CLAUDECODE_REQUIRED_BRIDGE_PATHS: &[&str] = &[
    "claudecode/channels/telegram/channel_bridge.py",
    "claudecode/skills/agent-ruler-safe-runtime.md",
    "claudecode/claudecode-agent-ruler-tools/.claude-plugin/plugin.json",
    "claudecode/claudecode-agent-ruler-tools/.mcp.json",
    "claudecode/claudecode-agent-ruler-tools/hooks/hooks.json",
    "claudecode/claudecode-agent-ruler-tools/hooks/pretooluse.py",
    "claudecode/claudecode-agent-ruler-tools/skills/agent-ruler-safe-runtime/SKILL.md",
    "shared/agent_ruler_mcp_server.py",
];
const OPENCODE_REQUIRED_BRIDGE_PATHS: &[&str] = &[
    "opencode/channels/telegram/channel_bridge.py",
    "opencode/skills/agent-ruler-safe-runtime.md",
    "opencode/opencode-agent-ruler-tools/index.mjs",
    "shared/agent_ruler_mcp_server.py",
];

pub fn ensure_embedded_bridge_assets(ruler_root: &Path) -> Result<()> {
    if !is_managed_installs_root(ruler_root) {
        return Ok(());
    }

    let bridge_root = ruler_root.join("bridge");
    if is_current_and_complete(&bridge_root)? {
        return Ok(());
    }

    fs::create_dir_all(&bridge_root)
        .with_context(|| format!("create bridge root {}", bridge_root.display()))?;
    extract_dir_entries(&EMBEDDED_BRIDGE_DIR, &bridge_root)?;
    let stamp =
        expected_bridge_stamp().context("compute embedded bridge stamp after extraction")?;
    fs::write(
        bridge_root.join(EMBEDDED_BRIDGE_STAMP_FILE),
        format!("{stamp}\n"),
    )
    .with_context(|| "write embedded bridge version stamp".to_string())?;

    Ok(())
}

/// Ensure the current install/runtime has the minimum bridge assets for a runner.
///
/// Managed installs auto-heal by re-extracting embedded bridge assets first.
/// Dev-source checkouts validate the checked-in bridge tree directly.
pub fn ensure_runner_bridge_assets(ruler_root: &Path, runner_kind: RunnerKind) -> Result<()> {
    ensure_embedded_bridge_assets(ruler_root)?;

    let bridge_root = ruler_root.join("bridge");
    for relative in required_bridge_paths_for_runner(runner_kind) {
        let path = bridge_root.join(relative);
        if !path.is_file() {
            return Err(anyhow!(
                "required {} bridge asset missing at {}",
                runner_kind.id(),
                path.display()
            ));
        }
    }

    Ok(())
}

fn is_current_and_complete(bridge_root: &Path) -> Result<bool> {
    let stamp_path = bridge_root.join(EMBEDDED_BRIDGE_STAMP_FILE);
    let stamp = match fs::read_to_string(&stamp_path) {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(err).with_context(|| format!("read bridge stamp {}", stamp_path.display()));
        }
    };

    let expected_stamp = expected_bridge_stamp().context("compute embedded bridge stamp")?;
    if stamp.trim() != expected_stamp {
        return Ok(false);
    }

    Ok(REQUIRED_BRIDGE_PATHS
        .iter()
        .all(|relative| bridge_root.join(relative).is_file()))
}

fn expected_bridge_stamp() -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(EMBEDDED_BRIDGE_VERSION.as_bytes());

    for relative in REQUIRED_BRIDGE_PATHS {
        hasher.update(relative.as_bytes());
        let file = EMBEDDED_BRIDGE_DIR
            .get_file(relative)
            .ok_or_else(|| anyhow!("embedded bridge file missing: {relative}"))?;
        hasher.update(file.contents());
    }

    let digest = hasher.finalize();
    Ok(format!(
        "{EMBEDDED_BRIDGE_VERSION}:{}",
        format!("{digest:x}")
    ))
}

fn required_bridge_paths_for_runner(runner_kind: RunnerKind) -> &'static [&'static str] {
    match runner_kind {
        RunnerKind::Openclaw => OPENCLAW_REQUIRED_BRIDGE_PATHS,
        RunnerKind::Claudecode => CLAUDECODE_REQUIRED_BRIDGE_PATHS,
        RunnerKind::Opencode => OPENCODE_REQUIRED_BRIDGE_PATHS,
    }
}

fn extract_dir_entries(dir: &Dir<'_>, bridge_root: &Path) -> Result<()> {
    for entry in dir.entries() {
        match entry {
            DirEntry::Dir(child) => extract_dir_entries(child, bridge_root)?,
            DirEntry::File(file) => {
                let rel = validate_relative_path(file.path())?;
                let destination = bridge_root.join(rel);
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("create {}", parent.display()))?;
                }
                fs::write(&destination, file.contents())
                    .with_context(|| format!("write {}", destination.display()))?;
                apply_file_mode(&destination, rel)?;
            }
        }
    }

    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<&Path> {
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(anyhow!(
                    "unsupported embedded path component: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(path)
}

fn is_managed_installs_root(path: &Path) -> bool {
    path_ends_with(path, &["agent-ruler", "installs"])
}

fn path_ends_with(path: &Path, suffix: &[&str]) -> bool {
    let components: Vec<String> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    if components.len() < suffix.len() {
        return false;
    }
    let start = components.len() - suffix.len();
    components[start..]
        .iter()
        .zip(suffix.iter())
        .all(|(left, right)| left == right)
}

fn apply_file_mode(path: &Path, relative_path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(path)
            .with_context(|| format!("read metadata {}", path.display()))?
            .permissions();
        let mode = if is_executable_script(relative_path) {
            0o755
        } else {
            0o644
        };
        perms.set_mode(mode);
        fs::set_permissions(path, perms)
            .with_context(|| format!("set permissions {}", path.display()))?;
    }

    #[cfg(not(unix))]
    let _ = (path, relative_path);

    Ok(())
}

fn is_executable_script(relative_path: &Path) -> bool {
    relative_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("sh"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use tempfile::tempdir;

    use crate::runners::RunnerKind;

    use super::{ensure_embedded_bridge_assets, ensure_runner_bridge_assets};

    #[test]
    fn extracts_bridge_assets_for_managed_installs_root() {
        let temp = tempdir().expect("tempdir");
        let installs_root = temp.path().join("agent-ruler").join("installs");
        fs::create_dir_all(&installs_root).expect("create installs root");

        ensure_embedded_bridge_assets(&installs_root).expect("extract embedded assets");

        assert!(
            installs_root
                .join("bridge/openclaw/channel_bridge.py")
                .is_file(),
            "bridge channel script should be extracted"
        );
        assert!(
            installs_root
                .join("bridge/channels/telegram/channel_bridge.py")
                .is_file(),
            "shared telegram channel script should be extracted"
        );
        assert!(
            installs_root
                .join("bridge/claudecode/channels/telegram/channel_bridge.py")
                .is_file(),
            "claudecode telegram bridge channel script should be extracted"
        );
        assert!(
            installs_root
                .join("bridge/claudecode/skills/agent-ruler-safe-runtime.md")
                .is_file(),
            "claudecode safe-runtime guidance should be extracted"
        );
        assert!(
            installs_root
                .join("bridge/claudecode/claudecode-agent-ruler-tools/.claude-plugin/plugin.json")
                .is_file(),
            "claudecode governance plugin metadata should be extracted"
        );
        assert!(
            installs_root
                .join("bridge/claudecode/claudecode-agent-ruler-tools/hooks/pretooluse.py")
                .is_file(),
            "claudecode governance pretool hook should be extracted"
        );
        assert!(
            installs_root
                .join("bridge/claudecode/claudecode-agent-ruler-tools/.mcp.json")
                .is_file(),
            "claudecode governance MCP config should be extracted"
        );
        assert!(
            installs_root
                .join(
                    "bridge/claudecode/claudecode-agent-ruler-tools/skills/agent-ruler-safe-runtime/SKILL.md"
                )
                .is_file(),
            "claudecode governance safe-runtime skill should be extracted"
        );
        assert!(
            installs_root
                .join("bridge/opencode/channels/telegram/channel_bridge.py")
                .is_file(),
            "opencode telegram bridge channel script should be extracted"
        );
        assert!(
            installs_root
                .join("bridge/opencode/skills/agent-ruler-safe-runtime.md")
                .is_file(),
            "opencode safe-runtime guidance should be extracted"
        );
        assert!(
            installs_root
                .join("bridge/opencode/opencode-agent-ruler-tools/index.mjs")
                .is_file(),
            "opencode governance plugin should be extracted"
        );
        assert!(
            installs_root
                .join("bridge/shared/agent_ruler_mcp_server.py")
                .is_file(),
            "shared Agent Ruler MCP server should be extracted"
        );
        assert!(
            installs_root
                .join("bridge/openclaw/openclaw-agent-ruler-tools/sanity-check.mjs")
                .is_file(),
            "tools adapter files should be extracted"
        );
    }

    #[test]
    fn noops_outside_managed_installs_root() {
        let temp = tempdir().expect("tempdir");
        let other_root = temp.path().join("dev-root");
        fs::create_dir_all(&other_root).expect("create root");

        ensure_embedded_bridge_assets(&other_root).expect("no-op extraction");

        assert!(
            !other_root.join("bridge").exists(),
            "bridge should not be written outside installs root"
        );
    }

    #[test]
    fn reextracts_when_required_file_is_missing() {
        let temp = tempdir().expect("tempdir");
        let installs_root = temp.path().join("agent-ruler").join("installs");
        fs::create_dir_all(&installs_root).expect("create installs root");

        ensure_embedded_bridge_assets(&installs_root).expect("initial extraction");
        let required = installs_root.join("bridge/openclaw/channel_bridge.py");
        fs::remove_file(&required).expect("remove required file");
        assert!(!required.exists(), "required file should be removed");

        ensure_embedded_bridge_assets(&installs_root).expect("re-extraction");
        assert!(
            required.exists(),
            "missing required file should be restored"
        );
    }

    #[test]
    fn validates_runner_bridge_assets_after_extraction() {
        let temp = tempdir().expect("tempdir");
        let installs_root = temp.path().join("agent-ruler").join("installs");
        fs::create_dir_all(&installs_root).expect("create installs root");

        ensure_embedded_bridge_assets(&installs_root).expect("extract embedded assets");

        ensure_runner_bridge_assets(&installs_root, RunnerKind::Openclaw)
            .expect("validate openclaw bridge assets");
        ensure_runner_bridge_assets(&installs_root, RunnerKind::Claudecode)
            .expect("validate claudecode bridge assets");
        ensure_runner_bridge_assets(&installs_root, RunnerKind::Opencode)
            .expect("validate opencode bridge assets");
    }
}
