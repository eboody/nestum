use nestum::nestum;

mod outer_mod {
    use super::nestum;

    pub mod inner_mod {
        use super::nestum;

        #[nestum]
        pub enum Inner {
            A,
            B(u8),
        }
    }

    #[nestum]
    pub enum Outer {
        #[nestum(external = "crate::outer_mod::inner_mod::Inner")]
        Wrap(Inner),
    }
}

fn main() {
    let _ = outer_mod::Outer::Wrap::A;
    let _ = outer_mod::Outer::Wrap::B(1);
}
