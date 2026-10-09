macro_rules! wrap {
    ($($t:tt)*) => {
        $($t)*
    };
}

fn a() {}

wrap!(
    fn generated() {}
);

fn b() {}
