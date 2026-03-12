use nestum::{nested, nestum};

#[nestum]
pub enum Leaf {
    A,
    B(u8),
}

#[nestum]
pub enum Mid {
    Leaf(Leaf),
    Other,
}

#[nestum]
pub enum Outer {
    Mid(Mid),
    Tail,
}

fn main() {
    let unit: Outer::Enum = Outer::Mid::Leaf::A;
    let tuple: Outer::Enum = Outer::Mid::Leaf::B(7);
    let direct: Outer::Enum = Outer::Mid::Leaf(Leaf::B(9));
    let explicit: Outer::Enum = Outer::Enum::Mid(Mid::Enum::Leaf(Leaf::Enum::B(11)));

    nested! {
        match unit {
            Outer::Mid::Leaf::A => {}
            Outer::Mid::Leaf::B(n) => {
                let _ = n;
            }
            Outer::Mid::Other => {}
            Outer::Tail => {}
        }
    }

    let _ = tuple;
    let _ = direct;
    let _ = explicit;
}
