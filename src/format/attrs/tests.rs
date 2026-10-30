use crate::format::file::format_file;
use crate::options::format::FormatConfig;
use crate::syntax::Edition;
use crate::verify::format::verify_format;

/// `source` formatted, checked for meaning and stability.
fn formatted(source: &str) -> String {
  let config = FormatConfig::default();
  let once = format_file(source, Edition::E2021, &config);
  let found = verify_format(source, &once, Edition::E2021);
  assert!(found.is_empty(), "{:?}\n{}", found, once);
  assert_eq!(
    format_file(&once, Edition::E2021, &config),
    once,
    "not stable"
  );
  once
}

/// A derive's names are joined token to token only when they are paths: an identifier
/// joined to a string would read as a reserved prefix (`Par"x"`). Found by the format fuzz
/// target.
#[test]
fn test_a_derive_of_anything_but_paths_keeps_its_spaces() {
  let out = formatted("#[derive(Debug, Clone, Par \"x\",\n    y = \"z\"\n)]\nstruct S;\n");
  assert!(out.contains("Par \"x\""), "{}", out);
  let out = formatted("#[derive(Debug, A B)]\nstruct S;\n");
  assert!(out.contains("A B"), "{}", out);
  assert_eq!(
    formatted("#[derive( serde :: Serialize ,crate::Thing,r#Raw)]\nstruct S;\n"),
    "#[derive(serde::Serialize, crate::Thing, r#Raw)]\nstruct S;\n"
  );
}

#[test]
fn test_attributes_join_onto_one_line() {
  assert_eq!(
    formatted(
      "#[\nderive(\nDebug,Clone)\n]\n#[doc =\n\"x\"]\n#[  allow(dead_code )  ]\nfn f() {}\n"
    ),
    "#[derive(Debug, Clone)]\n#[doc = \"x\"]\n#[allow(dead_code)]\nfn f() {}\n"
  );
  assert_eq!(
    formatted("#![cfg(feature  =\t\"sync\")] use a::b;\n"),
    "#![cfg(feature = \"sync\")]\nuse a::b;\n"
  );
}

#[test]
fn test_attributes_left_as_written() {
  for kept in [
    "#[attr(is = | _ | false)]\nfn f() {}\n",
    "#[cfg(any(\n    unix, // why\n    windows\n))]\nfn f() {}\n",
  ] {
    assert_eq!(formatted(kept), kept);
  }
}

#[test]
fn test_long_attributes_break_as_the_standard_formatter_breaks_them() {
  // Derives on one row inside when it fits, else one per line, with commas.
  let row = "#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Zzzzzzzz)]\nstruct A;\n";
  assert_eq!(
    formatted(row),
    "#[derive(\n    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Zzzzzzzz,\n)]\nstruct A;\n"
  );
  let names = [
    "Debug",
    "Clone",
    "Copy",
    "PartialEq",
    "Eq",
    "Default",
    "serde::Serialize",
    "Zzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    "Yyyyyyyyyyyy",
  ];
  let source = format!("#[derive({})]\nstruct B;\n", names.join(", "));
  let lines: String = names.iter().map(|n| format!("    {},\n", n)).collect();
  assert_eq!(
    formatted(&source),
    format!("#[derive(\n{})]\nstruct B;\n", lines)
  );
  // Other lists as calls, a trailing comma only where written.
  assert_eq!(
    formatted(
      "#[cfg_attr(feature = \"serde\", derive(serde::Serialize, serde::Deserialize), serde(rename_all = \"camelCase\"))]\nstruct D;\n"
    ),
    "#[cfg_attr(\n    feature = \"serde\",\n    derive(serde::Serialize, serde::Deserialize),\n    serde(rename_all = \"camelCase\")\n)]\nstruct D;\n"
  );
  assert_eq!(
    formatted(
      "#[cfg(all(feature = \"aaaaaaaaaaaaaaaa\", feature = \"bbbbbbbbbbbbbbbbbbbbb\", not(target_os = \"windows\"),))]\nstruct C;\n"
    ),
    "#[cfg(all(\n    feature = \"aaaaaaaaaaaaaaaa\",\n    feature = \"bbbbbbbbbbbbbbbbbbbbb\",\n    not(target_os = \"windows\"),\n))]\nstruct C;\n"
  );
}

/// An unsafe attribute is laid out inside its `unsafe(` and `)`; a derive in one, which the
/// standard formatter writes without its `unsafe`, is left as written.
#[test]
fn test_unsafe_attributes_are_laid_out_inside_their_wrapper() {
  assert_eq!(
    formatted("#![unsafe(b\n=\n\"\")]\n#![unsafe(c(d\n,e))]\n"),
    "#![unsafe(b = \"\")]\n#![unsafe(c(d, e))]\n"
  );
  assert_eq!(
    formatted("#[unsafe(derive(A\n,B))]\nstruct S;\n"),
    "#[unsafe(derive(A\n,B))]\nstruct S;\n"
  );
}
