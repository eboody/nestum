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

    pub fn check(value: Outer::Enum) {
        nested! {
            match value {
                crate::api::Outer::Wrap::A => {}
            }
        }
    }
}

fn main() {
    let _ = api::Inner::A;
    api::check(api::Outer::Wrap::A);
}
