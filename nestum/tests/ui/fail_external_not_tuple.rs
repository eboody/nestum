use nestum::nestum;

#[nestum]
pub enum Inner {
    A,
}

#[nestum]
pub enum Outer {
    #[nestum(external = "crate::Inner")]
    Bad,
}

fn main() {}
