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

fn b() -> u8 {
    zz!()
}

fn a() {}
