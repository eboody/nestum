use nestum::nestum;

#[nestum]
pub enum Outer {
    #[nestum(external = "crate::missing::Inner")]
    Wrap(Inner),
}

pub enum Inner {
    A,
}

fn main() {}
