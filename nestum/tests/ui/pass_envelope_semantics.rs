use nestum::{nested, nestum};

#[nestum]
pub enum DocumentEvent {
    Created,
    Deleted,
}

#[nestum]
pub enum UserEvent {
    SignedIn,
}

#[nestum]
pub enum Event {
    Document(DocumentEvent),
    User(UserEvent),
}

fn takes_inner(_: DocumentEvent::Enum) {}

fn takes_outer(_: Event::Enum) {}

fn main() {
    let inner: DocumentEvent::Enum = DocumentEvent::Created;
    let outer: Event::Enum = Event::Document::Created;
    let other: Event::Enum = Event::User::SignedIn;

    takes_inner(DocumentEvent::Deleted);
    takes_outer(Event::Document::Deleted);
    takes_outer(other);

    nested! {
        match outer {
            Event::Document::Created => {}
            Event::Document::Deleted => {}
            Event::User::SignedIn => {}
        }
    }

    let _ = inner;
}
