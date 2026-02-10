use nestum::nestum;

mod inner {
    use super::nestum;

    #[nestum]
    pub enum Inner { A }
}

#[nestum]
pub enum Outer {
    #[nestum(external = "crate::inner::Inner")]
    Wrap(Option<Inner>),
}

fn main() {}
