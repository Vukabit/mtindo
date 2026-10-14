//! Rust source built from fuzz bytes: items, statements, expressions, types and patterns,
//! nested to a few levels, with spaces, line breaks and comments placed at random between
//! the tokens. Byte mutation alone rarely gets past the parser into the formatter's deeper
//! rules; source made this way always parses, so every input reaches them.
//!
//! The bytes are read as a sequence of choices ([`Choices`]); once they run out every
//! choice is the first one, which is always the shortest, so any input gives a finite file
//! and a longer input a bigger one. Whatever the choices, the text is valid Rust 2021:
//! identifiers are never keywords, a struct literal never stands where a block may start,
//! operands that would chain comparisons or casts are parenthesized, and a line comment
//! always ends its line.

const COMMENTS: &[&str] = &["/* c */", "/* a longer note */", "/**/"];

/// How deep expressions, types and modules nest.
const DEPTH: usize = 4;

const FUNCTIONS: &[&str] = &["run", "parse", "new", "len", "get", "map", "with_capacity"];

/// The most items at one level.
const ITEMS: usize = 10;

const LINE_COMMENTS: &[&str] = &["// note", "// TODO: more", "//", "// x = 1"];

const MACROS: &[&str] = &["println", "vec", "format", "assert_eq", "matches", "write"];

const PRIMITIVES: &[&str] = &["i32", "u8", "usize", "f64", "bool", "char", "str", "String"];

/// The most statements in one block.
const STATEMENTS: usize = 6;

const TRAITS: &[&str] = &["Display", "Clone", "Iterator", "Shape", "From<u8>"];

const TYPES: &[&str] = &["Foo", "Bar", "Point", "Error", "State", "Config"];

const VALUES: &[&str] = &[
  "a", "b", "x", "y", "value", "item", "data", "count", "first",
];

/// The fuzz bytes, read as choices.
pub struct Choices<'a> {
  bytes: &'a [u8],
  at: usize,
}

impl<'a> Choices<'a> {
  pub fn new(bytes: &'a [u8]) -> Choices<'a> {
    Choices { bytes, at: 0 }
  }

  fn byte(&mut self) -> u8 {
    let byte = self.bytes.get(self.at).copied().unwrap_or(0);
    self.at += 1;
    byte
  }

  /// A number below `n`; 0 once the bytes run out.
  fn below(&mut self, n: usize) -> usize {
    if n <= 1 {
      return 0;
    }
    usize::from(self.byte()) % n
  }

  /// True `per_256` times in 256; false once the bytes run out.
  fn chance(&mut self, per_256: u8) -> bool {
    self.at < self.bytes.len() && self.byte() < per_256
  }

  /// One of `items`.
  fn pick<T: Copy>(&mut self, items: &[T]) -> T {
    items[self.below(items.len())]
  }

  fn exhausted(&self) -> bool {
    self.at >= self.bytes.len()
  }
}

struct Gen<'a> {
  c: Choices<'a>,
  out: String,
  /// Levels of indentation at the current line, for line breaks.
  indent: usize,
}

impl Gen<'_> {
  fn word(&mut self, text: &str) {
    self.out.push_str(text);
  }

  /// One of `texts`.
  fn one(&mut self, texts: &[&str]) {
    let text = self.c.pick(texts);
    self.out.push_str(text);
  }

  /// What separates two tokens that must not touch: mostly one space, sometimes more,
  /// sometimes a line break, sometimes a comment.
  fn space(&mut self) {
    match self.c.below(16) {
      0..=10 => self.out.push(' '),
      11 => self.out.push_str("  "),
      12 | 13 => self.newline(),
      14 => {
        let comment = self.c.pick(COMMENTS);
        self.out.push(' ');
        self.out.push_str(comment);
        self.out.push(' ');
      }
      _ => {
        let comment = self.c.pick(LINE_COMMENTS);
        self.out.push(' ');
        self.out.push_str(comment);
        self.newline();
      }
    }
  }

  /// What may separate two tokens that can touch: usually nothing.
  fn gap(&mut self) {
    if self.c.chance(48) {
      self.space();
    }
  }

  fn newline(&mut self) {
    self.out.push('\n');
    let width = self.indent * 4 + self.c.below(3);
    for _ in 0..width {
      self.out.push(' ');
    }
  }

  fn value_name(&mut self) -> &'static str {
    self.c.pick(VALUES)
  }

  fn type_name(&mut self) -> &'static str {
    self.c.pick(TYPES)
  }

  // Items.

  fn items(&mut self, depth: usize) {
    let count = 1 + self.c.below(ITEMS);
    for n in 0..count {
      if n > 0 {
        self.out.push('\n');
        if self.c.chance(160) {
          self.out.push('\n');
        }
        self.newline_here();
      }
      self.item(depth);
      if self.c.exhausted() {
        break;
      }
    }
  }

  /// A line break at the current indentation, nothing more.
  fn newline_here(&mut self) {
    for _ in 0..self.indent * 4 {
      self.out.push(' ');
    }
  }

  fn attributes(&mut self) {
    if self.c.chance(50) {
      let doc = self
        .c
        .pick(&["/// Does a thing.", "/// Two\n/// lines.", "/** Block. */"]);
      self.word(doc);
      self.out.push('\n');
      self.newline_here();
    }
    if self.c.chance(50) {
      let attr = self.c.pick(&[
        "#[derive(Debug, Clone)]",
        "#[inline]",
        "#[cfg(test)]",
        "#[allow(unused)]",
        "#[cfg(feature = \"std\")]",
        "#[rustfmt::skip]",
        "#[must_use]",
      ]);
      self.word(attr);
      self.space();
    }
  }

  fn visibility(&mut self) {
    match self.c.below(5) {
      1 | 2 => {
        self.word("pub");
        self.space();
      }
      3 => {
        self.word("pub(crate)");
        self.space();
      }
      _ => {}
    }
  }

  fn item(&mut self, depth: usize) {
    self.attributes();
    match self.c.below(14) {
      0 => self.function(depth, false),
      1 => self.structure(),
      2 => self.enumeration(),
      3 => self.implementation(depth),
      4 => self.use_item(),
      5 => self.constant(depth),
      6 if depth < 2 => self.module(depth),
      6 => self.word("mod parts;"),
      7 => self.alias(),
      8 => self.trait_item(),
      9 => self.macro_rules(),
      10 => {
        let name = self.c.pick(&["thread_local", "lazy", "define"]);
        self.word(name);
        self.word("! { ");
        self.expression(depth + 2, false);
        self.word(" }");
      }
      11 => self.function(depth, true),
      12 => {
        self.visibility();
        self.word("static");
        self.space();
        self.word("LIMIT: usize = ");
        self.expression(depth + 2, false);
        self.word(";");
      }
      _ => self.structure(),
    }
  }

  fn function(&mut self, depth: usize, in_impl: bool) {
    self.visibility();
    if self.c.chance(30) {
      self.word("const");
      self.space();
    }
    self.word("fn");
    self.space();
    let name = self.c.pick(FUNCTIONS);
    self.word(name);
    if self.c.chance(50) {
      self.word("<T: Clone>");
    }
    self.word("(");
    self.gap();
    let params = self.c.below(4);
    for n in 0..params {
      if n > 0 {
        self.word(",");
        self.space();
      }
      if n == 0 && in_impl && self.c.chance(160) {
        let receiver = self.c.pick(&["self", "&self", "&mut self", "mut self"]);
        self.word(receiver);
        continue;
      }
      let name = self.value_name();
      self.word(name);
      self.word(":");
      self.gap();
      self.ty(depth + 1);
    }
    if params > 0 && self.c.chance(40) {
      self.word(",");
    }
    self.word(")");
    if self.c.chance(128) {
      self.space();
      self.word("->");
      self.space();
      self.ty(depth + 1);
    }
    if self.c.chance(30) {
      self.space();
      self.word("where T: Default");
    }
    self.space();
    self.block(depth + 1);
  }

  fn structure(&mut self) {
    self.visibility();
    self.word("struct");
    self.space();
    let name = self.type_name();
    self.word(name);
    match self.c.below(3) {
      0 => self.word(";"),
      1 => {
        self.word("(");
        let count = 1 + self.c.below(3);
        for n in 0..count {
          if n > 0 {
            self.word(",");
            self.space();
          }
          self.visibility();
          self.ty(2);
        }
        self.word(");");
      }
      _ => {
        self.space();
        self.word("{");
        self.fields();
        self.word("}");
      }
    }
  }

  fn fields(&mut self) {
    self.indent += 1;
    let count = self.c.below(4);
    for _ in 0..count {
      self.newline();
      if self.c.chance(40) {
        self.word("/// A field.");
        self.newline();
      }
      self.visibility();
      let name = self.value_name();
      self.word(name);
      self.word(":");
      self.gap();
      self.ty(2);
      self.word(",");
    }
    self.indent -= 1;
    self.newline_here_after_break();
  }

  /// A line break, then the current indentation.
  fn newline_here_after_break(&mut self) {
    self.out.push('\n');
    self.newline_here();
  }

  fn enumeration(&mut self) {
    self.visibility();
    self.word("enum");
    self.space();
    let name = self.type_name();
    self.word(name);
    self.space();
    self.word("{");
    self.indent += 1;
    let count = 1 + self.c.below(4);
    for n in 0..count {
      self.newline();
      self.word(["A", "B", "C", "D"][n]);
      match self.c.below(4) {
        1 => {
          self.word("(");
          self.ty(2);
          self.word(")");
        }
        2 => {
          self.space();
          self.word("{ x:");
          self.gap();
          self.ty(2);
          self.word(" }");
        }
        3 => {
          self.word(" = ");
          let n = self.c.below(100);
          self.word(&n.to_string());
        }
        _ => {}
      }
      self.word(",");
    }
    self.indent -= 1;
    self.newline_here_after_break();
    self.word("}");
  }

  fn implementation(&mut self, depth: usize) {
    self.word("impl");
    if self.c.chance(40) {
      self.word("<T>");
    }
    self.space();
    if self.c.chance(128) {
      let name = self.c.pick(TRAITS);
      self.word(name);
      self.space();
      self.word("for");
      self.space();
    }
    let name = self.type_name();
    self.word(name);
    self.space();
    self.word("{");
    self.indent += 1;
    let count = self.c.below(4);
    for _ in 0..count {
      self.newline();
      match self.c.below(4) {
        0 => {
          self.word("type Item = ");
          self.ty(depth + 2);
          self.word(";");
        }
        1 => {
          self.word("const N: usize = ");
          self.expression(depth + 2, false);
          self.word(";");
        }
        _ => {
          self.attributes();
          self.function(depth + 1, true);
        }
      }
    }
    self.indent -= 1;
    self.newline_here_after_break();
    self.word("}");
  }

  fn trait_item(&mut self) {
    self.visibility();
    self.word("trait");
    self.space();
    let name = self.c.pick(&["Shape", "Source", "Visit"]);
    self.word(name);
    self.space();
    self.word("{");
    self.indent += 1;
    let count = self.c.below(3);
    for _ in 0..count {
      self.newline();
      let name = self.c.pick(FUNCTIONS);
      self.word("fn ");
      self.word(name);
      self.word("(&self)");
      if self.c.chance(128) {
        self.word(" -> ");
        self.ty(2);
      }
      if self.c.chance(80) {
        self.space();
        self.block(3);
      } else {
        self.word(";");
      }
    }
    self.indent -= 1;
    self.newline_here_after_break();
    self.word("}");
  }

  fn use_item(&mut self) {
    self.visibility();
    self.word("use");
    self.space();
    let root = self
      .c
      .pick(&["std", "crate", "super", "self", "core", "serde"]);
    self.word(root);
    self.word("::");
    self.use_tree(0);
    self.word(";");
  }

  fn use_tree(&mut self, depth: usize) {
    match self.c.below(if depth < 2 { 4 } else { 2 }) {
      0 => {
        let name = self.c.pick(&["fmt", "io", "Foo", "path", "collections"]);
        self.word(name);
        if self.c.chance(40) {
          self.word(" as Alias");
        }
      }
      1 => self.word("*"),
      2 => {
        let name = self.c.pick(&["fmt", "io", "sync"]);
        self.word(name);
        self.word("::");
        self.use_tree(depth + 1);
      }
      _ => {
        self.word("{");
        self.gap();
        let count = 1 + self.c.below(3);
        for n in 0..count {
          if n > 0 {
            self.word(",");
            self.space();
          }
          self.use_tree(depth + 1);
        }
        if self.c.chance(40) {
          self.word(",");
        }
        self.gap();
        self.word("}");
      }
    }
  }

  fn constant(&mut self, depth: usize) {
    self.visibility();
    self.word("const");
    self.space();
    let name = self.c.pick(&["MAX", "NAME", "SIZE"]);
    self.word(name);
    self.word(":");
    self.gap();
    self.ty(depth + 2);
    self.space();
    self.word("=");
    self.space();
    self.expression(depth + 1, false);
    self.word(";");
  }

  fn module(&mut self, depth: usize) {
    self.visibility();
    self.word("mod");
    self.space();
    let name = self.c.pick(&["tests", "inner", "parts"]);
    self.word(name);
    self.space();
    self.word("{");
    self.indent += 1;
    self.out.push('\n');
    self.newline_here();
    self.items(depth + 1);
    self.indent -= 1;
    self.newline_here_after_break();
    self.word("}");
  }

  fn alias(&mut self) {
    self.visibility();
    self.word("type");
    self.space();
    let name = self.type_name();
    self.word(name);
    self.word("Alias");
    self.space();
    self.word("=");
    self.space();
    self.ty(1);
    self.word(";");
  }

  fn macro_rules(&mut self) {
    if self.c.chance(80) {
      self.word("#[macro_export]");
      self.out.push('\n');
      self.newline_here();
    }
    let name = self.c.pick(&["square", "make", "both"]);
    self.word("macro_rules! ");
    self.word(name);
    self.word(" {");
    self.one(&[
      " ($x:expr) => { $x * $x }; ",
      "\n    () => {};\n    ($($t:tt)*) => { $($t)* };\n",
      " ( $a:ident , $b:ty ) => {  let $a : $b = Default::default(); } ",
    ]);
    self.word("}");
  }

  // Types.

  fn ty(&mut self, depth: usize) {
    if depth >= DEPTH {
      let primitive = self.c.pick(PRIMITIVES);
      return self.word(if primitive == "str" {
        "&str"
      } else {
        primitive
      });
    }
    match self.c.below(10) {
      0 => {
        let primitive = self.c.pick(PRIMITIVES);
        self.word(if primitive == "str" {
          "&str"
        } else {
          primitive
        });
      }
      1 => {
        let name = self.c.pick(&["Vec", "Option", "Box", "Rc"]);
        self.word(name);
        self.word("<");
        self.ty(depth + 1);
        self.word(">");
      }
      2 => {
        self.word("&");
        if self.c.chance(80) {
          self.word("'a ");
        }
        if self.c.chance(80) {
          self.word("mut ");
        }
        self.ty(depth + 1);
      }
      3 => {
        self.word("(");
        self.ty(depth + 1);
        self.word(",");
        self.gap();
        self.ty(depth + 1);
        self.word(")");
      }
      4 => {
        self.word("[");
        self.ty(depth + 1);
        self.word("; 4]");
      }
      5 => {
        self.word("HashMap<");
        self.ty(depth + 1);
        self.word(",");
        self.space();
        self.ty(depth + 1);
        self.word(">");
      }
      6 => {
        self.word("impl ");
        let name = self
          .c
          .pick(&["Iterator<Item = u8>", "Fn(u8) -> bool", "Display"]);
        self.word(name);
      }
      7 => {
        self.word("fn(");
        self.ty(depth + 1);
        self.word(") -> ");
        self.ty(depth + 1);
      }
      8 => self.word("Box<dyn Error + Send>"),
      _ => {
        let name = self.type_name();
        self.word(name);
      }
    }
  }

  // Statements and blocks.

  fn block(&mut self, depth: usize) {
    self.word("{");
    self.indent += 1;
    let count = if depth >= DEPTH {
      0
    } else {
      self.c.below(STATEMENTS)
    };
    for _ in 0..count {
      self.newline();
      self.statement(depth);
      if self.c.exhausted() {
        break;
      }
    }
    if depth < DEPTH && self.c.chance(80) {
      self.newline();
      self.expression(depth + 1, false);
    }
    self.indent -= 1;
    if count > 0 || self.c.chance(128) {
      self.newline_here_after_break();
    }
    self.word("}");
  }

  fn statement(&mut self, depth: usize) {
    match self.c.below(7) {
      0 | 1 => {
        self.word("let");
        self.space();
        if self.c.chance(60) {
          // `mut` binds a name, never a pattern.
          self.word("mut ");
          let name = self.value_name();
          self.word(name);
        } else {
          self.pattern(depth + 1, false);
        }
        if self.c.chance(80) {
          self.word(":");
          self.gap();
          self.ty(depth + 1);
        }
        self.space();
        self.word("=");
        self.space();
        self.expression(depth + 1, false);
        self.word(";");
      }
      2 => {
        let name = self.c.pick(MACROS);
        self.word(name);
        self.word("!(");
        self.word("\"{}\",");
        self.gap();
        self.expression(depth + 1, false);
        self.word(");");
      }
      3 if depth + 1 < DEPTH => {
        self.word("for");
        self.space();
        self.pattern(depth + 1, false);
        self.space();
        self.word("in");
        self.space();
        self.expression(depth + 1, true);
        self.space();
        self.block(depth + 1);
      }
      4 if depth + 1 < DEPTH => {
        self.word("if");
        self.space();
        self.expression(depth + 1, true);
        self.space();
        self.block(depth + 1);
        if self.c.chance(100) {
          self.space();
          self.word("else");
          self.space();
          self.block(depth + 1);
        }
      }
      5 if depth < 2 => self.item(depth + 1),
      _ => {
        self.expression(depth + 1, false);
        self.word(";");
      }
    }
  }

  // Expressions.

  /// An expression; `condition` when it stands where a block may follow (an `if`, a
  /// `match`, a `for`), so no struct literal may appear outside parentheses.
  fn expression(&mut self, depth: usize, condition: bool) {
    if depth >= DEPTH {
      return self.atom(condition);
    }
    match self.c.below(16) {
      0..=2 => self.atom(condition),
      3 => {
        // A call.
        let name = self.c.pick(FUNCTIONS);
        if self.c.chance(60) {
          let ty = self.type_name();
          self.word(ty);
          self.word("::");
        }
        self.word(name);
        self.arguments(depth);
      }
      4 => {
        // A method chain.
        self.receiver(depth, condition);
        let links = 1 + self.c.below(4);
        for _ in 0..links {
          if self.c.chance(60) {
            self.newline();
            self.word("    ");
          }
          self.word(".");
          let name = self
            .c
            .pick(&["iter", "map", "filter", "collect", "unwrap", "len"]);
          self.word(name);
          if self.c.chance(40) {
            self.word("::<Vec<_>>");
          }
          self.arguments(depth);
          if self.c.chance(40) {
            self.word("?");
          }
        }
      }
      5 => {
        // A binary operation: each side an operand that cannot chain with it.
        self.operand(depth, condition);
        self.space();
        let op = self.c.pick(&[
          "+", "-", "*", "/", "%", "&&", "||", "==", "<", ">=", "&", "|", "<<",
        ]);
        self.word(op);
        self.space();
        self.operand(depth, condition);
      }
      6 => {
        let op = self.c.pick(&["-", "!", "&", "&mut ", "*"]);
        self.word(op);
        self.operand(depth, condition);
      }
      7 if !condition => {
        // A closure.
        let opening = if self.c.chance(60) { "move |" } else { "|" };
        self.word(opening);
        let count = self.c.below(3);
        for n in 0..count {
          if n > 0 {
            self.word(",");
            self.space();
          }
          let name = self.value_name();
          self.word(name);
        }
        self.word("|");
        self.space();
        if self.c.chance(128) {
          self.block(depth + 1);
        } else {
          self.expression(depth + 1, false);
        }
      }
      8 => {
        self.word("if");
        self.space();
        self.expression(depth + 1, true);
        self.space();
        self.block(depth + 1);
        self.space();
        self.word("else");
        self.space();
        self.block(depth + 1);
      }
      9 => self.matching(depth),
      10 => self.block(depth + 1),
      11 => {
        self.word("[");
        self.gap();
        let count = self.c.below(5);
        for n in 0..count {
          if n > 0 {
            self.word(",");
            self.space();
          }
          self.expression(depth + 1, false);
        }
        self.gap();
        self.word("]");
      }
      12 => {
        self.word("(");
        self.expression(depth + 1, false);
        self.word(",");
        self.space();
        self.expression(depth + 1, false);
        self.word(")");
      }
      13 if !condition => {
        // A struct literal.
        let name = self.type_name();
        self.word(name);
        self.space();
        self.word("{");
        self.gap();
        let count = 1 + self.c.below(3);
        for n in 0..count {
          if n > 0 {
            self.word(",");
            self.space();
          }
          let field = self.value_name();
          self.word(field);
          if self.c.chance(200) {
            self.word(":");
            self.space();
            self.expression(depth + 1, false);
          }
        }
        if self.c.chance(50) {
          self.word(", ..Default::default()");
        }
        self.gap();
        self.word("}");
      }
      14 => {
        self.operand(depth, condition);
        self.word(" as ");
        let primitive = self.c.pick(&["u8", "i64", "usize", "f32"]);
        self.word(primitive);
      }
      _ => {
        self.receiver(depth, condition);
        self.word("[");
        self.expression(depth + 1, false);
        self.word("]");
      }
    }
  }

  /// A side of an operator: an atom, or anything else in parentheses.
  fn operand(&mut self, depth: usize, condition: bool) {
    if self.c.chance(128) {
      self.atom(condition);
    } else {
      self.word("(");
      self.expression(depth + 1, false);
      self.word(")");
    }
  }

  /// What a method or an index applies to: a name, a call or a parenthesized expression.
  fn receiver(&mut self, depth: usize, condition: bool) {
    match self.c.below(3) {
      0 => {
        let name = self.value_name();
        self.word(name);
      }
      1 => {
        let name = self.c.pick(FUNCTIONS);
        self.word(name);
        self.arguments(depth);
      }
      _ => self.operand(depth, condition),
    }
  }

  fn arguments(&mut self, depth: usize) {
    self.word("(");
    let count = self.c.below(4);
    for n in 0..count {
      if n > 0 {
        self.word(",");
        self.space();
      } else {
        self.gap();
      }
      self.expression(depth + 1, false);
    }
    if count > 0 && self.c.chance(30) {
      self.word(",");
    }
    self.word(")");
  }

  fn atom(&mut self, _condition: bool) {
    match self.c.below(9) {
      0 | 1 => {
        let name = self.value_name();
        self.word(name);
      }
      2 => {
        let n = self.c.below(1000);
        self.word(&n.to_string());
        if self.c.chance(60) {
          self.one(&["u8", "_usize", "i32"]);
        }
      }
      3 => self.one(&["1.5", "0.0", "2e3", "1_000.25"]),
      4 => self.one(&["\"text\"", "\"with {} braces\"", "r\"raw\"", "\"\""]),
      5 => self.one(&["'c'", "'\\n'", "b'x'"]),
      6 => self.one(&["true", "false"]),
      7 => {
        let ty = self.type_name();
        self.word(ty);
        self.word("::");
        self.one(&["A", "B", "new()", "default()"]);
      }
      _ => self.one(&["None", "Some(1)", "Ok(())", "self"]),
    }
  }

  fn matching(&mut self, depth: usize) {
    self.word("match");
    self.space();
    self.expression(depth + 1, true);
    self.space();
    self.word("{");
    self.indent += 1;
    let arms = 1 + self.c.below(4);
    for n in 0..arms {
      self.newline();
      if n + 1 == arms {
        self.word("_");
      } else {
        self.pattern(depth + 1, true);
      }
      if self.c.chance(40) {
        self.word(" if ");
        self.operand(depth + 1, true);
      }
      self.space();
      self.word("=>");
      self.space();
      if self.c.chance(100) {
        self.block(depth + 1);
        if self.c.chance(80) {
          self.word(",");
        }
      } else {
        self.expression(depth + 1, false);
        self.word(",");
      }
    }
    self.indent -= 1;
    self.newline_here_after_break();
    self.word("}");
  }

  // Patterns.

  /// A pattern; `alternatives` where `a | b` is allowed (a match arm).
  fn pattern(&mut self, depth: usize, alternatives: bool) {
    if depth >= DEPTH {
      let name = self.value_name();
      return self.word(name);
    }
    match self.c.below(8) {
      0 | 1 => {
        let name = self.value_name();
        self.word(name);
      }
      2 => self.word("_"),
      3 => {
        self.word("(");
        self.pattern(depth + 1, false);
        self.word(",");
        self.space();
        self.pattern(depth + 1, false);
        self.word(")");
      }
      4 => {
        self.word("Some(");
        self.pattern(depth + 1, false);
        self.word(")");
      }
      5 => {
        let name = self.type_name();
        self.word(name);
        self.word(" { ");
        let field = self.value_name();
        self.word(field);
        self.word(", .. }");
      }
      6 if alternatives => {
        let n = self.c.below(10);
        self.word(&n.to_string());
        self.space();
        self.word("|");
        self.space();
        self.word(&(n + 1).to_string());
      }
      _ => {
        let n = self.c.below(10);
        self.word(&n.to_string());
        self.word("..=");
        self.word(&(n + 5).to_string());
      }
    }
  }
}

/// The Rust source the bytes choose.
pub fn source(bytes: &[u8]) -> String {
  let mut g = Gen {
    c: Choices::new(bytes),
    out: String::new(),
    indent: 0,
  };
  if g.c.chance(40) {
    g.out.push_str("//! A file.\n");
  }
  if g.c.chance(30) {
    g.out.push_str("#![allow(dead_code)]\n");
  }
  g.items(0);
  g.out.push('\n');
  g.out
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::syntax::{Edition, parse};

  /// Bytes from a seed, as a fuzzer might give them.
  fn bytes(seed: u64, length: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..length)
      .map(|_| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as u8
      })
      .collect()
  }

  #[test]
  fn test_every_generated_file_parses() {
    for seed in 0..3000 {
      let text = source(&bytes(seed, 64 + (seed as usize % 7) * 128));
      let parsed = parse(&text, Edition::E2021);
      assert!(
        parsed.errors().is_empty(),
        "seed {}: {:?}\n{}",
        seed,
        parsed.errors(),
        text
      );
    }
  }

  #[test]
  fn test_no_bytes_give_a_short_file_and_more_bytes_a_bigger_one() {
    let empty = source(&[]);
    assert!(
      parse(&empty, Edition::E2021).errors().is_empty(),
      "{}",
      empty
    );
    assert!(empty.len() < 40, "{}", empty);
    assert!(source(&bytes(7, 2000)).len() > source(&bytes(7, 50)).len());
  }
}
