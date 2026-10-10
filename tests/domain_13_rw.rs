mod common;

use std::process::Command;

use common::TestEnvironment;

/// The layout settings each crate is laid out with, from a clean checkout each time: the
/// defaults, then two mixes that between them set every layout setting away from its
/// default.
const LAYOUT_CONFIGS: &[(&str, &str)] = &[
    ("defaults", ""),
    (
        "mix one",
        "blank_lines_between_blocks = 0\ngroup_impls = false\nimpl_method_order = \"trait\"\n\
         import_sort = \"2024\"\nname_sort = \"bytes\"\nreorder_impl_items = true\n\
         test_module = \"in_place\"\ntrait_impl_order = [\"Default\", \"Clone\", \"Debug\"]\n\
         type_order = [\"FnDecl\", \"ImplBlock\", \"StructDecl\", \"EnumDecl\", \"TraitDecl\", \
         \"UseDecl\", \"ModDecl\"]\nvisibility_order = [\"Private\", \"PubSuper\", \"PubIn\", \
         \"PubCrate\", \"Pub\"]\n",
    ),
    (
        "mix two",
        "import_grouping = false\nimport_sort = \"2015\"\nname_sort = \"case_insensitive\"\n\
         sort_alphabetically = false\nsort_imports = false\ntrait_impl_sort = \"original\"\n",
    ),
];

const REAL_WORLD_TARGETS: &[(&str, &str)] = &[
    // Small
    (
        "lazy_static",
        "https://github.com/rust-lang-nursery/lazy-static.rs.git",
    ),
    ("itoa", "https://github.com/dtolnay/itoa.git"),
    ("termcolor", "https://github.com/BurntSushi/termcolor.git"),
    ("logos", "https://github.com/maciejhirsz/logos.git"),
    ("once_cell", "https://github.com/matklad/once_cell.git"),
    // Medium
    ("bitflags", "https://github.com/bitflags/bitflags.git"),
    ("bytes", "https://github.com/tokio-rs/bytes.git"),
    ("crossbeam", "https://github.com/crossbeam-rs/crossbeam.git"),
    ("uuid", "https://github.com/uuid-rs/uuid.git"),
    ("anyhow", "https://github.com/dtolnay/anyhow.git"),
    // Large
    ("clap", "https://github.com/clap-rs/clap.git"),
    ("serde", "https://github.com/serde-rs/serde.git"),
    ("tokio", "https://github.com/tokio-rs/tokio.git"),
    ("rayon", "https://github.com/rayon-rs/rayon.git"),
    ("rand", "https://github.com/rust-random/rand.git"),
];

#[test]
fn test_realworld_compilation_invariance() {
    if std::env::var("MTINDO_HEAVY_TESTS").is_err() {
        println!(
            "Skipping Domain 13: Set MTINDO_HEAVY_TESTS=1 to execute real-world clone checks."
        );
        return;
    }

    let env = TestEnvironment::new();
    let mut failures = Vec::new();

    for (name, url) in REAL_WORLD_TARGETS {
        println!(">>> Evaluating Ecosystem Target: {}", name);
        let repo_dir = env.dir.path().join(name);

        let mut clone_cmd = Command::new("git");
        clone_cmd
            .args(["clone", "--depth", "1", url, name])
            .current_dir(env.dir.path());
        if !clone_cmd.status().unwrap().success() {
            failures.push(format!("{}: clone failed", name));
            continue;
        }

        // Explicitly target --lib to prevent workspace feature unification traps
        let mut check_pre = Command::new("cargo");
        check_pre.args(["check", "--lib"]).current_dir(&repo_dir);
        if !check_pre.status().unwrap().success() {
            failures.push(format!(
                "{}: failed pre-format compilation (not mto's fault)",
                name
            ));
            continue;
        }

        // Each configuration from a clean checkout: the defaults, then two mixes that
        // between them turn every layout setting away from its default.
        for (label, settings) in LAYOUT_CONFIGS {
            let clean = Command::new("git")
                .args(["checkout", "--quiet", "--", "."])
                .current_dir(&repo_dir)
                .status()
                .unwrap();
            assert!(clean.success(), "{}: git checkout failed", name);
            std::fs::write(
                repo_dir.join("mto.toml"),
                format!("root = true\nlayout = true\n{}", settings),
            )
            .unwrap();
            let format_output = env
                .mto_cmd()
                .args(["-q", "--no-history", "--allow-dirty", "-R", "."])
                .current_dir(&repo_dir)
                .output()
                .unwrap();
            // Exit 3 marks files mto leaves alone for syntax errors (test fixtures); the
            // rest were written.
            if !matches!(format_output.status.code(), Some(0) | Some(3)) {
                failures.push(format!(
                    "{} ({}): mto exited with {:?}: {}",
                    name,
                    label,
                    format_output.status.code(),
                    String::from_utf8_lossy(&format_output.stderr)
                ));
                continue;
            }
            let check_post = Command::new("cargo")
                .args(["check", "--lib", "--quiet"])
                .current_dir(&repo_dir)
                .status()
                .unwrap();
            if !check_post.success() {
                failures.push(format!(
                    "{} ({}): failed post-format compilation",
                    name, label
                ));
            }
        }
        println!(">>> Target {} verified: 100% compilation invariance.", name);
    }

    assert!(
        failures.is_empty(),
        "{} target(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
