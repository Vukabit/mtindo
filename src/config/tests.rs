use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh folder holding `files` (path, contents), removed when dropped.
struct Tree(PathBuf);

impl Tree {
    fn new(files: &[(&str, &str)]) -> Tree {
        let root = std::env::temp_dir().join(format!(
            "mtindo-config-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        for (path, text) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        Tree(root)
    }

    fn dir(&self, sub: &str) -> PathBuf {
        let dir = self.0.join(sub);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn config(tree: &Tree, sub: &str) -> (Rc<Config>, Vec<String>) {
    config_with(tree, sub, &Overrides::default(), Environment::default())
}

fn config_with(
    tree: &Tree,
    sub: &str,
    overrides: &Overrides,
    env: Environment,
) -> (Rc<Config>, Vec<String>) {
    let mut resolver = Resolver::new(overrides, env).unwrap();
    let config = resolver.config_for(&tree.dir(sub)).unwrap();
    (config, resolver.take_warnings())
}

fn error(tree: &Tree, sub: &str) -> String {
    let mut resolver = Resolver::new(&Overrides::default(), Environment::default()).unwrap();
    resolver.config_for(&tree.dir(sub)).unwrap_err().message
}

#[test]
fn test_blank_lines_stay_within_the_formatting_bounds() {
    let tree = Tree::new(&[(
        "mto.toml",
        "layout = true\nblank_lines_between_blocks = 2\n",
    )]);
    let (config, warnings) = config(&tree, "");
    assert_eq!(config.layout().blank_lines_between_blocks, 1);
    assert!(
        warnings[0].contains(
            "outside `blank_lines_lower_bound` and `blank_lines_upper_bound` (0 to 1), which formatting keeps to; mto uses 1"
        ),
        "{:?}",
        warnings
    );
    let tree = Tree::new(&[
        (
            "rustfmt.toml",
            "blank_lines_upper_bound = 3\nblank_lines_lower_bound = 1\n",
        ),
        (
            "mto.toml",
            "layout = true\nblank_lines_between_blocks = 2\n",
        ),
    ]);
    let (config, warnings) = super::tests::config(&tree, "");
    let layout = config.layout();
    assert_eq!(
        (layout.blank_lines_between_blocks, layout.blank_lines_tight),
        (2, 1)
    );
    assert!(warnings.is_empty(), "{:?}", warnings);
}

#[test]
fn test_cargo_edition_sets_the_style_edition() {
    let tree = Tree::new(&[(
        "Cargo.toml",
        "[package]\nname = \"x\"\nedition = \"2024\"\n",
    )]);
    let (config, _) = config(&tree, "src");
    assert_eq!(config.style_edition(), 2024);
    assert_eq!(
        config.source("style_edition"),
        Some(&Source::Derived("edition"))
    );
    assert_eq!(
        text(&config, "version"),
        "\"Two\"",
        "defaults follow the style edition"
    );
    assert_eq!(
        config.source("edition"),
        Some(&Source::File {
            path: tree.0.join("Cargo.toml"),
            line: 3
        })
    );
    // An explicit style edition wins over the crate's edition.
    fs::write(tree.0.join("rustfmt.toml"), "style_edition = \"2021\"\n").unwrap();
    let (config, _) = super::tests::config(&tree, "src");
    assert_eq!(config.style_edition(), 2021);
    assert_eq!(text(&config, "edition"), "\"2024\"");
}

#[test]
fn test_cargo_workspace_edition_and_bad_manifests() {
    let tree = Tree::new(&[
        (
            "Cargo.toml",
            "[workspace]\nmembers = [\"a\"]\n[workspace.package]\nedition = \"2021\"\n",
        ),
        (
            "a/Cargo.toml",
            "[package]\nname = \"a\"\nedition.workspace = true\n",
        ),
        ("b/Cargo.toml", "[package\n"),
    ]);
    let (config, _) = config(&tree, "a/src");
    assert_eq!(config.style_edition(), 2021);
    let (config, warnings) = super::tests::config(&tree, "b");
    assert_eq!(config.style_edition(), 2015);
    assert!(warnings[0].contains("b/Cargo.toml:1:"), "{:?}", warnings);
}

#[test]
fn test_command_line_wins_and_is_checked() {
    let tree = Tree::new(&[("rustfmt.toml", "max_width = 90\n")]);
    let overrides = Overrides {
        settings: vec![
            ("max_width".into(), "80".into()),
            ("newline_style".into(), "unix".into()),
            ("type_order".into(), "[\"FnDecl\"]".into()),
        ],
        style_edition: Some("2024".into()),
        ..Overrides::default()
    };
    let (config, _) = config_with(&tree, "", &overrides, Environment::default());
    assert_eq!(text(&config, "max_width"), "80");
    assert_eq!(config.source("max_width"), Some(&Source::CommandLine));
    assert_eq!(text(&config, "newline_style"), "\"Unix\"");
    assert_eq!(config.style_edition(), 2024);
    assert_eq!(config.layout().type_order, vec![ItemKind::FnDecl]);

    let bad = |key: &str, value: &str| {
        let overrides = Overrides {
            settings: vec![(key.into(), value.into())],
            ..Overrides::default()
        };
        Resolver::new(&overrides, Environment::default())
            .err()
            .unwrap()
            .message
    };
    assert_eq!(
        bad("style-edition", "2024"),
        "--config: unknown key `style-edition` (did you mean `style_edition`?)"
    );
    assert!(bad("max_width", "wide").contains("must be a whole number"));
    assert!(bad("newline_style", "dos").contains("`Auto`, `Windows`, `Unix` or `Native`"));
}

#[test]
fn test_config_path_replaces_the_search() {
    let tree = Tree::new(&[
        ("rustfmt.toml", "max_width = 90\n"),
        ("mto.toml", "group_impls = false\n"),
        ("other.toml", "hard_tabs = true\nsort_imports = false\n"),
    ]);
    let overrides = Overrides {
        config_path: Some(tree.0.join("other.toml")),
        ..Overrides::default()
    };
    let (config, _) = config_with(&tree, "", &overrides, Environment::default());
    assert_eq!(text(&config, "max_width"), "100");
    assert_eq!(text(&config, "group_impls"), "true");
    assert_eq!(text(&config, "hard_tabs"), "true");
    assert_eq!(text(&config, "sort_imports"), "false");
    assert_eq!(
        config.files(),
        &[(FileKind::Explicit, tree.0.join("other.toml"))]
    );
}

#[test]
fn test_defaults_without_any_file() {
    let tree = Tree::new(&[]);
    let (config, warnings) = config(&tree, "");
    assert!(warnings.is_empty());
    assert!(config.files().is_empty());
    assert_eq!(config.style_edition(), 2015);
    assert_eq!(text(&config, "max_width"), "100");
    assert_eq!(config.source("max_width"), Some(&Source::Default));
    assert_eq!(
        config.get("file_lines"),
        None,
        "internal settings have no value"
    );
    let layout = config.layout();
    let default = LayoutConfig::default();
    assert_eq!(layout.type_order, default.type_order);
    assert_eq!(layout.ignore_dirs, default.ignore_dirs);
}

#[test]
fn test_invalid_values_are_errors_at_their_place() {
    let tree = Tree::new(&[("a/rustfmt.toml", "\nnewline_style = \"dos\"\n")]);
    let message = error(&tree, "a");
    assert!(message.ends_with("rustfmt.toml:2:1: `newline_style` cannot be `dos`; legal values: `Auto`, `Windows`, `Unix` or `Native`"), "{}", message);
    let tree = Tree::new(&[("mto.toml", "max_width = -1\n")]);
    assert!(error(&tree, "").contains("must be zero or more"));
    let tree = Tree::new(&[("mto.toml", "type_order = [\"FnDecl\", \"Bogus\"]\n")]);
    assert!(error(&tree, "").contains("`type_order` cannot be `Bogus`; legal values: `ModDecl`"));
    let tree = Tree::new(&[("mto.toml", "type_order = [\"FnDecl\", \"macrocall\"]\n")]);
    assert!(error(&tree, "").contains(
        "`type_order` cannot hold `MacroCall`: `extern crate` items and item macro calls never move"
    ));
    let tree = Tree::new(&[("mto.toml", "type_order = [\"FnDecl\", \"fndecl\"]\n")]);
    assert!(error(&tree, "").contains("lists `FnDecl` twice"));
    let tree = Tree::new(&[("mto.toml", "max_width = \n")]);
    assert!(error(&tree, "").contains("mto.toml:1:13: expected a value"));
}

#[test]
fn test_mto_files_inherit_until_root() {
    let tree = Tree::new(&[
        ("mto.toml", "sort_imports = false\n"),
        ("a/mto.toml", "root = true\ngroup_impls = false\n"),
        ("a/b/.mto.toml", "blank_lines_between_blocks = 2\n"),
        ("a/b/mto.toml", "blank_lines_between_blocks = 3\n"),
    ]);
    let (config, warnings) = config(&tree, "a/b/c");
    assert_eq!(text(&config, "blank_lines_between_blocks"), "2");
    assert_eq!(text(&config, "group_impls"), "false");
    assert_eq!(
        text(&config, "sort_imports"),
        "true",
        "the root file stops the search"
    );
    assert_eq!(
        config.files(),
        &[
            (FileKind::Mto, tree.0.join("a/mto.toml")),
            (FileKind::Mto, tree.0.join("a/b/.mto.toml")),
        ]
    );
    assert!(warnings[0].starts_with("both "), "{:?}", warnings);
    let (config, _) = super::tests::config(&tree, "x");
    assert_eq!(text(&config, "sort_imports"), "false");
    assert_eq!(text(&config, "group_impls"), "true");
}

#[test]
fn test_mto_toml_wins_over_the_standard_file_with_a_warning() {
    let tree = Tree::new(&[
        ("rustfmt.toml", "max_width = 80\nhard_tabs = true\n"),
        ("mto.toml", "max_width = 90\nhard_tabs = true\n"),
    ]);
    let (config, warnings) = config(&tree, "");
    assert_eq!(text(&config, "max_width"), "90");
    assert_eq!(warnings.len(), 1, "{:?}", warnings);
    assert!(
        warnings[0].starts_with("`max_width` is 90 in "),
        "{}",
        warnings[0]
    );
    assert_eq!(config.layout().max_width, 90);
}

#[test]
fn test_the_standard_file_is_found_as_the_standard_formatter_finds_it() {
    let tree = Tree::new(&[
        ("rustfmt.toml", "max_width = 80\n"),
        ("a/rustfmt.toml", "hard_tabs = true\n"),
        ("a/.rustfmt.toml", "tab_spaces = 2\n"),
        ("home/rustfmt.toml", "max_width = 70\n"),
        ("config/rustfmt/rustfmt.toml", "max_width = 60\n"),
    ]);
    // The nearest file is used alone: the standard formatter does not merge.
    let (config, _) = config(&tree, "a/b");
    assert_eq!(text(&config, "max_width"), "100");
    assert_eq!(
        text(&config, "tab_spaces"),
        "2",
        ".rustfmt.toml comes first"
    );
    assert_eq!(text(&config, "hard_tabs"), "false");

    let outside = Tree::new(&[]);
    let mut env = Environment {
        home: Some(tree.0.join("home")),
        os_config_dir: Some(tree.0.join("config")),
        ..Environment::default()
    };
    let (config, _) = config_with(&outside, "", &Overrides::default(), env.clone());
    assert_eq!(text(&config, "max_width"), "70");
    env.home = None;
    let (config, _) = config_with(&outside, "", &Overrides::default(), env);
    assert_eq!(text(&config, "max_width"), "60");
}

#[test]
fn test_user_config_holds_no_project_settings() {
    let tree = Tree::new(&[("user.toml", "max_width = 80\n[history]\nkeep = 1\n")]);
    let env = Environment {
        mtindo_config: Some(tree.0.join("user.toml")),
        ..Environment::default()
    };
    let (config, warnings) = config_with(&tree, "", &Overrides::default(), env);
    assert_eq!(text(&config, "max_width"), "100");
    assert_eq!(
        config.files(),
        &[(FileKind::User, tree.0.join("user.toml"))]
    );
    assert!(
        warnings[0]
            .contains("`max_width` cannot be set in the user config; put it in rustfmt.toml"),
        "{:?}",
        warnings
    );
    assert!(
        warnings[1].contains("unknown key `history.keep`"),
        "{:?}",
        warnings
    );
}

#[test]
fn test_user_config_location_and_template() {
    let tree = Tree::new(&[]);
    let env = Environment {
        home: Some(tree.0.clone()),
        ..Environment::default()
    };
    assert_eq!(
        user::path(&env).unwrap(),
        Some(tree.0.join(".config/mtindo/config.toml"))
    );
    let env = Environment {
        home: Some(tree.0.clone()),
        xdg_config_home: Some(tree.0.join("xdg")),
        ..Environment::default()
    };
    assert_eq!(
        user::path(&env).unwrap(),
        Some(tree.0.join("xdg/mtindo/config.toml"))
    );
    let env = Environment {
        mtindo_config: Some(tree.0.join("missing.toml")),
        ..Environment::default()
    };
    assert!(user::path(&env).unwrap_err().message.contains("not a file"));
    let env = Environment {
        home: Some(tree.0.clone()),
        ..Environment::default()
    };
    let created = tree.0.join(".config/mtindo/config.toml");
    assert_eq!(user::create_if_missing(&env), Ok(Some(created.clone())));
    // Never twice, and never over a file.
    assert_eq!(user::create_if_missing(&env), Ok(None));
    let template = fs::read_to_string(&created).unwrap();
    assert!(template.starts_with("# mto's user configuration"));
    assert!(
        template.contains("[history]\n# Record every run"),
        "{}",
        template
    );
    assert!(template.contains("# keep_days = 30\n"), "{}", template);
    // Every line is a comment, a section or blank: the defaults stay the program's.
    let table = toml::parse(&template).unwrap();
    assert!(
        table
            .entries()
            .iter()
            .all(|e| matches!(&e.value, toml::Value::Table(t) if t.entries().is_empty()))
    );
    // The CI never gets one.
    let ci = Environment { ci: true, ..env };
    fs::remove_file(&created).unwrap();
    assert_eq!(user::create_if_missing(&ci), Ok(None));
}

#[test]
fn test_version_two_means_style_edition_2024() {
    let tree = Tree::new(&[("rustfmt.toml", "version = \"Two\"\n")]);
    let (config, _) = config(&tree, "");
    assert_eq!(config.style_edition(), 2024);
    assert_eq!(
        config.source("style_edition"),
        Some(&Source::Derived("version"))
    );
}

#[test]
fn test_wrong_file_unknown_and_deprecated_keys_warn() {
    let tree = Tree::new(&[(
        "rustfmt.toml",
        "type_order = [\"FnDecl\"]\nmax_widht = 80\nfn_args_layout = \"Compressed\"\nNewline_Style = \"x\"\n",
    )]);
    let (config, warnings) = config(&tree, "");
    assert_eq!(
        config.layout().type_order,
        LayoutConfig::default().type_order
    );
    assert_eq!(warnings.len(), 4, "{:#?}", warnings);
    assert!(warnings[0].ends_with("rustfmt.toml:1:1: `type_order` cannot be set in rustfmt.toml; put it in mto.toml; ignored"), "{}", warnings[0]);
    assert!(warnings[1].contains("unknown key `max_widht` (did you mean `max_width`?)"));
    assert!(warnings[2].contains("`fn_args_layout` is deprecated; use `fn_params_layout`"));
    assert!(
        warnings[3].contains("unknown key `Newline_Style` (did you mean `newline_style`?)"),
        "{}",
        warnings[3]
    );
}

fn text(config: &Config, name: &str) -> String {
    config.get(name).map(Value::to_toml).unwrap_or_default()
}
