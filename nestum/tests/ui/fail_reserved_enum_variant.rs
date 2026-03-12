use nestum::nestum;

#[nestum]
pub enum Inner {
    A,
}

#[nestum]
pub enum Outer {
    Enum(Inner),
    Other,
}

fn main() {}
