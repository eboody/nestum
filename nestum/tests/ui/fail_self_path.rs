use nestum::nestum;

mod demo {
    use super::nestum;

    #[nestum]
    pub enum Inner {
        A,
    }

    #[nestum]
    pub enum Outer {
        Wrap(self::Inner),
    }
}

fn main() {}
