use nestum::{nestum, nested};

mod inner_mod {
    use super::nestum;

    #[nestum]
    pub enum Inner {
        A,
        B(u8),
    }
}

#[nestum]
pub enum RelativeOuter {
    Wrap(inner_mod::Inner),
}

#[nestum]
pub enum AbsoluteOuter {
    Wrap(crate::inner_mod::Inner),
}

fn main() {
    let relative = RelativeOuter::Wrap::B(1);
    let absolute = AbsoluteOuter::Wrap::A;

    nested! {
        match relative {
            RelativeOuter::Wrap::A => {}
            RelativeOuter::Wrap::B(n) => { let _ = n; }
        }
    }

    nested! {
        match absolute {
            AbsoluteOuter::Wrap::A => {}
            AbsoluteOuter::Wrap::B(n) => { let _ = n; }
        }
    }
}
