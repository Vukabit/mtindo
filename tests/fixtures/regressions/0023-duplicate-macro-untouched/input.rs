macro_rules! m {
    () => {
        1
    };
}

fn b() -> u8 {
    m!()
}

macro_rules! m {
    () => {
        2
    };
}

fn a() -> u8 {
    m!()
}
