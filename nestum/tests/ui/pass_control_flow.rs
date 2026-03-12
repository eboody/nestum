use nestum::{nested, nestum};

#[nestum]
pub enum Inner {
    A,
    B(u8),
    Struct { x: i32 },
}

#[nestum]
pub enum Outer {
    Wrap(Inner),
    Other,
}

fn main() {
    let value = nested! { Outer::Wrap::Struct { x: 3 } };

    let matched = nested! { matches!(value, Outer::Wrap::Struct { x } if x > 0) };
    assert!(matched);

    nested! {
        if let Outer::Wrap::Struct { x } = value {
            let _ = x;
        } else {
            panic!("expected struct");
        }
    }

    let mut maybe = Some(Outer::Wrap::B(7));
    nested! {
        while let Some(Outer::Wrap::B(n)) = maybe.take() {
            let _ = n;
        }
    }

    let value = Outer::Wrap::A;
    let extracted = nested! {
        let Outer::Wrap::A = value else {
            panic!("expected A");
        };
        1usize
    };

    let _ = extracted;
}
