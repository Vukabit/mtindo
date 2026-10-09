struct Record<T> {
    pub x: T,
    pub(crate) y: Vec<T>,
    z: (),
}
struct Tuple<T>(pub T, pub(super) u8)
where
    T: Copy;
struct Unit;
struct Where<T>
where
    T: Default,
{
    t: T,
}
