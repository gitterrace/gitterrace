//! A count with the population it counts, so a reader never mistakes one for the other.
//!
//! Fields are declared in alphabetical order, as every API shape here.

use schemars::JsonSchema;
use serde::Serialize;

/// Which commits a count covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Population {
    /// The commits every insight counts: no merges, bots as the options say, the selection
    /// and the period applied.
    Counted,
    /// Every reachable commit, merges and bots included: the log and the overview.
    Every,
}

impl Population {
    /// The word a page writes in its `data-population` attribute.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Counted => "counted",
            Self::Every => "every",
        }
    }
}

/// A count and its population, as the API carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FigureItem {
    pub population: Population,
    pub value: u64,
}
