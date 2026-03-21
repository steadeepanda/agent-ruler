#![cfg(target_os = "linux")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::tempdir;

fn script_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("install")
        .join("install.sh")
}

#[test]
fn uninstall_removes_symlink_without_purging_data_by_default() {
    let home = tempdir().expect("home tempdir");
    let xdg = home.path().join(".local/share");
    let installs_root = xdg.join("agent-ruler/installs/dev");
    let projects_root = xdg.join("agent-ruler/projects/example");
    let link_dir = home.path().join(".local/bin");
    let link_path = link_dir.join("agent-ruler");
    let binary_path = installs_root.join("agent-ruler");

    fs::create_dir_all(&installs_root).expect("create installs root");
    fs::create_dir_all(&projects_root).expect("create projects root");
    fs::create_dir_all(&link_dir).expect("create link dir");
    fs::write(&binary_path, "#!/usr/bin/env bash\necho fake\n").expect("write fake binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(&binary_path, &link_path).expect("create symlink");
    }

    let output = Command::new("bash")
        .arg(script_path())
        .arg("--uninstall")
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", &xdg)
        .output()
        .expect("run uninstall");
    assert!(
        output.status.success(),
        "uninstall failed\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        !link_path.exists(),
        "agent-ruler symlink should be removed by uninstall"
    );
    assert!(
        !binary_path.exists(),
        "linked dev binary should be removed by uninstall"
    );
    assert!(
        projects_root.exists(),
        "runtime data should not be purged without --purge-data"
    );
}

#[test]
fn uninstall_with_purge_flags_cleans_installs_and_runtime_data() {
    let home = tempdir().expect("home tempdir");
    let xdg = home.path().join(".local/share");
    let installs_root = xdg.join("agent-ruler/installs/dev");
    let projects_root = xdg.join("agent-ruler/projects/example");
    let link_dir = home.path().join(".local/bin");
    let link_path = link_dir.join("agent-ruler");
    let binary_path = installs_root.join("agent-ruler");

    fs::create_dir_all(&installs_root).expect("create installs root");
    fs::create_dir_all(&projects_root).expect("create projects root");
    fs::create_dir_all(&link_dir).expect("create link dir");
    fs::write(&binary_path, "#!/usr/bin/env bash\necho fake\n").expect("write fake binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(&binary_path, &link_path).expect("create symlink");
    }

    let output = Command::new("bash")
        .arg(script_path())
        .args(["--uninstall", "--purge-installs", "--purge-data"])
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", &xdg)
        .output()
        .expect("run uninstall with purge flags");
    assert!(
        output.status.success(),
        "uninstall with purge flags failed\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(!link_path.exists(), "agent-ruler symlink should be removed");
    assert!(
        !installs_root.exists(),
        "installs root should be removed with --purge-installs"
    );
    assert!(
        !xdg.join("agent-ruler/projects").exists(),
        "runtime data should be removed with --purge-data"
    );
}

#[test]
fn release_install_downloads_verifies_and_links_binary() {
    let home = tempdir().expect("home tempdir");
    let xdg = home.path().join(".local/share");
    let fake_bin = home.path().join("fake-bin");
    let fixtures = home.path().join("fixtures");
    let link_dir = home.path().join(".local/bin");
    let installs_root = xdg.join("agent-ruler/installs/v9.9.9");
    let installed_bridge_openclaw =
        xdg.join("agent-ruler/installs/bridge/openclaw/channel_bridge.py");
    let installed_bridge_claudecode =
        xdg.join("agent-ruler/installs/bridge/claudecode/channels/telegram/channel_bridge.py");
    let installed_bridge_opencode =
        xdg.join("agent-ruler/installs/bridge/opencode/channels/telegram/channel_bridge.py");
    let installed_docs_index =
        xdg.join("agent-ruler/installs/docs-site/docs/.vitepress/dist/index.html");
    let installed_binary = installs_root.join("agent-ruler");
    let runtime_root = xdg.join("agent-ruler/projects/example-runtime");
    let runtime_config = runtime_root.join("state/config.yaml");
    let runner_setup = runtime_root.join("user_data/runners/openclaw/setup.json");
    let link_path = link_dir.join("agent-ruler");

    fs::create_dir_all(&fake_bin).expect("create fake bin");
    fs::create_dir_all(&fixtures).expect("create fixtures");
    fs::create_dir_all(&link_dir).expect("create link dir");
    fs::create_dir_all(runtime_config.parent().expect("runtime config parent"))
        .expect("create runtime state dir");
    fs::create_dir_all(runner_setup.parent().expect("runner setup parent"))
        .expect("create runner setup dir");
    fs::write(
        &runtime_config,
        "ui_bind: 127.0.0.1:4622\nrunner:\n  kind: openclaw\n",
    )
    .expect("write runtime config sentinel");
    fs::write(
        &runner_setup,
        "{\"managed_home\":\"/tmp/openclaw-home\",\"managed_workspace\":\"/tmp/openclaw-workspace\"}\n",
    )
    .expect("write runner setup sentinel");

    let dist = fixtures.join("dist");
    fs::create_dir_all(&dist).expect("create dist");
    let artifact_binary = dist.join("agent-ruler");
    let bridge_openclaw = dist.join("bridge/openclaw/channel_bridge.py");
    let bridge_claudecode = dist.join("bridge/claudecode/channels/telegram/channel_bridge.py");
    let bridge_opencode = dist.join("bridge/opencode/channels/telegram/channel_bridge.py");
    fs::write(&artifact_binary, "#!/usr/bin/env bash\necho agent-ruler\n")
        .expect("write artifact binary");
    fs::create_dir_all(
        bridge_openclaw
            .parent()
            .expect("openclaw bridge parent directory"),
    )
    .expect("create openclaw bridge parent directory");
    fs::create_dir_all(
        bridge_claudecode
            .parent()
            .expect("claudecode bridge parent directory"),
    )
    .expect("create claudecode bridge parent directory");
    fs::create_dir_all(
        bridge_opencode
            .parent()
            .expect("opencode bridge parent directory"),
    )
    .expect("create opencode bridge parent directory");
    fs::write(&bridge_openclaw, "# openclaw bridge fixture\n").expect("write openclaw bridge");
    fs::write(&bridge_claudecode, "# claudecode bridge fixture\n")
        .expect("write claudecode bridge");
    fs::write(&bridge_opencode, "# opencode bridge fixture\n").expect("write opencode bridge");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&artifact_binary)
            .expect("metadata")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&artifact_binary, perms).expect("chmod artifact binary");
    }

    let tarball = fixtures.join("agent-ruler-linux-x86_64.tar.gz");
    let docs_dist = dist.join("docs-site/docs/.vitepress/dist");
    fs::create_dir_all(&docs_dist).expect("create docs dist");
    fs::write(
        docs_dist.join("index.html"),
        "<html><body>Docs</body></html>",
    )
    .expect("write docs index");
    let tar_status = Command::new("tar")
        .args([
            "-C",
            dist.to_str().expect("dist utf-8"),
            "-czf",
            tarball.to_str().expect("tarball utf-8"),
            "agent-ruler",
            "bridge",
            "docs-site",
        ])
        .status()
        .expect("build fixture tarball");
    assert!(tar_status.success(), "tar should create fixture archive");

    let sums_output = Command::new("sha256sum")
        .arg(&tarball)
        .output()
        .expect("sha256sum fixture tarball");
    assert!(sums_output.status.success(), "sha256sum should succeed");
    fs::write(fixtures.join("SHA256SUMS.txt"), sums_output.stdout).expect("write sums");

    let release_json = r#"{
  "tag_name": "v9.9.9",
  "assets": [
    {
      "id": 101,
      "name": "agent-ruler-linux-x86_64.tar.gz",
      "browser_download_url": "https://downloads.example/agent-ruler-linux-x86_64.tar.gz"
    },
    {
      "id": 102,
      "name": "SHA256SUMS.txt",
      "browser_download_url": "https://downloads.example/SHA256SUMS.txt"
    }
  ]
}"#
    .to_string();
    fs::write(fixtures.join("release.json"), release_json).expect("write release metadata");

    let fake_curl = fake_bin.join("curl");
    let fake_curl_script = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
url="${{@: -1}}"
out=""
for ((i=1; i <= $#; i++)); do
  arg="${{!i}}"
  if [[ "${{arg}}" == "-o" ]]; then
    j=$((i + 1))
    out="${{!j}}"
  fi
done

case "${{url}}" in
  "https://api.github.com/repos/test-owner/test-repo/releases/tags/v9.9.9")
    cat "{release_json_path}"
    ;;
  "https://api.github.com/repos/test-owner/test-repo/releases/assets/101")
    cp "{tarball_path}" "${{out}}"
    ;;
  "https://api.github.com/repos/test-owner/test-repo/releases/assets/102")
    cp "{sums_path}" "${{out}}"
    ;;
  "https://downloads.example/agent-ruler-linux-x86_64.tar.gz")
    cp "{tarball_path}" "${{out}}"
    ;;
  "https://downloads.example/SHA256SUMS.txt")
    cp "{sums_path}" "${{out}}"
    ;;
  *)
    echo "unexpected curl url: ${{url}}" >&2
    exit 17
    ;;
esac
"#,
        release_json_path = fixtures.join("release.json").display(),
        tarball_path = tarball.display(),
        sums_path = fixtures.join("SHA256SUMS.txt").display(),
    );
    fs::write(&fake_curl, fake_curl_script).expect("write fake curl");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&fake_curl).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fake_curl, perms).expect("chmod fake curl");
    }

    let output = Command::new("bash")
        .arg(script_path())
        .args(["--release", "--version", "v9.9.9"])
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", &xdg)
        .env("AGENT_RULER_GITHUB_REPO", "test-owner/test-repo")
        .env(
            "PATH",
            format!(
                "{}:{}",
                fake_bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .expect("run release installer");
    assert!(
        output.status.success(),
        "release install failed\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        installed_binary.is_file(),
        "release binary should be installed at versioned path"
    );
    assert!(
        installed_bridge_openclaw.is_file(),
        "release bridge bundle should include openclaw bridge assets"
    );
    assert!(
        installed_bridge_claudecode.is_file(),
        "release bridge bundle should include claudecode bridge assets"
    );
    assert!(
        installed_bridge_opencode.is_file(),
        "release bridge bundle should include opencode bridge assets"
    );
    assert!(
        installed_docs_index.is_file(),
        "release docs bundle should be installed under shared installs root"
    );
    assert!(
        runtime_config.is_file(),
        "release install should preserve existing runtime config"
    );
    assert!(
        runner_setup.is_file(),
        "release install should preserve existing runner setup files"
    );
    assert_eq!(
        fs::read_to_string(&runtime_config).expect("read runtime config"),
        "ui_bind: 127.0.0.1:4622\nrunner:\n  kind: openclaw\n",
        "runtime config content should remain unchanged during release install"
    );
    assert_eq!(
        fs::read_to_string(&runner_setup).expect("read runner setup"),
        "{\"managed_home\":\"/tmp/openclaw-home\",\"managed_workspace\":\"/tmp/openclaw-workspace\"}\n",
        "runner setup content should remain unchanged during release install"
    );
    assert!(link_path.exists(), "agent-ruler link should exist");
    #[cfg(unix)]
    {
        let target = fs::read_link(&link_path).expect("read link target");
        assert_eq!(
            target, installed_binary,
            "link should point to release binary"
        );
    }
}
