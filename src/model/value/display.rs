use std::fmt;

use crate::model::eval::ResolvedRef;

#[derive(Clone, Copy)]
pub struct DisplayCtx<'a> {
    pub resolved: ResolvedRef<'a>,
}

// Things that can be displayed given the context
pub trait DisplayWith {
    fn fmt<'a>(
        &self,
        f: &mut fmt::Formatter<'_>,
        resolved: ResolvedRef<'a>,
    ) -> fmt::Result;
}

pub struct WithCtx<'a, T: ?Sized>(&'a T, DisplayCtx<'a>);

impl<T> fmt::Display for WithCtx<'_, T>
where
    T: DisplayWith + ?Sized,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f, self.1.resolved)
    }
}

pub trait WithDisplayCtxExt: DisplayWith {
    fn with_ctx<'a>(
        &'a self,
        ctx: impl Into<DisplayCtx<'a>>,
    ) -> WithCtx<'a, Self> {
        WithCtx(self, ctx.into())
    }
}

impl<T: DisplayWith + ?Sized> WithDisplayCtxExt for T {}
