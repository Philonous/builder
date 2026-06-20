use super::string::QuoteError;
use chumsky::input::Stream;
use chumsky::{input::ValueInput, prelude::*};

use super::lexer::{
    command, normal, string, CommandToken, NormalToken, StringToken, Token, Tokenizer,
};
use super::string::Quoted;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ident<'a>(pub &'a str);

impl<'a> std::fmt::Display for Ident<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<'a> Default for Ident<'a> {
    fn default() -> Self {
        Self("")
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Lit<'a> {
    String(Quoted<'a>),
}

impl<'a> std::fmt::Display for Lit<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String(str) => write!(f, "{}", str),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum MaybeQuoted<'a> {
    Unquoted(&'a str),
    Quoted(Quoted<'a>),
}

impl<'a> MaybeQuoted<'a> {
    fn resolve(&self) -> Result<String, QuoteError> {
        Ok(match self {
            Self::Unquoted(s) => String::from(*s),
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
    pub fn resolve_strings(self) -> Result<Command<String>, QuoteError> {
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
    Named { name: Ident<'a>, expr: Expr<'a> },
}

impl<'tok> Argument<'tok> {
    fn parse<'src: 'tok, I, E>(
        expr: E,
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
        E: Parser<'tok, I, Expr<'tok>, extra::Err<Rich<'tok, Token<'src>>>> + Clone,
    {
        Ident::parse()
            .then_ignore(just(normal!(Equals)))
            .then(expr.clone())
            .map(|(name, expr)| Argument::Named { name, expr })
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
        E: Parser<'tok, I, Expr<'tok>, extra::Err<Rich<'tok, Token<'src>>>> + Clone,
    {
        Argument::parse(expr)
            .spanned()
            .separated_by(just(normal!(Comma)))
            .collect()
            .parenthesized()
            .map(Arguments)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum StringExprPart<'a> {
    Lit(Quoted<'a>),
    Splice(Spanned<Ident<'a>>),
}

#[derive(Clone, PartialEq, Debug)]
pub struct StringExpr<'a>(pub Vec<StringExprPart<'a>>);

#[derive(Clone, PartialEq, Debug)]
pub struct NeedExpr<'a>(pub Box<Expr<'a>>);

#[derive(Clone, PartialEq, Debug)]
pub enum ExprKind<'a> {
    Lit(Lit<'a>),
    String(StringExpr<'a>),
    Ident(Ident<'a>),
    Command(Command<MaybeQuoted<'a>>),
    FunApp(Ident<'a>, Spanned<Arguments<'a>>),
    Need(NeedExpr<'a>),
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

#[derive(Clone, PartialEq, Debug, Default)]
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
            .parenthesized()
            .map(LitArguments)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum PatternKind<'a> {
    Lit(Lit<'a>),
    Bind(Ident<'a>),
    Wildcard,
}

impl<'a> std::fmt::Display for PatternKind<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lit(lit) => {
                write!(f, "{}", lit)
            }
            Self::Bind(var) => {
                write!(f, "{}", var)
            }
            Self::Wildcard => {
                write!(f, "_")
            }
        }
    }
}

impl<'tok> PatternKind<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        let bind = Ident::parse().map(Self::Bind);
        let lit = Lit::parse().map(Self::Lit);
        let wildcard = just(normal!(Underscore)).map(|_| PatternKind::Wildcard);
        bind.or(lit).or(wildcard)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Pattern<'a> {
    Bare(PatternKind<'a>),
    Named {
        name: Ident<'a>,
        pattern: PatternKind<'a>,
    },
}

impl<'tok> Pattern<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        Ident::parse()
            .then_ignore(just(normal!(Equals)))
            .then(PatternKind::parse())
            .map(|(name, pattern)| Self::Named { name, pattern })
            .or(PatternKind::parse().map(Self::Bare))
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Patterns<'a>(pub Vec<Spanned<Pattern<'a>>>);

impl<'tok> Patterns<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        Pattern::parse()
            .spanned()
            .separated_by(just(normal!(Comma)))
            .collect()
            .parenthesized()
            .map(Patterns)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Block<'a> {
    pub statements: Vec<Statement<'a>>,
    pub returns_last_value: bool,
}

impl<'tok> Block<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        Statement::parse()
            .separated_by(just(normal!(Semicolon)))
            .collect::<Vec<_>>()
            .then(just(normal!(Semicolon)).or_not())
            .delimited_by(just(normal!(BraceOpen)), just(normal!(BraceClose)))
            .map(|(statements, scln)| Block {
                statements,
                returns_last_value: matches!(scln, None),
            })
    }
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
        // String consisting of a single lit part is a literal string
        // We make this distinction because in some contexts, only
        // literals are allowed
        select! {
                string!(LitPart(s)) => Lit::String(s),
        }
        .delimited_by(just(normal!(StringStart)), just(string!(StringEnd)))
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
            .delimited_by(just(normal!(CommandStart)), just(command!(CommandEnd)))
            .map(|(executable, args)| Command { executable, args })
            .labelled("command")
    }
}

impl<'tok> StringExpr<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        select! {
            string!(LitPart(s)) => StringExprPart::Lit(s),
            string!(SpliceIdent(s)) = tok =>
              StringExprPart::Splice(Spanned{ inner: Ident(s), span: tok.span()})
        }
        .repeated()
        .collect()
        .delimited_by(just(normal!(StringStart)), just(string!(StringEnd)))
        .map(Self)
    }
}

impl<'tok> NeedExpr<'tok> {
    fn parse<'src: 'tok, I, E>(
        expr: E,
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
        E: Parser<'tok, I, Expr<'tok>, extra::Err<Rich<'tok, Token<'src>>>> + Clone,
    {
        just(normal!(Need))
            .ignore_then(expr.parenthesized())
            .map(|exp| NeedExpr(Box::new(exp)))
    }
}

impl<'tok> Expr<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        recursive(|expr| {
            let arg_list = Arguments::parse(expr.clone());
            let fun_app = Ident::parse()
                .then(arg_list.spanned())
                .map(|(ident, params)| ExprKind::FunApp(ident, params));
            Command::parse()
                .map(ExprKind::Command)
                .or(fun_app)
                .or(NeedExpr::parse(expr.clone()).map(ExprKind::Need))
                .or(Lit::parse().map(ExprKind::Lit))
                .or(Ident::parse().map(ExprKind::Ident))
                // This has to come after Lit because they overlap
                .or(StringExpr::parse().map(ExprKind::String))
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

#[derive(Clone, PartialEq, Debug)]
pub enum TargetIdent<'a> {
    Normal {
        name: Spanned<Ident<'a>>, // E.g. target file("dist/exe")
        pattern: Patterns<'a>,
    },
    AtTarget(&'a str), // E.g. @dist/exe
}

impl<'tok> TargetIdent<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        let at_decl = select! { normal!(AtString(s)) => Self::AtTarget(s) };
        let normal_target_decl = Ident::parse()
            .spanned()
            .then(Patterns::parse())
            .map(|(name, pattern)| Self::Normal { name, pattern });
        at_decl.or(normal_target_decl)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Decl<'a> {
    Target {
        args: LitArguments<'a>,
        ident: TargetIdent<'a>,

        block: Block<'a>,
    },
}

impl<'tok> Decl<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        let target_header = select! {normal!(Ident("target")) => ()}.ignore_then(
            LitArguments::parse()
                .or_not()
                .map(|args| args.unwrap_or_default()),
        );
        let target = target_header
            .then(TargetIdent::parse())
            .then(Block::parse())
            .map(|((args, ident), block)| Self::Target { args, ident, block });

        target
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

pub fn parse_str<'a>(src: &'a str) -> Result<Ast<'a>, Vec<chumsky::error::Rich<'a, Token<'a>>>> {
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

// Helpers
/////////////////////////////////

trait ParenExt<'src, 'tok, I, O, E>: Parser<'src, I, O, E> + Clone + Sized
where
    I: Input<'src, Token = Token<'tok>>,
    E: extra::ParserExtra<'src, I>,
{
    fn parenthesized(self) -> impl Parser<'src, I, O, E> + Clone {
        self.delimited_by(just(normal!(ParenOpen)), just(normal!(ParenClose)))
    }
}

impl<'src, 'tok, I, O, E, P> ParenExt<'src, 'tok, I, O, E> for P
where
    P: Parser<'src, I, O, E> + Clone,
    I: Input<'src, Token = Token<'tok>>,
    E: extra::ParserExtra<'src, I>,
{
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
