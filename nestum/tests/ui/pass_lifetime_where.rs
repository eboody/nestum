use nestum::nestum;

#[nestum]
pub enum Inner<'a, T>
where
    T: Copy,
{
    Ref(&'a T),
}

#[nestum]
pub enum Outer<'a, T>
where
    T: Copy,
{
    Wrap(Inner<'a, T>),
    Other,
}

fn main() {
    let value = 7u32;
    let _wrapped: Outer::Enum<'_, u32> = Outer::Wrap::Ref(&value);
    let _other: Outer::Enum<'_, u32> = Outer::Other();
}
