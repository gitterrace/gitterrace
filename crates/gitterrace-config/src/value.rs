//! How a setting value is read from TOML and shown.

use gitterrace_types::{ColorScheme, DateStyle, DiffPalette, LinkStyle, Period, StringEnum};

/// The input the settings form shows for a type. Derived from the type alone, so a new setting
/// gets its input without a list to update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    Checkbox,
    /// Unset, on or off: a switch whose default depends on the server's mode.
    Tri,
    /// A whole number. Its range, when it has one, comes from [`crate::BOUNDS`].
    Number,
    Text,
    /// One string per row.
    List,
    /// One of the enum's canonical values, in declaration order.
    Select(Vec<&'static str>),
}

/// A type a setting can have.
pub trait Show: Sized {
    fn show(&self) -> String;
    const TYPE: &'static str;
    fn control() -> Control;
    fn from_toml(v: &toml::Value) -> Option<Self>;
    /// What a value that [`Show::from_toml`] refuses should have been, for the error.
    #[must_use]
    fn expected() -> String {
        Self::TYPE.to_owned()
    }
}

impl Show for bool {
    const TYPE: &'static str = "bool";
    fn control() -> Control {
        Control::Checkbox
    }
    fn show(&self) -> String {
        self.to_string()
    }
    fn from_toml(v: &toml::Value) -> Option<Self> {
        v.as_bool()
    }
}

/// A switch with no fixed default: unset leaves the choice to the server's mode.
impl Show for Option<bool> {
    const TYPE: &'static str = "bool";
    fn control() -> Control {
        Control::Tri
    }
    fn show(&self) -> String {
        self.map_or_else(|| "unset".to_owned(), |b| b.to_string())
    }
    fn from_toml(v: &toml::Value) -> Option<Self> {
        v.as_bool().map(Some)
    }
}

macro_rules! int {
    ($t:ty) => {
        impl Show for $t {
            const TYPE: &'static str = "integer";
            fn control() -> Control {
                Control::Number
            }
            fn show(&self) -> String {
                self.to_string()
            }
            fn from_toml(v: &toml::Value) -> Option<Self> {
                v.as_integer().and_then(|i| Self::try_from(i).ok())
            }
        }
    };
}
int!(u64);
int!(usize);

/// A cap that is off unless set: unset means no limit.
impl Show for Option<usize> {
    const TYPE: &'static str = "integer";
    fn control() -> Control {
        Control::Number
    }
    fn show(&self) -> String {
        self.map_or_else(|| "unset".to_owned(), |n| n.to_string())
    }
    fn from_toml(v: &toml::Value) -> Option<Self> {
        v.as_integer().and_then(|i| usize::try_from(i).ok()).map(Some)
    }
}

impl Show for String {
    const TYPE: &'static str = "string";
    fn control() -> Control {
        Control::Text
    }
    fn show(&self) -> String {
        format!("{self:?}")
    }
    fn from_toml(v: &toml::Value) -> Option<Self> {
        v.as_str().map(str::to_owned)
    }
}

impl Show for Vec<String> {
    const TYPE: &'static str = "list of strings";
    fn control() -> Control {
        Control::List
    }
    fn show(&self) -> String {
        format!("{self:?}")
    }
    fn from_toml(v: &toml::Value) -> Option<Self> {
        v.as_array()?.iter().map(|x| x.as_str().map(str::to_owned)).collect()
    }
}

/// A setting that picks one value of a fixed set: written and read through the type's one codec,
/// so the accepted values and the error that lists them come from the enum.
macro_rules! one_of {
    ($($t:ty),+) => {$(
        impl Show for $t {
            const TYPE: &'static str = "string";
            fn control() -> Control {
                Control::Select(<$t as StringEnum>::ALL.iter().map(|v| v.as_str()).collect())
            }
            fn show(&self) -> String {
                format!("{:?}", self.as_str())
            }
            fn from_toml(v: &toml::Value) -> Option<Self> {
                <$t as StringEnum>::parse(v.as_str()?)
            }
            fn expected() -> String {
                <$t as StringEnum>::expected()
            }
        }
    )+};
}
one_of!(ColorScheme, DateStyle, DiffPalette, LinkStyle, Period);
