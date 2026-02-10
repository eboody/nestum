use nestum::nestum;

#[nestum]
pub enum Inner { A }

#[nestum]
pub enum Outer {
    #[nestum(external = "crate::")] // invalid path
    Wrap(Inner),
}

fn main() {}
