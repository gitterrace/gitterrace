//! The one codec of a string-encoded enum.
//!
//! A variant meets its text only in the table given to [`string_enum!`]: `as_str`, `Display`,
//! `FromStr`, the list of accepted values and (when asked) serde all read that table, so a
//! surface can never spell a value its own way.

/// An enum whose values are fixed strings, declared with [`string_enum!`].
pub trait StringEnum: Copy + Sized + 'static {
    /// Every variant, in declaration order.
    const ALL: &'static [Self];
    /// The canonical text, the form every surface writes.
    fn as_str(self) -> &'static str;
    /// Parses the canonical text or an alias. Case-sensitive.
    fn parse(s: &str) -> Option<Self>;

    /// The canonical values as prose, `a, b or c`, for error messages and docs.
    #[must_use]
    fn expected() -> String {
        let all: Vec<&str> = Self::ALL.iter().copied().map(Self::as_str).collect();
        match all.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
            Some((last, _)) => (*last).to_owned(),
            None => String::new(),
        }
    }
}

/// Text that names no value of an enum. Its message lists the accepted values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub got: String,
    pub expected: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "expected {}, got {:?}", self.expected, self.got)
    }
}

impl std::error::Error for ParseError {}

/// Parses `s` through the codec of `T`, with an error naming the accepted values.
///
/// # Errors
///
/// When `s` is neither a canonical value nor an alias of `T`.
pub fn parse<T: StringEnum>(s: &str) -> Result<T, ParseError> {
    T::parse(s).ok_or_else(|| ParseError {
        got: s.to_owned(),
        expected: T::expected(),
    })
}

/// Declares a string-encoded enum and its one codec.
///
/// `Variant => "text"` gives the canonical text, `| "alias"` adds text that parses to the same
/// variant but is never written. `#[default]` marks the default variant. `serde` after the
/// name makes serde read and write the canonical text.
#[macro_export]
macro_rules! string_enum {
    (
        $(#[$attr:meta])*
        pub enum $name:ident $([$serde:ident])? {
            $( $(#[$vattr:meta])* $variant:ident => $text:literal $(| $alias:literal)* ),+ $(,)?
        }
    ) => {
        $(#[$attr])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $( $(#[$vattr])* $variant, )+ }

        impl $crate::StringEnum for $name {
            const ALL: &'static [Self] = &[$(Self::$variant),+];
            fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text,)+ }
            }
            fn parse(s: &str) -> Option<Self> {
                match s {
                    $( $text $(| $alias)* => Some(Self::$variant), )+
                    _ => None,
                }
            }
        }

        impl $name {
            /// The canonical text.
            #[must_use]
            pub fn as_str(self) -> &'static str {
                <Self as $crate::StringEnum>::as_str(self)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::ParseError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                $crate::codec::parse(s)
            }
        }

        $( $crate::string_enum!(@$serde $name); )?
    };
    (@serde $name:ident) => {
        impl $crate::__serde::Serialize for $name {
            fn serialize<S: $crate::__serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> $crate::__serde::Deserialize<'de> for $name {
            fn deserialize<D: $crate::__serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <std::borrow::Cow<'de, str> as $crate::__serde::Deserialize>::deserialize(d)?;
                s.parse().map_err(<D::Error as $crate::__serde::de::Error>::custom)
            }
        }

        impl $crate::__schemars::JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                std::borrow::Cow::Borrowed(stringify!($name))
            }

            fn json_schema(_: &mut $crate::__schemars::SchemaGenerator) -> $crate::__schemars::Schema {
                let values: Vec<&str> = <Self as $crate::StringEnum>::ALL
                    .iter()
                    .map(|v| v.as_str())
                    .collect();
                $crate::__schemars::json_schema!({ "type": "string", "enum": values })
            }
        }
    };
}

/// Asserts the codec of `T` round-trips: every canonical text parses back to its variant, and
/// no two variants share a text.
///
/// # Panics
///
/// When the table of `T` breaks either rule.
#[cfg(any(test, feature = "test-utils"))]
pub fn assert_round_trip<T: StringEnum + PartialEq + std::fmt::Debug>() {
    for (i, v) in T::ALL.iter().enumerate() {
        assert_eq!(T::parse(v.as_str()), Some(*v), "{} parses back", v.as_str());
        assert!(
            T::ALL[..i].iter().all(|w| w.as_str() != v.as_str()),
            "{} is written by two variants",
            v.as_str()
        );
    }
}
