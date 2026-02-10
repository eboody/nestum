use nestum::nestum;

#[nestum]
pub enum Inner { A }

#[nestum]
pub enum Outer {
    #[nestum()]
    Wrap(Inner),
}

fn main() {}
