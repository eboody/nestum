use nestum::{nested, nestum};

#[nestum]
pub enum Leaf {
    Node { x: i32 },
}

#[nestum]
pub enum Mid {
    Leaf(Leaf),
}

#[nestum]
pub enum Outer {
    Mid(Mid),
    Tail,
}

fn main() {
    let value: Outer::Enum = nested! { Outer::Mid::Leaf::Node { x: 5 } };
    let pair = nested! { (Outer::Mid::Leaf::Node { x: 6 }, Some(Outer::Mid::Leaf::Node { x: 7 })) };

    nested! {
        match value {
            Outer::Mid::Leaf::Node { x } => {
                let _ = x;
            }
            Outer::Tail => {}
        }
    }

    let _ = pair;
}
