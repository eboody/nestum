use nestum::nestum;

mod demo {
    use super::nestum;

    #[nestum]
    pub enum Inner {
        A,
    }

    pub mod child {
        use super::nestum;

        #[nestum]
        pub enum Outer {
            Wrap(super::Inner),
        }
    }
}

fn main() {}
