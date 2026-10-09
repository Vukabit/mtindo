macro_rules! wrap {
    ($($t:tt)*) => {
        $($t)*
    };
}

fn b() {}

wrap!(
    fn generated() {}
);

fn a() {}
