macro_rules! square {
    ($x:expr) => {
        $x * $x
    };
}
macro_rules! paren (() => {});
thread_local! {
    static KEY: u8 = 0;
}
foo!();
a::b![x];
type T = ty!(u8);
