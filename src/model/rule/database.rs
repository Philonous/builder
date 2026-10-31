use lasso::Rodeo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::{self};
use crate::model::eval::{self};
use crate::model::parameters::Arguments;
use crate::model::rule::{ModuleId, Target};
use crate::model::value::{Ident, Record};
use crate::{
    model::{
        eval::{Resolved, ResolvedRef, Value},
        rule::{FastTrace, Trace},
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
    #[error("Database includes module {0} which is not known")]
    Module(String),
    #[error("Database refers to target {0} which is not known")]
    Target(String),
    #[error("Error reading database")]
    Database,
    #[error("{0}")]
    Eval(#[from] eval::Error),
}

type Resolver = Resolved<Rodeo>;

impl Resolver {
    fn intern(&mut self, s: &str) -> Ident {
        Ident(self.resolver.get_or_intern(s))
    }
}

// Helper types that can be serialized
#[derive(Debug, Serialize, Deserialize, Hash, Clone)]
pub enum SerializedValue {
    Bool(bool),
    Int(i64),
    Record(SerializedRecord),
    String(String),
    Unit,
}

impl SerializedValue {
    pub fn resolve(self, resolver: &mut Resolver) -> Result<Value, Error> {
        Ok(match self {
            SerializedValue::Unit => Value::Unit,
            SerializedValue::String(s) => Value::String(s),
            SerializedValue::Record(record) => {
                Value::Record(record.resolve(resolver)?)
            }
            SerializedValue::Int(i) => Value::Int(i),
            SerializedValue::Bool(b) => Value::Bool(b),
        })
    }

    fn from_resolved(v: &Value, resolved: ResolvedRef<'_>) -> Self {
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

#[derive(Debug, Serialize, Deserialize, Hash, Clone)]
pub struct SerializedRecord {
    name: String,
    values: Vec<(String, SerializedValue)>,
}

impl SerializedRecord {
    pub fn resolve(
        &self,
        resolver: &mut Resolver,
    ) -> Result<record::Record, Error> {
        // Intern record name
        let ident = resolver.intern(&self.name);
        // resolve fields
        let arguments = Arguments {
            positional: Vec::new(),
            key_value: self
                .values
                .iter()
                .cloned()
                .map(
                    |(k, v)| -> Result<error::Spanned<(Ident, Value)>, Error> {
                        let ident = resolver.intern(&k);
                        Ok(error::Spanned {
                            inner: (ident, v.resolve(resolver)?),
                            span: None,
                        })
                    },
                )
                .collect::<Result<Vec<_>, Error>>()?,
        };
        // Look up schema
        let Some((schema_id, schema)) = resolver.records.find(ident) else {
            return Err(Error::Record);
        };
        let values = schema
            .fields
            .match_arguments(
                &resolver.resolver,
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

    fn from_resolved(r: &Record, resolved: ResolvedRef<'_>) -> Self {
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

#[derive(Debug, Serialize, Deserialize, Hash)]
pub struct SerializedTarget {
    module: SerializedModule,
    record: SerializedRecord,
}

impl SerializedTarget {
    pub fn resolve(self, resolved: &mut Resolver) -> Result<Target, Error> {
        Ok(Target {
            module: self.module.resolve(resolved)?,
            record: self.record.resolve(resolved)?,
        })
    }

    fn from_resolved(r: &Target, resolved: ResolvedRef<'_>) -> Self {
        Self {
            module: SerializedModule::from_resolved(r.module, resolved),
            record: SerializedRecord::from_resolved(&r.record, resolved),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Hash)]
struct SerializedModule(Vec<String>);

impl std::fmt::Display for SerializedModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut iter = self.0.iter();
        if let Some(root) = iter.next() {
            write!(f, ":{}", root)?;
            for i in self.0.iter() {
                write!(f, ":{}", i)?;
            }
        }
        Ok(())
    }
}

impl SerializedModule {
    pub fn resolve(self, resolver: &mut Resolver) -> Result<ModuleId, Error> {
        // Intern record name
        let name: Vec<Ident> =
            self.0.iter().map(|nm| resolver.intern(nm)).collect();
        resolver
            .modules
            .find(&name)
            .ok_or_else(|| Error::Module(self.to_string()))
    }

    fn from_resolved(mid: ModuleId, resolved: ResolvedRef<'_>) -> Self {
        Self(
            resolved[mid]
                .name
                .iter()
                .map(|name| resolved[*name].to_owned())
                .collect(),
        )
    }
}

#[derive(Serialize, Deserialize, Debug, Hash)]
pub struct SerializedDatabaseEntry {
    pub module: SerializedModule,
    pub target: SerializedRecord,
    pub fast_trace: Option<FastTrace>,
    pub trace: Option<Trace>,
    pub result: SerializedValue,
    pub dependencies: Vec<Vec<SerializedTarget>>,
}

impl SerializedDatabaseEntry {
    pub fn resolve(
        self,
        resolver: &mut Resolver,
    ) -> Result<DatabaseEntry, Error> {
        Ok(DatabaseEntry {
            target: Target {
                module: self.module.resolve(resolver)?,
                record: self.target.resolve(resolver)?,
            },
            fast_trace: self.fast_trace,
            trace: self.trace,
            result: self.result.resolve(resolver)?,
            dependencies: self
                .dependencies
                .into_iter()
                .map(|group| {
                    group
                        .into_iter()
                        .map(|d| d.resolve(resolver))
                        .collect::<Result<_, Error>>()
                })
                .collect::<Result<_, Error>>()?,
        })
    }

    fn from_resolved(dbe: &DatabaseEntry, resolved: ResolvedRef<'_>) -> Self {
        SerializedDatabaseEntry {
            module: SerializedModule::from_resolved(
                dbe.target.module,
                resolved,
            ),
            target: SerializedRecord::from_resolved(
                &dbe.target.record,
                resolved,
            ),
            fast_trace: dbe.fast_trace.clone(),
            trace: dbe.trace.clone(),
            result: SerializedValue::from_resolved(&dbe.result, resolved),
            dependencies: dbe
                .dependencies
                .iter()
                .map(|group| {
                    group
                        .iter()
                        .map(|r| SerializedTarget::from_resolved(r, resolved))
                        .collect()
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DatabaseEntry {
    pub target: Target,
    pub fast_trace: Option<FastTrace>,
    pub trace: Option<Trace>,
    pub result: Value,
    // Layers of dependencies. Dependencies are in a layers where
    // dependencies in later laters may depend on dependencies in
    // earlier layers. Dependencies in the same layer are independent
    // and be handled in parallel
    pub dependencies: Vec<Vec<Target>>,
}

pub struct Database {
    store: store::Store,
    entries: HashMap<Target, DatabaseEntry>,
    duplicates: usize,
}

impl Database {
    pub fn load(
        path: impl AsRef<std::path::Path>,
        block: bool,
        resolved: &mut Resolver,
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
            let entry = serialized_entry.resolve(resolved)?;

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
        resolved: ResolvedRef<'_>,
    ) -> Result<(), Error> {
        let serialized = postcard::to_stdvec(
            &SerializedDatabaseEntry::from_resolved(&entry, resolved),
        )
        .unwrap();
        self.store.append_record(&serialized)?;
        if let Some(_) = self.entries.insert(entry.target.clone(), entry) {
            self.duplicates += 1;
            if self.duplicates > self.entries.len() {
                self.compact(resolved)?
            }
        }
        Ok(())
    }

    fn compact(&mut self, resolved: ResolvedRef<'_>) -> Result<(), Error> {
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

    pub fn get(&self, target: &Target) -> Option<&DatabaseEntry> {
        self.entries.get(target)
    }
}
