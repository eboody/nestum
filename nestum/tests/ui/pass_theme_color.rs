use nestum::nestum;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Color(String);

impl std::str::FromStr for Color {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_owned()))
    }
}

#[nestum]
pub enum Variant {
    Main(Color),
    Darker(Color),
    Lighter(Color),
    Lightest(Color),
}

#[nestum]
pub enum Theme {
    Teal(Variant),
    Pink(Variant),
}

fn main() {
    let _a = Theme::Pink::Main("#ff00ff".parse::<Color>().expect("valid color"));
    let _b = Theme::Teal::Darker("#008080".parse::<Color>().expect("valid color"));
}
