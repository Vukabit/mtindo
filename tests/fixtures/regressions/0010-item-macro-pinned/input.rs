macro_rules! make {
    () => {
        pub fn made() {}
    };
}

fn d() {}

make!();

fn c() {}

thread_local! {
    static COUNTER: u8 = 0;
}

fn b() {}
