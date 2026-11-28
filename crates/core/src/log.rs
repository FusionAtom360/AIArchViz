//! The command log, history, undo, redo and snapshots (spec 5.5, 8.4; Q-10, Q-11).
//!
//! A Commit applies its commands and then derives the document (solve,
//! positions, joins, space boundaries, acknowledgments; Q-35), so replay
//! reproduces committed documents exactly.
//!
//! The log is append-only. Each entry is an envelope holding the session
//! metadata that must stay out of the document (sequence, caller-supplied
//! time, actor) and one event: an atomic Commit of commands, an Undo or a
//! Redo. Undo and redo restore stored prior documents, so they are exact by
//! construction. A new commit clears the redo stack.
//!
//! A [`Snapshot`] is a document at a version. Replaying from it rebuilds the
//! undo history only from that point on, so an undo past it is an error.
//! Duplicate idempotency keys are detected among the events this history has
//! seen (all of them when replaying from the start).
//!
//! The history also keeps the baseline of 6.7: every entity of the document
//! it started from (the genesis document or a snapshot, Q-10), plus each
//! later entity as it was in the first commit that held it. Undo and redo
//! restore the baseline with the document, so an undone commit leaves no
//! trace in it.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::canonical::{CanonicalError, to_canonical_json};
use crate::command::{Command, CommandError, apply};
use crate::document::Document;
use crate::identity::Uuid;
use crate::proposal::{DeriveError, derive};

/// What happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    /// Commands applied atomically, in order (8.4).
    Commit {
        commands: Vec<Command>,
    },
    Undo,
    Redo,
}

/// One log line: the event and its session metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogEntry {
    /// 1 for the first event, then consecutive.
    pub sequence: u64,
    /// Milliseconds since the Unix epoch, supplied by the caller.
    pub timestamp_ms: i64,
    /// Who acted, e.g. "user" or an agent session id.
    pub actor: String,
    pub event: Event,
}

/// A document at a log position (Q-10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    /// The sequence of the last event applied, 0 for the genesis document.
    pub version: u64,
    pub document: Document,
}

/// Why an event cannot be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogError {
    /// Entries must arrive with consecutive sequence numbers.
    OutOfSequence {
        expected: u64,
        found: u64,
    },
    /// The idempotency key was already used, first at `first_sequence`.
    DuplicateCommand {
        key: Uuid,
        first_sequence: u64,
    },
    /// Command `index` of the commit failed; nothing was applied.
    Command {
        index: usize,
        error: CommandError,
    },
    /// The document could not be derived after the commands (Q-35).
    Derive(DeriveError),
    NothingToUndo,
    NothingToRedo,
    /// Undo reached the snapshot this history started from.
    UndoBeyondSnapshot {
        snapshot_version: u64,
    },
}

impl fmt::Display for LogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogError::OutOfSequence { expected, found } => {
                write!(f, "expected log sequence {expected}, found {found}")
            }
            LogError::DuplicateCommand {
                key,
                first_sequence,
            } => write!(
                f,
                "idempotency key {key} was already used at sequence {first_sequence}"
            ),
            LogError::Command { index, error } => write!(f, "command {index} failed: {error}"),
            LogError::Derive(e) => write!(f, "{e}"),
            LogError::NothingToUndo => f.write_str("nothing to undo"),
            LogError::NothingToRedo => f.write_str("nothing to redo"),
            LogError::UndoBeyondSnapshot { snapshot_version } => write!(
                f,
                "cannot undo past the snapshot at version {snapshot_version}; replay from the start"
            ),
        }
    }
}

impl std::error::Error for LogError {}

/// A document and the baseline that goes with it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    document: Document,
    baseline: Document,
}

/// The current document plus everything needed for undo and redo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    document: Document,
    baseline: Document,
    version: u64,
    base_version: u64,
    undo: Vec<State>,
    redo: Vec<State>,
    /// The sequence of the commit each `undo` and `redo` entry would revert
    /// or reapply, in the same order (for the echo text, Q-76).
    undo_commits: Vec<u64>,
    redo_commits: Vec<u64>,
    keys: BTreeMap<Uuid, u64>,
}

/// Adds to `baseline` every entity of `document` it does not hold yet.
fn extend_baseline(baseline: &mut Document, document: &Document) {
    fn extend<T: Clone>(into: &mut BTreeMap<Uuid, T>, from: &BTreeMap<Uuid, T>) {
        for (id, entity) in from {
            into.entry(*id).or_insert_with(|| entity.clone());
        }
    }
    // Listing every field, with no `..`, makes a new collection a compile error here.
    let Document {
        schema_version: _,
        project: _,
        building: _,
        storeys,
        datums,
        nodes,
        walls,
        layer_sets,
        openings,
        slabs,
        ceilings,
        spaces,
        materials,
        element_types,
        occurrences,
        features,
        constraints,
        variant_slots,
        assets,
        roofs,
        underlays,
        camera_bookmarks,
        default_walk_start: _,
        labels: _,
        acknowledged: _,
    } = document;
    extend(&mut baseline.storeys, storeys);
    extend(&mut baseline.datums, datums);
    extend(&mut baseline.nodes, nodes);
    extend(&mut baseline.walls, walls);
    extend(&mut baseline.layer_sets, layer_sets);
    extend(&mut baseline.openings, openings);
    extend(&mut baseline.slabs, slabs);
    extend(&mut baseline.ceilings, ceilings);
    extend(&mut baseline.spaces, spaces);
    extend(&mut baseline.materials, materials);
    extend(&mut baseline.element_types, element_types);
    extend(&mut baseline.occurrences, occurrences);
    extend(&mut baseline.features, features);
    extend(&mut baseline.constraints, constraints);
    extend(&mut baseline.variant_slots, variant_slots);
    extend(&mut baseline.assets, assets);
    extend(&mut baseline.roofs, roofs);
    extend(&mut baseline.underlays, underlays);
    extend(&mut baseline.camera_bookmarks, camera_bookmarks);
}

impl History {
    /// A history starting at the genesis document (version 0).
    pub fn new(genesis: Document) -> Self {
        Self::from_snapshot(Snapshot {
            version: 0,
            document: genesis,
        })
    }

    /// A history starting at a snapshot.
    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        History {
            baseline: snapshot.document.clone(),
            document: snapshot.document,
            version: snapshot.version,
            base_version: snapshot.version,
            undo: Vec::new(),
            redo: Vec::new(),
            undo_commits: Vec::new(),
            redo_commits: Vec::new(),
            keys: BTreeMap::new(),
        }
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// The baseline of 6.7: each entity as it was in the document this
    /// history started from, or in the first commit that held it. Only the
    /// entity maps are meaningful; the other fields are the starting ones.
    pub fn baseline(&self) -> &Document {
        &self.baseline
    }

    /// The sequence of the last event applied (the head version of 8.4).
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The document an Undo would restore, if any.
    pub fn peek_undo(&self) -> Option<&Document> {
        self.undo.last().map(|s| &s.document)
    }

    /// The document a Redo would restore, if any.
    pub fn peek_redo(&self) -> Option<&Document> {
        self.redo.last().map(|s| &s.document)
    }

    /// The sequence of the commit an Undo would revert, if any.
    pub fn peek_undo_commit(&self) -> Option<u64> {
        self.undo_commits.last().copied()
    }

    /// The sequence of the commit a Redo would reapply, if any.
    pub fn peek_redo_commit(&self) -> Option<u64> {
        self.redo_commits.last().copied()
    }

    /// The current state as a snapshot.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            version: self.version,
            document: self.document.clone(),
        }
    }

    /// Applies one log entry. On error nothing changes.
    pub fn apply_entry(&mut self, entry: &LogEntry) -> Result<(), LogError> {
        let expected = self.version.saturating_add(1);
        if entry.sequence != expected {
            return Err(LogError::OutOfSequence {
                expected,
                found: entry.sequence,
            });
        }
        match &entry.event {
            Event::Commit { commands } => {
                let mut seen = BTreeMap::new();
                for c in commands {
                    let first = self
                        .keys
                        .get(&c.idempotency_key)
                        .or(seen.get(&c.idempotency_key));
                    if let Some(&first_sequence) = first {
                        return Err(LogError::DuplicateCommand {
                            key: c.idempotency_key,
                            first_sequence,
                        });
                    }
                    seen.insert(c.idempotency_key, entry.sequence);
                }
                let mut next = self.document.clone();
                for (index, c) in commands.iter().enumerate() {
                    next = apply(&next, c).map_err(|error| LogError::Command { index, error })?;
                }
                // Derive positions, joins, boundaries and acknowledgments (Q-35).
                let next = derive(next).map_err(LogError::Derive)?.document;
                let mut baseline = self.baseline.clone();
                extend_baseline(&mut baseline, &next);
                let prior = self.replace(State {
                    document: next,
                    baseline,
                });
                self.undo.push(prior);
                self.redo.clear();
                self.undo_commits.push(entry.sequence);
                self.redo_commits.clear();
                self.keys.append(&mut seen);
            }
            Event::Undo => {
                let prior = self.undo.pop().ok_or(if self.base_version > 0 {
                    LogError::UndoBeyondSnapshot {
                        snapshot_version: self.base_version,
                    }
                } else {
                    LogError::NothingToUndo
                })?;
                let current = self.replace(prior);
                self.redo.push(current);
                if let Some(c) = self.undo_commits.pop() {
                    self.redo_commits.push(c);
                }
            }
            Event::Redo => {
                let next = self.redo.pop().ok_or(LogError::NothingToRedo)?;
                let current = self.replace(next);
                self.undo.push(current);
                if let Some(c) = self.redo_commits.pop() {
                    self.undo_commits.push(c);
                }
            }
        }
        self.version = entry.sequence;
        Ok(())
    }

    /// Makes `state` current and returns the state it replaces.
    fn replace(&mut self, state: State) -> State {
        State {
            document: std::mem::replace(&mut self.document, state.document),
            baseline: std::mem::replace(&mut self.baseline, state.baseline),
        }
    }

    /// Applies entries in order, stopping at the first error.
    pub fn replay(&mut self, entries: &[LogEntry]) -> Result<(), LogError> {
        entries.iter().try_for_each(|e| self.apply_entry(e))
    }
}

/// The log as JSON Lines: one canonical entry per line, each ending in `\n`.
pub fn log_to_jsonl(entries: &[LogEntry]) -> Result<String, CanonicalError> {
    let mut out = String::new();
    for e in entries {
        out.push_str(&to_canonical_json(e)?);
        out.push('\n');
    }
    Ok(out)
}

/// Parses JSON Lines written by [`log_to_jsonl`]. Blank lines are skipped.
pub fn log_from_jsonl(text: &str) -> Result<Vec<LogEntry>, CanonicalError> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(|e| CanonicalError::Json(e.to_string())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::tests::{cmd, create_node, genesis, set_storey};
    use crate::command::{Action, NoArgs, SetProjectUnits};
    use crate::units::ProjectUnit;
    use proptest::prelude::*;

    fn bytes(doc: &Document) -> String {
        to_canonical_json(doc).unwrap_or_else(|e| e.to_string())
    }

    fn entry(sequence: u64, event: Event) -> LogEntry {
        LogEntry {
            sequence,
            timestamp_ms: 1_764_000_000_000 + i64::try_from(sequence).unwrap_or(0) * 1000,
            actor: "user".into(),
            event,
        }
    }

    fn commit(commands: Vec<Command>) -> Event {
        Event::Commit { commands }
    }

    fn units(n: u8, unit: ProjectUnit) -> Command {
        cmd(
            n,
            Action::SetProjectUnits(SetProjectUnits { default_unit: unit }),
        )
    }

    /// A short session: two commits, an undo, a redo, an undo, a new commit.
    fn session() -> Vec<LogEntry> {
        vec![
            entry(
                1,
                commit(vec![
                    cmd(1, set_storey(None, "Ground")),
                    cmd(2, create_node(0, 0)),
                ]),
            ),
            entry(2, commit(vec![units(3, ProjectUnit::Feet)])),
            entry(3, Event::Undo),
            entry(4, Event::Redo),
            entry(5, Event::Undo),
            entry(6, commit(vec![cmd(4, create_node(3_048_000_000, 0))])),
        ]
    }

    fn run(entries: &[LogEntry]) -> Result<History, LogError> {
        let mut h = History::new(genesis());
        h.replay(entries)?;
        Ok(h)
    }

    #[test]
    fn replaying_a_log_is_byte_identical() -> Result<(), String> {
        // Live: apply entries one by one, as a running app would.
        let mut live = History::new(genesis());
        for e in session() {
            live.apply_entry(&e).map_err(|e| e.to_string())?;
        }
        // Replayed from the serialized log text.
        let text = log_to_jsonl(&session()).map_err(|e| e.to_string())?;
        let parsed = log_from_jsonl(&text).map_err(|e| e.to_string())?;
        assert_eq!(parsed, session());
        let replayed = run(&parsed).map_err(|e| e.to_string())?;
        assert_eq!(bytes(replayed.document()), bytes(live.document()));
        assert_eq!(replayed.version(), 6);
        Ok(())
    }

    #[test]
    fn log_text_golden() -> Result<(), String> {
        let text = log_to_jsonl(&session()).map_err(|e| e.to_string())?;
        insta::assert_binary_snapshot!("session_log.jsonl", text.into_bytes());
        Ok(())
    }

    #[test]
    fn undo_restores_the_exact_prior_state() -> Result<(), String> {
        let before = run(&session()[..1]).map_err(|e| e.to_string())?;
        let after_undo = run(&session()[..3]).map_err(|e| e.to_string())?;
        assert_eq!(bytes(after_undo.document()), bytes(before.document()));
        let committed = run(&session()[..2]).map_err(|e| e.to_string())?;
        let redone = run(&session()[..4]).map_err(|e| e.to_string())?;
        assert_eq!(bytes(redone.document()), bytes(committed.document()));
        // Undo all the way back reaches the genesis document.
        let mut h = run(&session()[..1]).map_err(|e| e.to_string())?;
        h.apply_entry(&entry(2, Event::Undo))
            .map_err(|e| e.to_string())?;
        assert_eq!(bytes(h.document()), bytes(&genesis()));
        assert_eq!(
            h.apply_entry(&entry(3, Event::Undo)),
            Err(LogError::NothingToUndo)
        );
        Ok(())
    }

    #[test]
    fn a_new_commit_clears_redo() -> Result<(), String> {
        let mut h = run(&session()).map_err(|e| e.to_string())?;
        assert_eq!(
            h.apply_entry(&entry(7, Event::Redo)),
            Err(LogError::NothingToRedo)
        );
        assert_eq!(
            h.version(),
            6,
            "a failed event does not advance the version"
        );
        Ok(())
    }

    #[test]
    fn commits_are_atomic() -> Result<(), String> {
        let mut h = History::new(genesis());
        let failing = commit(vec![
            cmd(1, create_node(0, 0)),
            cmd(2, Action::DeleteEntity(NoArgs {})),
        ]);
        assert_eq!(
            h.apply_entry(&entry(1, failing)),
            Err(LogError::Command {
                index: 1,
                error: CommandError::NotYetImplemented("delete_entity")
            })
        );
        assert_eq!(bytes(h.document()), bytes(&genesis()));
        assert_eq!(h.version(), 0);
        // The failed commit consumed nothing: its keys can be used again.
        h.apply_entry(&entry(1, commit(vec![cmd(1, create_node(0, 0))])))
            .map_err(|e| e.to_string())?;
        assert_eq!(h.document().nodes.len(), 1);
        Ok(())
    }

    #[test]
    fn duplicate_idempotency_keys_are_rejected() -> Result<(), String> {
        let mut h = run(&session()[..2]).map_err(|e| e.to_string())?;
        let repeat = commit(vec![units(3, ProjectUnit::Inches)]);
        assert_eq!(
            h.apply_entry(&entry(3, repeat)),
            Err(LogError::DuplicateCommand {
                key: units(3, ProjectUnit::Inches).idempotency_key,
                first_sequence: 2
            })
        );
        // Undo does not free a key: the log is append-only.
        h.apply_entry(&entry(3, Event::Undo))
            .map_err(|e| e.to_string())?;
        assert!(matches!(
            h.apply_entry(&entry(4, commit(vec![units(3, ProjectUnit::Feet)]))),
            Err(LogError::DuplicateCommand {
                first_sequence: 2,
                ..
            })
        ));
        // A key repeated inside one commit.
        let twice = commit(vec![
            units(9, ProjectUnit::Feet),
            units(9, ProjectUnit::Inches),
        ]);
        assert!(matches!(
            h.apply_entry(&entry(4, twice)),
            Err(LogError::DuplicateCommand {
                first_sequence: 4,
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn sequences_must_be_consecutive() {
        let mut h = History::new(genesis());
        assert_eq!(
            h.apply_entry(&entry(2, Event::Undo)),
            Err(LogError::OutOfSequence {
                expected: 1,
                found: 2
            })
        );
    }

    #[test]
    fn the_baseline_holds_first_committed_states_and_follows_undo() -> Result<(), String> {
        let node = |n: u8| Uuid::derive(cmd(n, create_node(0, 0)).idempotency_key, "command", "0");
        let x = |h: &History, n: u8| h.baseline().nodes.get(&node(n)).map(|v| v.x.nanometers());
        let mut h = History::new(genesis());
        h.replay(&[entry(1, commit(vec![cmd(1, create_node(5, 0))]))])
            .map_err(|e| e.to_string())?;
        assert_eq!(x(&h, 1), Some(5));
        // An undone commit leaves nothing in the baseline; redo brings it back.
        h.replay(&[entry(2, Event::Undo)])
            .map_err(|e| e.to_string())?;
        assert_eq!(x(&h, 1), None);
        h.replay(&[entry(3, Event::Redo)])
            .map_err(|e| e.to_string())?;
        assert_eq!(x(&h, 1), Some(5));
        // A history started from a snapshot takes the snapshot as its baseline (Q-10).
        let from = History::from_snapshot(h.snapshot());
        assert_eq!(from.baseline(), h.document());
        assert_eq!(x(&from, 1), Some(5));
        Ok(())
    }

    #[test]
    fn replay_from_a_snapshot() -> Result<(), String> {
        let all = run(&session()).map_err(|e| e.to_string())?;
        let mut from_snap =
            History::from_snapshot(run(&session()[..5]).map_err(|e| e.to_string())?.snapshot());
        from_snap
            .replay(&session()[5..])
            .map_err(|e| e.to_string())?;
        assert_eq!(bytes(from_snap.document()), bytes(all.document()));
        // Undo past the snapshot is refused.
        let mut h = History::from_snapshot(all.snapshot());
        assert_eq!(
            h.apply_entry(&entry(7, Event::Undo)),
            Err(LogError::UndoBeyondSnapshot {
                snapshot_version: 6
            })
        );
        // Snapshots serialize canonically and read back.
        let text = to_canonical_json(&all.snapshot()).map_err(|e| e.to_string())?;
        let back: Snapshot = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        assert_eq!(back, all.snapshot());
        Ok(())
    }

    #[test]
    fn log_parsing_is_strict() {
        let good = log_to_jsonl(&session()[..1]).unwrap_or_default();
        assert!(log_from_jsonl(&good).is_ok());
        assert!(log_from_jsonl(&good.replace("\"actor\"", "\"who\"")).is_err());
        assert!(log_from_jsonl(&good.replace("\"commit\"", "\"rollback\"")).is_err());
        assert_eq!(log_from_jsonl(""), Ok(vec![]));
    }

    #[derive(Debug, Clone)]
    enum Op {
        Node(i64),
        Units(bool),
        Undo,
        Redo,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            (-1000i64..1000).prop_map(Op::Node),
            any::<bool>().prop_map(Op::Units),
            Just(Op::Undo),
            Just(Op::Redo),
        ]
    }

    proptest! {
        /// For any session: replay equals the live history byte for byte,
        /// and every undo lands exactly on the state before its commit.
        #[test]
        fn replay_and_undo_are_exact(ops in proptest::collection::vec(op(), 0..40)) {
            let mut live = History::new(genesis());
            let mut log = Vec::new();
            let mut model: Vec<String> = vec![bytes(&genesis())];
            let mut cursor = 0usize;
            for (i, op) in ops.into_iter().enumerate() {
                let n = u8::try_from(i).unwrap_or(u8::MAX);
                let event = match op {
                    Op::Node(x) => commit(vec![cmd(n, create_node(x, 0))]),
                    Op::Units(feet) => commit(vec![units(n, if feet { ProjectUnit::Feet } else { ProjectUnit::Inches })]),
                    Op::Undo => Event::Undo,
                    Op::Redo => Event::Redo,
                };
                let e = entry(live.version() + 1, event.clone());
                if live.apply_entry(&e).is_ok() {
                    match event {
                        Event::Commit { .. } => {
                            model.truncate(cursor + 1);
                            model.push(bytes(live.document()));
                            cursor += 1;
                        }
                        Event::Undo => cursor -= 1,
                        Event::Redo => cursor += 1,
                    }
                    log.push(e);
                }
                prop_assert_eq!(&bytes(live.document()), &model[cursor]);
            }
            let replayed = run(&log).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(bytes(replayed.document()), bytes(live.document()));
        }
    }
}
