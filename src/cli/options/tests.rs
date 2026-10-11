use super::*;

#[test]
fn test_every_setting_is_listed_under_its_file() {
  let listing = render(false);
  for setting in options::all().filter(|s| !s.hidden && s.kind != Kind::Internal) {
    assert!(
      listing
        .lines()
        .any(|l| l.split_whitespace().next() == Some(setting.name)),
      "{} is not listed",
      setting.name
    );
  }
  let mto = listing.find("mto.toml (").unwrap();
  let format = listing.find("rustfmt.toml (").unwrap();
  let user = listing.find("config.toml (").unwrap();
  assert!(mto < format && format < user);
  // The standard formatter's settings say whether mto honors them.
  let row = |name: &str| {
    listing
      .lines()
      .find(|l| l.split_whitespace().next() == Some(name))
      .unwrap()
      .to_string()
  };
  assert!(row("max_width").ends_with("partly"), "{}", row("max_width"));
  assert!(
    row("newline_style").ends_with("yes"),
    "{}",
    row("newline_style")
  );
  assert!(row("layout").contains("true | false"), "{}", row("layout"));
}

#[test]
fn test_status_table_is_sorted_for_lookup() {
  assert!(support::OPTIONS.windows(2).all(|w| w[0].0 < w[1].0));
}

#[test]
fn test_the_toml_form_is_every_setting_commented_out_at_its_default() {
  let toml = render(true);
  assert!(toml.contains("\n# layout = false\n"), "{}", toml);
  assert!(toml.contains("\n# max_width = 100\n"), "{}", toml);
  assert!(toml.lines().all(|l| l.is_empty() || l.starts_with('#')));
}
