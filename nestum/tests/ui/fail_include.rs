use nestum::nestum;

mod inner {
    use super::nestum;
    include!("inner_included.rs");
}

#[nestum]
pub enum Outer {
    #[nestum(external = "crate::inner::Inner")]
    Wrap(Inner),
}

fn main() {}
