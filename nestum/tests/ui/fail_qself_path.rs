use nestum::nestum;

#[nestum]
pub enum Inner {
    A,
}

trait HasNested {
    type Nested;
}

struct Marker;

impl HasNested for Marker {
    type Nested = Inner;
}

#[nestum]
pub enum Outer {
    Wrap(<Marker as HasNested>::Nested),
}

fn main() {}
