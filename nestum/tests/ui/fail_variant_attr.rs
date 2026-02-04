use nestum::nestum;

#[nestum]
pub enum Outer {
    #[nestum]
    Bad(Inner),
}

#[nestum]
pub enum Inner {
    A,
}

fn main() {}
