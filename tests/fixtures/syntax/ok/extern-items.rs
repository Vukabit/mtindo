// edition: 2024
extern crate alloc;
extern crate self as me;
extern "C" {
    fn printf(fmt: *const u8, ...) -> i32;
    static ERRNO: i32;
}
unsafe extern "C" {
    pub safe fn abs(x: i32) -> i32;
    pub unsafe static mut COUNTER: u32;
}
