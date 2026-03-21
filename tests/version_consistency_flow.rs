use serde_json::Value;
use std::fs;
use std::path::Path;

fn read_file(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

fn parse_manifest_version(toml: &str) -> String {
    toml.lines()
        .find_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with("version = \"") {
                return None;
            }
            let value = trimmed
                .strip_prefix("version = \"")
                .and_then(|rest| rest.strip_suffix('"'))?;
            Some(value.to_string())
        })
        .expect("package version in Cargo.toml")
}

fn parse_package_json_version(raw: &str) -> String {
    raw.lines()
        .find_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with("\"version\":") {
                return None;
            }
            let value = trimmed
                .strip_prefix("\"version\":")?
                .trim()
                .trim_end_matches(',')
                .trim();
            Some(value.trim_matches('"').to_string())
        })
        .expect("version in package.json")
}

fn manifest_section_by_id<'a>(manifest: &'a Value, id: &str) -> &'a Value {
    manifest["sections"]
        .as_array()
        .and_then(|sections| sections.iter().find(|section| section["id"] == id))
        .unwrap_or_else(|| panic!("manifest section '{id}'"))
}

fn manifest_routes(section: &Value) -> Vec<String> {
    section["items"]
        .as_array()
        .expect("section items")
        .iter()
        .map(|item| {
            item["route"]
                .as_str()
                .unwrap_or_else(|| panic!("route string in item {item}"))
                .to_string()
        })
        .collect()
}

#[test]
fn compiled_cli_version_matches_cargo_manifest_version() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cargo_toml = read_file(&root.join("Cargo.toml"));
    let cargo_version = parse_manifest_version(&cargo_toml);
    assert_eq!(env!("CARGO_PKG_VERSION"), cargo_version);
}

#[test]
fn docs_package_version_matches_cargo_manifest_version() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cargo_toml = read_file(&root.join("Cargo.toml"));
    let cargo_version = parse_manifest_version(&cargo_toml);

    let docs_package = read_file(&root.join("docs-site/package.json"));
    let docs_version = parse_package_json_version(&docs_package);

    assert_eq!(docs_version, cargo_version);
}

#[test]
fn docs_config_uses_cargo_version_source_of_truth() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let docs_config = read_file(&root.join("docs-site/docs/.vitepress/config.ts"));
    let docs_css = read_file(&root.join("docs-site/docs/.vitepress/theme/custom.css"));

    assert!(docs_config.contains("cargoTomlPath"));
    assert!(docs_config.contains("readFileSync(cargoTomlPath"));
    assert!(docs_config.contains("versionMatch"));
    assert!(docs_config.contains("const version = versionMatch ? versionMatch[1] : 'unknown';"));
    assert!(
        docs_css.contains(".VPNavBar .VPNavBarTitle .title::after"),
        "docs header version badge should be scoped to the title/logo container"
    );
    assert!(
        !docs_css.contains(".VPNavBar .title::after"),
        "generic .VPNavBar .title::after causes duplicate version badges"
    );
}

#[test]
fn docs_nav_integrations_uses_consolidated_guide_route() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let docs_config = read_file(&root.join("docs-site/docs/.vitepress/config.ts"));

    assert!(docs_config.contains("{ text: 'Integrations', link: '/integrations/guide' }"));
    assert!(!docs_config.contains("/integrations/openclaw-guide"));
    assert!(!docs_config.contains("/integrations/claudecode-guide"));
    assert!(!docs_config.contains("/integrations/opencode-guide"));
}

#[test]
fn docs_manifest_integrations_routes_are_consolidated_and_no_historical_audit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest_raw = read_file(&root.join("docs-site/docs/.vitepress/docs-manifest.json"));
    let manifest: Value = serde_json::from_str(&manifest_raw).expect("parse docs manifest");

    let integrations = manifest_section_by_id(&manifest, "integrations");
    let integration_routes = manifest_routes(integrations);
    assert_eq!(
        integration_routes,
        vec![
            "integrations/guide".to_string(),
            "integrations/api-reference".to_string()
        ]
    );

    for section in manifest["sections"].as_array().expect("manifest sections") {
        for item in section["items"].as_array().expect("manifest section items") {
            let title = item["title"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let route = item["route"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            assert!(
                !title.contains("historical audit"),
                "historical audit title should not appear in docs manifest: {title}"
            );
            assert!(
                !route.contains("historical-audit"),
                "historical audit route should not appear in docs manifest: {route}"
            );
        }
    }
}
