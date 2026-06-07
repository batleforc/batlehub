use std::path::{Path, PathBuf};

pub struct ProjectDetection {
    pub registry_type: &'static str,
    pub package_name: Option<String>,
    /// Multi-line instructions shown in the TUI detail pane.
    pub instructions: String,
    /// Path relative to the scan root (empty string = root itself).
    pub relative_path: String,
}

/// Directories that are never entered during recursive scanning.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "vendor",
    ".git",
    ".github",
    ".hg",
    "dist",
    ".build",
    "__pycache__",
    ".tox",
    ".venv",
    "venv",
    ".mypy_cache",
];

/// Recursively scan `root` (up to `max_depth` levels of subdirectories) for
/// known project manifests and return one [`ProjectDetection`] per hit.
/// `max_depth = 0` restricts the scan to the root directory itself.
pub fn scan_project_types(
    root: &Path,
    server_url: &str,
    max_depth: usize,
) -> Vec<ProjectDetection> {
    scan_recursive(root, root, server_url, max_depth)
}

fn scan_recursive(
    root: &Path,
    dir: &Path,
    server_url: &str,
    remaining_depth: usize,
) -> Vec<ProjectDetection> {
    let rel = dir
        .strip_prefix(root)
        .unwrap_or(Path::new(""))
        .to_string_lossy()
        .replace('\\', "/");

    // Read the directory once: collect file names (for manifest detection) and
    // subdir paths (for recursion) in a single pass, filtering by entry type.
    let mut file_names: Vec<String> = Vec::new();
    let mut subdirs: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            if ft.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    file_names.push(name.to_string());
                }
            } else if ft.is_dir() && remaining_depth > 0 {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if !name.starts_with('.') && !SKIP_DIRS.contains(&name) {
                        subdirs.push(path);
                    }
                }
            }
        }
        subdirs.sort();
    }

    let mut out: Vec<ProjectDetection> = detect_project_types_in(dir, server_url, &file_names)
        .into_iter()
        .map(|mut d| {
            d.relative_path = rel.clone();
            d
        })
        .collect();

    for sub in subdirs {
        out.extend(scan_recursive(root, &sub, server_url, remaining_depth - 1));
    }

    out
}

fn detect_project_types_in(
    dir: &Path,
    server_url: &str,
    dir_names: &[String],
) -> Vec<ProjectDetection> {
    let mut out = Vec::new();

    // Cargo (Rust)
    let cargo_toml = dir.join("Cargo.toml");
    if cargo_toml.exists() {
        let name = read_toml_field(&cargo_toml, &["package", "name"]);
        let pkg = name.as_deref().unwrap_or("<package>").to_string();
        out.push(ProjectDetection {
            relative_path: String::new(),
            registry_type: "cargo",
            package_name: name,
            instructions: format!(
                "Registry type : cargo\n\
                 Package       : {pkg}\n\
                 \n\
                 ~/.cargo/config.toml:\n\
                 [registries]\n\
                 batlehub = {{ index = \"sparse+{server_url}/proxy/<registry>/cargo/\" }}\n\
                 \n\
                 Publish:\n\
                 cargo publish --registry batlehub"
            ),
        });
    }

    // Go modules
    let go_mod = dir.join("go.mod");
    if go_mod.exists() {
        let name = read_gomod_module(&go_mod);
        let pkg = name.as_deref().unwrap_or("<module>").to_string();
        out.push(ProjectDetection {
            relative_path: String::new(),
            registry_type: "gomodules",
            package_name: name,
            instructions: format!(
                "Registry type : gomodules\n\
                 Module        : {pkg}\n\
                 \n\
                 Environment:\n\
                 export GOPROXY={server_url}/proxy/<registry>/go,direct\n\
                 \n\
                 Use:\n\
                 go get {pkg}"
            ),
        });
    }

    // npm / Node.js
    let pkg_json = dir.join("package.json");
    if pkg_json.exists() {
        let name = read_json_field(&pkg_json, "name");
        let pkg = name.as_deref().unwrap_or("<package>").to_string();
        out.push(ProjectDetection {
            relative_path: String::new(),
            registry_type: "npm",
            package_name: name,
            instructions: format!(
                "Registry type : npm\n\
                 Package       : {pkg}\n\
                 \n\
                 .npmrc:\n\
                 registry={server_url}/proxy/<registry>/npm/\n\
                 \n\
                 Publish:\n\
                 npm publish"
            ),
        });
    }

    // PyPI (pyproject.toml / setup.py)
    let pyproject = dir.join("pyproject.toml");
    let setup_py = dir.join("setup.py");
    if pyproject.exists() || setup_py.exists() {
        let name = read_toml_field(&pyproject, &["project", "name"])
            .or_else(|| read_toml_field(&pyproject, &["tool", "poetry", "name"]));
        let pkg = name.as_deref().unwrap_or("<package>").to_string();
        out.push(ProjectDetection {
            relative_path: String::new(),
            registry_type: "pypi",
            package_name: name,
            instructions: format!(
                "Registry type : pypi\n\
                 Package       : {pkg}\n\
                 \n\
                 pip.conf / pip.ini:\n\
                 [global]\n\
                 index-url = {server_url}/proxy/<registry>/pypi/\n\
                 \n\
                 Publish:\n\
                 twine upload dist/*"
            ),
        });
    }

    // Maven (pom.xml)
    let pom_xml = dir.join("pom.xml");
    if pom_xml.exists() {
        let name = read_xml_tag(&pom_xml, "artifactId");
        let pkg = name.as_deref().unwrap_or("<artifactId>").to_string();
        out.push(ProjectDetection {
            relative_path: String::new(),
            registry_type: "maven",
            package_name: name,
            instructions: format!(
                "Registry type : maven\n\
                 Artifact      : {pkg}\n\
                 \n\
                 settings.xml:\n\
                 <repository>\n\
                   <id>batlehub</id>\n\
                   <url>{server_url}/proxy/<registry>/maven/</url>\n\
                 </repository>\n\
                 \n\
                 Publish:\n\
                 mvn deploy"
            ),
        });
    }

    // Composer (PHP)
    let composer_json = dir.join("composer.json");
    if composer_json.exists() {
        let name = read_json_field(&composer_json, "name");
        let pkg = name.as_deref().unwrap_or("<package>").to_string();
        out.push(ProjectDetection {
            relative_path: String::new(),
            registry_type: "composer",
            package_name: name,
            instructions: format!(
                "Registry type : composer\n\
                 Package       : {pkg}\n\
                 \n\
                 composer.json:\n\
                 \"repositories\": [{{\n\
                   \"type\": \"composer\",\n\
                   \"url\": \"{server_url}/proxy/<registry>/composer/\"\n\
                 }}]"
            ),
        });
    }

    // RubyGems
    let has_gemspec = dir_names.iter().any(|n| n.ends_with(".gemspec"));
    let has_gemfile = dir_names.iter().any(|n| n == "Gemfile");
    if has_gemspec || has_gemfile {
        let name = dir_names
            .iter()
            .find(|n| n.ends_with(".gemspec"))
            .and_then(|n| n.strip_suffix(".gemspec"))
            .map(str::to_string);
        let pkg = name.as_deref().unwrap_or("<gem>").to_string();
        out.push(ProjectDetection {
            relative_path: String::new(),
            registry_type: "rubygems",
            package_name: name,
            instructions: format!(
                "Registry type : rubygems\n\
                 Gem           : {pkg}\n\
                 \n\
                 ~/.gemrc:\n\
                 :sources:\n\
                 - {server_url}/proxy/<registry>/gems/\n\
                 \n\
                 Publish:\n\
                 gem push *.gem --host {server_url}/proxy/<registry>/gems/"
            ),
        });
    }

    // NuGet (.NET)
    let has_nuspec = dir_names.iter().any(|n| n.ends_with(".nuspec"));
    let has_csproj = dir_names.iter().any(|n| n.ends_with(".csproj"));
    if has_nuspec || has_csproj {
        let name = dir_names
            .iter()
            .find(|n| n.ends_with(".nuspec"))
            .and_then(|n| n.strip_suffix(".nuspec"))
            .map(str::to_string)
            .or_else(|| {
                dir_names
                    .iter()
                    .find(|n| n.ends_with(".csproj"))
                    .and_then(|n| n.strip_suffix(".csproj"))
                    .map(str::to_string)
            });
        let pkg = name.as_deref().unwrap_or("<package>").to_string();
        out.push(ProjectDetection {
            relative_path: String::new(),
            registry_type: "nuget",
            package_name: name,
            instructions: format!(
                "Registry type : nuget\n\
                 Package       : {pkg}\n\
                 \n\
                 Add NuGet source:\n\
                 dotnet nuget add source \\\n\
                   {server_url}/proxy/<registry>/nuget/v3/index.json \\\n\
                   --name batlehub\n\
                 \n\
                 Publish:\n\
                 dotnet nuget push *.nupkg --source batlehub"
            ),
        });
    }

    // Terraform
    let has_tf = dir_names.iter().any(|n| n.ends_with(".tf"));
    if has_tf {
        out.push(ProjectDetection {
            relative_path: String::new(),
            registry_type: "terraform",
            package_name: dir.file_name().and_then(|s| s.to_str()).map(str::to_string),
            instructions: format!(
                "Registry type : terraform\n\
                 \n\
                 ~/.terraformrc:\n\
                 provider_installation {{\n\
                   network_mirror {{\n\
                     url = \"{server_url}/proxy/<registry>/terraform/\"\n\
                   }}\n\
                 }}"
            ),
        });
    }

    // Conda
    let env_yml = dir.join("environment.yml");
    if env_yml.exists() {
        let name = grep_key(&env_yml, "name:");
        let pkg = name.as_deref().unwrap_or("<env>").to_string();
        out.push(ProjectDetection {
            relative_path: String::new(),
            registry_type: "conda",
            package_name: name,
            instructions: format!(
                "Registry type : conda\n\
                 Environment   : {pkg}\n\
                 \n\
                 ~/.condarc:\n\
                 channels:\n\
                   - {server_url}/proxy/<registry>/conda/\n\
                 \n\
                 Publish:\n\
                 batlehub-cli publish *.conda"
            ),
        });
    }

    out
}

// ── Manifest parsing helpers ───────────────────────────────────────────────

fn read_toml_field(path: &Path, keys: &[&str]) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: toml::Value = toml::from_str(&content).ok()?;
    let mut cur = &value;
    for key in keys {
        cur = cur.get(key)?;
    }
    cur.as_str().map(str::to_string)
}

fn read_gomod_module(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let module_path = content
        .lines()
        .find(|l| l.starts_with("module "))?
        .strip_prefix("module ")?
        .trim()
        .to_string();
    // Return the last segment as a short name
    let short = module_path.rsplit('/').next().unwrap_or(&module_path);
    Some(short.to_string())
}

fn read_json_field(path: &Path, field: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value.get(field)?.as_str().map(str::to_string)
}

fn read_xml_tag(path: &Path, tag: &str) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    // Strip <parent>…</parent> so that a parent-POM's <artifactId> is not
    // returned instead of the project's own top-level element.
    let content = strip_xml_block(&raw, "parent");
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = content.find(&open)? + open.len();
    let end = content[start..].find(&close)?;
    Some(content[start..start + end].trim().to_string())
}

fn strip_xml_block(content: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    match (content.find(&open), content.find(&close)) {
        (Some(s), Some(e)) if s < e => {
            format!("{}{}", &content[..s], &content[e + close.len()..])
        }
        _ => content.to_string(),
    }
}

fn grep_key(path: &Path, prefix: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    content.lines().find_map(|line| {
        line.strip_prefix(prefix)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    })
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn detects_cargo_toml() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let results = scan_project_types(dir.path(), "http://localhost:8080", 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].registry_type, "cargo");
        assert_eq!(results[0].package_name.as_deref(), Some("my-crate"));
        assert!(results[0].instructions.contains("cargo publish"));
    }

    #[test]
    fn detects_gomod() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("go.mod"),
            "module github.com/example/myapp\n\ngo 1.21\n",
        )
        .unwrap();
        let results = scan_project_types(dir.path(), "http://localhost:8080", 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].registry_type, "gomodules");
        assert_eq!(results[0].package_name.as_deref(), Some("myapp"));
    }

    #[test]
    fn detects_package_json() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"my-app","version":"1.0.0"}"#,
        )
        .unwrap();
        let results = scan_project_types(dir.path(), "http://localhost:8080", 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].registry_type, "npm");
        assert_eq!(results[0].package_name.as_deref(), Some("my-app"));
    }

    #[test]
    fn empty_dir_returns_nothing() {
        let dir = TempDir::new().unwrap();
        let results = scan_project_types(dir.path(), "http://localhost:8080", 0);
        assert!(results.is_empty());
    }

    #[test]
    fn detects_pom_xml() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("pom.xml"),
            "<project><artifactId>my-lib</artifactId></project>",
        )
        .unwrap();
        let results = scan_project_types(dir.path(), "http://localhost:8080", 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].registry_type, "maven");
        assert_eq!(results[0].package_name.as_deref(), Some("my-lib"));
        assert!(results[0].instructions.contains("mvn deploy"));
    }

    #[test]
    fn detects_nuspec() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("MyPkg.nuspec"), "<package/>").unwrap();
        let results = scan_project_types(dir.path(), "http://localhost:8080", 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].registry_type, "nuget");
        assert_eq!(results[0].package_name.as_deref(), Some("MyPkg"));
        assert!(results[0].instructions.contains("dotnet nuget push"));
    }

    #[test]
    fn detects_tf_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("main.tf"), "provider \"aws\" {}").unwrap();
        let results = scan_project_types(dir.path(), "http://localhost:8080", 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].registry_type, "terraform");
        assert!(results[0].instructions.contains("terraform"));
    }

    #[test]
    fn detects_environment_yml() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("environment.yml"),
            "name: myenv\ndependencies:\n  - numpy\n",
        )
        .unwrap();
        let results = scan_project_types(dir.path(), "http://localhost:8080", 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].registry_type, "conda");
        assert_eq!(results[0].package_name.as_deref(), Some("myenv"));
    }

    #[test]
    fn detects_multiple_types_in_same_dir() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"my-crate\"\n",
        )
        .unwrap();
        fs::write(dir.path().join("package.json"), r#"{"name":"my-app"}"#).unwrap();
        let results = scan_project_types(dir.path(), "http://localhost:8080", 0);
        assert_eq!(results.len(), 2);
        let types: Vec<&str> = results.iter().map(|r| r.registry_type).collect();
        assert!(types.contains(&"cargo"), "expected cargo in {types:?}");
        assert!(types.contains(&"npm"), "expected npm in {types:?}");
    }

    #[test]
    fn detects_composer_json() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("composer.json"),
            r#"{"name":"vendor/my-package"}"#,
        )
        .unwrap();
        let results = scan_project_types(dir.path(), "http://localhost:8080", 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].registry_type, "composer");
        assert_eq!(
            results[0].package_name.as_deref(),
            Some("vendor/my-package")
        );
    }

    #[test]
    fn detects_gemspec() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("my_gem.gemspec"),
            "Gem::Specification.new do |s| end",
        )
        .unwrap();
        let results = scan_project_types(dir.path(), "http://localhost:8080", 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].registry_type, "rubygems");
        assert_eq!(results[0].package_name.as_deref(), Some("my_gem"));
    }
}
