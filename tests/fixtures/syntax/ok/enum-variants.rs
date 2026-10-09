#[derive(Debug)]
pub enum E<T> {
    Unit,
    #[default]
    Tuple(T, u8),
    Record { a: T, b: Option<T> },
    Explicit = 1 << 2,
}
