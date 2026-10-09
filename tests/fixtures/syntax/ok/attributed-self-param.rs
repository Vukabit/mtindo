impl S {
    fn f(
        #[cfg_attr(a, allow(unused_mut))]
        mut self: Pin<&mut Self>,
        #[cfg(x)] y: u8,
    ) {}
}
