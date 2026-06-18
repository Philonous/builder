use std::cmp::Ordering;
use std::collections::HashMap;

use super::eval;
use super::parameters::{Parameter, Parameters};
use super::value::record;
use super::value::Ident;
use super::value::{RecordPattern, RecordSchemas, Value};
use crate::error::{self, Spanned, WithSpan};
use crate::model::eval::RecordId;
use crate::model::value::WithDisplayCtxExt;
use lasso::Rodeo;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Target type {0} is already declared")]
    DuplicateTargetType(String),
    #[error("Could not find target {0}")]
    TargetNotFoundByString(String),
    #[error("Multiple targets matching {0}")]
    TargetFoundMultiple(String),
}

pub struct FastTrace {
    last_modified: Option<u64>, // Unix time, if available
    size: Option<u64>,          // In bytes, if available
}

pub struct Trace {
    hash: u64,
}

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
        let file = std::fs::File::open(path)?;
        let hash = rapidhash::v3::rapidhash_v3_file_inline::<
            &std::fs::File,
            false,
        >(&file, &rapidhash::v3::DEFAULT_RAPID_SECRETS)?;
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
    // Resolved variable slots for running the block
    pub slots: eval::Slots,
    pub block: eval::Block,
}

impl Target {
    // Run the code
    pub fn execute(
        &self,
        resolved: &eval::Resolved,
        scheduler: &mut Scheduler,
    ) -> Result<Value, error::Spanned<eval::Error>> {
        let mut variables = Default::default();
        let mut ctx = eval::EvalContext {
            variables: &mut variables,
            resolved,
            scheduler,
        };
        // let mut ctx = ctx.enter(eval::Slots(Vec::new()));
        self.block.run(&mut ctx.enter(eval::Slots(Vec::new())))
    }

    // Try to match the target to a single string (e.g. cli)
    pub fn matches_string(&self, s: &str) -> bool {
        match &self.name {
            None => false,
            Some(n) => n == s,
        }
    }

    pub fn run(
        &self,
        resolved: &eval::Resolved,
        scheduler: &mut Scheduler,
    ) -> Result<Value, error::Spanned<eval::Error>> {
        // TODO: Check database for dependencies and query them
        // TODO: Compare traces
        let v = self.execute(resolved, scheduler);
        // The block might not have forced all dependencies, so we flush them now
        scheduler.flush(resolved).without_span()?;
        v
        // TODO: Compare traces
        // TODO: Cleanness
    }
}

#[derive(Default, Debug)]
pub struct Targets(Vec<Target>);

#[derive(Default, Debug, PartialEq, Eq, Clone, Copy)]
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
    Done(Value),
    Error,
}

struct TargetRequest {
    state: TargetState,
}

#[derive(Debug, Default)]
// single-threaded scheduler
pub struct Scheduler {
    requested: HashMap<record::Record, TargetState>,
    outstanding: Vec<(TargetId, record::Record)>,
}

impl Scheduler {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn add(
        &mut self,
        resolved: &eval::Resolved,
        request: record::Record,
        // Resolved target
        target: TargetId,
    ) -> Result<(), Spanned<eval::Error>> {
        let old = self.requested.get(&request);
        match old {
            None => {
                self.requested
                    .insert(request.clone(), TargetState::Requested);
                self.outstanding.push((target, request));
                Ok(())
            }
            // Target was already built
            Some(TargetState::Done(_)) => Ok(()),
            Some(TargetState::Running) => Err(eval::Error::CircularDependency(
                request.with_ctx(resolved).to_string(),
            ))
            .without_span(),
            Some(TargetState::Error) => {
                Err(eval::Error::DependencyError).without_span()
            }
            // Multiple need() on the same target - that's OK
            Some(TargetState::Requested) => Ok(()),
        }
    }

    pub fn flush(
        &mut self,
        resolved: &eval::Resolved,
    ) -> Result<(), eval::Error> {
        let items = std::mem::take(&mut self.outstanding);
        for (target, request) in items {
            match resolved[target].run(resolved, self) {
                Ok(v) => {
                    *self.requested.get_mut(&request).expect("flush") =
                        TargetState::Done(v)
                    // TODO: update variables in evaluation context
                }
                Err(_) => {
                    *self.requested.get_mut(&request).expect("flush") =
                        TargetState::Error;
                    return Err(eval::Error::DependencyError);
                }
            }
        }
        Ok(())
    }
}
