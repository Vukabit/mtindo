use std::fmt;

macro_rules! fmt_impl {
    ($name:ident) => {
        pub fn $name() -> &'static str {
            stringify!($name)
        }
    };
}

mod hex {
    fmt_impl!(hex);
}

mod debug {
    fmt_impl!(debug);
}

pub fn show(f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}{}", hex::hex(), debug::debug())
}
