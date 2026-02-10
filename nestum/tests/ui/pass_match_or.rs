use nestum::{nestum, nested};

#[nestum]
pub enum Inner {
    A,
    B,
}

#[nestum]
pub enum Outer {
    Wrap(Inner),
}

fn main() {
    let value = Outer::Wrap::A;
    nested! {
        match value {
            Outer::Wrap::A | Outer::Wrap::B => {}
        }
    }
}
