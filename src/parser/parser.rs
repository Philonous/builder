use chumsky::input::Stream;
use chumsky::{input::ValueInput, prelude::*};
use std::borrow::Cow;

use super::error::ParseError;
use super::lexer;
use super::lexer::CommandToken;
use super::lexer::Token;
use super::string::Quoted;
use logos::Logos;

#[derive(Clone, PartialEq, Debug)]
pub struct Ident<'a>(pub &'a str);

#[derive(Clone, PartialEq, Debug)]
pub enum Lit<'a> {
    String(&'a str),
}

#[derive(Clone, PartialEq, Debug)]
enum CommandPart<'a> {
    Unquoted(&'a str),
    Quoted(Quoted<'a>),
}

#[derive(Clone, PartialEq, Debug)]
struct Command<'a>(Vec<CommandPart<'a>>);

impl<'a> Command<'a> {
    pub fn resolve_to_strings(
        &'a self,
    ) -> Result<Vec<Cow<'a, str>>, ParseError> {
        let mut res = Vec::with_capacity(self.0.len());
        for part in &self.0 {
            res.push(match part {
                CommandPart::Unquoted(s) => Cow::Borrowed(*s),
                CommandPart::Quoted(s) => s.unquote()?,
            });
        }
        Ok(res)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Expr<'a> {
    Lit(Lit<'a>),
    Ident(Ident<'a>),
    Command(Command<'a>),
    FunApp(Ident<'a>, Vec<Expr<'a>>),
}

#[derive(Clone, PartialEq, Debug)]
pub enum Statement<'a> {
    Let(Ident<'a>, Expr<'a>),
    Expr(Expr<'a>),
}

#[derive(Clone, PartialEq, Debug)]
pub enum DeclHeader<'a> {
    /// Type and definition, e.g. "target" and "file("dist/exe")
    Explicit(Expr<'a>, Expr<'a>), // E.g. target file("dist/exe")
    AtDecl(Ident<'a>), // E.g. @dist/exe
}

#[derive(Clone, PartialEq, Debug)]
pub struct Decl<'a> {
    pub header: DeclHeader<'a>,
    pub definition: Vec<Statement<'a>>,
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
            Token::Ident(str) => Ident(str),
        }
    }

    fn at<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        select! {
            Token::AtIdent(str) => Ident(str),
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
            Token::StringLit(str) => Lit::String(str),
        }
    }
}

impl<'tok> Command<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        select! {
            Token::Command(parts) => {
                let parts = parts.iter().filter_map(|p| match p {
                    CommandToken::UnquotedString(s) =>
                    Some(CommandPart::Unquoted(s)),
                    CommandToken::QuotedString(s) => Some(CommandPart::Quoted(s)),
                    CommandToken::CommandEnd => None,
                }).collect();
                Command(parts)
            }
        }
    }
}

impl<'tok> Expr<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        recursive(|expr| {
            let param_list = expr
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .delimited_by(just(Token::ParenOpen), just(Token::ParenClose));
            let fun_app = Ident::parse()
                .then(param_list)
                .map(|(ident, params)| Expr::FunApp(ident, params));
            Command::parse()
                .map(Expr::Command)
                .or(fun_app)
                .or(Lit::parse().map(Expr::Lit))
                .or(Ident::parse().map(Expr::Ident))
        })
    }
}

impl<'tok> Statement<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        let let_stmt = just(Token::Let)
            .ignore_then(Ident::parse())
            .then_ignore(just(Token::Equals))
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
        let explicit = Expr::parse()
            .then(Expr::parse())
            .map(|(typ, decl)| Self::Explicit(typ, decl));
        Ident::at().map(DeclHeader::AtDecl).or(explicit)
    }
}

impl<'tok> Decl<'tok> {
    fn parse<'src: 'tok, I>(
    ) -> impl Parser<'tok, I, Self, extra::Err<Rich<'tok, Token<'src>>>> + Clone
    where
        I: ValueInput<'tok, Token = Token<'src>, Span = SimpleSpan>,
    {
        let decls = Statement::parse()
            .separated_by(just(Token::Semicolon))
            .collect::<Vec<_>>()
            .delimited_by(just(Token::BraceOpen), just(Token::BraceClose));
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
) -> Result<Ast<'a>, Vec<chumsky::error::Rich<'a, lexer::Token<'a>>>> {
    let token_iter = Token::lexer(src)
        .spanned()
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
    use super::*;

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
