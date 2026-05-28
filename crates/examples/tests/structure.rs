use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

// ── workspace root ────────────────────────────────────────────────────────────

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = crates/examples  →  parent = crates/  →  grandparent = root
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

// ── fake $HOME ────────────────────────────────────────────────────────────────

struct FakeHome {
    _dir: TempDir,
    saved_home: Option<String>,
    saved_xdg_config: Option<String>,
    saved_xdg_data: Option<String>,
    saved_xdg_cache: Option<String>,
    saved_cargo_home: Option<String>,
    saved_gopath: Option<String>,
}

impl FakeHome {
    fn setup() -> Self {
        let dir = tempfile::tempdir().expect("create temp HOME");
        let p = dir.path();

        let saved_home = env::var("HOME").ok();
        let saved_xdg_config = env::var("XDG_CONFIG_HOME").ok();
        let saved_xdg_data = env::var("XDG_DATA_HOME").ok();
        let saved_xdg_cache = env::var("XDG_CACHE_HOME").ok();
        let saved_cargo_home = env::var("CARGO_HOME").ok();
        let saved_gopath = env::var("GOPATH").ok();

        env::set_var("HOME", p);
        env::set_var("XDG_CONFIG_HOME", p.join(".config"));
        env::set_var("XDG_DATA_HOME", p.join(".local/share"));
        env::set_var("XDG_CACHE_HOME", p.join(".cache"));
        env::set_var("CARGO_HOME", p.join(".cargo"));
        env::set_var("GOPATH", p.join("go"));
        env::set_var("NPM_CONFIG_USERCONFIG", p.join(".npmrc"));

        Self {
            _dir: dir,
            saved_home,
            saved_xdg_config,
            saved_xdg_data,
            saved_xdg_cache,
            saved_cargo_home,
            saved_gopath,
        }
    }
}

impl Drop for FakeHome {
    fn drop(&mut self) {
        fn restore(key: &str, val: &Option<String>) {
            match val {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }
        restore("HOME", &self.saved_home);
        restore("XDG_CONFIG_HOME", &self.saved_xdg_config);
        restore("XDG_DATA_HOME", &self.saved_xdg_data);
        restore("XDG_CACHE_HOME", &self.saved_xdg_cache);
        restore("CARGO_HOME", &self.saved_cargo_home);
        restore("GOPATH", &self.saved_gopath);
        env::remove_var("NPM_CONFIG_USERCONFIG");
    }
}

// ── example spec ──────────────────────────────────────────────────────────────

struct ExampleSpec {
    /// Subdirectory name under `examples/`
    name: &'static str,
    /// All files that must exist (relative to the example directory)
    required_files: &'static [&'static str],
    /// `(file, url_fragment)` — file content must contain `url_fragment`
    proxy_references: &'static [(&'static str, &'static str)],
    /// Files that must parse as valid JSON (not JSONC)
    json_files: &'static [&'static str],
    /// Additional files that must parse as valid TOML (`.mise.toml` is always checked)
    extra_toml_files: &'static [&'static str],
    /// Files that must start with a `#!/` shebang line
    shebang_files: &'static [&'static str],
}

fn all_specs() -> &'static [ExampleSpec] {
    &[
        ExampleSpec {
            name: "npm",
            required_files: &[".mise.toml", ".npmrc", "package.json", "src/index.js"],
            proxy_references: &[(".npmrc", "localhost:8080/proxy/my-npm")],
            json_files: &["package.json"],
            extra_toml_files: &[],
            shebang_files: &[],
        },
        ExampleSpec {
            name: "cargo",
            required_files: &[
                ".mise.toml",
                ".cargo/config.toml",
                "Cargo.toml",
                "credentials.toml",
                "src/main.rs",
            ],
            proxy_references: &[(".cargo/config.toml", "localhost:8080/proxy/my-cargo")],
            json_files: &[],
            extra_toml_files: &[".cargo/config.toml", "Cargo.toml", "credentials.toml"],
            shebang_files: &[],
        },
        ExampleSpec {
            name: "go",
            required_files: &[".mise.toml", ".env", "go.mod", "main.go"],
            proxy_references: &[(".env", "localhost:8080/proxy/my-go")],
            json_files: &[],
            extra_toml_files: &[],
            shebang_files: &[],
        },
        ExampleSpec {
            name: "pypi",
            required_files: &[
                ".mise.toml",
                "pip.conf",
                "pyproject.toml",
                "requirements.txt",
                "src/main.py",
            ],
            proxy_references: &[("pip.conf", "localhost:8080/proxy/my-pypi")],
            json_files: &[],
            extra_toml_files: &["pyproject.toml"],
            shebang_files: &[],
        },
        ExampleSpec {
            name: "rubygems",
            required_files: &[".mise.toml", ".bundle/config", "config.ru", "Gemfile"],
            proxy_references: &[("Gemfile", "localhost:8080/proxy/my-gems")],
            json_files: &[],
            extra_toml_files: &[],
            shebang_files: &[],
        },
        ExampleSpec {
            name: "composer",
            required_files: &[".mise.toml", "auth.json", "composer.json", "src/App.php"],
            proxy_references: &[("composer.json", "localhost:8080/proxy/my-composer")],
            json_files: &["auth.json", "composer.json"],
            extra_toml_files: &[],
            shebang_files: &[],
        },
        ExampleSpec {
            name: "maven",
            required_files: &[
                ".mise.toml",
                "pom.xml",
                "settings.xml",
                "src/main/java/com/example/App.java",
            ],
            proxy_references: &[("settings.xml", "localhost:8080/proxy/my-maven")],
            json_files: &[],
            extra_toml_files: &[],
            shebang_files: &[],
        },
        ExampleSpec {
            name: "maven-quarkus",
            required_files: &[
                ".mise.toml",
                "pom.xml",
                "settings.xml",
                "src/main/java/com/example/GreetingResource.java",
                "src/main/resources/application.properties",
                "src/test/java/com/example/GreetingResourceTest.java",
            ],
            proxy_references: &[("settings.xml", "localhost:8080/proxy/my-maven")],
            json_files: &[],
            extra_toml_files: &[],
            shebang_files: &[],
        },
        ExampleSpec {
            name: "terraform",
            required_files: &[".mise.toml", ".terraformrc", "main.tf", "variables.tf"],
            proxy_references: &[(".terraformrc", "localhost:8080/proxy/my-terraform")],
            json_files: &[],
            extra_toml_files: &[],
            shebang_files: &[],
        },
        ExampleSpec {
            name: "github",
            required_files: &[".mise.toml", "download.sh", "install.sh"],
            proxy_references: &[("download.sh", "localhost:8080/proxy/my-github")],
            json_files: &[],
            extra_toml_files: &[],
            shebang_files: &["download.sh", "install.sh"],
        },
        ExampleSpec {
            name: "openvsx",
            required_files: &[
                ".mise.toml",
                ".vscode/extensions.json",
                ".vscode/settings.json",
                "install-extension.sh",
            ],
            proxy_references: &[("install-extension.sh", "localhost:8080/proxy/my-openvsx")],
            json_files: &[".vscode/extensions.json"],
            extra_toml_files: &[],
            shebang_files: &["install-extension.sh"],
        },
        ExampleSpec {
            name: "vscode-marketplace",
            required_files: &[
                ".mise.toml",
                ".vscode/extensions.json",
                "install-extension.sh",
            ],
            proxy_references: &[(
                "install-extension.sh",
                "localhost:8080/proxy/my-vscode-marketplace",
            )],
            json_files: &[".vscode/extensions.json"],
            extra_toml_files: &[],
            shebang_files: &["install-extension.sh"],
        },
    ]
}

// ── assertion helpers ─────────────────────────────────────────────────────────

fn read_file(dir: &Path, rel: &str, failures: &mut Vec<String>) -> Option<String> {
    let path = dir.join(rel);
    match fs::read_to_string(&path) {
        Ok(s) => Some(s),
        Err(e) => {
            failures.push(format!("{}: {}", path.display(), e));
            None
        }
    }
}

fn check_required_files(spec: &ExampleSpec, dir: &Path, failures: &mut Vec<String>) {
    for &rel in spec.required_files {
        let path = dir.join(rel);
        if !path.exists() {
            failures.push(format!(
                "examples/{}/{}: file does not exist",
                spec.name, rel
            ));
        }
    }
}

fn check_mise_toml(spec: &ExampleSpec, dir: &Path, failures: &mut Vec<String>) {
    let rel = ".mise.toml";
    let Some(content) = read_file(dir, rel, failures) else {
        return;
    };
    match content.parse::<toml::Table>() {
        Err(e) => failures.push(format!(
            "examples/{}/.mise.toml: invalid TOML: {}",
            spec.name, e
        )),
        Ok(table) => {
            for section in ["tools", "tasks"] {
                if !table.contains_key(section) {
                    failures.push(format!(
                        "examples/{}/.mise.toml: missing [{section}] section",
                        spec.name
                    ));
                }
            }
        }
    }
}

fn check_proxy_references(spec: &ExampleSpec, dir: &Path, failures: &mut Vec<String>) {
    for &(rel, fragment) in spec.proxy_references {
        let Some(content) = read_file(dir, rel, failures) else {
            continue;
        };
        if !content.contains(fragment) {
            failures.push(format!(
                "examples/{}/{}: does not reference proxy URL '{}' ",
                spec.name, rel, fragment
            ));
        }
    }
}

fn check_json_files(spec: &ExampleSpec, dir: &Path, failures: &mut Vec<String>) {
    for &rel in spec.json_files {
        let Some(content) = read_file(dir, rel, failures) else {
            continue;
        };
        if let Err(e) = serde_json::from_str::<serde_json::Value>(&content) {
            failures.push(format!(
                "examples/{}/{}: invalid JSON: {}",
                spec.name, rel, e
            ));
        }
    }
}

fn check_extra_toml_files(spec: &ExampleSpec, dir: &Path, failures: &mut Vec<String>) {
    for &rel in spec.extra_toml_files {
        let Some(content) = read_file(dir, rel, failures) else {
            continue;
        };
        if let Err(e) = content.parse::<toml::Table>() {
            failures.push(format!(
                "examples/{}/{}: invalid TOML: {}",
                spec.name, rel, e
            ));
        }
    }
}

fn check_shebang_files(spec: &ExampleSpec, dir: &Path, failures: &mut Vec<String>) {
    for &rel in spec.shebang_files {
        let Some(content) = read_file(dir, rel, failures) else {
            continue;
        };
        if !content.starts_with("#!/") {
            failures.push(format!(
                "examples/{}/{}: missing shebang (does not start with '#!')",
                spec.name, rel
            ));
        }
    }
}

// ── test entry point ──────────────────────────────────────────────────────────

#[test]
fn all_examples_are_complete() {
    // Redirect HOME and all tool-specific env vars to a throw-away temp directory
    // so the test is fully isolated from the developer's real home directory.
    let _home = FakeHome::setup();

    let examples = workspace_root().join("examples");
    let mut failures: Vec<String> = Vec::new();

    for spec in all_specs() {
        let dir = examples.join(spec.name);

        if !dir.is_dir() {
            failures.push(format!(
                "examples/{}: directory does not exist",
                spec.name
            ));
            continue;
        }

        check_required_files(spec, &dir, &mut failures);
        check_mise_toml(spec, &dir, &mut failures);
        check_proxy_references(spec, &dir, &mut failures);
        check_json_files(spec, &dir, &mut failures);
        check_extra_toml_files(spec, &dir, &mut failures);
        check_shebang_files(spec, &dir, &mut failures);
    }

    if !failures.is_empty() {
        panic!(
            "\n{} example structure failure(s):\n  • {}\n",
            failures.len(),
            failures.join("\n  • ")
        );
    }
}
