use nestum::{nestum, nested};

mod inner_mod {
    use super::nestum;

    #[nestum]
    pub enum Inner<T> {
        A(T),
        B,
    }
}

#[nestum]
pub enum RelativeOuter<T> {
    Wrap(inner_mod::Inner<T>),
    Other,
}

#[nestum]
pub enum AbsoluteOuter<T> {
    Wrap(crate::inner_mod::Inner<T>),
}

fn main() {
    let _ = inner_mod::Inner::A(0u32);
    let _ = inner_mod::Inner::B::<u32>();
    let relative = RelativeOuter::Wrap::A(1u32);
    let absolute = AbsoluteOuter::Wrap::A(2u32);

    nested! {
        match relative {
            RelativeOuter::Wrap::A(n) => {
                let _ = n;
            }
            RelativeOuter::Wrap::B => {}
            RelativeOuter::Other => {}
        }
    }

    nested! {
        match absolute {
            AbsoluteOuter::Wrap::A(n) => {
                let _ = n;
            }
            AbsoluteOuter::Wrap::B => {}
        }
    }
}
