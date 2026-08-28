use lasso::Rodeo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::{self};
use crate::model::eval::RecordId;
use crate::model::parameters::Arguments;
use crate::model::value::{Ident, Record, RecordSchema, RecordSchemas};
use crate::{
    model::{
        eval::{Resolved, Value},
        target::{FastTrace, Trace},
        value::record,
    },
    store,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Error deserializing database entry")]
    Deserialization,
    #[error("{0}")]
    IOError(#[from] std::io::Error),
    #[error("Error reconstructing record")]
    Record,
    #[error("Error reading database")]
    Database,
}

#[derive(Clone, Copy)]
struct Resolver<'a> {
    resolver: &'a dyn lasso::Resolver,
    records: &'a RecordSchemas,
}

impl<'a> From<&'a Resolved> for Resolver<'a> {
    fn from(resolved: &'a Resolved) -> Self {
        Resolver {
            resolver: &resolved.resolver,
            records: &resolved.records,
        }
    }
}

impl<'a> std::ops::Index<Ident> for Resolver<'a> {
    type Output = str;

    fn index(&self, index: Ident) -> &Self::Output {
        &self.resolver.resolve(&index.0)
    }
}

impl<'a> std::ops::Index<RecordId> for Resolver<'a> {
    type Output = RecordSchema;

    fn index(&self, index: RecordId) -> &Self::Output {
        &self.records[index]
    }
}

// Helper types that can be serialized
#[derive(Debug, Serialize, Deserialize, Hash)]
pub enum SerializedValue {
    Unit,
    String(String),
    Record(SerializedRecord),
    Int(i64),
    Bool(bool),
}

impl SerializedValue {
    pub fn resolve(
        self,
        interner: &mut Rodeo,
        records: &RecordSchemas,
    ) -> Result<Value, Error> {
        Ok(match self {
            SerializedValue::Unit => Value::Unit,
            SerializedValue::String(s) => Value::String(s),
            SerializedValue::Record(record) => {
                Value::Record(record.resolve(interner, records)?)
            }
            SerializedValue::Int(i) => Value::Int(i),
            SerializedValue::Bool(b) => Value::Bool(b),
        })
    }

    fn from_resolved(v: &Value, resolved: Resolver) -> Self {
        match v {
            Value::Unit => Self::Unit,
            Value::String(s) => Self::String(s.clone()),
            Value::Record(record) => {
                Self::Record(SerializedRecord::from_resolved(record, resolved))
            }
            Value::Int(i) => Self::Int(*i),
            Value::Bool(b) => Self::Bool(*b),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Hash)]
pub struct SerializedRecord {
    name: String,
    values: Vec<(String, SerializedValue)>,
}

impl SerializedRecord {
    pub fn resolve(
        self,
        interner: &mut Rodeo,
        records: &RecordSchemas,
    ) -> Result<record::Record, Error> {
        // Intern record name
        let ident = Ident(interner.get_or_intern(&self.name));
        // Look up schema
        let Some((schema_id, schema)) = records.find(ident) else {
            return Err(Error::Record);
        };
        // resolve fields
        let arguments = Arguments {
            positional: Vec::new(),
            key_value: self
                .values
                .into_iter()
                .map(
                    |(k, v)| -> Result<error::Spanned<(Ident, Value)>, Error> {
                        let ident = Ident(interner.get_or_intern(k));
                        Ok(error::Spanned {
                            inner: (ident, v.resolve(interner, records)?),
                            span: None,
                        })
                    },
                )
                .collect::<Result<Vec<_>, Error>>()?,
        };
        let values = schema
            .fields
            .match_arguments(
                interner,
                error::Spanned {
                    inner: &arguments,
                    span: None,
                },
            )
            .map_err(|_| Error::Record)?;
        Ok(Record {
            schema: schema_id,
            values,
        })
    }
}

impl SerializedRecord {
    fn from_resolved(r: &Record, resolved: Resolver) -> Self {
        Self {
            name: resolved[resolved[r.schema].name].to_owned(),
            values: resolved[r.schema]
                .fields
                .0
                .iter()
                .zip(r.values.iter())
                .map(|((name, _), v)| {
                    (
                        resolved[*name].to_owned(),
                        SerializedValue::from_resolved(v, resolved),
                    )
                })
                .collect(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Hash)]
pub struct SerializedDatabaseEntry {
    pub target: SerializedRecord,
    pub fast_trace: Option<FastTrace>,
    pub trace: Option<Trace>,
    pub result: SerializedValue,
    pub dependencies: Vec<Vec<SerializedRecord>>,
}

impl SerializedDatabaseEntry {
    pub fn resolve(
        self,
        interner: &mut Rodeo,
        records: &RecordSchemas,
    ) -> Result<DatabaseEntry, Error> {
        Ok(DatabaseEntry {
            target: self.target.resolve(interner, records)?,
            fast_trace: self.fast_trace,
            trace: self.trace,
            result: self.result.resolve(interner, records)?,
            dependencies: self
                .dependencies
                .into_iter()
                .map(|group| {
                    group
                        .into_iter()
                        .map(|d| d.resolve(interner, records))
                        .collect::<Result<_, Error>>()
                })
                .collect::<Result<_, Error>>()?,
        })
    }

    fn from_resolved(dbe: &DatabaseEntry, resolved: Resolver) -> Self {
        SerializedDatabaseEntry {
            target: SerializedRecord::from_resolved(&dbe.target, resolved),
            fast_trace: dbe.fast_trace.clone(),
            trace: dbe.trace.clone(),
            result: SerializedValue::from_resolved(&dbe.result, resolved),
            dependencies: dbe
                .dependencies
                .iter()
                .map(|group| {
                    group
                        .iter()
                        .map(|r| SerializedRecord::from_resolved(r, resolved))
                        .collect()
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DatabaseEntry {
    pub target: record::Record,
    pub fast_trace: Option<FastTrace>,
    pub trace: Option<Trace>,
    pub result: Value,
    // Layers of dependencies. Dependencies are in a layers where
    // dependencies in later laters may depend on dependencies in
    // earlier layers. Dependencies in the same layer are independent
    // and be handled in parallel
    pub dependencies: Vec<Vec<record::Record>>,
}

pub struct Database {
    store: store::Store,
    entries: HashMap<record::Record, DatabaseEntry>,
    duplicates: usize,
}

impl Database {
    pub fn load(
        path: impl AsRef<std::path::Path>,
        block: bool,
        interner: &mut Rodeo,
        records: &RecordSchemas,
    ) -> Result<Option<Self>, Error> {
        let Some(mut store) = store::Store::open(path, block)? else {
            return Ok(None);
        };
        let mut entries = HashMap::new();
        let mut duplicates = 0usize;
        for row in &mut store {
            let item = match row {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::error!("Error reading from database: {}", e);
                    return Err(Error::Record);
                }
            };
            let serialized_entry: SerializedDatabaseEntry =
                match postcard::from_bytes(&item) {
                    Ok(dbentry) => dbentry,
                    Err(e) => {
                        tracing::error!(
                            "Could not deserialize database entry: {e}"
                        );
                        return Err(Error::Deserialization);
                    }
                };
            tracing::debug!("Read db Entry: {:?}", serialized_entry);
            let entry = serialized_entry.resolve(interner, records)?;

            if let Some(_) = entries.insert(entry.target.clone(), entry) {
                duplicates += 1;
            }
        }

        let database = Database {
            store,
            entries,
            duplicates,
        };

        Ok(Some(database))
    }

    pub fn add(
        &mut self,
        entry: DatabaseEntry,
        resolved: &Resolved,
    ) -> Result<(), Error> {
        let serialized = postcard::to_stdvec(
            &SerializedDatabaseEntry::from_resolved(&entry, resolved.into()),
        )
        .unwrap();
        self.store.append_record(&serialized)?;
        if let Some(_) = self.entries.insert(entry.target.clone(), entry) {
            self.duplicates += 1;
            if self.duplicates > self.entries.len() {
                self.compact(resolved.into())?
            }
        }
        Ok(())
    }

    fn compact(&mut self, resolved: Resolver) -> Result<(), Error> {
        let mut path = self.store.path.clone();
        path.add_extension("tmp");
        // Create new empty store as temporary file
        let mut out = store::Store::new(path.clone())?;
        // Copy live entries into new store
        out.append_records(self.entries.values().map(|item| {
            postcard::to_stdvec(&SerializedDatabaseEntry::from_resolved(
                &item, resolved,
            ))
            .unwrap()
        }))?;

        // Replace old store with new store
        out.mv(&self.store.path)?;

        // Keep working with new store
        self.store = out;
        self.duplicates = 0;
        // TODO: What happens if another process is waiting on the
        // file lock of the overwritten file?
        Ok(())
    }

    pub fn get(&self, request: &record::Record) -> Option<&DatabaseEntry> {
        self.entries.get(request)
    }
}
