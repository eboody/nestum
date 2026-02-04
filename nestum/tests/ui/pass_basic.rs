use nestum::nestum;

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
    let _ = Enum1::Variant1::VariantA;
    let _ = Enum1::Variant1::VariantB(1);
    let _ = Enum1::Variant1::VariantC(2);
    let _ = Enum1::Enum1::Other;
}
