mod api {
    use nestum::{nestum, nested};

    #[nestum]
    pub enum Inner {
        A,
    }

    #[nestum]
    pub enum Outer {
        Wrap(Inner),
    }

    pub fn check(value: Outer::Outer) {
        nested! {
            match value {
                crate::api::Outer::Wrap::A => {}
            }
        }
    }
}

fn main() {
    api::check(api::Outer::Wrap::A);
}
