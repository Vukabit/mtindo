mod common;

use std::process::Command;

use common::TestEnvironment;

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

        let mut format_cmd = env.mto_cmd();
        format_cmd.arg("-R").arg(".").current_dir(&repo_dir);
        let format_output = format_cmd.output().unwrap();
        if !format_output.status.success() {
            failures.push(format!(
                "{}: mto exited with {:?}",
                name,
                format_output.status.code()
            ));
            continue;
        }

        let mut check_post = Command::new("cargo");
        check_post.args(["check", "--lib"]).current_dir(&repo_dir);
        if !check_post.status().unwrap().success() {
            failures.push(format!("{}: failed post-format compilation", name));
            continue;
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
