use nestum::{nested, nestum};

struct Payload;

#[nestum]
enum Outer {
    Wrap(Payload),
}

fn main() {
    let _ = nested! { Outer::Wrap::Missing };
}
