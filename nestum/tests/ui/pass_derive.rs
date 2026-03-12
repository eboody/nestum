use nestum::{nested, nestum};

#[nestum]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inner {
    A,
    B(u8),
}

#[nestum]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outer {
    Wrap(Inner),
    Other,
}

fn accept_outer(_: Outer::Enum) {}

fn main() {
    let value: Outer::Enum = Outer::Wrap::B(7);
    let cloned = value.clone();

    assert_eq!(cloned, Outer::Wrap::B(7));
    let _debug = format!("{cloned:?}");
    accept_outer(Outer::Wrap::A);

    nested! {
        match cloned {
            Outer::Wrap::A => {}
            Outer::Wrap::B(n) => {
                let _ = n;
            }
            Outer::Other => {}
        }
    }
}
