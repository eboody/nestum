mod document {
    use nestum::{nested, nestum};

    #[nestum]
    pub enum Inner {
        Created,
    }

    #[nestum]
    pub enum Outer {
        Wrap(Inner),
    }

    pub fn build() -> Outer::Enum {
        Outer::Wrap::Created
    }

    pub fn check(value: Outer::Enum) {
        nested! {
            match value {
                crate::document::Outer::Wrap::Created => {}
            }
        }
    }
}

mod image {
    use nestum::{nested, nestum};

    #[nestum]
    pub enum Inner {
        Archived,
    }

    #[nestum]
    pub enum Outer {
        Wrap(Inner),
    }

    pub fn build() -> Outer::Enum {
        Outer::Wrap::Archived
    }

    pub fn check(value: Outer::Enum) {
        nested! {
            match value {
                crate::image::Outer::Wrap::Archived => {}
            }
        }
    }
}

fn main() {
    let document = document::build();
    document::check(document);

    let image = image::build();
    image::check(image);
}
