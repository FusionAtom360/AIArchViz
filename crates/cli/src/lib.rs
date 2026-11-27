//! C2 cli: JSON in, JSON out (spec 3, 3.1; Q-47 to Q-50).
//!
//! `archviz <subcommand> <project-folder>` reads one JSON request on stdin
//! (empty means `{}`) and writes one JSON response on stdout:
//! `{"ok": true, "result": ...}` or
//! `{"ok": false, "error": {"code": ..., "message": ..., "details": ...}}`.
//! The exit status is 0 on success and fixed per error code. Ids and
//! timestamps the request leaves out are generated here, never in core.

pub mod host;
pub mod store;

use std::path::Path;

use archviz_core::{
    Building, CommitError, Document, LayerA, LogEntry, MIN_JAMB_DISTANCE, Project as ProjectInfo,
    ProjectUnit, Proposal, ProposeError, ProposedAction, RequiredParameter, Solution, Solver, Uuid,
    Violation, blocks_commit, commit, propose, required_parameters, to_canonical_json,
    to_pretty_json, validate,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable error codes (Q-50). Never change a code's meaning once released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    BadRequest,
    NotAProject,
    ProjectExists,
    Locked,
    CorruptProject,
    CommandFailed,
    Stale,
    Altered,
    Blocked,
    NothingToUndo,
    NotFound,
    Io,
    Unauthorized,
}

impl ErrorCode {
    /// The process exit status for this error.
    pub fn exit_status(self) -> i32 {
        match self {
            ErrorCode::BadRequest => 2,
            ErrorCode::NotAProject => 3,
            ErrorCode::ProjectExists => 4,
            ErrorCode::Locked => 5,
            ErrorCode::CorruptProject => 6,
            ErrorCode::CommandFailed => 7,
            ErrorCode::Stale => 8,
            ErrorCode::Altered => 9,
            ErrorCode::Blocked => 10,
            ErrorCode::NothingToUndo => 11,
            ErrorCode::NotFound => 12,
            ErrorCode::Io => 13,
            ErrorCode::Unauthorized => 14,
        }
    }
}

/// A failed request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CliError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<Value>,
}

impl CliError {
    pub fn new(code: ErrorCode, message: String) -> Self {
        CliError {
            code,
            message,
            details: None,
        }
    }
}

/// The response every subcommand writes.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Response<T> {
    pub ok: bool,
    pub result: Option<T>,
    pub error: Option<CliError>,
}

/// `new`: creates a project folder.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewRequest {
    pub name: String,
    pub building: String,
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub building_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct NewResult {
    pub project_id: Uuid,
    pub building_id: Uuid,
    pub version: u64,
}

/// `apply`: proposes and commits in one step.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyRequest {
    pub action: ProposedAction,
    #[serde(default)]
    pub timestamp_ms: Option<i64>,
    #[serde(default)]
    pub actor: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CommitResult {
    pub entry: LogEntry,
    pub version: u64,
    pub proposal: Proposal,
}

/// `propose`: returns the proposal; stores nothing.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposeRequest {
    pub action: ProposedAction,
}

/// `commit`: commits a proposal made earlier, if nothing changed since.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommitRequest {
    pub proposal_id: Uuid,
    pub base_version: u64,
    pub action: ProposedAction,
    #[serde(default)]
    pub timestamp_ms: Option<i64>,
    #[serde(default)]
    pub actor: Option<String>,
}

/// `undo`: commits an Undo.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UndoRequest {
    #[serde(default)]
    pub timestamp_ms: Option<i64>,
    #[serde(default)]
    pub actor: Option<String>,
}

/// `query`: the document, one collection, or one entity.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryRequest {
    #[serde(default)]
    pub collection: Option<String>,
    #[serde(default)]
    pub id: Option<Uuid>,
}

/// Subcommands that take no input.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmptyRequest {}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ValidateResult {
    pub violations: Vec<Violation>,
    pub blocks_commit: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RequiredParametersResult {
    pub parameters: Vec<RequiredParameter>,
    pub provisional: bool,
}

/// What the host gives `run`: the clock and a source of fresh ids (Q-49).
pub trait Host {
    fn now_ms(&mut self) -> i64;
    fn new_uuid(&mut self) -> Uuid;
}

/// Runs one invocation on the project files: returns the JSON response and
/// the exit status.
pub fn run(args: &[String], stdin: &str, host: &mut dyn Host) -> (String, i32) {
    respond(dispatch(args, stdin, host))
}

/// Runs one invocation against a project a host already holds open (3.1).
/// The folder argument is not used: the client found the host through it.
/// `new` is refused, since the folder already holds this project.
pub fn run_on(
    project: &mut store::Project,
    args: &[String],
    stdin: &str,
    host: &mut dyn Host,
) -> (String, i32) {
    let result = match args {
        [sub, _] if sub == "new" => Err(CliError::new(
            ErrorCode::ProjectExists,
            "the app has this project open".to_string(),
        )),
        [sub, _] => parse_request(sub, stdin).and_then(|req| execute(project, req, host)),
        _ => Err(bad(USAGE.to_string())),
    };
    respond(result)
}

/// The response text and exit status for a result.
pub(crate) fn respond(result: Result<Value, CliError>) -> (String, i32) {
    let (response, status) = match result {
        Ok(result) => (
            Response {
                ok: true,
                result: Some(result),
                error: None,
            },
            0,
        ),
        Err(e) => {
            let status = e.code.exit_status();
            (
                Response {
                    ok: false,
                    result: None,
                    error: Some(e),
                },
                status,
            )
        }
    };
    let text = to_canonical_json(&response).unwrap_or_else(|e| fallback_response(&e.to_string()));
    (text, status)
}

/// The response when canonical serialization fails, built as a JSON value so
/// the message is escaped (REVIEW-1 M-9). Keys are written in sorted order,
/// as the canonical writer would.
fn fallback_response(message: &str) -> String {
    let value = serde_json::json!({
        "error": {"code": "IO", "details": null, "message": message},
        "ok": false,
        "result": null,
    });
    serde_json::to_string(&value).unwrap_or_else(|_| {
        String::from(
            r#"{"error":{"code":"IO","details":null,"message":"response serialization failed"},"ok":false,"result":null}"#,
        )
    })
}

const USAGE: &str = "usage: archviz <new|apply|propose|commit|undo|query|solve|validate|required-parameters> <project-folder>";

fn bad(message: String) -> CliError {
    CliError::new(ErrorCode::BadRequest, message)
}

fn parse<T: DeserializeOwned>(stdin: &str) -> Result<T, CliError> {
    let text = if stdin.trim().is_empty() { "{}" } else { stdin };
    serde_json::from_str(text).map_err(|e| bad(format!("invalid request: {e}")))
}

fn value<T: Serialize>(v: &T) -> Result<Value, CliError> {
    serde_json::to_value(v).map_err(|e| CliError::new(ErrorCode::Io, e.to_string()))
}

fn propose_error(e: ProposeError) -> CliError {
    match e {
        ProposeError::NothingToUndo | ProposeError::NothingToRedo => {
            CliError::new(ErrorCode::NothingToUndo, e.to_string())
        }
        other => CliError::new(ErrorCode::CommandFailed, other.to_string()),
    }
}

fn commit_error(e: CommitError) -> CliError {
    match e {
        CommitError::Stale { .. } => CliError::new(ErrorCode::Stale, e.to_string()),
        CommitError::Altered => CliError::new(ErrorCode::Altered, e.to_string()),
        CommitError::Blocked => CliError::new(ErrorCode::Blocked, e.to_string()),
        CommitError::Propose(p) => propose_error(p),
        CommitError::Log(l) => CliError::new(ErrorCode::CommandFailed, l.to_string()),
    }
}

/// Commits `proposal` to an open project and appends it to the log.
fn commit_to(
    project: &mut store::Project,
    proposal: Proposal,
    timestamp_ms: Option<i64>,
    actor: Option<String>,
    host: &mut dyn Host,
) -> Result<Value, CliError> {
    if proposal.blocked() {
        let mut e = CliError::new(
            ErrorCode::Blocked,
            "violations, conflicts or ambiguities block this commit".to_string(),
        );
        e.details = Some(value(&proposal)?);
        return Err(e);
    }
    let timestamp_ms = timestamp_ms.unwrap_or_else(|| host.now_ms());
    let actor = actor.unwrap_or_else(|| "cli".to_string());
    let entry =
        commit(&mut project.history, &proposal, timestamp_ms, &actor).map_err(commit_error)?;
    project.append(&entry)?;
    value(&CommitResult {
        entry,
        version: project.history.version(),
        proposal,
    })
}

/// Creates a project folder (`new`).
pub fn new_project(
    dir: &Path,
    req: NewRequest,
    host: &mut dyn Host,
) -> Result<NewResult, CliError> {
    let project_id = req.project_id.unwrap_or_else(|| host.new_uuid());
    let building_id = req.building_id.unwrap_or_else(|| host.new_uuid());
    let genesis = Document::new(
        ProjectInfo {
            id: project_id,
            name: req.name,
            default_unit: ProjectUnit::Inches,
        },
        Building {
            id: building_id,
            name: req.building,
        },
    );
    store::create(dir, &genesis)?;
    Ok(NewResult {
        project_id,
        building_id,
        version: 0,
    })
}

/// A parsed request for a subcommand that works on an open project.
enum Request {
    Apply(ApplyRequest),
    Propose(ProposeRequest),
    Commit(CommitRequest),
    Undo(UndoRequest),
    Query(QueryRequest),
    Solve,
    Validate,
    RequiredParameters,
}

/// Parses the request of a subcommand other than `new`, before the project
/// is opened.
fn parse_request(sub: &str, stdin: &str) -> Result<Request, CliError> {
    Ok(match sub {
        "apply" => Request::Apply(parse(stdin)?),
        "propose" => Request::Propose(parse(stdin)?),
        "commit" => Request::Commit(parse(stdin)?),
        "undo" => Request::Undo(parse(stdin)?),
        "query" => Request::Query(parse(stdin)?),
        "solve" => {
            let _: EmptyRequest = parse(stdin)?;
            Request::Solve
        }
        "validate" => {
            let _: EmptyRequest = parse(stdin)?;
            Request::Validate
        }
        "required-parameters" => {
            let _: EmptyRequest = parse(stdin)?;
            Request::RequiredParameters
        }
        other => return Err(bad(format!("unknown subcommand {other:?}; {USAGE}"))),
    })
}

fn dispatch(args: &[String], stdin: &str, host: &mut dyn Host) -> Result<Value, CliError> {
    let [sub, dir] = args else {
        return Err(bad(USAGE.to_string()));
    };
    let dir = Path::new(dir);
    if sub == "new" {
        return value(&new_project(dir, parse(stdin)?, host)?);
    }
    let req = parse_request(sub, stdin)?;
    let mut project = store::open(dir)?;
    execute(&mut project, req, host)
}

/// Runs a parsed request on an open project.
fn execute(
    project: &mut store::Project,
    req: Request,
    host: &mut dyn Host,
) -> Result<Value, CliError> {
    match req {
        Request::Apply(req) => {
            let proposal = propose(&project.history, &req.action).map_err(propose_error)?;
            commit_to(project, proposal, req.timestamp_ms, req.actor, host)
        }
        Request::Propose(req) => {
            value(&propose(&project.history, &req.action).map_err(propose_error)?)
        }
        Request::Commit(req) => {
            let head = project.history.version();
            if req.base_version != head {
                let mut e = commit_error(CommitError::Stale {
                    base_version: req.base_version,
                    head_version: head,
                });
                e.details = Some(
                    serde_json::json!({ "base_version": req.base_version, "head_version": head }),
                );
                return Err(e);
            }
            let fresh = propose(&project.history, &req.action).map_err(propose_error)?;
            if fresh.proposal_id != req.proposal_id {
                return Err(commit_error(CommitError::Altered));
            }
            commit_to(project, fresh, req.timestamp_ms, req.actor, host)
        }
        Request::Undo(req) => {
            let proposal =
                propose(&project.history, &ProposedAction::Undo).map_err(propose_error)?;
            commit_to(project, proposal, req.timestamp_ms, req.actor, host)
        }
        Request::Query(req) => {
            let doc = value(project.history.document())?;
            let not_found = |what: String| CliError::new(ErrorCode::NotFound, what);
            match (req.collection, req.id) {
                (None, None) => Ok(doc),
                (None, Some(_)) => Err(bad("query by id needs a collection".to_string())),
                (Some(c), id) => {
                    let part = doc
                        .get(&c)
                        .cloned()
                        .ok_or_else(|| not_found(format!("no collection {c}")))?;
                    match id {
                        None => Ok(part),
                        Some(id) => part
                            .get(id.to_string())
                            .cloned()
                            .ok_or_else(|| not_found(format!("no {c} entity {id}"))),
                    }
                }
            }
        }
        Request::Solve => {
            let solution = LayerA::default()
                .solve(project.history.document())
                .map_err(|e| CliError::new(ErrorCode::CommandFailed, e.to_string()))?;
            value(&solution)
        }
        Request::Validate => {
            let violations = validate(project.history.document(), MIN_JAMB_DISTANCE)
                .map_err(|e| CliError::new(ErrorCode::CommandFailed, e.to_string()))?;
            value(&ValidateResult {
                blocks_commit: blocks_commit(&violations),
                violations,
            })
        }
        Request::RequiredParameters => {
            let doc = project.history.document();
            let parameters = required_parameters(doc)
                .map_err(|e| CliError::new(ErrorCode::CommandFailed, e.to_string()))?;
            value(&RequiredParametersResult {
                provisional: !parameters.is_empty(),
                parameters,
            })
        }
    }
}

fn schema_text<T: JsonSchema>() -> Result<String, String> {
    to_pretty_json(&schemars::schema_for!(T)).map_err(|e| e.to_string())
}

/// Every subcommand's request and response schema, as (file name, JSON):
/// the content of schemas/cli/.
pub fn schemas() -> Result<Vec<(String, String)>, String> {
    let pair = |name: &str, request: String, response: String| {
        vec![
            (format!("{name}.request.schema.json"), request),
            (format!("{name}.response.schema.json"), response),
        ]
    };
    let mut out = Vec::new();
    out.extend(pair(
        "new",
        schema_text::<NewRequest>()?,
        schema_text::<Response<NewResult>>()?,
    ));
    out.extend(pair(
        "apply",
        schema_text::<ApplyRequest>()?,
        schema_text::<Response<CommitResult>>()?,
    ));
    out.extend(pair(
        "propose",
        schema_text::<ProposeRequest>()?,
        schema_text::<Response<Proposal>>()?,
    ));
    out.extend(pair(
        "commit",
        schema_text::<CommitRequest>()?,
        schema_text::<Response<CommitResult>>()?,
    ));
    out.extend(pair(
        "undo",
        schema_text::<UndoRequest>()?,
        schema_text::<Response<CommitResult>>()?,
    ));
    out.extend(pair(
        "query",
        schema_text::<QueryRequest>()?,
        schema_text::<Response<Value>>()?,
    ));
    out.extend(pair(
        "solve",
        schema_text::<EmptyRequest>()?,
        schema_text::<Response<Solution>>()?,
    ));
    out.extend(pair(
        "validate",
        schema_text::<EmptyRequest>()?,
        schema_text::<Response<ValidateResult>>()?,
    ));
    out.extend(pair(
        "required-parameters",
        schema_text::<EmptyRequest>()?,
        schema_text::<Response<RequiredParametersResult>>()?,
    ));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every committed schema file is what the code generates; regenerate with
    /// `cargo xtask golden`.
    #[test]
    fn schema_files_are_current() -> Result<(), String> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/cli");
        let generated = schemas()?;
        assert_eq!(generated.len(), 18, "9 subcommands x request and response");
        for (name, text) in generated {
            let on_disk =
                std::fs::read_to_string(dir.join(&name)).map_err(|e| format!("{name}: {e}"))?;
            assert_eq!(on_disk, text, "{name} is stale");
        }
        Ok(())
    }

    /// REVIEW-1 M-9: the fallback used when canonical serialization fails
    /// escapes its message, so stdout is still valid JSON.
    #[test]
    fn the_fallback_error_is_valid_json() -> Result<(), String> {
        let message = "bad \"quote\", back\\slash, bell \u{7}, nul \u{0}, line\nbreak";
        let text = fallback_response(message);
        let parsed: Value = serde_json::from_str(&text).map_err(|e| format!("{e}: {text}"))?;
        assert_eq!(
            parsed,
            serde_json::json!({
                "error": {"code": "IO", "details": null, "message": message},
                "ok": false,
                "result": null,
            })
        );
        assert_eq!(parsed["error"]["message"].as_str(), Some(message));
        assert!(!text.contains('\n'), "one line: {text}");
        Ok(())
    }

    #[test]
    fn exit_statuses_are_fixed() {
        use ErrorCode::*;
        let all = [
            BadRequest,
            NotAProject,
            ProjectExists,
            Locked,
            CorruptProject,
            CommandFailed,
            Stale,
            Altered,
            Blocked,
            NothingToUndo,
            NotFound,
            Io,
            Unauthorized,
        ];
        let statuses: Vec<i32> = all.iter().map(|c| c.exit_status()).collect();
        assert_eq!(statuses, (2..=14).collect::<Vec<_>>());
    }
}
