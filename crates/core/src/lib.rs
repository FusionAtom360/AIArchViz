//! C1 core: document model, solver, commands, validation and pure export logic.

// No panics on bad input (CLAUDE.md hard rules, T-199b): outside test code,
// core may not unwrap, expect, panic or index a slice directly.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

mod canonical;
mod clearance;
mod command;
mod create;
mod defaults;
mod document;
mod edit;
mod geometry;
mod grid;
mod identity;
mod inverse;
mod items;
mod length;
mod log;
mod params;
mod proposal;
mod roof;
mod solver;
mod space;
mod units;
mod validity;

pub use canonical::{
    CanonicalError, document_schema_json, from_canonical_json, to_canonical_json, to_pretty_json,
};
pub use clearance::{ClearanceError, ClearanceRequest, clearance_resolve, wall_normal_axis};
pub use command::{
    Action, Command, CommandError, CreateNode, CreateOpening, DimensionTarget, Motion,
    MoveOccurrence, MoveOpening, MoveWall, NoArgs, SetConstraintValue, SetProjectUnits, SetStorey,
    SetVariantSelection, apply,
};
pub use create::{
    AddConstraint, CreateCeiling, CreateDatum, CreateElementType, CreateMaterial, CreateRoof,
    CreateSlab, CreateSpace, CreateWall, PlaceOccurrence, catalog_materials,
};
pub use defaults::{
    CATALOG_VERSION, CEILING_HEIGHT, CEILING_THICKNESS, CLEARANCE, EAVE_OVERHANG, EYE_HEIGHT,
    FASCIA_DEPTH, FLOOR_TO_FLOOR, MEASURED_TOLERANCE, MIN_JAMB_DISTANCE, OpeningSize,
    ROOF_THICKNESS, SLAB_THICKNESS, WALL_BASE_OFFSET, interior_wall, opening_size, proposed,
};
pub use document::*;
pub use edit::{Candidate, Case, MoveError, MoveOutcome};
pub use geometry::{
    GeometryError, OpeningProblem, OpeningViolation, Void, WallGeometry, WallProfile, node_joins,
    opening_violations, update_joins, wall_geometry, wall_profiles,
};
pub use identity::{Label, LabelAllocator, LabelError, PROJECT_NAMESPACE, Uuid};
pub use inverse::{InverseError, inverse};
pub use items::{Rect, clearance_zone, footprint, height_range, proxy_box};
pub use length::{Length, LengthError};
pub use log::{Event, History, LogEntry, LogError, Snapshot, log_from_jsonl, log_to_jsonl};
pub use params::{
    DEPENDENCY_ORDER, Reason, RequiredParameter, Unit, is_provisional, proposed_items,
    required_parameters,
};
pub use proposal::{
    Ambiguity, AmbiguityKind, BaselineValue, Change, CommitError, DeriveError, Derived, DiffEntry,
    Flag, Proposal, ProposeError, ProposedAction, commit, derive, propose,
};
pub use roof::{RoofGeometry, RoofProblem, derive_roofs, roof_geometry, roof_outline};
pub use solver::{
    Conflict, ConstraintReport, FreeClass, LayerA, Member, Outcome, Solution, SolveError, Solver,
    reference_coordinate,
};
pub use space::{
    PolygonError, SpaceError, boundary_outline, check_polygon, polygon_area, space_boundary,
    spaces_outline, update_spaces,
};
pub use units::{
    GRID_NM, InputContext, InputRounding, ParseError, ParsedLength, ProjectUnit, echo_length,
    format_length, parse_length,
};
pub use validity::{
    Detail, Violation, WallTopProblem, blocks_commit, prune_acknowledged, validate,
};
