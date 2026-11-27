//! C1 core: document model, solver, commands, validation and pure export logic.

mod canonical;
mod command;
mod defaults;
mod document;
mod geometry;
mod identity;
mod items;
mod length;
mod log;
mod proposal;
mod solver;
mod space;
mod units;
mod validity;

pub use canonical::{CanonicalError, document_schema_json, from_canonical_json, to_canonical_json};
pub use command::{
    Action, Command, CommandError, CreateNode, CreateOpening, NoArgs, SetProjectUnits, SetStorey,
    SetVariantSelection, apply,
};
pub use defaults::{CATALOG_VERSION, MIN_JAMB_DISTANCE, OpeningSize, opening_size};
pub use document::*;
pub use geometry::{
    GeometryError, OpeningProblem, OpeningViolation, Void, WallGeometry, WallProfile, node_joins,
    opening_violations, update_joins, wall_geometry, wall_profiles,
};
pub use identity::{Label, LabelAllocator, LabelError, PROJECT_NAMESPACE, Uuid};
pub use items::{Rect, clearance_zone, footprint, height_range, proxy_box};
pub use length::{Length, LengthError};
pub use log::{Event, History, LogEntry, LogError, Snapshot, log_from_jsonl, log_to_jsonl};
pub use proposal::{
    Ambiguity, AmbiguityKind, Change, CommitError, DeriveError, Derived, DiffEntry, Flag, Proposal,
    ProposeError, ProposedAction, commit, derive, propose,
};
pub use solver::{
    Conflict, ConstraintReport, FreeClass, LayerA, Member, Outcome, Solution, SolveError, Solver,
};
pub use space::{
    PolygonError, SpaceError, boundary_outline, check_polygon, polygon_area, space_boundary,
    update_spaces,
};
pub use units::{
    GRID_NM, InputContext, InputRounding, ParseError, ParsedLength, ProjectUnit, echo_length,
    format_length, parse_length,
};
pub use validity::{Detail, Violation, blocks_commit, prune_acknowledged, validate};
