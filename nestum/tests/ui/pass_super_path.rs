use nestum::{nestum, nested};

mod demo {
    use super::{nested, nestum};

    #[nestum]
    pub enum Inner {
        A,
    }

    pub mod child {
        use super::{nested, nestum};

        #[nestum]
        pub enum Outer {
            Wrap(super::Inner),
        }

        pub fn run() {
            let _ = super::Inner::A;
            let value = nested! { super::child::Outer::Wrap::A };
            nested! {
                match value {
                    super::child::Outer::Wrap::A => {}
                }
            }
        }
    }
}

fn main() {
    demo::child::run();
}
