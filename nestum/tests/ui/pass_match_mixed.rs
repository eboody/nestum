use nestum::{nestum, nested};

#[nestum]
pub enum Inner {
    VariantA,
    VariantB(u8),
}

#[nestum]
pub enum Outer {
    Variant1,
    Variant2(Inner),
}

fn main() {
    let event = Outer::Variant2::VariantA;
    nested! {
        match event {
            Outer::Variant1 => {}
            Outer::Variant2::VariantA => {}
            Outer::Variant2::VariantB(n) => { let _ = n; }
        }
    }
}
