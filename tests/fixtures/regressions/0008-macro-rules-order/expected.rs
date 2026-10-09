macro_rules! zz {
    () => {
        aa!()
    };
}

macro_rules! aa {
    () => {
        1
    };
}

fn a() {}

fn b() -> u8 {
    zz!()
}
