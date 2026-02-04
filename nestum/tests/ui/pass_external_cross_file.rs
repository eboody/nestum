use nestum::nestum;

mod external_inner;

#[nestum]
pub enum Outer {
    #[nestum(external = "crate::external_inner::Inner")]
    Wrap(Inner),
}

fn main() {
    let _ = Outer::Wrap::A;
}
