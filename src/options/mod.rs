//! Layer 1: plain settings types, with no parsing or I/O.
//!
//! The settings table: every key mto accepts in a configuration file, with its type, legal
//! values, defaults and documentation. The formatting keys are generated from the standard
//! formatter's own source (the generated `options/rustfmt.rs`, written by `cargo xtask
//! settings`); mto's own keys are written by hand (`mto.rs`). The manual, `mto config
//! print` and configuration checking all read this table, so they cannot disagree.

pub mod format;
pub mod layout;
mod mto;
mod rustfmt;
pub mod user;

pub use rustfmt::RUSTFMT_VERSION;

/// Which configuration files may hold a setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum File {
    /// The standard formatter's settings: `rustfmt.toml`, `.rustfmt.toml`, `mto.toml` or
    /// `.mto.toml`.
    Format,
    /// mto's layout settings: `mto.toml` or `.mto.toml` only.
    Mto,
    /// Tool behavior: the per-user `config.toml` only.
    User,
}

/// The type of a setting's value.
#[derive(Debug, PartialEq, Eq)]
pub enum Kind {
    Bool,
    /// A non-negative integer.
    Integer,
    /// Any string.
    Text,
    /// A size in bytes, written as a string such as `"256MB"` (see [`user::bytes`]).
    Size,
    /// One of a fixed set of strings.
    Choice(&'static [Choice]),
    /// An array of strings.
    List,
    /// An array of values from a fixed set, in order.
    Sequence(&'static [Choice]),
    /// A value the standard formatter sets itself (on its command line or from other
    /// settings); accepted in a file for compatibility and otherwise ignored.
    Internal,
}

/// Whether a release channel of the standard formatter has a setting or value, and whether it
/// calls it stable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stability {
    Absent,
    Unstable,
    Stable,
}

/// A setting's or value's [`Stability`] in the standard formatter's stable and nightly releases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Channels {
    pub stable: Stability,
    pub nightly: Stability,
}

/// One legal value of a [`Kind::Choice`] or [`Kind::Sequence`] setting.
#[derive(Debug, PartialEq, Eq)]
pub struct Choice {
    /// The value as written in a configuration file; matched without regard to case.
    pub name: &'static str,
    pub doc: &'static str,
    /// `None` for mto's own values.
    pub channels: Option<Channels>,
}

/// One configuration key.
#[derive(Debug, PartialEq, Eq)]
pub struct Setting {
    pub name: &'static str,
    pub file: File,
    pub kind: Kind,
    /// The default as TOML (`100`, `false`, `"Auto"`, `[]`); empty for [`Kind::Internal`].
    pub default: &'static str,
    /// The default under style edition 2024 and later, when it differs.
    pub default_2024: Option<&'static str>,
    /// One line, for listings.
    pub summary: &'static str,
    /// The full description, in paragraphs separated by blank lines.
    pub doc: &'static str,
    /// `None` for mto's own settings.
    pub channels: Option<Channels>,
    /// Left out of the standard formatter's own listings (deprecated or internal).
    pub hidden: bool,
    /// The setting that replaces a deprecated one.
    pub replaced_by: Option<&'static str>,
}

impl Setting {
    /// The default under `style_edition` (2015, 2018, 2021, 2024...).
    pub fn default_for(&self, style_edition: u16) -> &'static str {
        match self.default_2024 {
            Some(default) if style_edition >= 2024 => default,
            _ => self.default,
        }
    }

    /// The legal values of a choice or sequence setting.
    pub fn choices(&self) -> &'static [Choice] {
        match self.kind {
            Kind::Choice(choices) | Kind::Sequence(choices) => choices,
            _ => &[],
        }
    }
}

/// Every setting: the standard formatter's in name order, then mto's own, then the per-user ones.
pub fn all() -> impl Iterator<Item = &'static Setting> {
    rustfmt::SETTINGS
        .iter()
        .chain(mto::SETTINGS.iter())
        .chain(user::SETTINGS.iter())
}

pub fn find(name: &str) -> Option<&'static Setting> {
    all().find(|setting| setting.name == name)
}

/// Why `value` is refused for `setting` when it looks like it should be legal.
pub fn refusal(setting: &Setting, value: &str) -> Option<String> {
    let pinned = mto::PINNED_KINDS
        .iter()
        .find(|kind| kind.eq_ignore_ascii_case(value))?;
    (setting.name == "type_order").then(|| {
        format!(
            "`type_order` cannot hold `{}`: `extern crate` items and item macro calls never \
             move, so they have no place in the order; remove it",
            pinned
        )
    })
}

#[cfg(test)]
mod tests;
