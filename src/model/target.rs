use super::eval;
use super::parameters::{Parameter, Parameters};
use super::value::record;
use super::value::Ident;
use super::value::{RecordPattern, RecordSchemas, Value};
use crate::error;
use crate::model::eval::RecordId;
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

// pub enum TraceFun {
//     Builtin (fn (&Record) -> )
// }

#[derive(Debug, Default)]
pub struct TargetType {
    // Schema that defines how we refer to targets of this type
    pub request_schema: RecordId,
    pub name_is: usize,
    // pub gen_trace:
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
    ) -> Result<TargetTypeId, Error> {
        let req_id = records
            .register(interner, request_name, request_fields)
            .map_err(|record::Error::DuplicateRecordSchema(s)| {
            Error::DuplicateTargetType(s)
        })?;

        self.0.push(TargetType {
            request_schema: req_id,

            name_is,
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

pub fn register_target_type_file(
    interner: &mut Rodeo,
    records: &mut RecordSchemas,
    target_types: &mut TargetTypes,
) -> Result<TargetTypeId, Error> {
    let request_parameters = Parameters::builder(interner)
        .add_static("path", None)
        .build();

    target_types.register(interner, records, "file", request_parameters, 0)
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
    target_types.register(interner, records, "task", parameters, 0)
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
    pub fn run(
        &self,
        resolved: &eval::Resolved,
    ) -> Result<Value, error::Spanned<eval::Error>> {
        let mut variables = Default::default();
        let mut ctx = eval::EvalContext {
            variables: &mut variables,
            resolved,
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
}

#[derive(Default, Debug)]
pub struct Targets(Vec<Target>);

impl Targets {
    pub fn add(&mut self, t: Target) -> Result<(), Error> {
        self.0.push(t);
        // TODO: Check for name collisions so find_string never fails
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
}
