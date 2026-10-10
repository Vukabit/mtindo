//! Rust editions. They change which words are keywords and how some tokens are lexed.

/// A Rust edition, oldest first, so editions compare with `<` and `>=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Edition {
    E2015,
    E2018,
    E2021,
    E2024,
}

impl Edition {
    /// The edition the standard formatter assumes when none is given.
    pub const DEFAULT: Edition = Edition::E2015;
    pub const LATEST: Edition = Edition::E2024;

    /// Parses `2015`, `2018`, `2021` or `2024`.
    pub fn from_year(year: &str) -> Option<Edition> {
        match year.trim() {
            "2015" => Some(Edition::E2015),
            "2018" => Some(Edition::E2018),
            "2021" => Some(Edition::E2021),
            "2024" => Some(Edition::E2024),
            _ => None,
        }
    }

    pub fn year(self) -> &'static str {
        match self {
            Edition::E2015 => "2015",
            Edition::E2018 => "2018",
            Edition::E2021 => "2021",
            Edition::E2024 => "2024",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_editions_order_and_parse() {
        assert!(Edition::E2015 < Edition::E2018);
        assert!(Edition::E2021 >= Edition::E2021);
        assert!(Edition::LATEST > Edition::E2021);
        for edition in [
            Edition::E2015,
            Edition::E2018,
            Edition::E2021,
            Edition::E2024,
        ] {
            assert_eq!(Edition::from_year(edition.year()), Some(edition));
        }
        assert_eq!(Edition::from_year("2027"), None);
    }
}
