use nestum::nestum;

#[nestum]
pub enum Inner { A }

#[nestum]
pub enum Outer {
    #[nestum(foo = "bar")]
    Wrap(Inner),
}

fn main() {}
