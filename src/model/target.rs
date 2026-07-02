use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;

use super::eval;
use super::parameters::{Parameter, Parameters};
use super::value::record;
use super::value::Ident;
use super::value::{RecordPattern, RecordSchemas, Value};
use crate::error::{self, Spanned, WithSpan};
use crate::log::blog;
use crate::model::eval::RecordId;
use crate::model::value::WithDisplayCtxExt;
use database::{Database, DatabaseEntry};
use lasso::Rodeo;
use tracing;

pub mod database;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Target type {0} is already declared")]
    DuplicateTargetType(String),
    #[error("Could not find target {0}")]
    TargetNotFoundByString(String),
    #[error("Multiple targets matching {0}")]
    TargetFoundMultiple(String),
}

#[derive(Debug, Serialize, Deserialize, Hash, Clone)]
pub struct FastTrace {
    last_modified: Option<u64>, // Unix time, if available
    size: Option<u64>,          // In bytes, if available
}

#[derive(
    Debug, PartialOrd, Ord, PartialEq, Eq, Serialize, Deserialize, Hash, Clone,
)]
pub struct Trace {
    hash: u64,
}

impl std::fmt::Display for Trace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[T:{:X}]", self.hash)
    }
}

/// Trace function determines the trace of the target such that when
/// the trace stays the same, the target hasn't changed
///
/// Fast trace function
pub enum TraceFun {
    Builtin {
        // Return of None means resource does not exist
        fast: fn(
            &record::Record,
            &eval::Resolved,
        ) -> Result<Option<FastTrace>, eval::Error>,
        trace: fn(
            &record::Record,
            &eval::Resolved,
        ) -> Result<Option<Trace>, eval::Error>,
    },
}

impl TraceFun {
    fn fast(
        &self,
        resolved: &eval::Resolved,
        request: &record::Record,
    ) -> Result<Option<FastTrace>, eval::Error> {
        match self {
            TraceFun::Builtin { fast, .. } => fast(request, resolved),
        }
    }
    fn trace(
        &self,
        resolved: &eval::Resolved,
        request: &record::Record,
    ) -> Result<Option<Trace>, eval::Error> {
        match self {
            TraceFun::Builtin { trace, .. } => trace(request, resolved),
        }
    }
}

impl Default for TraceFun {
    fn default() -> Self {
        TraceFun::Builtin {
            fast: |_, _| Ok(None),
            trace: |_, _| Ok(None),
        }
    }
}

impl std::fmt::Debug for TraceFun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<TraceFun>")
    }
}

#[derive(Debug, Default)]
pub struct TargetType {
    // Schema that defines how we refer to targets of this type
    pub request_schema: RecordId,
    pub name_is: usize,
    pub trace: TraceFun,
}

#[derive(Debug, Clone)]
pub struct TargetResult {
    pub value: Value,
    pub clean: bool,
}

impl TargetType {
    /// Construct the request record from the name (e.g. from CLI)
    pub fn request_from_name(
        &self,
        resolved: &eval::Resolved,
        name: &str,
    ) -> Option<record::Record> {
        let schema = &resolved[self.request_schema];
        let mut values = Vec::with_capacity(schema.fields.0.len());
        for (ix, (_ident, param)) in schema.fields.0.iter().enumerate() {
            let v = if self.name_is == ix {
                Value::String(name.to_owned())
            } else {
                param.default.clone()?
            };
            values.push(v);
        }
        Some(record::Record {
            schema: self.request_schema,
            values,
        })
    }
}

#[derive(Debug, Default)]
pub struct TargetTypes(Vec<TargetType>);

impl std::ops::Index<TargetTypeId> for TargetTypes {
    type Output = TargetType;

    fn index(&self, index: TargetTypeId) -> &Self::Output {
        &self.0[index.0]
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TargetTypeId(pub usize);

impl TargetTypes {
    pub fn register(
        &mut self,
        interner: &mut Rodeo,
        records: &mut RecordSchemas,
        request_name: &str,
        request_fields: Parameters,
        name_is: usize,
        trace: TraceFun,
    ) -> Result<TargetTypeId, Error> {
        let req_id = records
            .register(interner, request_name, request_fields)
            .map_err(|record::Error::DuplicateRecordSchema(s)| {
            Error::DuplicateTargetType(s)
        })?;

        self.0.push(TargetType {
            request_schema: req_id,
            name_is,
            trace,
        });
        return Ok(TargetTypeId(self.0.len() - 1));
    }

    pub fn find_by_request(
        &self,
        schema: RecordId,
    ) -> Option<(TargetTypeId, &TargetType)> {
        self.0.iter().enumerate().find_map(|(ix, tt)| {
            if tt.request_schema == schema {
                Some((TargetTypeId(ix), tt))
            } else {
                None
            }
        })
    }
}

const FILE_TRACE_FUN: TraceFun = TraceFun::Builtin {
    fast: |rec, resolved| {
        let path = rec.values[0].expect_string(resolved)?;
        let meta = match std::fs::metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None)
            }
            Err(e) => Err(e)?,
        };

        Ok(Some(FastTrace {
            size: Some(meta.len()),
            last_modified: Some(
                meta.modified()?
                    .duration_since(std::time::UNIX_EPOCH)
                    // File creation time before UNIX EPOCH: return 0
                    .unwrap_or_default()
                    .as_secs(),
            ),
        }))
    },
    trace: |rec, resolved| {
        let path = rec.values[0].expect_string(resolved)?;
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None)
            }
            Err(e) => Err(e)?,
        };
        let hash = rapidhash::v3::rapidhash_v3_file_inline::<
            &std::fs::File,
            false,
        >(&file, &rapidhash::v3::DEFAULT_RAPID_SECRETS)?;
        tracing::debug!("trace fun hashed file {}: {:X}", path, hash);
        Ok(Some(Trace { hash }))
    },
};

pub fn register_target_type_file(
    interner: &mut Rodeo,
    records: &mut RecordSchemas,
    target_types: &mut TargetTypes,
) -> Result<TargetTypeId, Error> {
    let request_parameters = Parameters::builder(interner)
        .add_static("path", None)
        .build();

    target_types.register(
        interner,
        records,
        "file",
        request_parameters,
        0,
        FILE_TRACE_FUN,
    )
}

pub fn register_target_type_task(
    interner: &mut Rodeo,
    records: &mut RecordSchemas,
    target_types: &mut TargetTypes,
) -> Result<TargetTypeId, Error> {
    let name = interner.get_or_intern("name");
    let parameters = Parameters(vec![(
        Ident(name),
        Parameter {
            default: None,
            span: None,
        },
    )]);
    target_types.register(
        interner,
        records,
        "task",
        parameters,
        0,
        // Tasks don't have traces
        TraceFun::Builtin {
            fast: |_, _| Ok(None),
            trace: |_, _| Ok(None),
        },
    )
}

#[derive(Debug, Default)]
pub struct Target {
    pub kind: TargetTypeId,
    // Display name, mostly for CLI
    pub name: Option<String>,
    // Determine if requested resource can be built by this target
    pub pattern: RecordPattern,
    // Resolved variable slots for target parameters
    pub slots: eval::Slots,
    // How to match request arguments to target parameter slots. If the
    // ith entry is k, then put the kth value of the request record
    // into slot i
    pub matchers: Vec<usize>,
    pub block: eval::Block,
}

// List of independent dependencies (each group can be run in parallel)
#[derive(Debug)]
pub struct Dependencies(Vec<Vec<record::Record>>);

impl Dependencies {
    // Invariants
    // * Contains at least 1 (possibly empty vec)
    // * Only the last vec is allowed to be empty
    pub fn new() -> Self {
        Dependencies(vec![vec![]])
    }
    pub fn add(&mut self, dep: record::Record) {
        self.0.last_mut().unwrap().push(dep);
        tracing::trace!("Added outstanding dependency, deps now: {:?}", self.0);
    }
    // We encountered a barrier, start a new group
    pub fn new_layer(&mut self) {
        if self.0.last().unwrap().len() > 0 {
            self.0.push(vec![]);
        }
    }

    pub fn len(&self) -> usize {
        self.0.iter().map(|v| v.len()).sum()
    }
}

impl Target {
    // (Only) Run the code
    pub fn execute(
        &self,
        resolved: &eval::Resolved,
        scheduler: &mut Scheduler,
        request: &record::Record,
    ) -> Result<(Value, Dependencies), error::Spanned<eval::Error>> {
        let mut variables = Default::default();
        let mut dependencies: Dependencies = Dependencies::new();
        let mut outstanding = Vec::new();
        let res = {
            let mut ctx = eval::EvalContext {
                variables: &mut variables,
                resolved,
                scheduler,
                dependencies: &mut dependencies,
                outstanding: &mut outstanding,
            };
            // Fill in variables with record elements
            let mut slots = self.slots.clone();
            for (to, from) in self.matchers.iter().enumerate() {
                slots.0[to].value = eval::SlotState::Value(
                    request.values[*from].clone().into(),
                );
            }
            let res = {
                let mut scope = ctx.enter(slots);
                // tracing::trace!("Starting build");
                self.block
                    .run(&mut scope)?
                    .force(&mut scope)
                    .without_span()?
            };
            tracing::trace!("Outstanding deps: {:?}", &ctx.outstanding);
            ctx.flush_dependencies().without_span()?;
            res
        };

        Ok((res, dependencies))
    }

    // Try to match the target to a single string (e.g. cli)
    pub fn matches_string(&self, s: &str) -> bool {
        match &self.name {
            None => false,
            Some(n) => n == s,
        }
    }

    /// Given a target database entry, check if all registered
    /// dependencies are clean
    fn all_deps_clean(
        scheduler: &mut Scheduler,
        resolved: &eval::Resolved,
        tgt: &str,
        c: &DatabaseEntry,
    ) -> Result<bool, error::Spanned<eval::Error>> {
        // We can process dependencies within each layer in parallel,
        // because there are no data dependencies between them. If the
        // layer is clean, we continue to the next layer - the output
        // would have been the same, hence we assume the target would
        // take the same path
        for layer in &c.dependencies {
            let mut outstanding = Vec::new();
            // First schedule all dependencies
            for request in layer {
                outstanding.push(Request {
                    target: scheduler.add(resolved, &request)?,
                    request: request.clone(),
                });
            }
            // Now demand them (splitting schedule/demand
            // anticipates parallel scheduler and works
            // just fine for the sequential one)
            for request in outstanding {
                let dep = request.request.with_ctx(resolved).to_string();
                let res = scheduler.demand(resolved, request).without_span()?;
                if !res.clean {
                    tracing::info!("Dependency {dep} not clean, running {tgt}",);
                    // Found a dirty dependency. We stop
                    // querying dependencies and continue
                    // building the target. Dependencies
                    // will be re-requested - but that's
                    // fine, that will just use the stored
                    // values
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    pub fn run(
        &self,
        resolved: &eval::Resolved,
        scheduler: &mut Scheduler,
        request: &record::Record,
    ) -> Result<TargetResult, error::Spanned<eval::Error>> {
        let target_type = &resolved[self.kind];
        let tgt = request.with_ctx(resolved).to_string();

        blog!(info, target = tgt, "Running target");
        tracing::error_span!("target", target = tgt);

        let before_trace =
            target_type.trace.trace(resolved, &request).without_span()?;

        let cached = scheduler.database.get(request).cloned();

        match cached {
            Some(c) => {
                let trace_clean = match (&before_trace, &c.trace) {
                    // No registered trace before and after is not
                    // considered "clean"
                    (Some(b), Some(c)) => b == c,
                    _ => false,
                };

                if trace_clean
                    && Target::all_deps_clean(scheduler, resolved, &tgt, &c)?
                {
                    blog!(
                        info,
                        target = tgt,
                        "Target is clean and doesn't need to be run"
                    );

                    return Ok(TargetResult {
                        value: c.result,
                        clean: true,
                    });
                };
            }
            // Nothing cached, continue
            _ => {}
        };

        let (value, dependencies) =
            match self.execute(resolved, scheduler, request) {
                Ok((v, dependencies)) => (v, dependencies),
                Err(e) => {
                    // TODO: Make use of span
                    blog!(
                        error,
                        target = request.with_ctx(resolved),
                        "Target failed: {}",
                        e.inner
                    );
                    return Err(e);
                }
            };

        match value {
            Value::Unit => {
                blog!(info, target = tgt, "Target finished");
            }
            _ => {
                blog!(
                    info,
                    target = tgt,
                    "Target finished, return value: {}",
                    value.with_ctx(resolved)
                );
            }
        }
        if dependencies.len() > 0 {
            tracing::debug!(
                "Target {} registered dependencies: {:?}",
                tgt,
                dependencies
            );
        }

        // TODO: Make use of fast trace
        // let fast_trace = target_type.trace.fast(resolved, &request).without_span()?;
        let after_trace =
            target_type.trace.trace(resolved, &request).without_span()?;

        scheduler
            .database
            .add(
                DatabaseEntry {
                    target: request.clone(),
                    fast_trace: None, // TODO
                    trace: after_trace.clone(),
                    result: value.clone(),
                    dependencies: dependencies.0,
                },
                resolved,
            )
            .unwrap();

        after_trace
            .as_ref()
            .map(|trace| tracing::info!("Target finished, trace: {}", trace));

        let clean = match (&before_trace, &after_trace) {
            (Some(b), Some(a)) if b == a => {
                blog!(info, target = tgt, "Target is unchanged");
                true
            }
            _ => false,
        };

        Ok(TargetResult { value, clean })
    }
}

#[derive(Default, Debug)]
pub struct Targets(Vec<Target>);

#[derive(Default, Hash, Debug, PartialEq, Eq, Clone, Copy)]
pub struct TargetId(usize);

impl<'a> std::ops::Index<TargetId> for Targets {
    type Output = Target;

    fn index(&self, index: TargetId) -> &Self::Output {
        &self.0[index.0]
    }
}

impl Targets {
    pub fn add(&mut self, t: Target) -> Result<(), Error> {
        self.0.push(t);
        // TODO: Check for name collisions so find_by_string never fails
        Ok(())
    }
    pub fn find_by_string(&self, s: &str) -> Result<&Target, Error> {
        match self
            .0
            .iter()
            .filter(|t| t.matches_string(s))
            .collect::<Vec<_>>()[..]
        {
            [] => Err(Error::TargetNotFoundByString(String::from(s))),
            [t] => Ok(t),
            _ => Err(Error::TargetFoundMultiple(String::from(s))),
        }
    }
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().filter_map(|t| t.name.as_deref())
    }

    pub fn match_(
        &self,
        resolved: &eval::Resolved,
        request: &record::Record,
    ) -> Result<Option<TargetId>, eval::Error> {
        // Filter targets that match the given request
        let candidates = self
            .0
            .iter()
            .enumerate()
            .filter(|(ix, target)| target.pattern.matches(request))
            .collect::<Vec<_>>();

        // Find the most specific target
        let (mut ix, mut cand) = match candidates.first() {
            Some((ix, c)) => (*ix, *c),
            None => return Ok(None),
        };
        for (i, p) in &candidates[1..] {
            if p.pattern.patterns.cmp_specificity(&cand.pattern.patterns)
                == Some(Ordering::Greater)
            {
                cand = *p;
                ix = *i;
            }
        }
        // Check that this maximum is the (unique) greatest element
        if candidates.iter().all(|(i, p)| {
            *i == ix
                || cand.pattern.patterns.cmp_specificity(&p.pattern.patterns)
                    == Some(Ordering::Greater)
        }) {
            Ok(Some(TargetId(ix)))
        } else {
            Err(eval::Error::AmbiguousTarget(
                request.with_ctx(resolved).to_string(),
            ))
        }
    }
}

#[derive(Debug)]
enum TargetState {
    Requested,
    Running,
    Done(TargetResult),
    Error,
}

#[derive(Debug)]
pub struct Request {
    pub target: TargetId,
    pub request: record::Record,
}

#[derive(Debug, Hash, PartialEq, Eq)]
pub struct Dependency {
    target: TargetId,
    request: record::Record,
}

// single-threaded scheduler
pub struct Scheduler {
    requested: HashMap<record::Record, TargetState>,
    database: Database,
}

impl Scheduler {
    pub fn new(database: Database) -> Self {
        Self {
            requested: Default::default(),
            database,
        }
    }

    pub fn add(
        &mut self,
        resolved: &eval::Resolved,
        request: &record::Record,
        // Resolved target
    ) -> Result<TargetId, Spanned<eval::Error>> {
        let target =
            match resolved.targets.match_(resolved, &request).without_span()? {
                Some(target) => target,
                None => {
                    todo!()
                    // No target matches requested resource
                }
            };
        let old = self.requested.get(&request);
        match old {
            None => {
                tracing::trace!("New target {:?} requested", request);
                self.requested
                    .insert(request.clone(), TargetState::Requested);
                Ok(())
            }
            // Target was already built
            Some(TargetState::Done(_)) => {
                tracing::trace!("Target {:?} was already done", request);
                Ok(())
            }
            Some(TargetState::Running) => {
                tracing::error!("Target request {:?} is circular", request);
                Err(eval::Error::CircularDependency(
                    request.with_ctx(resolved).to_string(),
                ))
                .without_span()
            }
            Some(TargetState::Error) => {
                tracing::trace!(
                    "Target {:?} had already finished with an error",
                    request
                );
                Err(eval::Error::DependencyError(
                    request.with_ctx(resolved).to_string(),
                ))
                .without_span()
            }
            // Multiple need() on the same target - that's OK
            Some(TargetState::Requested) => {
                tracing::trace!(
                    "Target {:?} had already been requested but was not yet finished",
                    request
                );
                Ok(())
            }
        }?;
        Ok(target)
    }

    fn find_result(
        &self,
        resolved: &eval::Resolved,
        request: &record::Record,
    ) -> Result<Option<TargetResult>, eval::Error> {
        match self
            .requested
            .get(request)
            .expect("find_result on resource that has not be scheduled")
        {
            TargetState::Done(v) => Ok(Some(v.clone())),
            TargetState::Error => Err(eval::Error::DependencyError(
                request.with_ctx(resolved).to_string(),
            )),
            TargetState::Requested | TargetState::Running => Ok(None),
        }
    }

    fn run_one(
        &mut self,
        resolved: &eval::Resolved,
        request: Request,
    ) -> Result<TargetResult, eval::Error> {
        match resolved[request.target].run(resolved, self, &request.request) {
            Ok(res) => {
                *self
                    .requested
                    .get_mut(&request.request)
                    // add() has added the item to requested as well
                    .expect("flush") = TargetState::Done(res.clone());
                Ok(res)
            }
            Err(e) => {
                *self
                    .requested
                    .get_mut(&request.request)
                    // Entry must exist by construction
                    .expect("flush") = TargetState::Error;
                let tgt = request.request.with_ctx(resolved).to_string();
                blog!(error, target = tgt, "Target failed with: {e}");
                Err(eval::Error::DependencyError(tgt))
            }
        }
    }

    // Demand a single target
    pub fn demand(
        &mut self,
        resolved: &eval::Resolved,
        request: Request,
    ) -> Result<TargetResult, eval::Error> {
        tracing::trace!("Resource {:?} demanded", request);
        // Check if value is already present
        match self.find_result(resolved, &request.request)? {
            // Target already finished, return result
            Some(v) => Ok(v),
            // Target still outstanding, run
            None => Ok(self.run_one(resolved, request)?),
        }
    }
}
