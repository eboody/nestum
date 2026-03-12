use nestum::nestum;

#[nestum]
pub enum Inner {
    A,
}

#[nestum]
pub enum Outer {
    Wrap(Inner),
    Other,
}

fn main() {
    let inner = Inner::A;
    let _ = Outer::Wrap(inner);
}
