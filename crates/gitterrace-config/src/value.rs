//! How a setting value is read from TOML and shown.

use gitterrace_types::{ColorScheme, DateStyle, LinkStyle, Period, StringEnum};

/// A type a setting can have.
pub trait Show: Sized {
    /// Display form, the TOML literal.
    fn show(&self) -> String;
    /// The type name for the docs.
    const TYPE: &'static str;
    fn from_toml(v: &toml::Value) -> Option<Self>;
    /// What a value that [`Show::from_toml`] refuses should have been, for the error.
    #[must_use]
    fn expected() -> String {
        Self::TYPE.to_owned()
    }
}

impl Show for bool {
    const TYPE: &'static str = "bool";
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

impl Show for String {
    const TYPE: &'static str = "string";
    fn show(&self) -> String {
        format!("{self:?}")
    }
    fn from_toml(v: &toml::Value) -> Option<Self> {
        v.as_str().map(str::to_owned)
    }
}

impl Show for Vec<String> {
    const TYPE: &'static str = "list of strings";
    fn show(&self) -> String {
        format!("{self:?}")
    }
    fn from_toml(v: &toml::Value) -> Option<Self> {
        v.as_array()?
            .iter()
            .map(|x| x.as_str().map(str::to_owned))
            .collect()
    }
}

/// A setting that picks one value of a fixed set: written and read through the type's one codec,
/// so the accepted values and the error that lists them come from the enum.
macro_rules! one_of {
    ($($t:ty),+) => {$(
        impl Show for $t {
            const TYPE: &'static str = "string";
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
one_of!(ColorScheme, DateStyle, LinkStyle, Period);
