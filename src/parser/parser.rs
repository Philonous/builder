use super::string::QuoteError;
use ariadne::{Color, Label, Report, ReportKind};
use chumsky::input::Stream;
use chumsky::{input::ValueInput, prelude::*};
use std::borrow::Cow;

use super::lexer::{
    command, normal, CommandToken, NormalToken, Token, Tokenizer,
};
use super::string::Quoted;
pub use crate::common::Ident;

#[derive(Clone, PartialEq, Debug)]
pub enum Lit<'a> {
    String(Quoted<'a>),
}

#[derive(Clone, PartialEq, Debug)]
pub enum MaybeQuoted<'a> {
    Unquoted(&'a str),
    Quoted(Quoted<'a>),
}

impl<'a> MaybeQuoted<'a> {
    fn resolve(&self) -> Result<Cow<'a, str>, QuoteError> {
        Ok(match self {
            Self::Unquoted(s) => Cow::Borrowed(*s),
            Self::Quoted(q) => q.unquote()?,
        })
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Command<S: Clone + PartialEq + std::fmt::Debug> {
    pub executable: S,
    pub args: Vec<S>,
}

impl<'a> Command<MaybeQuoted<'a>> {
    pub fn resolve_strings(self) -> Result<Command<Cow<'a, str>>, QuoteError> {
        let executable = self.executable.resolve()?;
        let mut args = Vec::with_capacity(self.args.len());
        for part in &self.args {
            args.push(part.resolve()?);
        }
        Ok(Command { executable, args })
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Argument<'a> {
    Bare(Expr<'a>),
    Named { name: Ident<'a>, value: Expr<'a> },
}

impl<'tok> Argument<'tok> {
    fn parse<'src: 'tok, I, E>(
        expr: E,
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
        E: Parser<'tok, I, Expr<'tok>, extra::Err<Rich<'tok, Token<'src>>>>
            + Clone,
    {
        Ident::parse()
            .then_ignore(just(normal!(Equals)))
            .then(expr.clone())
            .map(|(name, value)| Argument::Named { name, value })
            .or(expr.map(Argument::Bare))
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Arguments<'a>(pub Vec<Spanned<Argument<'a>>>);

impl<'tok> Arguments<'tok> {
    fn parse<'src: 'tok, I, E>(
        expr: E,
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
        E: Parser<'tok, I, Expr<'tok>, extra::Err<Rich<'tok, Token<'src>>>>
            + Clone,
    {
        Argument::parse(expr)
            .spanned()
            .separated_by(just(normal!(Comma)))
            .collect()
            .delimited_by(just(normal!(ParenOpen)), just(normal!(ParenClose)))
            .map(Arguments)
    }
}
#[derive(Clone, PartialEq, Debug)]
pub enum ExprKind<'a> {
    Lit(Lit<'a>),
    Ident(Ident<'a>),
    Command(Command<MaybeQuoted<'a>>),
    FunApp(Ident<'a>, Arguments<'a>),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Expr<'a>(pub Spanned<ExprKind<'a>>);

#[derive(Clone, PartialEq, Debug)]
pub enum Statement<'a> {
    Let(Spanned<Ident<'a>>, Expr<'a>),
    Expr(Expr<'a>),
}

// Like Argument, but only allows literals, e.g. for constant contexts
#[derive(Clone, PartialEq, Debug)]
pub enum LitArgument<'a> {
    Bare(Lit<'a>),
    Named { name: Ident<'a>, value: Lit<'a> },
}

impl<'tok> LitArgument<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        Ident::parse()
            .then_ignore(just(normal!(Equals)))
            .then(Lit::parse())
            .map(|(name, value)| LitArgument::Named { name, value })
            .or(Lit::parse().map(LitArgument::Bare))
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct LitArguments<'a>(pub Vec<Spanned<LitArgument<'a>>>);

impl<'tok> LitArguments<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        LitArgument::parse()
            .spanned()
            .separated_by(just(normal!(Comma)))
            .collect()
            .delimited_by(just(normal!(ParenOpen)), just(normal!(ParenClose)))
            .map(LitArguments)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum DeclHeader<'a> {
    /// Type and definition, e.g. "target" and "file("dist/exe")
    Explicit {
        decl: Spanned<Ident<'a>>,
        decl_args: LitArguments<'a>,
        name: Spanned<Ident<'a>>, // E.g. target file("dist/exe")
        args: LitArguments<'a>,
    },
    AtDecl(&'a str), // E.g. @dist/exe
}

#[derive(Clone, PartialEq, Debug)]
pub struct Block<'a>(pub Vec<Statement<'a>>);

impl<'tok> Block<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        Statement::parse()
            .separated_by(just(normal!(Semicolon)))
            .collect::<Vec<_>>()
            .delimited_by(just(normal!(BraceOpen)), just(normal!(BraceClose)))
            .map(Block)
    }
}

// TODO: This assumes all decls have target-shape, need to allow
// functions and constants at least
#[derive(Clone, PartialEq, Debug)]
pub struct Decl<'a> {
    pub header: DeclHeader<'a>,
    pub definition: Block<'a>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Ast<'a> {
    pub decls: Vec<Decl<'a>>,
}

impl<'tok> Ident<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        select! {
            normal!(Ident(s)) => Self(s),
        }
    }
}

impl<'tok> Lit<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        select! {
            normal!(StringLit(s)) => Lit::String(s),
        }
    }
}

impl<'tok> Command<MaybeQuoted<'tok>> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        let part = select! {
            command!(UnquotedString(s)) => MaybeQuoted::Unquoted(s),
            command!(QuotedString(s)) => MaybeQuoted::Quoted(s)
        };
        part.then(part.repeated().collect::<Vec<_>>())
            .delimited_by(
                just(normal!(CommandStart)),
                just(command!(CommandEnd)),
            )
            .map(|(executable, args)| Command { executable, args })
            .labelled("command")
    }
}

impl<'tok> Expr<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        recursive(|expr| {
            let arg_list = Arguments::parse(expr);
            let fun_app = Ident::parse()
                .then(arg_list)
                .map(|(ident, params)| ExprKind::FunApp(ident, params));
            Command::parse()
                .map(ExprKind::Command)
                .or(fun_app)
                .or(Lit::parse().map(ExprKind::Lit))
                .or(Ident::parse().map(ExprKind::Ident))
                .spanned()
                .map(Expr)
        })
    }
}

impl<'tok> Statement<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        let let_stmt = just(normal!(Let))
            .ignore_then(Ident::parse().spanned())
            .then_ignore(just(normal!(Equals)))
            .then(Expr::parse())
            .map(|(ident, expr)| Statement::Let(ident, expr));
        let_stmt.or(Expr::parse().map(Statement::Expr))
    }
}

impl<'tok> DeclHeader<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        let explicit = Ident::parse()
            .spanned()
            .then(LitArguments::parse())
            .then(Ident::parse().spanned())
            .then(LitArguments::parse())
            .map(|(((decl, decl_args), name), args)| Self::Explicit {
                decl,
                decl_args,
                name,
                args,
            });
        let at_decl = select! { normal!(AtString(s)) => DeclHeader::AtDecl(s) };
        at_decl.or(explicit)
    }
}

impl<'tok> Decl<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        let decls = Block::parse();
        DeclHeader::parse()
            .then(decls)
            .map(|(header, definition)| Decl { header, definition })
    }
}

impl<'tok> Ast<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        Decl::parse()
            .repeated()
            .collect::<Vec<_>>()
            .map(|decls| Ast { decls })
    }
}

pub fn parse_str<'a>(
    src: &'a str,
) -> Result<Ast<'a>, Vec<chumsky::error::Rich<'a, Token<'a>>>> {
    let token_iter = Tokenizer::new(src)
        // Convert logos errors into tokens. We want parsing to be recoverable and not fail at the lexing stage, so
        // we have a dedicated `Token::Error` variant that represents a token error that was previously encountered
        .map(|(tok, span)| match tok {
            // Turn the `Range<usize>` spans logos gives us into chumsky's `SimpleSpan` via `Into`, because it's easier
            // to work with
            Ok(tok) => (tok, span.into()),
            Err(_) => (Token::Error, span.into()),
        });

    // Turn the token iterator into a stream that chumsky can use for things like backtracking
    let token_stream = Stream::from_iter(token_iter)
        // Tell chumsky to split the (Token, SimpleSpan) stream into its parts so that it can handle the spans for us
        // This involves giving chumsky an 'end of input' span: we just use a zero-width span at the end of the string
        .map((0..src.len()).into(), |(t, s): (_, _)| (t, s));
    // Parse the token stream with our chumsky parser
    Ast::parse().parse(token_stream).into_result()
}

#[cfg(test)]
mod test {

    use insta;
    use std::fs;

    #[test]
    fn golden() {
        insta::glob!("test/parser", "*.in", |path| {
            let input = fs::read_to_string(path).unwrap();
            insta::assert_debug_snapshot!(
                path.file_stem().unwrap().to_str().unwrap(),
                super::parse_str(&input)
            );
        });
    }
}
