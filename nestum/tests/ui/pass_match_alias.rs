use nestum::{nestum, nested};

#[nestum]
pub enum Inner {
    VariantA,
    VariantB(u8),
}

#[nestum]
pub enum Outer {
    Wrap(Inner),
    Other,
}

fn main() {
    let value = Outer::Wrap::VariantA;
    nested! {
        match value {
            Outer::Wrap::VariantA => {}
            Outer::Wrap::VariantB(n) => {
                let _ = n;
            }
            Outer::Outer::Other => {}
        }
    }
}
