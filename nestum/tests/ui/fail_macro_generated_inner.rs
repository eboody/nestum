use nestum::nestum;

macro_rules! define_inner {
    () => {
        #[nestum]
        enum Inner {
            A,
        }
    };
}

define_inner!();

#[nestum]
enum Outer {
    Wrap(Inner),
}

fn main() {
    let _ = Outer::Wrap::A;
}
