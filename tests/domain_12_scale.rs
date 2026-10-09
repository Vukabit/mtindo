mod common;

use std::process::Command;
use std::time::Instant;

use common::TestEnvironment;

fn build_large_scale_repository(env: &TestEnvironment) {
    let crates = ["core", "storage", "network", "engine", "analytics"];
    let files_per_crate = 100;

    let root_toml = r#"
[workspace]
members = ["core", "storage", "network", "engine", "analytics"]
resolver = "2"
"#;
    env.write_file("Cargo.toml", root_toml);

    let mto_toml = r#"
sort_alphabetically = true
group_impls = true
type_order = ["ModDecl", "UseDecl", "MacroDecl", "ConstDecl", "StaticDecl", "TraitDecl", "EnumDecl", "StructDecl", "ImplBlock", "FnDecl"]
ignore_dirs = [".git", "target"]
"#;
    env.write_file("mto.toml", mto_toml);

    for &krate in &crates {
        let manifest_path = format!("{}/Cargo.toml", krate);
        let manifest_content = format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            krate
        );
        env.write_file(&manifest_path, &manifest_content);

        let mut lib_rs_content = String::new();

        for file_idx in 0..files_per_crate {
            let module_idx = file_idx % 10;
            let relative_file_path = format!("{}/src/component_{}.rs", krate, file_idx);

            lib_rs_content.push_str(&format!("pub mod component_{};\n", file_idx));

            let source_content = generate_synthetic_rust_source(krate, module_idx, file_idx);
            env.write_file(&relative_file_path, &source_content);
        }

        env.write_file(&format!("{}/src/lib.rs", krate), &lib_rs_content);
    }
}

fn generate_synthetic_rust_source(crate_name: &str, module_idx: usize, file_idx: usize) -> String {
    let mut source = String::new();

    source.push_str("use std::collections::HashMap;\n");
    source.push_str("use std::sync::Arc;\n");
    source.push_str("use std::sync::atomic::{AtomicU64, Ordering};\n\n");

    source.push_str(&format!(
        "pub mod nested_scope_{}_{} {{\n    pub fn probe() -> usize {{ {} }}\n}}\n\n",
        module_idx, file_idx, file_idx
    ));

    source.push_str(&format!(
        "pub trait Interface_{}_{} {{\n    fn execute(&self) -> u64;\n}}\n\n",
        module_idx, file_idx
    ));

    source.push_str(&format!(
        "pub struct Record_{}_{} {{\n    pub counter: AtomicU64,\n    pub label: String,\n}}\n\n",
        module_idx, file_idx
    ));

    source.push_str(&format!(
        "impl Interface_{}_{} for Record_{}_{} {{\n    fn execute(&self) -> u64 {{\n        self.counter.load(Ordering::SeqCst) + {} \n    }}\n}}\n\n",
        module_idx, file_idx, module_idx, file_idx, file_idx * 10
    ));

    source.push_str(&format!(
        "impl Record_{}_{} {{\n    pub fn new() -> Self {{\n        Self {{\n            counter: AtomicU64::new({}),\n            label: String::from(\"{}_{}\"),\n        }}\n    }}\n}}\n\n",
        module_idx, file_idx, file_idx, crate_name, file_idx
    ));

    source.push_str(&format!(
        "pub fn dispatch_entrypoint_{}_{}() -> u64 {{\n    let rec = Record_{}_{}::new();\n    rec.execute()\n}}\n",
        module_idx, file_idx, module_idx, file_idx
    ));

    source
}

#[test]
fn test_massive_repository_lifecycle_with_compilation() {
    let env = TestEnvironment::new();
    build_large_scale_repository(&env);

    let mut check_pre = Command::new("cargo");
    check_pre.arg("check").current_dir(env.dir.path());
    let status_pre = check_pre.status().expect("Failed to execute cargo check");
    assert!(
        status_pre.success(),
        "Synthetic repository failed to compile pre-format"
    );

    let start_time = Instant::now();
    let mut format_cmd = env.mto_cmd();
    format_cmd.arg("-R").arg(".");
    format_cmd.assert().success();
    let elapsed = start_time.elapsed();
    println!(
        "Batch format completed for 500 files in {:.2}s",
        elapsed.as_secs_f64()
    );

    let mut check_post = Command::new("cargo");
    check_post.arg("check").current_dir(env.dir.path());
    let status_post = check_post.status().expect("Failed to execute cargo check");
    assert!(
        status_post.success(),
        "Synthetic repository failed to compile post-format"
    );
}
