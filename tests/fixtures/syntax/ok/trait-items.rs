pub unsafe auto trait Marker {}
trait Shape<T>: Clone + Send + 'static
where
    T: Copy,
{
    const SIDES: usize;
    const DEFAULT: u8 = 3;
    type Item: Into<T> = T;
    type Lend<'a>
    where
        Self: 'a;
    fn area(&self) -> f64;
    fn name(&self) -> &str {
        "shape"
    }
}
