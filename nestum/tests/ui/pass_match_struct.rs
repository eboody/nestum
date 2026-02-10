use nestum::{nestum, nested};

#[nestum]
pub enum Inner {
    Unit,
    Tuple(u8),
    Struct { x: i32 },
}

#[nestum]
pub enum Outer {
    Wrap(Inner),
}

fn main() {
    let value = Outer::Wrap::Struct(5);
    nested! {
        match value {
            Outer::Wrap::Unit => {}
            Outer::Wrap::Tuple(n) => { let _ = n; }
            Outer::Wrap::Struct { x } => { let _ = x; }
        }
    }
}
