use super::*;
use crate::options::layout::{ItemKind, LayoutConfig, NameSort, Visibility};

/// The strings of a TOML array of strings, as written in a default.
fn strings(default: &str) -> Vec<String> {
    default
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[test]
fn test_defaults_are_legal_values() {
    for setting in all() {
        let names: Vec<&str> = setting.choices().iter().map(|c| c.name).collect();
        let defaults = [Some(setting.default), setting.default_2024];
        for default in defaults.into_iter().flatten() {
            match setting.kind {
                Kind::Bool => assert!(default == "true" || default == "false", "{}", setting.name),
                Kind::Integer => assert!(default.parse::<u64>().is_ok(), "{}", setting.name),
                Kind::Text | Kind::Size => assert!(default.starts_with('"'), "{}", setting.name),
                Kind::Choice(_) => assert!(
                    names.contains(&default.trim_matches('"')),
                    "{} = {}",
                    setting.name,
                    default
                ),
                Kind::List => assert!(default.starts_with('['), "{}", setting.name),
                Kind::Sequence(_) => {
                    for item in strings(default) {
                        assert!(names.contains(&item.as_str()), "{}: {}", setting.name, item);
                    }
                }
                Kind::Internal => assert!(default.is_empty(), "{}", setting.name),
            }
        }
    }
}

#[test]
fn test_every_formatting_option_is_in_the_table() {
    // The capabilities reader and the settings generator read the standard formatter's
    // source separately; they must find the same options.
    let surface = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("compat/rustfmt-surface.txt"),
    )
    .unwrap();
    let mut checked = 0;
    for line in surface.lines() {
        let mut words = line.split_whitespace();
        if words.next() != Some("option") {
            continue;
        }
        let name = words.next().unwrap();
        let sources = words.next().unwrap();
        let upstream =
            sources == "in=all" || sources.contains("stable") || sources.contains("nightly");
        if upstream {
            let setting = find(name).unwrap_or_else(|| panic!("`{}` is not in the table", name));
            assert_eq!(setting.file, File::Format, "{}", name);
            checked += 1;
        }
    }
    assert!(checked > 80, "only {} options read", checked);
}

#[test]
fn test_lookup_and_style_edition_defaults() {
    assert_eq!(find("max_width").map(|s| s.default), Some("100"));
    assert_eq!(find("nope"), None);
    let style_edition = find("style_edition").unwrap();
    assert_eq!(style_edition.default_for(2021), "\"2015\"");
    assert_eq!(style_edition.default_for(2024), "\"2024\"");
    assert_eq!(
        find("fn_args_layout").unwrap().replaced_by,
        Some("fn_params_layout")
    );
    assert!(find("type_order").unwrap().channels.is_none());
}

#[test]
fn test_mto_defaults_match_the_layout_config() {
    let config = LayoutConfig::default();
    let default = |name: &str| find(name).unwrap().default;
    assert_eq!(
        default("blank_lines_between_blocks"),
        config.blank_lines_between_blocks.to_string()
    );
    assert_eq!(default("group_impls"), config.group_impls.to_string());
    assert_eq!(
        default("import_grouping"),
        config.import_grouping.to_string()
    );
    assert_eq!(
        default("sort_alphabetically"),
        config.sort_alphabetically.to_string()
    );
    assert_eq!(default("sort_imports"), config.sort_imports.to_string());
    assert_eq!(strings(default("ignore_dirs")), config.ignore_dirs);
    let order: Vec<ItemKind> = strings(default("type_order"))
        .iter()
        .map(|name| ItemKind::from_name(name))
        .collect();
    assert_eq!(order, config.type_order);
    let visibilities: Vec<Visibility> = strings(default("visibility_order"))
        .iter()
        .filter_map(|name| Visibility::from_name(name))
        .collect();
    assert_eq!(visibilities, config.visibility_order);
    assert_eq!(config.name_sort, NameSort::Version);
    assert_eq!(default("name_sort"), "\"version\"");
}

#[test]
fn test_names_are_unique_and_item_kinds_known() {
    let mut names: Vec<&str> = all().map(|s| s.name).collect();
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), count, "a key is listed twice");
    for choice in mto::ITEM_KINDS {
        let kind = ItemKind::from_name(choice.name);
        assert!(
            kind != ItemKind::Unknown || choice.name == "Unknown",
            "{}",
            choice.name
        );
    }
}
