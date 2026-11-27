//! Proposals and commit (spec 8.3, 8.4, 9.2; Q-34 to Q-36).
//!
//! `propose` applies an action to a copy of the head document, derives
//! everything (solve, positions, join types, space boundaries, validation),
//! and reports what would change. It stores nothing: the caller holds the
//! [`Proposal`] and passes it to `commit` (Q-34). The proposal id is the
//! UUIDv5 of the base version and the resolved action, so `commit` can check
//! the proposal is unaltered by proposing again and comparing.
//!
//! `commit` succeeds only when the base version is the head version and
//! nothing blocks: no unacknowledged violation, no solver Conflict, no
//! Referent, Direction or ReferenceSemantics ambiguity (Q-35). It appends
//! one log event atomically.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::canonical::{CanonicalError, to_canonical_json};
use crate::command::{Action, Command, CommandError, CreateOpening, apply};
use crate::defaults::{MIN_JAMB_DISTANCE, opening_size};
use crate::document::{Document, Function, Placement, Source, Sourced};
use crate::edit::{Candidate, MoveError, MoveOutcome, request_for, resolve_move};
use crate::geometry::{GeometryError, update_joins};
use crate::grid::check_user_grid;
use crate::identity::{PROJECT_NAMESPACE, Uuid};
use crate::length::Length;
use crate::log::{Event, History, LogEntry, LogError};
use crate::solver::{Conflict, ConstraintReport, LayerA, Solution, SolveError, Solver};
use crate::space::space_boundary;
use crate::units::format_length;
use crate::validity::{Violation, prune_acknowledged, validate};

/// Why a document could not be derived after an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeriveError {
    Solve(SolveError),
    Geometry(GeometryError),
}

impl fmt::Display for DeriveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeriveError::Solve(e) => write!(f, "cannot solve: {e}"),
            DeriveError::Geometry(e) => write!(f, "cannot check geometry: {e}"),
        }
    }
}

impl std::error::Error for DeriveError {}

/// A derived document with its solution and violations.
pub struct Derived {
    pub document: Document,
    pub solution: Solution,
    pub violations: Vec<Violation>,
}

/// Solves `document` and stores everything derived from the solution:
/// positions, join types, space boundaries (a space whose boundary cannot
/// be derived keeps its previous one and appears as a violation), then
/// validates and prunes acknowledgments of violations that are gone (Q-35).
pub fn derive(document: Document) -> Result<Derived, DeriveError> {
    let mut doc = document;
    let solution = LayerA::default().solve(&doc).map_err(DeriveError::Solve)?;
    solution.apply_to(&mut doc);
    // Bad walls make join types underivable; validation reports them.
    let _ = update_joins(&mut doc);
    let spaces: Vec<Uuid> = doc.spaces.keys().copied().collect();
    for id in spaces {
        if let Ok(boundary) = space_boundary(&doc, id)
            && let Some(space) = doc.spaces.get_mut(&id)
        {
            space.boundary = boundary;
        }
    }
    crate::roof::derive_roofs(&mut doc);
    let violations = validate(&doc, MIN_JAMB_DISTANCE).map_err(DeriveError::Geometry)?;
    prune_acknowledged(&mut doc, &violations);
    Ok(Derived {
        document: doc,
        solution,
        violations,
    })
}

/// What a proposal would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProposedAction {
    Commands { commands: Vec<Command> },
    Undo,
    Redo,
}

/// Ambiguity kinds (9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AmbiguityKind {
    Referent,
    Direction,
    Magnitude,
    ReferenceSemantics,
    PolicyChoice,
}

impl AmbiguityKind {
    /// Whether an unresolved ambiguity of this kind blocks commit (9.2).
    pub fn blocks_commit(self) -> bool {
        matches!(
            self,
            AmbiguityKind::Referent | AmbiguityKind::Direction | AmbiguityKind::ReferenceSemantics
        )
    }
}

/// An unresolved choice (9.2): {kind, candidates, default if any}. Each
/// candidate names the dimension it is about, with its resulting edits and
/// what moves.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub struct Ambiguity {
    pub kind: AmbiguityKind,
    pub candidates: Vec<Candidate>,
    pub default: Option<Uuid>,
}

/// Something the user should notice before confirming (Q-36).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Flag {
    /// An added or changed value with source Proposed (6.4, 7.4).
    ProposedValue {
        collection: String,
        id: Option<Uuid>,
        path: String,
    },
    /// An existing element, or one hosted on an existing wall, changes (6.7).
    ExistingConditionEdit {
        collection: String,
        id: Uuid,
        /// For each changed path of the entity, its baseline value.
        baseline: Vec<BaselineValue>,
    },
}

/// The value at one path of an entity in the baseline snapshot (6.7).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub struct BaselineValue {
    /// The JSON pointer within the entity, as in `DiffEntry::changed_paths`.
    pub path: String,
    /// The baseline value in nanometers; none if the path did not exist.
    pub value: Option<Value>,
    /// The same value with every Length shown in feet-inches-fractions.
    pub display: Option<Value>,
}

/// How an entity changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Added,
    Removed,
    Changed,
}

/// One changed entity, before and after, in nanometers and display units.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub struct DiffEntry {
    /// The document field, e.g. "walls"; singletons such as "project" have no id.
    pub collection: String,
    pub id: Option<Uuid>,
    pub change: Change,
    pub before: Option<Value>,
    pub after: Option<Value>,
    /// The same values with every Length shown in feet-inches-fractions.
    pub before_display: Option<Value>,
    pub after_display: Option<Value>,
    /// JSON pointers, within the entity, of every value that differs.
    pub changed_paths: Vec<String>,
}

/// The result of `propose` (8.3).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub struct Proposal {
    pub proposal_id: Uuid,
    pub base_version: u64,
    /// The action as requested.
    pub action: ProposedAction,
    /// The action with every value explicit: moves become SetConstraintValue
    /// commands (Q-39).
    pub resolved: ProposedAction,
    /// The resolved commands in canonical plain language, one entry per
    /// submitted command; a move reads as the move, the dimensions it sets
    /// and what moves with it (9.4, Q-76). The user confirms this text.
    pub echo: Vec<String>,
    /// Entities that move as a consequence of a move command (8.7).
    pub consequences: Vec<Uuid>,
    /// The resulting model rests on Proposed values (7.4, Q-41).
    pub provisional: bool,
    pub diff: Vec<DiffEntry>,
    pub constraint_status: BTreeMap<Uuid, ConstraintReport>,
    pub conflicts: Vec<Conflict>,
    pub violations: Vec<Violation>,
    pub ambiguities: Vec<Ambiguity>,
    pub flags: Vec<Flag>,
}

impl Proposal {
    /// Whether anything stops this proposal from being committed (Q-35).
    pub fn blocked(&self) -> bool {
        self.violations.iter().any(|v| !v.acknowledged)
            || !self.conflicts.is_empty()
            || self.ambiguities.iter().any(|a| a.kind.blocks_commit())
    }
}

/// Why a proposal could not be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposeError {
    Command { index: usize, error: CommandError },
    Move { index: usize, error: MoveError },
    Derive(DeriveError),
    NothingToUndo,
    NothingToRedo,
    Json(CanonicalError),
}

impl fmt::Display for ProposeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProposeError::Command { index, error } => write!(f, "command {index}: {error}"),
            ProposeError::Move { index, error } => write!(f, "command {index}: {error}"),
            ProposeError::Derive(e) => write!(f, "{e}"),
            ProposeError::NothingToUndo => f.write_str("nothing to undo"),
            ProposeError::NothingToRedo => f.write_str("nothing to redo"),
            ProposeError::Json(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ProposeError {}

/// Why a commit was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitError {
    /// The head moved since the proposal was made; propose again (8.4).
    Stale {
        base_version: u64,
        head_version: u64,
    },
    /// The proposal does not match its id.
    Altered,
    /// Violations, Conflicts or ambiguities block it (Q-35).
    Blocked,
    Propose(ProposeError),
    Log(LogError),
}

impl fmt::Display for CommitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommitError::Stale {
                base_version,
                head_version,
            } => write!(
                f,
                "proposal is based on version {base_version} but the head is {head_version}"
            ),
            CommitError::Altered => f.write_str("the proposal does not match its id"),
            CommitError::Blocked => {
                f.write_str("unacknowledged violations, conflicts or ambiguities block commit")
            }
            CommitError::Propose(e) => write!(f, "{e}"),
            CommitError::Log(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CommitError {}

/// Fills every value a command leaves to defaults, so it is explicit (8.3).
/// Arguments that cannot be filled are left as they are; `apply` reports them.
fn resolve(document: &Document, command: &Command) -> Command {
    use crate::create;
    let mut c = command.clone();
    match &mut c.action {
        Action::CreateOpening(args) => {
            let d = opening_size(args.kind);
            let fill = |v: &mut Option<Sourced<Length>>, value| {
                if v.is_none() {
                    *v = Some(Sourced {
                        value,
                        source: Source::Proposed,
                    });
                }
            };
            let CreateOpening {
                width,
                height,
                sill,
                ..
            } = args;
            fill(width, d.width);
            fill(height, d.height);
            fill(sill, d.sill);
        }
        Action::CreateWall(args) => {
            let _ = create::fill_wall(document, args);
        }
        Action::AddConstraint(args) => {
            let _ = create::fill_constraint(args);
        }
        Action::PlaceOccurrence(args) => create::fill_occurrence(args),
        Action::CreateSlab(args) => create::fill_slab(args),
        Action::CreateCeiling(args) => {
            let _ = create::fill_ceiling(document, args);
        }
        Action::CreateRoof(args) => {
            let _ = create::fill_roof(args);
        }
        Action::CreateElementType(args) => create::fill_element_type(args),
        _ => {}
    }
    c
}

fn proposal_id(base_version: u64, action: &ProposedAction) -> Result<Uuid, CanonicalError> {
    let text = to_canonical_json(action)?;
    Ok(Uuid::derive(
        PROJECT_NAMESPACE,
        "proposal",
        &format!("{base_version}:{text}"),
    ))
}

/// Proposes `action` against the head of `history`. Stores nothing.
pub fn propose(history: &History, action: &ProposedAction) -> Result<Proposal, ProposeError> {
    let base = history.document();
    let mut ambiguities = Vec::new();
    let mut consequences = BTreeSet::new();
    let mut obstructions = Vec::new();
    let mut echo = Vec::new();
    let (resolved, after, solution, violations) = match action {
        ProposedAction::Commands { commands } => {
            let mut resolved = Vec::new();
            let mut next = base.clone();
            for (index, c) in commands.iter().enumerate() {
                // Only submitted commands: what defaults and move resolution
                // generate below is not user-stated (5.3, T-190).
                check_user_grid(&c.action)
                    .map_err(|error| ProposeError::Command { index, error })?;
                let Some(request) = request_for(&next, &c.action) else {
                    let r = resolve(&next, c);
                    let applied =
                        apply(&next, &r).map_err(|error| ProposeError::Command { index, error })?;
                    echo.push(crate::echo::render(&next, &applied, &r));
                    next = applied;
                    resolved.push(r);
                    continue;
                };
                let request = request.map_err(|error| ProposeError::Move { index, error })?;
                // Resolve against the solved state so far (8.7).
                let current = derive(next.clone()).map_err(ProposeError::Derive)?.document;
                let outcome = resolve_move(&current, &request)
                    .map_err(|error| ProposeError::Move { index, error })?;
                echo.push(crate::echo::move_entry(&current, c, &outcome));
                let edits = match outcome {
                    MoveOutcome::Edits {
                        edits,
                        moved,
                        alternatives,
                        ..
                    } => {
                        consequences.extend(moved);
                        if !alternatives.is_empty() {
                            ambiguities.push(Ambiguity {
                                kind: AmbiguityKind::PolicyChoice,
                                candidates: alternatives,
                                default: None,
                            });
                        }
                        edits
                    }
                    MoveOutcome::Ambiguous { candidates } => {
                        ambiguities.push(Ambiguity {
                            kind: AmbiguityKind::ReferenceSemantics,
                            candidates,
                            default: None,
                        });
                        Vec::new()
                    }
                    MoveOutcome::Obstructed { fixed, delta } => {
                        obstructions.push(Conflict {
                            constraint: request.entity,
                            with: fixed,
                            residual: Some(delta),
                        });
                        Vec::new()
                    }
                };
                for (i, e) in edits.into_iter().enumerate() {
                    let edit = Command {
                        id: Uuid::derive(c.id, "edit", &i.to_string()),
                        idempotency_key: Uuid::derive(c.idempotency_key, "edit", &i.to_string()),
                        action: Action::SetConstraintValue(e),
                    };
                    next = apply(&next, &edit)
                        .map_err(|error| ProposeError::Command { index, error })?;
                    resolved.push(edit);
                }
            }
            let d = derive(next).map_err(ProposeError::Derive)?;
            (
                ProposedAction::Commands { commands: resolved },
                d.document,
                d.solution,
                d.violations,
            )
        }
        ProposedAction::Undo | ProposedAction::Redo => {
            let target = match action {
                ProposedAction::Undo => {
                    echo.push(crate::echo::undo_redo("Undo", history.peek_undo_commit()));
                    history.peek_undo().ok_or(ProposeError::NothingToUndo)?
                }
                _ => {
                    echo.push(crate::echo::undo_redo("Redo", history.peek_redo_commit()));
                    history.peek_redo().ok_or(ProposeError::NothingToRedo)?
                }
            };
            // Undo and redo restore a stored state exactly; report on it as is.
            let solution = LayerA::default()
                .solve(target)
                .map_err(|e| ProposeError::Derive(DeriveError::Solve(e)))?;
            let violations = validate(target, MIN_JAMB_DISTANCE)
                .map_err(|e| ProposeError::Derive(DeriveError::Geometry(e)))?;
            (action.clone(), target.clone(), solution, violations)
        }
    };
    let diff = diff_documents(base, &after)?;
    let flags = flags(base, &after, history.baseline(), &diff)?;
    let mut conflicts = solution.conflicts;
    conflicts.extend(obstructions);
    Ok(Proposal {
        proposal_id: proposal_id(history.version(), action).map_err(ProposeError::Json)?,
        base_version: history.version(),
        action: action.clone(),
        resolved,
        echo,
        consequences: consequences.into_iter().collect(),
        provisional: crate::params::is_provisional(&after),
        diff,
        constraint_status: solution.constraints,
        conflicts,
        violations,
        ambiguities,
        flags,
    })
}

/// Commits `proposal`, appending one event to `history` (8.4).
pub fn commit(
    history: &mut History,
    proposal: &Proposal,
    timestamp_ms: i64,
    actor: &str,
) -> Result<LogEntry, CommitError> {
    if proposal.base_version != history.version() {
        return Err(CommitError::Stale {
            base_version: proposal.base_version,
            head_version: history.version(),
        });
    }
    // Proposing again checks the proposal against its id and recomputes
    // what blocks it, so nothing in the caller's copy is trusted.
    let fresh = propose(history, &proposal.action).map_err(CommitError::Propose)?;
    if fresh != *proposal {
        return Err(CommitError::Altered);
    }
    if fresh.blocked() {
        return Err(CommitError::Blocked);
    }
    let event = match &fresh.resolved {
        ProposedAction::Commands { commands } => Event::Commit {
            commands: commands.clone(),
        },
        ProposedAction::Undo => Event::Undo,
        ProposedAction::Redo => Event::Redo,
    };
    let entry = LogEntry {
        sequence: history.version().saturating_add(1),
        timestamp_ms,
        actor: actor.to_string(),
        event,
    };
    history.apply_entry(&entry).map_err(CommitError::Log)?;
    Ok(entry)
}

fn json(doc: &Document) -> Result<Value, ProposeError> {
    serde_json::to_value(doc).map_err(|e| ProposeError::Json(CanonicalError::Json(e.to_string())))
}

/// The JSON Schema of the document, through which Lengths are found.
fn document_schema() -> Result<Value, ProposeError> {
    serde_json::to_value(schemars::schema_for!(Document))
        .map_err(|e| ProposeError::Json(CanonicalError::Json(e.to_string())))
}

/// The schema of one entity of the collection `key`.
fn entity_schema(root: &Value, key: &str) -> Value {
    root.get("properties")
        .and_then(|p| p.get(key))
        .and_then(|f| f.get("additionalProperties"))
        .cloned()
        .unwrap_or(Value::Null)
}

/// One entry per changed entity, ordered by collection then id (Q-36).
fn diff_documents(base: &Document, after: &Document) -> Result<Vec<DiffEntry>, ProposeError> {
    let (a, b) = (json(base)?, json(after)?);
    let root = document_schema()?;
    let empty = serde_json::Map::new();
    let (am, bm) = (
        a.as_object().unwrap_or(&empty),
        b.as_object().unwrap_or(&empty),
    );
    let keys: BTreeSet<&String> = am.keys().chain(bm.keys()).collect();
    let mut out = Vec::new();
    for key in keys {
        let field_schema = root
            .get("properties")
            .and_then(|p| p.get(key))
            .cloned()
            .unwrap_or(Value::Null);
        let mut push = |id: Option<Uuid>, x: Option<&Value>, y: Option<&Value>, schema: &Value| {
            if x == y {
                return;
            }
            let change = match (x, y) {
                (None, Some(_)) => Change::Added,
                (Some(_), None) => Change::Removed,
                _ => Change::Changed,
            };
            let mut paths = Vec::new();
            if let (Some(x), Some(y)) = (x, y) {
                changed_paths(x, y, "", &mut paths);
            }
            out.push(DiffEntry {
                collection: key.clone(),
                id,
                change,
                before: x.cloned(),
                after: y.cloned(),
                before_display: x.map(|v| display(v, schema, &root)),
                after_display: y.map(|v| display(v, schema, &root)),
                changed_paths: paths,
            });
        };
        if COLLECTIONS.contains(&key.as_str()) {
            let entity_schema = entity_schema(&root, key);
            let (xa, ya) = (
                am.get(key).and_then(Value::as_object).unwrap_or(&empty),
                bm.get(key).and_then(Value::as_object).unwrap_or(&empty),
            );
            let ids: BTreeSet<&String> = xa.keys().chain(ya.keys()).collect();
            for id in ids {
                let uuid: Option<Uuid> = serde_json::from_value(Value::String(id.clone())).ok();
                push(uuid, xa.get(id), ya.get(id), &entity_schema);
            }
        } else {
            push(None, am.get(key), bm.get(key), &field_schema);
        }
    }
    Ok(out)
}

/// JSON pointers of every `{source: "proposed", value}` object in `v`.
fn proposed_paths(v: &Value, at: &str, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            if map.get("source").and_then(Value::as_str) == Some("proposed")
                && map.contains_key("value")
            {
                out.push(at.to_string());
            }
            for (k, x) in map {
                proposed_paths(x, &format!("{at}/{k}"), out);
            }
        }
        Value::Array(items) => {
            for (i, x) in items.iter().enumerate() {
                proposed_paths(x, &format!("{at}/{i}"), out);
            }
        }
        _ => {}
    }
}

/// ProposedValue flags, sorted, then ExistingConditionEdit flags in diff
/// order (collection, then id), each with the baseline values (6.7) of the
/// entity's changed paths.
fn flags(
    base: &Document,
    after: &Document,
    baseline: &Document,
    diff: &[DiffEntry],
) -> Result<Vec<Flag>, ProposeError> {
    let existing = |wall: &Uuid| {
        [base, after].iter().any(|d| {
            d.walls
                .get(wall)
                .is_some_and(|w| w.function == Function::Existing)
        })
    };
    let mut proposed = BTreeSet::new();
    let mut edits = Vec::new();
    // The baseline as JSON and the schema, made only when first needed.
    let mut baseline_json: Option<(Value, Value)> = None;
    for d in diff {
        if let (Some(after_value), true) = (&d.after, d.change != Change::Removed) {
            let mut paths = Vec::new();
            proposed_paths(after_value, "", &mut paths);
            for p in paths {
                let touched = d.change == Change::Added
                    || d.changed_paths
                        .iter()
                        .any(|c| c == &p || c.starts_with(&format!("{p}/")));
                if touched {
                    proposed.insert((d.collection.clone(), d.id, p));
                }
            }
        }
        let Some(id) = d.id else { continue };
        let on_existing = match d.collection.as_str() {
            "walls" => existing(&id),
            "openings" => [base, after]
                .iter()
                .any(|doc| doc.openings.get(&id).is_some_and(|o| existing(&o.host))),
            "occurrences" => [base, after].iter().any(|doc| {
                doc.occurrences
                    .get(&id)
                    .is_some_and(|o| match &o.placement {
                        Placement::WallMounted { wall, .. } => existing(wall),
                        _ => false,
                    })
            }),
            _ => false,
        };
        let is_wall_edit = d.collection != "walls" || d.change != Change::Added;
        if on_existing && is_wall_edit {
            let mut values = Vec::new();
            if !d.changed_paths.is_empty() {
                let (json_baseline, root) = match baseline_json.take() {
                    Some(made) => made,
                    None => (json(baseline)?, document_schema()?),
                };
                let entity = serde_json::to_value(id)
                    .ok()
                    .and_then(|key| {
                        json_baseline
                            .get(&d.collection)
                            .and_then(|c| c.get(key.as_str()?))
                    })
                    .cloned();
                let shown = entity
                    .as_ref()
                    .map(|v| display(v, &entity_schema(&root, &d.collection), &root));
                for path in &d.changed_paths {
                    values.push(BaselineValue {
                        path: path.clone(),
                        value: entity.as_ref().and_then(|v| v.pointer(path)).cloned(),
                        display: shown.as_ref().and_then(|v| v.pointer(path)).cloned(),
                    });
                }
                baseline_json = Some((json_baseline, root));
            }
            edits.push(Flag::ExistingConditionEdit {
                collection: d.collection.clone(),
                id,
                baseline: values,
            });
        }
    }
    let mut out: Vec<Flag> = proposed
        .into_iter()
        .map(|(collection, id, path)| Flag::ProposedValue {
            collection,
            id,
            path,
        })
        .collect();
    out.extend(edits);
    Ok(out)
}

/// The document's entity maps, keyed by id; every other field is a singleton.
const COLLECTIONS: &[&str] = &[
    "assets",
    "camera_bookmarks",
    "ceilings",
    "constraints",
    "datums",
    "element_types",
    "features",
    "layer_sets",
    "materials",
    "nodes",
    "occurrences",
    "openings",
    "roofs",
    "slabs",
    "spaces",
    "storeys",
    "underlays",
    "variant_slots",
    "walls",
];

/// JSON pointers of every value that differs between `a` and `b`.
fn changed_paths(a: &Value, b: &Value, at: &str, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                let path = format!("{at}/{}", k.replace('~', "~0").replace('/', "~1"));
                match (x.get(k), y.get(k)) {
                    (Some(p), Some(q)) => changed_paths(p, q, &path, out),
                    _ => out.push(path),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                changed_paths(p, q, &format!("{at}/{i}"), out);
            }
        }
        _ if a != b => out.push(at.to_string()),
        _ => {}
    }
}

/// `value` with every Length (found through `schema`) shown in feet and inches.
fn display(value: &Value, schema: &Value, root: &Value) -> Value {
    if let Some(name) = schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix("#/$defs/"))
    {
        if name == "Length"
            && let Some(n) = value.as_i64()
        {
            return Value::String(format_length(Length::from_nanometers(n)));
        }
        if let Some(def) = root.get("$defs").and_then(|d| d.get(name)) {
            return display(value, def, root);
        }
        return value.clone();
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(branches) = schema.get(key).and_then(Value::as_array) {
            let pick = branches.iter().find(|b| matches_branch(value, b, root));
            return pick.map_or_else(|| value.clone(), |b| display(value, b, root));
        }
    }
    match value {
        Value::Object(map) => {
            let props = schema.get("properties");
            let extra = schema.get("additionalProperties").filter(|v| v.is_object());
            Value::Object(
                map.iter()
                    .map(|(k, v)| {
                        let sub = props.and_then(|p| p.get(k)).or(extra);
                        (
                            k.clone(),
                            sub.map_or_else(|| v.clone(), |s| display(v, s, root)),
                        )
                    })
                    .collect(),
            )
        }
        Value::Array(items) => {
            let sub = schema.get("items");
            Value::Array(
                items
                    .iter()
                    .map(|v| sub.map_or_else(|| v.clone(), |s| display(v, s, root)))
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

/// Whether `value` belongs to the schema branch `b` of an anyOf or oneOf.
fn matches_branch(value: &Value, b: &Value, root: &Value) -> bool {
    let b = match b
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix("#/$defs/"))
        .and_then(|name| root.get("$defs").and_then(|d| d.get(name)))
    {
        Some(def) if def.get("properties").is_some() || def.get("type").is_some() => def,
        _ => b,
    };
    match value {
        Value::Null => b.get("type").and_then(Value::as_str) == Some("null"),
        Value::Object(map) => {
            let props = b.get("properties");
            if let (Some(kind), Some(expected)) = (
                map.get("kind"),
                props
                    .and_then(|p| p.get("kind"))
                    .and_then(|k| k.get("const")),
            ) {
                return kind == expected;
            }
            if map.len() == 1
                && let (Some(k), Some(req)) = (
                    map.keys().next(),
                    b.get("required").and_then(Value::as_array),
                )
            {
                return matches!(req.as_slice(), [only] if only.as_str() == Some(k.as_str()));
            }
            b.get("type").and_then(Value::as_str) != Some("null")
        }
        _ => b.get("type").and_then(Value::as_str) != Some("null"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::tests::{cmd, create_node, genesis, set_storey, with_wall};
    use crate::command::{
        DimensionTarget, SetConstraintValue, SetProjectUnits, SetVariantSelection,
    };
    use crate::document::*;
    use crate::units::ProjectUnit;

    fn bytes(doc: &Document) -> String {
        to_canonical_json(doc).unwrap_or_else(|e| e.to_string())
    }

    fn commands(c: Vec<Command>) -> ProposedAction {
        ProposedAction::Commands { commands: c }
    }

    fn uid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn door(n: u8, host: Uuid, from: Uuid, feet: i64) -> Command {
        cmd(
            n,
            Action::CreateOpening(CreateOpening {
                kind: OpeningKind::Door,
                host,
                reference: Reference::Node { node: from },
                anchor: OpeningEdge::Start,
                offset: Sourced {
                    value: Length::from_nanometers(feet * 304_800_000),
                    source: Source::UserStated,
                },
                width: None,
                height: None,
                sill: None,
            }),
        )
    }

    use crate::command::{Motion, MoveWall};
    use crate::edit::tests::{center, chain};

    fn move_b(edit: Option<Uuid>, absorb: Vec<Uuid>, wall: Uuid) -> ProposedAction {
        commands(vec![cmd(
            40,
            Action::MoveWall(MoveWall {
                wall,
                reference: Reference::WallCenter { wall },
                motion: Motion::Delta {
                    delta: Length::from_nanometers(12 * 25_400_000),
                },
                edit,
                absorb,
            }),
        )])
    }

    /// T-199e, Q-76 (b): a move echoes as the move, the dimensions it sets
    /// and what moves with it; other commands one sentence each.
    #[test]
    fn a_proposal_echoes_each_command_in_plain_language() -> Result<(), ProposeError> {
        let (doc, [_, b, _], _, _) = crate::echo::tests::labeled_chain();
        let history = History::new(doc);
        let mut action = move_b(None, vec![], b);
        if let ProposedAction::Commands { commands } = &mut action {
            commands.push(cmd(41, create_node(0, 0)));
        }
        let p = propose(&history, &action)?;
        assert_eq!(
            p.echo,
            vec![
                r#"Move W-002 center 1'-0" toward +X: set the distance from W-001 center to W-002 center along X from 8'-0" to 9'-0"; W-003, the window in W-003 at 2'-0" from node at (24'-0", 0'-0") move with it"#,
                r#"Create a node at (0'-0", 0'-0")"#,
            ]
        );
        Ok(())
    }

    /// Done when: the same proposal always renders the same text.
    #[test]
    fn the_same_proposal_renders_the_same_text() -> Result<(), ProposeError> {
        let (doc, [_, b, _], [_, _, c3], _) = crate::echo::tests::labeled_chain();
        let first = History::new(doc.clone());
        let second = History::new(doc);
        for action in [move_b(None, vec![], b), move_b(None, vec![c3], b)] {
            let p = propose(&first, &action)?;
            assert_eq!(p.echo.len(), 1);
            assert_eq!(propose(&first, &action)?.echo, p.echo);
            assert_eq!(propose(&second, &action)?.echo, p.echo);
        }
        Ok(())
    }

    /// Undo and redo name the commit they act on.
    #[test]
    fn undo_and_redo_echo_the_commit_number() -> Result<(), CommitError> {
        let mut history = History::new(genesis());
        for (n, unit) in [(1, ProjectUnit::Feet), (2, ProjectUnit::Inches)] {
            let set = Action::SetProjectUnits(SetProjectUnits { default_unit: unit });
            let p =
                propose(&history, &commands(vec![cmd(n, set)])).map_err(CommitError::Propose)?;
            commit(&mut history, &p, i64::from(n), "user")?;
        }
        let undo = propose(&history, &ProposedAction::Undo).map_err(CommitError::Propose)?;
        assert_eq!(undo.echo, vec!["Undo commit 2"]);
        commit(&mut history, &undo, 3, "user")?;
        let undo = propose(&history, &ProposedAction::Undo).map_err(CommitError::Propose)?;
        assert_eq!(undo.echo, vec!["Undo commit 1"]);
        let redo = propose(&history, &ProposedAction::Redo).map_err(CommitError::Propose)?;
        assert_eq!(redo.echo, vec!["Redo commit 2"]);
        Ok(())
    }

    #[test]
    fn a_chain_move_commits_as_explicit_edits() -> Result<(), CommitError> {
        let (doc, [_, b, c], [_, c2, c3], window) = chain();
        let _ = center(b);
        let mut history = History::new(doc.clone());
        let p = propose(&history, &move_b(None, vec![], b)).map_err(CommitError::Propose)?;
        let mut downstream = vec![c, window];
        downstream.sort();
        assert_eq!(p.consequences, downstream);
        assert_eq!(p.ambiguities.len(), 1);
        assert_eq!(p.ambiguities[0].kind, AmbiguityKind::PolicyChoice);
        assert_eq!(
            p.ambiguities[0]
                .candidates
                .iter()
                .map(|c| c.choice)
                .collect::<Vec<_>>(),
            vec![c3]
        );
        assert!(!p.blocked());
        let entry = commit(&mut history, &p, 1_000, "user")?;
        let Event::Commit { commands } = &entry.event else {
            return Err(CommitError::Altered);
        };
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0].action,
            Action::SetConstraintValue(SetConstraintValue { target: DimensionTarget::Constraint { constraint }, value })
                if constraint == c2 && value == Length::from_nanometers(108 * 25_400_000)
        ));
        let mut replayed = History::new(doc);
        replayed.replay(&[entry]).map_err(CommitError::Log)?;
        assert_eq!(bytes(replayed.document()), bytes(history.document()));
        Ok(())
    }

    #[test]
    fn reference_semantics_blocks_until_answered() -> Result<(), CommitError> {
        let mut doc = crate::edit::tests::base();
        let a = crate::edit::tests::wall_at(&mut doc, 1, 120);
        let c1 = crate::edit::tests::dist(
            &mut doc,
            1,
            crate::edit::tests::datum(crate::edit::tests::WEST),
            center(a),
            120,
        );
        crate::edit::tests::dist(
            &mut doc,
            4,
            center(a),
            crate::edit::tests::datum(crate::edit::tests::EAST),
            120,
        );
        let mut history = History::new(doc);
        let p = propose(&history, &move_b(None, vec![], a)).map_err(CommitError::Propose)?;
        assert_eq!(
            p.ambiguities.first().map(|a| a.kind),
            Some(AmbiguityKind::ReferenceSemantics)
        );
        assert!(p.blocked());
        assert_eq!(
            commit(&mut history, &p, 1_000, "user"),
            Err(CommitError::Blocked)
        );
        let p = propose(&history, &move_b(Some(c1), vec![], a)).map_err(CommitError::Propose)?;
        assert!(!p.blocked());
        commit(&mut history, &p, 1_000, "user")?;
        Ok(())
    }

    #[test]
    fn propose_persists_nothing() -> Result<(), ProposeError> {
        let history = History::new(genesis());
        let before = history.clone();
        let action = commands(vec![
            cmd(1, set_storey(None, "Ground")),
            cmd(2, create_node(0, 0)),
        ]);
        let a = propose(&history, &action)?;
        assert_eq!(history, before);
        assert_eq!(bytes(history.document()), bytes(&genesis()));
        assert_eq!(history.version(), 0);
        // Deterministic: the same action proposes the same proposal.
        assert_eq!(propose(&history, &action)?, a);
        Ok(())
    }

    /// T-190: an off-grid user-stated value fails the proposal with the
    /// command's index and the nearest grid value (5.3).
    #[test]
    fn an_off_grid_user_stated_value_is_rejected() {
        let history = History::new(genesis());
        let mut storey = set_storey(None, "Ground");
        if let Action::SetStorey(s) = &mut storey {
            s.floor_to_floor.value = Length::from_nanometers(3_048_000_001);
        }
        let action = commands(vec![cmd(1, create_node(0, 0)), cmd(2, storey)]);
        assert_eq!(
            propose(&history, &action),
            Err(ProposeError::Command {
                index: 1,
                error: CommandError::OffGrid {
                    argument: "/args/floor_to_floor".into(),
                    value: Length::from_nanometers(3_048_000_001),
                    nearest: Length::from_nanometers(3_048_000_000),
                },
            })
        );
    }

    #[test]
    fn a_stale_base_version_is_rejected() -> Result<(), CommitError> {
        let mut history = History::new(genesis());
        let first = propose(&history, &commands(vec![cmd(1, create_node(0, 0))]))
            .map_err(CommitError::Propose)?;
        let second = propose(&history, &commands(vec![cmd(2, create_node(5, 5))]))
            .map_err(CommitError::Propose)?;
        commit(&mut history, &first, 1_000, "user")?;
        assert_eq!(
            commit(&mut history, &second, 2_000, "user"),
            Err(CommitError::Stale {
                base_version: 0,
                head_version: 1
            })
        );
        assert_eq!(history.version(), 1);
        Ok(())
    }

    #[test]
    fn an_altered_proposal_is_rejected() -> Result<(), CommitError> {
        let mut history = History::new(genesis());
        let mut p = propose(&history, &commands(vec![cmd(1, create_node(0, 0))]))
            .map_err(CommitError::Propose)?;
        p.resolved = commands(vec![cmd(1, create_node(9, 9))]);
        assert_eq!(
            commit(&mut history, &p, 1_000, "user"),
            Err(CommitError::Altered)
        );
        assert_eq!(history.version(), 0);
        Ok(())
    }

    fn entries(p: &Proposal) -> Vec<(String, Option<Uuid>, Change)> {
        p.diff
            .iter()
            .map(|d| (d.collection.clone(), d.id, d.change))
            .collect()
    }

    #[test]
    fn the_diff_lists_every_changed_entity() -> Result<(), ProposeError> {
        let history = History::new(genesis());
        let storey = cmd(1, set_storey(None, "Ground"));
        let node = cmd(2, create_node(7, -3));
        let units = cmd(
            3,
            Action::SetProjectUnits(SetProjectUnits {
                default_unit: ProjectUnit::Feet,
            }),
        );
        let select = cmd(
            4,
            Action::SetVariantSelection(SetVariantSelection {
                slot: uid(10),
                selected: uid(12),
            }),
        );
        let new = |c: &Command| Some(Uuid::derive(c.idempotency_key, "command", "0"));
        let p = propose(
            &history,
            &commands(vec![storey.clone(), node.clone(), units, select]),
        )?;
        let mut expected = vec![
            ("nodes".to_string(), new(&node), Change::Added),
            ("project".to_string(), None, Change::Changed),
            ("storeys".to_string(), new(&storey), Change::Added),
            ("variant_slots".to_string(), Some(uid(10)), Change::Changed),
        ];
        expected.sort_by_key(|e| (e.0.clone(), e.1));
        assert_eq!(entries(&p), expected);
        let project = p.diff.iter().find(|d| d.collection == "project");
        assert_eq!(
            project.map(|d| d.changed_paths.clone()),
            Some(vec!["/default_unit".to_string()])
        );
        let slot = p.diff.iter().find(|d| d.collection == "variant_slots");
        assert_eq!(
            slot.map(|d| d.changed_paths.clone()),
            Some(vec!["/selected".to_string()])
        );
        Ok(())
    }

    #[test]
    fn the_diff_shows_display_units_and_flags_proposed_values() -> Result<(), ProposeError> {
        let (doc, wall, a) = with_wall();
        let history = History::new(doc);
        let c = door(1, wall, a, 4);
        let p = propose(&history, &commands(vec![c.clone()]))?;
        let id = Some(Uuid::derive(c.idempotency_key, "command", "0"));
        let added = p
            .diff
            .iter()
            .find(|d| d.collection == "openings" && d.id == id);
        assert_eq!(added.map(|d| d.change), Some(Change::Added));
        let shown = added
            .and_then(|d| d.after_display.clone())
            .unwrap_or_default();
        assert_eq!(shown["width"]["value"], Value::String("3'-0\"".into()));
        assert_eq!(shown["start"], Value::String("4'-0\"".into()));
        let raw = added.and_then(|d| d.after.clone()).unwrap_or_default();
        assert_eq!(raw["width"]["value"], Value::from(914_400_000_i64));
        // The defaults it filled in are Proposed, and the resolved command says so.
        for field in ["/height", "/sill", "/width"] {
            assert!(
                p.flags.contains(&Flag::ProposedValue {
                    collection: "openings".into(),
                    id,
                    path: field.into()
                }),
                "{field}: {:?}",
                p.flags
            );
        }
        let ProposedAction::Commands { commands } = &p.resolved else {
            return Err(ProposeError::NothingToUndo);
        };
        assert!(
            matches!(&commands[0].action, Action::CreateOpening(o) if o.width.is_some() && o.sill.is_some())
        );
        Ok(())
    }

    #[test]
    fn edits_to_existing_conditions_are_flagged() -> Result<(), ProposeError> {
        let (mut doc, wall, a) = with_wall();
        if let Some(w) = doc.walls.get_mut(&wall) {
            w.function = Function::Existing;
        }
        let history = History::new(doc);
        let c = door(1, wall, a, 4);
        let p = propose(&history, &commands(vec![c.clone()]))?;
        let id = Uuid::derive(c.idempotency_key, "command", "0");
        // An added opening has no changed paths, so no baseline values.
        assert!(p.flags.contains(&Flag::ExistingConditionEdit {
            collection: "openings".into(),
            id,
            baseline: Vec::new(),
        }));
        Ok(())
    }

    fn commit_commands(history: &mut History, c: Vec<Command>) -> Result<(), String> {
        let p = propose(history, &commands(c)).map_err(|e| e.to_string())?;
        commit(history, &p, 1_000, "user")
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn set_left_extent(n: u8, wall: Uuid, nm: i64) -> Command {
        let user = |nm: i64| Sourced {
            value: Length::from_nanometers(nm),
            source: Source::UserStated,
        };
        cmd(
            n,
            Action::SetWallLayers(crate::command::SetWallLayers {
                wall,
                layers: vec![Layer {
                    material: uid(25),
                    thickness: user(114_300_000),
                    core: true,
                }],
                left_extent: user(nm),
            }),
        )
    }

    fn baseline_of(p: &Proposal, collection: &str, id: Uuid) -> Option<Vec<BaselineValue>> {
        p.flags.iter().find_map(|f| match f {
            Flag::ExistingConditionEdit {
                collection: c,
                id: i,
                baseline,
            } if c == collection && *i == id => Some(baseline.clone()),
            _ => None,
        })
    }

    fn baseline_at<'a>(baseline: &'a [BaselineValue], path: &str) -> Option<&'a BaselineValue> {
        baseline.iter().find(|b| b.path == path)
    }

    #[test]
    fn a_second_edit_to_an_existing_wall_shows_the_baseline_value() -> Result<(), String> {
        let (mut doc, wall, _) = with_wall();
        if let Some(w) = doc.walls.get_mut(&wall) {
            w.function = Function::Existing;
        }
        let mut history = History::new(doc);
        // First edit: 2 1/4 in to 2 in.
        commit_commands(&mut history, vec![set_left_extent(1, wall, 50_800_000)])?;
        // Second edit: 2 in to 1 1/2 in.
        let p = propose(
            &history,
            &commands(vec![set_left_extent(2, wall, 38_100_000)]),
        )
        .map_err(|e| e.to_string())?;
        let entry = p
            .diff
            .iter()
            .find(|d| d.collection == "walls" && d.id == Some(wall))
            .ok_or("the wall is not in the diff")?;
        // The diff's before is the head, after the first edit.
        assert_eq!(
            entry
                .before
                .as_ref()
                .map(|v| v["left_extent"]["value"].clone()),
            Some(Value::from(50_800_000_i64))
        );
        let baseline = baseline_of(&p, "walls", wall).ok_or(format!("no flag: {:?}", p.flags))?;
        // One baseline value per changed path.
        let paths: Vec<&str> = baseline.iter().map(|b| b.path.as_str()).collect();
        assert_eq!(paths, entry.changed_paths);
        assert_eq!(
            baseline_at(&baseline, "/left_extent/value"),
            Some(&BaselineValue {
                path: "/left_extent/value".into(),
                value: Some(Value::from(57_150_000_i64)),
                display: Some(Value::String("0'-2 1/4\"".into())),
            })
        );
        Ok(())
    }

    #[test]
    fn an_existing_wall_added_later_uses_its_own_first_committed_state() -> Result<(), String> {
        let (doc, _, a) = with_wall();
        let mut history = History::new(doc);
        let node = cmd(1, create_node(0, 10 * 304_800_000));
        let c = Uuid::derive(node.idempotency_key, "command", "0");
        commit_commands(&mut history, vec![node])?;
        let create = cmd(
            2,
            Action::CreateWall(crate::create::CreateWall {
                storey: uid(23),
                node_a: a,
                node_b: c,
                direction: Direction::PosY,
                function: Function::Existing,
                layers: None,
                left_extent: None,
                base_offset: None,
                top: None,
                join_priority: None,
            }),
        );
        let wall = Uuid::derive(create.idempotency_key, "command", "0");
        commit_commands(&mut history, vec![create])?;
        let first = history
            .document()
            .walls
            .get(&wall)
            .map(|w| w.left_extent.value.nanometers())
            .ok_or("the wall was not created")?;
        assert_ne!(first, 50_800_000);
        commit_commands(&mut history, vec![set_left_extent(3, wall, 50_800_000)])?;
        let p = propose(
            &history,
            &commands(vec![set_left_extent(4, wall, 38_100_000)]),
        )
        .map_err(|e| e.to_string())?;
        let baseline = baseline_of(&p, "walls", wall).ok_or(format!("no flag: {:?}", p.flags))?;
        assert_eq!(
            baseline_at(&baseline, "/left_extent/value").map(|b| (&b.value, &b.display)),
            Some((
                &Some(Value::from(first)),
                &Some(Value::String(format_length(Length::from_nanometers(first))))
            ))
        );
        Ok(())
    }

    #[test]
    fn a_proposal_touching_no_existing_element_has_no_such_flag() -> Result<(), String> {
        let is_existing_edit = |f: &Flag| matches!(f, Flag::ExistingConditionEdit { .. });
        // A door on a new wall.
        let (doc, wall, a) = with_wall();
        let p = propose(&History::new(doc), &commands(vec![door(1, wall, a, 4)]))
            .map_err(|e| e.to_string())?;
        assert!(!p.flags.is_empty(), "the door's defaults are flagged");
        assert!(!p.flags.iter().any(is_existing_edit), "{:?}", p.flags);
        // An existing wall in the document, but the proposal leaves it alone.
        let (mut doc, wall, _) = with_wall();
        if let Some(w) = doc.walls.get_mut(&wall) {
            w.function = Function::Existing;
        }
        let units = cmd(
            2,
            Action::SetProjectUnits(SetProjectUnits {
                default_unit: ProjectUnit::Feet,
            }),
        );
        let p = propose(&History::new(doc), &commands(vec![units])).map_err(|e| e.to_string())?;
        assert!(!p.diff.is_empty());
        assert!(!p.flags.iter().any(is_existing_edit), "{:?}", p.flags);
        Ok(())
    }

    #[test]
    fn violations_and_conflicts_block_commit() -> Result<(), CommitError> {
        // A door 19 ft from the start of a 20 ft wall sticks out past its end.
        let (doc, wall, a) = with_wall();
        let mut history = History::new(doc);
        let p = propose(&history, &commands(vec![door(1, wall, a, 19)]))
            .map_err(CommitError::Propose)?;
        assert!(p.blocked());
        assert_eq!(
            commit(&mut history, &p, 1_000, "user"),
            Err(CommitError::Blocked)
        );
        // With the violation acknowledged in the document, the same commit goes through.
        let (mut doc, wall, a) = with_wall();
        let c = door(1, wall, a, 19);
        let id = Uuid::derive(c.idempotency_key, "command", "0");
        doc.acknowledged.insert(ViolationKey {
            kind: ViolationKind::OpeningOutsideHost,
            entities: vec![id],
        });
        let mut history = History::new(doc);
        let p = propose(&history, &commands(vec![c])).map_err(CommitError::Propose)?;
        assert!(!p.blocked());
        commit(&mut history, &p, 1_000, "user")?;
        // A document with clashing constraints: every proposal is blocked.
        let (mut doc, _, a) = with_wall();
        doc.constraints.insert(
            uid(77),
            Constraint {
                kind: ConstraintKind::Fixed {
                    reference: Reference::Node { node: a },
                    axis: Axis::X,
                    value: Length::from_nanometers(5),
                },
                status: ConstraintStatus::Driving,
                source: Source::UserStated,
                provenance: None,
            },
        );
        let mut history = History::new(doc);
        let p = propose(&history, &commands(vec![cmd(1, create_node(0, 0))]))
            .map_err(CommitError::Propose)?;
        assert_eq!(p.conflicts.len(), 1);
        assert_eq!(
            commit(&mut history, &p, 1_000, "user"),
            Err(CommitError::Blocked)
        );
        Ok(())
    }

    #[test]
    fn undo_and_redo_go_through_proposals() -> Result<(), CommitError> {
        let (doc, wall, a) = with_wall();
        let mut history = History::new(doc.clone());
        let p = propose(&history, &commands(vec![door(1, wall, a, 4)]))
            .map_err(CommitError::Propose)?;
        commit(&mut history, &p, 1_000, "user")?;
        let after = bytes(history.document());
        let undo = propose(&history, &ProposedAction::Undo).map_err(CommitError::Propose)?;
        assert_eq!(
            undo.diff.iter().map(|d| d.change).collect::<Vec<_>>(),
            vec![Change::Removed]
        );
        commit(&mut history, &undo, 2_000, "user")?;
        assert_eq!(history.document().openings.len(), 0);
        let redo = propose(&history, &ProposedAction::Redo).map_err(CommitError::Propose)?;
        commit(&mut history, &redo, 3_000, "user")?;
        assert_eq!(bytes(history.document()), after);
        assert_eq!(
            propose(&history, &ProposedAction::Redo),
            Err(ProposeError::NothingToRedo)
        );
        Ok(())
    }

    #[test]
    fn a_committed_log_replays_byte_identically() -> Result<(), CommitError> {
        let (doc, wall, a) = with_wall();
        let mut history = History::new(doc.clone());
        let mut log = Vec::new();
        let p = propose(
            &history,
            &commands(vec![door(1, wall, a, 4), cmd(2, create_node(1, 1))]),
        )
        .map_err(CommitError::Propose)?;
        log.push(commit(&mut history, &p, 1_000, "user")?);
        let p = propose(&history, &ProposedAction::Undo).map_err(CommitError::Propose)?;
        log.push(commit(&mut history, &p, 2_000, "agent:1")?);
        let p = propose(&history, &ProposedAction::Redo).map_err(CommitError::Propose)?;
        log.push(commit(&mut history, &p, 3_000, "user")?);
        let mut replayed = History::new(doc);
        replayed.replay(&log).map_err(CommitError::Log)?;
        assert_eq!(bytes(replayed.document()), bytes(history.document()));
        // The log holds the resolved commands: the defaults are explicit.
        let Event::Commit { commands } = &log[0].event else {
            return Err(CommitError::Altered);
        };
        assert!(matches!(&commands[0].action, Action::CreateOpening(o) if o.width.is_some()));
        Ok(())
    }
}
