use nestum::{nestum, nestum_scope};

#[nestum]
#[derive(Debug, PartialEq, Eq)]
pub enum DocumentEvent {
    Created,
    Renamed { title: &'static str },
}

#[nestum]
#[derive(Debug, PartialEq, Eq)]
pub enum UserCommand {
    Create(&'static str),
}

#[nestum]
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Document(DocumentEvent),
}

#[nestum]
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    User(UserCommand),
}

#[nestum_scope]
fn build_event() -> Event::Enum {
    let event = Event::Document::Renamed { title: "scope-fn" };
    if let Event::Document::Renamed { title } = event {
        assert_eq!(title, "scope-fn");
    }
    Event::Document::Created
}

struct Runner;

#[nestum_scope]
impl Runner {
    fn build_command() -> Command::Enum {
        Command::User::Create("via-impl")
    }
}

struct MethodRunner;

impl MethodRunner {
    #[nestum_scope]
    fn build_event_again(&self) -> Event::Enum {
        let value = Event::Document::Renamed { title: "method" };
        assert_eq!(value, Event::Document::Renamed { title: "method" });
        Event::Document::Created
    }
}

#[nestum_scope]
mod scoped_mod {
    use super::{Command, Event};

    pub fn build_nested() -> (Event::Enum, Command::Enum) {
        let event = super::Event::Document::Created;
        let command = super::Command::User::Create("scoped-mod");

        assert!(matches!(event, super::Event::Document::Created));
        assert_eq!(command, super::Command::User::Create("scoped-mod"));

        (event, command)
    }
}

#[nestum_scope]
fn main() {
    let event = build_event();
    assert!(event == Event::Document::Created);

    let command = Runner::build_command();
    let command_ok = matches!(command, Command::User::Create(email) if email == "via-impl");
    assert!(command_ok);

    let method_event = MethodRunner.build_event_again();
    let method_event_ok = matches!(method_event, Event::Document::Created);
    assert!(method_event_ok);

    let (scoped_event, scoped_command) = scoped_mod::build_nested();
    let scoped_event_ok = matches!(scoped_event, Event::Document::Created);
    assert!(scoped_event_ok);
    let scoped_command_ok =
        matches!(scoped_command, Command::User::Create(email) if email == "scoped-mod");
    assert!(scoped_command_ok);
}
