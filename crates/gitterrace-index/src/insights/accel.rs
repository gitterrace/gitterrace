//! Fast paths for [`crate::Counted`] reducers. An accelerator may answer a reducer from
//! precomputed tables (the segment aggregates) only when it can prove the answer equals the
//! stream over [`crate::Scope::admits`]. It is never a second definition of a count.
//!
//! Contract of [`Accel::try_reduce`]:
//! - `None`: the accelerator cannot prove equality for this scope. Nothing else means `None`.
//! - `Some(Ok(out))`: exactly what the stream gives, an empty set included.
//! - `Some(Err(e))`: the tables could not be read. The caller reports the error and never
//!   answers from the stream instead.
//!
//! No fast path is wired yet: `count_parity_fuzz` (P1) must hold before the first one is.

use crate::insights::{
    CountContext, Label, LabelBucket, LabelCount, PeriodStats, PersonTally, Scope, SeriesPoint,
    SizeBucket, When,
};
use crate::seg::SegError;

/// Which reducer a caller asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReducerKind {
    Totals,
    Series,
    When,
    PerPerson,
    PerLabel(Label),
    Sizes,
}

impl ReducerKind {
    /// Every kind, for tests that sweep them.
    pub const ALL: [Self; 7] = [
        Self::Totals,
        Self::Series,
        Self::When,
        Self::PerPerson,
        Self::PerLabel(Label::Kind),
        Self::PerLabel(Label::Scope),
        Self::Sizes,
    ];
}

/// What a reducer returns, one variant per [`ReducerKind`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReducerOut {
    Totals(PeriodStats),
    Series(Vec<SeriesPoint>),
    When(Box<When>),
    PerPerson(Vec<PersonTally>),
    PerLabel(Vec<LabelCount>, Vec<LabelBucket>),
    Sizes(Vec<SizeBucket>),
}

/// A provable fast path, see the module docs. It sees the [`Scope`] only, never web or API
/// parameters.
pub trait Accel {
    fn try_reduce(
        &self,
        scope: &Scope,
        cx: &CountContext,
        kind: ReducerKind,
    ) -> Option<Result<ReducerOut, SegError>>;
}

/// No fast path: every reducer streams.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoAccel;

impl Accel for NoAccel {
    fn try_reduce(
        &self,
        _: &Scope,
        _: &CountContext,
        _: ReducerKind,
    ) -> Option<Result<ReducerOut, SegError>> {
        None
    }
}
