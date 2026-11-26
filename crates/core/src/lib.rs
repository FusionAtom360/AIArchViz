//! C1 core: document model, solver, commands, validation and pure export logic.

mod canonical;
mod command;
mod document;
mod identity;
mod length;
mod log;
mod solver;
mod units;

pub use canonical::{CanonicalError, document_schema_json, from_canonical_json, to_canonical_json};
pub use command::{
    Action, Command, CommandError, CreateNode, NoArgs, SetProjectUnits, SetStorey,
    SetVariantSelection, apply,
};
pub use document::*;
pub use identity::{Label, LabelAllocator, LabelError, PROJECT_NAMESPACE, Uuid};
pub use length::{Length, LengthError};
pub use log::{Event, History, LogEntry, LogError, Snapshot, log_from_jsonl, log_to_jsonl};
pub use solver::{
    Conflict, ConstraintReport, FreeClass, LayerA, Member, Outcome, Solution, SolveError, Solver,
};
pub use units::{
    GRID_NM, InputContext, InputRounding, ParseError, ParsedLength, ProjectUnit, echo_length,
    format_length, parse_length,
};
