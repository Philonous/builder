use std::fmt;

use crate::model::{eval::Resolved, value::RecordSchemas};

#[derive(Clone, Copy)]
pub struct DisplayCtx<'a> {
    pub resolved: &'a Resolved,
}

// Things that can be displayed given the context
pub trait DisplayWith {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
        resolved: &Resolved,
    ) -> fmt::Result;
}

pub struct WithCtx<'a, T: ?Sized>(&'a T, DisplayCtx<'a>);

impl<T: DisplayWith + ?Sized> fmt::Display for WithCtx<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f, self.1.resolved)
    }
}

// Ergonomic `.display(ctx)` on every DisplayWith type, for free.
pub trait WithDisplayCtxExt: DisplayWith {
    fn with_ctx<'a>(
        &'a self,
        ctx: impl Into<DisplayCtx<'a>>,
    ) -> WithCtx<'a, Self> {
        WithCtx(self, ctx.into())
    }

    fn to_string_with<'a>(&self, ctx: impl Into<DisplayCtx<'a>>) -> String {
        WithCtx(self, ctx.into()).to_string()
    }
}

impl<T: DisplayWith + ?Sized> WithDisplayCtxExt for T {}
