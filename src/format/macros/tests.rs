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

#[test]
fn test_macro_rules_bodies() {
  let messy = "macro_rules!\n    context_type\t{\n\t($name:ident)\t=>\n{\n\n#[derive(Debug)]\nstruct $name { message: &'static str }\n} ;\n    ($a:expr, $b:expr) => ( $a + $b );\n\n    () => {{ let x = 1; x }}\n}\n";
  let tidy = "macro_rules! context_type {\n    ($name:ident) => {\n        #[derive(Debug)]\n        struct $name {\n            message: &'static str,\n        }\n    };\n    ($a:expr, $b:expr) => {\n        $a + $b\n    };\n\n    () => {{\n        let x = 1;\n        x\n    }};\n}\n";
  assert_eq!(formatted(messy), tidy);
  assert_eq!(formatted(tidy), tidy);
}
