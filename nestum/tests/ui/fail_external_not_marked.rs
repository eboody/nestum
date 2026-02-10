use nestum::nestum;

mod inner {
    pub enum Inner { A }
}

#[nestum]
pub enum Outer {
    #[nestum(external = "crate::inner::Inner")]
    Wrap(Inner),
}

fn main() {}
