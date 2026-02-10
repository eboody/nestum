use nestum::nestum;

#[cfg(false)]
mod inner {
    use super::nestum;

    #[nestum]
    pub enum Inner { A }
}

#[nestum]
pub enum Outer {
    #[nestum(external = "crate::inner::Inner")]
    Wrap(Inner),
}

fn main() {}
