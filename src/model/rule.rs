use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;
use yansi::Paint as _;

use super::eval;
use super::parameters::{Parameter, Parameters};
use super::value::record;
use super::value::Ident;
use super::value::{RecordPattern, RecordSchemas, Value};
use crate::error::{self, Spanned, WithSpan};
use crate::log::blog;
use crate::model::eval::RecordId;
use crate::model::value::{DisplayWith, WithDisplayCtxExt};
use database::{Database, DatabaseEntry};
use lasso::Rodeo;
use tracing;

pub mod database;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Target type {0} is already declared")]
    DuplicateTargetType(String),
    #[error("Could not find target {0}")]
    TargetNotFound(String),
    #[error("Multiple rules matching target {0}")]
    AmbiguousTarget(String),
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
#[derive(Clone, Copy)]
pub enum TraceFun {
    Builtin {
        // Return of None means resource does not exist
        fast: fn(
            &Target,
            eval::ResolvedRef<'_>,
            &Path,
        ) -> Result<Option<FastTrace>, eval::Error>,
        trace: fn(
            &Target,
            eval::ResolvedRef<'_>,
            &Path,
        ) -> Result<Option<Trace>, eval::Error>,
    },
}

// Function for displaying a target (e.g. special syntax for files or tasks)
#[derive(Clone, Copy)]
pub enum DisplFun {
    Builtin {
        fmt: fn(
            &Target,
            eval::ResolvedRef<'_>,
            &mut std::fmt::Formatter,
        ) -> std::fmt::Result,
    },
}

impl DisplFun {
    pub fn fmt(
        &self,
        // TODO
        target: &Target,
        resolved: eval::ResolvedRef<'_>,
        f: &mut std::fmt::Formatter,
    ) -> std::fmt::Result {
        match self {
            Self::Builtin { fmt } => fmt(target, resolved, f),
        }
    }
}

impl Default for DisplFun {
    fn default() -> Self {
        Self::Builtin {
            fmt: |_, _, _f| Ok(()),
        }
    }
}

impl std::fmt::Debug for DisplFun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<DisplFun>")
    }
}

impl TraceFun {
    fn fast(
        &self,
        target: &Target,
        resolved: eval::ResolvedRef<'_>,
        base_dir: &Path,
    ) -> Result<Option<FastTrace>, eval::Error> {
        match self {
            TraceFun::Builtin { fast, .. } => fast(target, resolved, base_dir),
        }
    }
    fn trace(
        &self,
        target: &Target,
        resolved: eval::ResolvedRef<'_>,
        base_dir: &Path,
    ) -> Result<Option<Trace>, eval::Error> {
        match self {
            TraceFun::Builtin { trace, .. } => {
                trace(target, resolved, base_dir)
            }
        }
    }
}

impl Default for TraceFun {
    fn default() -> Self {
        TraceFun::Builtin {
            fast: |_, _, _| Ok(None),
            trace: |_, _, _| Ok(None),
        }
    }
}

impl std::fmt::Debug for TraceFun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<TraceFun>")
    }
}

#[derive(Debug, Default, PartialOrd, PartialEq, Ord, Eq, Clone, Copy, Hash)]
pub struct ModuleId(usize);

#[derive(Debug, Default, Clone)]
pub struct Module {
    pub name: Vec<Ident>,
    pub workdir: PathBuf,
}

impl DisplayWith for Module {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        resolved: eval::ResolvedRef<'_>,
    ) -> std::fmt::Result {
        for i in self.name.iter() {
            write!(f, "{}:", &resolved[*i])?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct Modules(Vec<Module>);

impl Modules {
    pub fn new() -> Self {
        // Instantiate with root module as default
        Modules(vec![Module {
            name: Vec::with_capacity(0),
            workdir: PathBuf::from("."),
        }])
    }
    pub fn find(&self, name: &[Ident]) -> Option<ModuleId> {
        self.0.iter().position(|md| md.name == name).map(ModuleId)
    }
}

impl std::ops::Index<ModuleId> for Modules {
    type Output = Module;

    fn index(&self, index: ModuleId) -> &Self::Output {
        &self.0[index.0 as usize]
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TargetType {
    pub id: TargetTypeId,
    // Schema that defines how we refer to targets of this type
    pub schema: RecordId,
    pub name_is: usize,
    pub trace: TraceFun,
    pub display: DisplFun,
}

#[derive(Debug, Clone)]
pub struct TargetResult {
    pub value: Value,
    pub clean: bool,
    // True if the rule ran, false if it was skipped because its
    // dependencies were clean
    // TODO
    pub ran: bool,
}

impl TargetType {
    /// Construct the target record from the name (e.g. from CLI)
    pub fn target_from_name(
        &self,
        resolved: eval::ResolvedRef<'_>,
        name: &str,
    ) -> Option<Target> {
        let schema = &resolved[self.schema];
        let mut values = Vec::with_capacity(schema.fields.0.len());
        for (ix, (_ident, param)) in schema.fields.0.iter().enumerate() {
            let v = if self.name_is == ix {
                Value::String(name.to_owned())
            } else {
                param.default.clone()?
            };
            values.push(v);
        }
        Some(Target {
            record: record::Record {
                schema: self.schema,
                values,
            },
            module: ModuleId(0),
        })
    }
}

#[derive(Debug, Default)]
pub struct TargetTypes(Vec<TargetType>);

impl TargetTypes {
    pub fn find_by_record(&self, r: &record::Record) -> Option<&TargetType> {
        self.0.iter().find(|tt| tt.schema == r.schema)
    }
}

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
        display: DisplFun,
    ) -> Result<TargetType, Error> {
        let req_id = records
            .register(interner, request_name, request_fields)
            .map_err(|record::Error::DuplicateRecordSchema(s)| {
            Error::DuplicateTargetType(s)
        })?;
        let target_type = TargetType {
            id: TargetTypeId(self.0.len()),
            schema: req_id,
            name_is,
            trace,
            display,
        };
        self.0.push(target_type);
        return Ok(target_type);
    }

    pub fn find_by_schema(
        &self,
        schema: RecordId,
    ) -> Option<(TargetTypeId, &TargetType)> {
        self.0.iter().enumerate().find_map(|(ix, tt)| {
            if tt.schema == schema {
                Some((TargetTypeId(ix), tt))
            } else {
                None
            }
        })
    }
}

const FILE_TRACE_FUN: TraceFun = TraceFun::Builtin {
    fast: |target, resolved, base_dir| {
        let module = &resolved[target.module];
        let rec = &target.record;
        let path = base_dir
            .join(module.workdir.join(rec.values[0].expect_string(resolved)?));

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
    trace: |target, resolved, base_dir| {
        let module = &resolved[target.module];
        let rec = &target.record;
        let path = base_dir
            .join(module.workdir.join(rec.values[0].expect_string(resolved)?));
        tracing::debug!(
            "FILE_TRACE_FUN: tracing {}",
            path.to_str().unwrap_or("can't display path")
        );
        let file = match std::fs::File::open(&path) {
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
        tracing::debug!(
            "trace fun hashed file {}: {:X}",
            path.to_string_lossy(),
            hash
        );
        Ok(Some(Trace { hash }))
    },
};

const FILE_DISPL_FUN: DisplFun = DisplFun::Builtin {
    fmt: |target, resolved, f| {
        let module = &resolved[target.module];
        if module.name.len() > 0 {
            write!(f, "{}:", module.with_ctx(resolved))?;
        }
        write!(
            f,
            "./{}",
            target.record.values[0].expect_string(resolved).unwrap()
        )
    },
};

pub fn register_target_type_file(
    interner: &mut Rodeo,
    records: &mut RecordSchemas,
    target_types: &mut TargetTypes,
) -> Result<TargetType, Error> {
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
        FILE_DISPL_FUN,
    )
}

const TASK_DISPL_FUN: DisplFun = DisplFun::Builtin {
    fmt: |target, resolved, f| {
        let module = &resolved[target.module];
        write!(
            f,
            "{}:{}",
            module.with_ctx(resolved),
            target.record.values[0].expect_string(resolved).unwrap()
        )
    },
};

pub fn register_target_type_task(
    interner: &mut Rodeo,
    records: &mut RecordSchemas,
    target_types: &mut TargetTypes,
) -> Result<TargetType, Error> {
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
            fast: |_, _, _| Ok(None),
            trace: |_, _, _| Ok(None),
        },
        TASK_DISPL_FUN,
    )
}

#[derive(Debug, Default)]
pub struct Rule {
    pub kind: TargetTypeId,
    // Target name if the rule builds a single named target, mostly for CLI
    pub target: Option<String>,
    // Determine if requested resource can be built by this rule
    pub pattern: RecordPattern,
    // Resolved variable slots for rule parameters
    pub slots: eval::Slots,
    // How to match target arguments to rule parameter slots. If the
    // ith entry is k, then put the kth value of the target record
    // into slot i
    pub matchers: Vec<usize>,
    pub block: eval::Block,
    pub module: ModuleId,
}

// List of independent dependencies (each group can be run in parallel)
#[derive(Debug)]
pub struct Dependencies(Vec<Vec<Target>>);

impl Dependencies {
    // Invariants
    // * Contains at least 1 (possibly empty vec)
    // * Only the last vec is allowed to be empty
    pub fn new() -> Self {
        Dependencies(vec![vec![]])
    }
    pub fn add(&mut self, dep: Target) {
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

impl Rule {
    // (Only) Run the code
    pub fn execute(
        &self,
        resolved: eval::ResolvedRef<'_>,
        scheduler: &mut Scheduler,
        target: &Target,
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
                module: self.module,
            };
            // Fill in variables with record elements
            let mut slots = self.slots.clone();
            for (to, from) in self.matchers.iter().enumerate() {
                slots.0[to].value = eval::SlotState::Value(
                    target.record.values[*from].clone().into(),
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

    // Check if the rule builds the named target (e.g. from cli)
    pub fn matches_target(&self, s: &str) -> bool {
        match &self.target {
            None => false,
            Some(n) => n == s,
        }
    }

    /// Given a database entry, check if all registered
    /// dependencies are clean
    fn all_deps_clean(
        scheduler: &mut Scheduler,
        resolved: eval::ResolvedRef<'_>,
        tgt: &str,
        c: &DatabaseEntry,
    ) -> Result<bool, error::Spanned<eval::Error>> {
        // We can process dependencies within each layer in parallel,
        // because there are no data dependencies between them. If the
        // layer is clean, we continue to the next layer - the output
        // would have been the same, hence we assume the rule would
        // take the same path
        for layer in &c.dependencies {
            let mut outstanding = Vec::new();
            // First schedule all dependencies
            for target in layer {
                scheduler.add(resolved, target)?;
                outstanding.push(Target {
                    module: target.module,
                    record: target.record.clone(),
                });
            }
            // Now demand them (splitting schedule/demand
            // anticipates parallel scheduler and works
            // just fine for the sequential one)
            for target in outstanding {
                let dep = target.record.with_ctx(resolved).to_string();
                let res = scheduler.demand(resolved, target).without_span()?;
                if !res.clean {
                    tracing::info!("Dependency {dep} not clean, running {tgt}",);
                    // Found a dirty dependency. We stop
                    // querying dependencies and continue
                    // building the rule. Dependencies
                    // will be re-requested - but that's
                    // fine, that will just use the stored
                    // values
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn run(
        &self,
        resolved: eval::ResolvedRef<'_>,
        scheduler: &mut Scheduler,
        target: &Target,
    ) -> Result<TargetResult, error::Spanned<eval::Error>> {
        let target_type = &resolved[self.kind];
        let tgt = target.with_ctx(resolved).to_string();

        tracing::error_span!("target", target = tgt);

        let before_trace = target_type
            .trace
            .trace(target, resolved, &scheduler.base_dir)
            .without_span()?;

        let cached = scheduler.database.get(target).cloned();

        match cached {
            Some(c) => {
                let trace_clean = match (&before_trace, &c.trace) {
                    // No registered trace before and after is not
                    // considered "clean"
                    (Some(b), Some(c)) => b == c,
                    _ => false,
                };

                if trace_clean
                    && Rule::all_deps_clean(scheduler, resolved, &tgt, &c)?
                {
                    blog!(
                        info,
                        target = tgt,
                        "Target is up to date"
                    );

                    return Ok(TargetResult {
                        value: c.result,
                        clean: true,
                        ran: false,
                    });
                };
            }
            // Nothing cached, continue
            _ => {}
        };
        blog!(info, target = tgt, "Starting");
        let (value, dependencies) =
            match self.execute(resolved, scheduler, target) {
                Ok((v, dependencies)) => (v, dependencies),
                Err(e) => {
                    // TODO: Make use of span
                    blog!(
                        error,
                        target = target.with_ctx(resolved),
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
        // let fast_trace = target_type.trace.fast(resolved, &target).without_span()?;
        let after_trace = target_type
            .trace
            .trace(&target, resolved, &scheduler.base_dir)
            .without_span()?;

        scheduler
            .database
            .add(
                DatabaseEntry {
                    target: target.clone(),
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

        Ok(TargetResult {
            value,
            clean,
            ran: true,
        })
    }
}

#[derive(Default, Debug)]
pub struct Rules(Vec<Rule>);

#[derive(Default, Hash, Debug, PartialEq, Eq, Clone, Copy)]
pub struct RuleId(usize);

impl std::ops::Index<RuleId> for Rules {
    type Output = Rule;

    fn index(&self, index: RuleId) -> &Self::Output {
        &self.0[index.0]
    }
}

impl Rules {
    pub fn add(&mut self, t: Rule) -> Result<(), Error> {
        self.0.push(t);
        // TODO: Check for target collisions so find_by_target never fails
        Ok(())
    }
    pub fn find_by_target(&self, s: &str) -> Result<&Rule, Error> {
        match self
            .0
            .iter()
            .filter(|t| t.matches_target(s))
            .collect::<Vec<_>>()[..]
        {
            [] => Err(Error::TargetNotFound(String::from(s))),
            [t] => Ok(t),
            _ => Err(Error::AmbiguousTarget(String::from(s))),
        }
    }
    pub fn targets(&self) -> impl Iterator<Item = &str> {
        self.0.iter().filter_map(|t| t.target.as_deref())
    }

    pub fn match_(
        &self,
        resolved: eval::ResolvedRef<'_>,
        target: &Target,
    ) -> Result<Option<RuleId>, eval::Error> {
        // Filter rules that match the given target
        let candidates = self
            .0
            .iter()
            .enumerate()
            .filter(|(_ix, rule)| {
                rule.module == target.module
                    && rule.pattern.matches(&target.record)
            })
            .collect::<Vec<_>>();

        // Find the most specific rule
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
            Ok(Some(RuleId(ix)))
        } else {
            Err(eval::Error::AmbiguousTarget(
                target.with_ctx(resolved).to_string(),
            ))
        }
    }
}

#[derive(Debug)]
enum TargetState {
    Requested,
    Running,
    Done(TargetResult),
    Error(Spanned<eval::Error>),
}

#[derive(Debug)]
struct RequestedTarget {
    kind: TargetTypeId,
    state: TargetState,
    seq: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Target {
    pub module: ModuleId,
    // Invariant: The record matches the target type's schema
    pub record: record::Record,
}

impl DisplayWith for Target {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        resolved: eval::ResolvedRef<'_>,
    ) -> std::fmt::Result {
        if let Some(target_type) =
            resolved.target_types.find_by_record(&self.record)
        {
            // Target resolves to a target type. Use the target type's display
            // function to format the target
            target_type.display.fmt(self, resolved, f)
        } else {
            // Target record does not resolve to a target type. Use a
            // generic formatting method
            write!(
                f,
                "{}:{}",
                resolved[self.module].with_ctx(resolved),
                self.record.with_ctx(resolved)
            )
        }
    }
}

#[derive(Debug, Hash, PartialEq, Eq)]
pub struct Dependency {
    rule: RuleId,
    record: record::Record,
}

// single-threaded scheduler
pub struct Scheduler {
    next: AtomicU32,
    requested: HashMap<Target, RequestedTarget>,
    database: Database,
    // The base dir the entire build runs in (not to be confused with the directory a *module* runs in)
    pub base_dir: PathBuf,
}

impl Scheduler {
    pub fn new(base_dir: PathBuf, database: Database) -> Self {
        Self {
            next: AtomicU32::new(0),
            requested: Default::default(),
            database,
            base_dir,
        }
    }

    fn next_seq(&self) -> u32 {
        self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    pub fn add(
        &mut self,
        resolved: eval::ResolvedRef<'_>,
        target: &Target,
        // Resolved rule
    ) -> Result<RuleId, Spanned<eval::Error>> {
        let rule =
            match resolved.rules.match_(resolved, &target).without_span()? {
                Some(rule) => rule,
                None => {
                    todo!()
                    // No rule matches requested resource
                }
            };
        let old = self.requested.get(target);
        match old {
            None => {
                tracing::trace!("New target {:?} requested", target);
                self.requested.insert(
                    target.clone(),
                    RequestedTarget {
                        kind: resolved[rule].kind,
                        seq: self.next_seq(),
                        state: TargetState::Requested,
                    },
                );
                Ok(())
            }
            // Target was already built
            Some(s) => match s.state {
                TargetState::Done(_) => {
                    tracing::trace!("Target {:?} was already done", target);
                    Ok(())
                }
                TargetState::Running => {
                    // This won't work with the parallel scheduler
                    // since a diamond dependency might schedule the
                    // target while another path legally requests it
                    tracing::error!("Target {:?} is circular", target);
                    Err(eval::Error::CircularDependency(
                        target.with_ctx(resolved).to_string(),
                    ))
                    .without_span()
                }
                TargetState::Error(_) => {
                    tracing::trace!(
                        "Target {:?} had already finished with an error",
                        target
                    );
                    Err(eval::Error::DependencyError(
                        target.with_ctx(resolved).to_string(),
                    ))
                    .without_span()
                }
                // Multiple need() on the same target - that's OK
                TargetState::Requested => {
                    tracing::trace!(
                    "Target {:?} had already been requested but was not yet finished",
                    target
                );
                    Ok(())
                }
            },
        }?;
        Ok(rule)
    }

    fn find_result(
        &self,
        resolved: eval::ResolvedRef<'_>,
        target: &Target,
    ) -> Result<Option<TargetResult>, eval::Error> {
        match &self
            .requested
            .get(target)
            .expect("find_result on resource that has not been scheduled")
            .state
        {
            TargetState::Done(v) => Ok(Some(v.clone())),
            TargetState::Error(_) => Err(eval::Error::DependencyError(
                target.with_ctx(resolved).to_string(),
            )),
            TargetState::Requested | TargetState::Running => Ok(None),
        }
    }

    fn run_one(
        &mut self,
        resolved: eval::ResolvedRef<'_>,
        target: Target,
    ) -> Result<TargetResult, eval::Error> {
        let rule =
            resolved.rules.match_(resolved, &target)?.ok_or_else(|| {
                eval::Error::RuleNotFound(
                    target.with_ctx(resolved).to_string(),
                )
            })?;
        (*self.requested.get_mut(&target).expect("run_one")).state =
            TargetState::Running;
        match resolved[rule].run(resolved, self, &target) {
            Ok(res) => {
                (*self
                    .requested
                    .get_mut(&target)
                    // add() has added the item to requested as well
                    .expect("run_one"))
                .state = TargetState::Done(res.clone());
                Ok(res)
            }
            Err(e) => {
                let tgt = target.with_ctx(resolved).to_string();
                blog!(error, target = tgt, "Target failed with: {e}");
                (*self
                    .requested
                    .get_mut(&target)
                    // Entry must exist by construction
                    .expect("flush"))
                .state = TargetState::Error(e);
                Err(eval::Error::DependencyError(tgt))
            }
        }
    }

    // Demand a single target
    pub fn demand(
        &mut self,
        resolved: eval::ResolvedRef<'_>,
        target: Target,
    ) -> Result<TargetResult, eval::Error> {
        tracing::trace!("Resource {:?} demanded", target);
        // Check if value is already present
        match self.find_result(resolved, &target)? {
            // Target already finished, return result
            Some(v) => Ok(v),
            // Target still outstanding, run
            None => Ok(self.run_one(resolved, target)?),
        }
    }

    fn take(&mut self) -> HashMap<Target, RequestedTarget> {
        self.next.store(0, std::sync::atomic::Ordering::Relaxed);
        std::mem::take(&mut self.requested)
    }

    // Entrypoint for main
    // Errors when the targets could not be added
    pub fn demand_all(
        &mut self,
        resolved: eval::ResolvedRef<'_>,
        targets: &[Target],
    ) -> Result<Vec<TargetReport>, Spanned<eval::Error>> {
        for r in targets {
            let rule = self.add(resolved, r)?;
        }

        let mut res = Vec::with_capacity(targets.len());
        for r in targets.into_iter() {
            let _ = self
                .demand(resolved, r.clone())
                .without_span()
                .map(|v| res.push(v.value));
        }

        let mut report: Vec<TargetReport> = self
            .take()
            .into_iter()
            .map(|(target, st)| TargetReport {
                seq: st.seq,
                target: target.with_ctx(resolved).to_string(),
                result: match st.state {
                    TargetState::Error(error) => ReportResult::Error { error },

                    // TODO: Check that this state can be reached and corresponds to a cancelled dependency
                    TargetState::Requested => ReportResult::Cancelled,
                    TargetState::Running => ReportResult::Cancelled,
                    TargetState::Done(res) if res.ran => ReportResult::Success {
                        clean: res.clean,
                        value: res.value,
                    },
                    TargetState::Done(_res) => ReportResult::Skipped,
                },
            })
            .collect();
        report.sort_unstable_by_key(|r| r.seq);
        Ok(report)
    }
}

// Reporting
////////////

#[derive(Debug)]
pub enum ReportResult {
    /// Target was built successfully
    Success {
        clean: bool,
        value: eval::Value,
    },
    /// Scheduler decided rule did not need to be run (e.g. cache
    /// hit, dependencies clean)
    Skipped,
    /// Scheduled to run but not finished, e.g. build failed
    /// before target was handled
    Cancelled,
    Error {
        error: Spanned<eval::Error>,
    },
}

pub struct TargetReport {
    /// Rendered name
    pub seq: u32,
    pub target: String,
    pub result: ReportResult,
}

impl TargetReport {
    pub fn pprint(&self, sources: impl ariadne::Cache<()>) {
        // Status colour
        let (color) = match &self.result {
            ReportResult::Success { .. } => (yansi::Color::Green),
            ReportResult::Skipped => (yansi::Color::White),
            ReportResult::Cancelled => (yansi::Color::Yellow),
            ReportResult::Error { .. } => (yansi::Color::Red),
        };
        eprint!("[{}] ", self.target.paint(yansi::Color::BrightBlue));
        match &self.result {
            ReportResult::Success { clean, value } => {
                eprint!("{}", "completed".paint(color));
                if *clean {
                    eprint!(" (no changes)")
                }
            }
            ReportResult::Skipped => {
                eprint!("{}", "skipped".paint(color))
            }
            ReportResult::Cancelled => {
                eprint!("{}", "cancelled".paint(color))
            }
            ReportResult::Error { error } => {
                eprint!("{}\n", "error: ".paint(color),);
                error::report(error).print(sources).unwrap()
            }
        }
        eprintln!("")
    }
}
