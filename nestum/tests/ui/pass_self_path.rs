use nestum::{nestum, nested};

mod demo {
    use super::{nested, nestum};

    #[nestum]
    pub enum Inner {
        A,
    }

    #[nestum]
    pub enum Outer {
        Wrap(self::Inner),
    }

    pub fn run() {
        let _ = self::Inner::A;
        let value = nested! { self::Outer::Wrap::A };
        nested! {
            match value {
                self::Outer::Wrap::A => {}
            }
        }
    }
}

fn main() {
    demo::run();
}
