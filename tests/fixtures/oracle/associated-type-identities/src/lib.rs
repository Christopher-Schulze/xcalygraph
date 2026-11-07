pub trait Provider {
    type Item;
    fn value(&self) -> Self::Item;
}
pub mod other;
