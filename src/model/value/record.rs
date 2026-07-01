use std::fmt::Display;

use lasso::Rodeo;

use crate::model::eval::RecordId;
use crate::model::value::display::WithDisplayCtxExt as _;
use crate::model::value::{DisplayCtx, DisplayWith};

use super::super::parameters::{Parameters, Patterns};
use super::Ident;
use super::Value;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Record schema {0} is already declared")]
    DuplicateRecordSchema(String),
}

#[derive(Debug, Default, Clone)]
pub struct RecordSchema {
    pub name: Ident,
    pub fields: Parameters,
}

#[derive(Debug, Default, Clone)]
pub struct RecordSchemas(Vec<RecordSchema>);

impl RecordSchemas {
    pub fn register(
        &mut self,
        interner: &mut Rodeo,
        name: &str,
        fields: Parameters,
    ) -> Result<RecordId, Error> {
        let ident = Ident(interner.get_or_intern(name));
        if let Some(_) = self.0.iter().find(|r| r.name == ident) {
            return Err(Error::DuplicateRecordSchema(String::from(name)));
        }
        self.0.push(RecordSchema {
            name: ident,
            fields,
        });

        Ok(RecordId(self.0.len() - 1))
    }

    pub fn find(&self, name: Ident) -> Option<(RecordId, &RecordSchema)> {
        self.0.iter().enumerate().find_map(|(ix, r)| {
            if r.name == name {
                Some((RecordId(ix), r))
            } else {
                None
            }
        })
    }
}

impl std::ops::Index<RecordId> for RecordSchemas {
    type Output = RecordSchema;

    fn index(&self, index: RecordId) -> &Self::Output {
        &self.0[index.0]
    }
}

#[derive(Debug, Default)]
pub struct RecordPattern {
    pub schema: RecordId,
    pub patterns: Patterns,
}

impl RecordPattern {
    pub fn matches(&self, record: &Record) -> bool {
        self.schema == record.schema && self.patterns.matches(&record.values)
    }
}

#[derive(Debug, Clone)]
pub struct Record {
    pub schema: RecordId,
    pub values: Vec<Value>,
}

impl<'a> DisplayWith for Record {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>, ctx: DisplayCtx) -> std::fmt::Result {
        let schema = &ctx.records[self.schema];
        schema.name.with_ctx(ctx).fmt(f)?;
        write!(f, "(")?;
        let mut it = schema.fields.0.iter().zip(self.values.iter());
        if let Some(((name, _param), value)) = it.next() {
            write!(f, "{}={}", name.with_ctx(ctx), value.with_ctx(ctx))?;

            for ((name, param), value) in it {
                write!(f, ", {}={}", name.with_ctx(ctx), value.with_ctx(ctx))?;
            }
            write!(f, ")")?;
        }
        Ok(())
    }
}

impl<'a> PartialEq for Record {
    fn eq(&self, other: &Self) -> bool {
        self.schema == other.schema && self.values == other.values
    }
}

impl<'a> Eq for Record {}

// impl<'a> PartialOrd for Record {
//     fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
//         Some(self.cmp(other))
//     }
// }

// impl<'a> Ord for Record {
//     fn cmp(&self, other: &Self) -> std::cmp::Ordering {
//         match self.schema.name.cmp(&other.schema.name) {
//             core::cmp::Ordering::Equal => {}
//             ord => return ord,
//         }
//         self.values.cmp(&other.values)
//     }
// }

impl<'a> std::hash::Hash for Record {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.schema.0.hash(state);
        self.values.hash(state);
    }
}

impl<'a> Default for Record {
    fn default() -> Self {
        Self {
            schema: Default::default(),
            values: Default::default(),
        }
    }
}
