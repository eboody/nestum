use nestum::nestum;

#[path = "missing_inner.rs"]
mod inner;

#[nestum]
pub enum Outer {
    #[nestum(external = "crate::inner::Inner")]
    Wrap(Inner),
}

fn main() {}
