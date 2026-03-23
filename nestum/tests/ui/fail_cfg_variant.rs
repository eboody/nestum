use nestum::nestum;

#[nestum]
pub enum Inner {
    #[cfg(false)]
    A,
    B,
}

#[nestum]
pub enum Outer {
    Wrap(Inner),
}

fn main() {}
