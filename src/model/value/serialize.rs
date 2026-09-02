use serde::{Serialize, Serializer};

use crate::model::value::RecordSchemas;

pub struct SerializeCtx<'a> {
    pub resolver: &'a lasso::RodeoResolver,
    pub records: &'a RecordSchemas,
}

pub trait SerializeWith {
    fn serialize_with<S>(&self, ctx: &SerializeCtx<'_>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer;
}

pub struct WithSerializeCtx<'a, T: ?Sized>(&'a T, SerializeCtx<'a>);

impl<'a, T: SerializeWith + ?Sized> Serialize for WithSerializeCtx<'a, T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize_with(&self.1, serializer)
    }
}
