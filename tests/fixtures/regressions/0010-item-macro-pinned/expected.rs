macro_rules! make {
    () => {
        pub fn made() {}
    };
}

fn b() {}

make!();

fn c() {}

thread_local! {
    static COUNTER: u8 = 0;
}

fn d() {}
