//! How a setting value is read from TOML and shown.

/// A type a setting can have.
pub trait Show: Sized {
    /// Display form, the TOML literal.
    fn show(&self) -> String;
    /// The type name for the docs.
    const TYPE: &'static str;
    fn from_toml(v: &toml::Value) -> Option<Self>;
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
