use nestum::nestum;

#[nestum]
pub enum Inner { A }

#[nestum]
pub enum Outer {
    #[nestum(external = 123)]
    Wrap(Inner),
}

fn main() {}
