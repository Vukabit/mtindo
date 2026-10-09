#[macro_use]
mod macros {
    macro_rules! m {
        () => {
            1
        };
    }
}

mod alpha {
    pub fn a() -> u8 {
        m!()
    }
}

mod zeta {
    pub fn z() -> u8 {
        m!()
    }
}
