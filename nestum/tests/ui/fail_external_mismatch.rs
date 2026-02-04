use nestum::nestum;

mod a {
    use super::nestum;

    #[nestum]
    pub enum Inner {
        A,
    }
}

#[nestum]
pub enum Outer {
    #[nestum(external = "crate::a::Inner")]
    Wrap(Other),
}

pub enum Other {
    A,
}

fn main() {}
