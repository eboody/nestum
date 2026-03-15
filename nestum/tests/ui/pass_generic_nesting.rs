use nestum::{nestum, nested};

#[nestum]
pub enum Inner<T> {
    Ready,
    Value(T),
}

#[nestum]
pub enum Outer<T> {
    Wrap(Inner<T>),
    Other,
}

fn main() {
    let ready: Outer::Enum<u32> = Outer::Wrap::Ready();
    let value: Outer::Enum<u32> = Outer::Wrap::Value(1);
    let other: Outer::Enum<u32> = Outer::Other();

    nested! {
        match ready {
            Outer::Wrap::Ready => {}
            Outer::Wrap::Value(n) => {
                let _ = n;
            }
            Outer::Other => {}
        }
    }

    let _ = value;
    let _ = other;
}
