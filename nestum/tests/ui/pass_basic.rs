use nestum::{nestum, nested};

#[nestum]
pub enum Enum2 {
    VariantA,
    VariantB(u8),
    VariantC { x: i32 },
}

#[nestum]
pub enum Enum1 {
    Variant1(Enum2),
    Other,
}

fn main() {
    let explicit: Enum1::Enum = Enum1::Enum::Variant1(Enum2::Enum::VariantA);
    let _ = Enum1::Variant1::VariantA;
    let _ = Enum1::Variant1::VariantB(1);
    let _ = nested! { Enum1::Variant1::VariantC { x: 2 } };
    let _ = Enum1::Other;
    let _ = explicit;
}
