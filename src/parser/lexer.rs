use std::fmt;

use logos::{Lexer, Logos, Span};

use thiserror::Error;
use super::string::Quoted;

#[derive(Debug, Error, PartialEq, Clone, Default)]
pub enum LexError {
    #[default]
    #[error("Other lexing error")]
    Other,
}

#[derive(Debug, Logos, PartialEq, Clone)]
#[logos(error(LexError))]
#[logos(skip r"[ \t\r\n\f]+")]
pub enum NormalToken<'a> {
    #[regex(r"[a-zA-Z][a-zA-Z/-]*", |lex| lex.slice())]
    Ident(&'a str),

    #[regex(r"@[.a-zA-Z][.a-zA-Z/-]*", |lex| &lex.slice()[1..])]
    AtIdent(&'a str),

    #[token("(")]
    ParenOpen,
    #[token(")")]
    ParenClose,
    #[token("{")]
    BraceOpen,
    #[token("}")]
    BraceClose,
    #[token(";")]
    Semicolon,
    #[token(",")]
    Comma,
    #[token("=")]
    Equals,

    // Literals

    // Taken from https://logos.maciej.codes/examples/json.html
    #[regex(r#""([^"\\\x00-\x1F]|\\(["\\bnfrt/]|u[a-fA-F0-9]{4}))*""#,
        |s| Quoted{raw: s.slice(), delim: '"'})]
    StringLit(Quoted<'a>),

    // Keywords
    #[token("let")]
    Let, // let

    // Comment
    #[regex(r"#[^\n]*", logos::skip, allow_greedy = true)]
    Comment,

    // Command blocks
    #[token("`")]
    CommandStart,
}

impl fmt::Display for NormalToken<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Ident(s) => write!(f, "{s}"),
            Self::AtIdent(s) => write!(f, "@{s}"),
            Self::ParenOpen => write!(f, "("),
            Self::ParenClose => write!(f, ")"),
            Self::BraceOpen => write!(f, "{{"),
            Self::BraceClose => write!(f, "}}"),
            Self::Semicolon => write!(f, ";"),
            Self::Comma => write!(f, ","),
            Self::Equals => write!(f, "="),
            Self::StringLit(s) => {
                let sraw = s.raw;
                write!(f, "{sraw}")
            },
            Self::CommandStart => write!(f, "`"),
            Self::Let => write!(f, "let"),
            Self::Comment => write!(f, "#"),
        }
    }
}

#[derive(Debug, Logos, PartialEq, Clone)]
#[logos(error(LexError))]
#[logos(skip r"[ \t\r\n\f]+")]
pub enum CommandToken<'a> {
    #[regex(r#"[^ `\"$\t\r\n\f]+"#, |lex| lex.slice())]
    UnquotedString(&'a str),
    #[regex(r#""([^"\\\x00-\x1F]|\\(["\\bnfrt/]|u[a-fA-F0-9]{4}))*""#,
        |s| Quoted{raw: s.slice(), delim: '"'})]
    QuotedString(Quoted<'a>),
    #[token("`")]
    CommandEnd,
}

impl fmt::Display for CommandToken<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::UnquotedString(s) => write!(f, "{s}"),
            Self::QuotedString(s) => {
                let sraw=s.raw;
                write!(f, "{sraw}")}
            Self::CommandEnd => Ok(()),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Token<'a> {
    Normal(NormalToken<'a>),
    Command(CommandToken<'a>),
        // Error token, never produced directly. We synthesize it as a
    // marker for chumsky to consume when lexing fails
    Error,
}

impl fmt::Display for Token<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Normal(s) => write!(f, "{s}"),
            Self::Command(s) => write!(f, "{s}"),
            Self::Error => write!(f, ""),
        }
    }
}

// Shortcuts for patterns

macro_rules! normal {
    // normal!(Semicolon)
    ($variant:ident) => {
        Token::Normal(NormalToken::$variant)
    };
    // normal!(String(str)), normal!(Pair(a, b)), etc.
    ($variant:ident( $($inner:tt)* )) => {
        Token::Normal(NormalToken::$variant( $($inner)* ))
    };
}

pub (crate) use normal;

macro_rules! command {
    // normal!(Semicolon)
    ($variant:ident) => {
        Token::Command(CommandToken::$variant)
    };
    // normal!(String(str)), normal!(Pair(a, b)), etc.
    ($variant:ident( $($inner:tt)* )) => {
        Token::Command(CommandToken::$variant( $($inner)* ))
    };
}

pub (crate) use command;

#[derive(Clone, Copy, Debug)]
enum Mode { Normal, Command }

#[derive(Debug, Default)]
enum LexerState<'a> {
    Normal(Lexer<'a, NormalToken<'a>>),
    Command(Lexer<'a, CommandToken<'a>>),
    #[default] Taken,
}



pub struct Tokenizer<'a> {
    current: LexerState<'a>,
    stack:   Vec<Mode>,        // modes to return to, deepest first
}



// macro_rules! advance {
//     ($self:expr, $lexer:expr,
//         state $from:ident, wrap $wrap:ident,
//         $( on $trans:pat => state $to:ident ),+ $(,)?
//     ) =>
//      {{
//         match $lexer.next().map(|token| (token, $lexer.span())) {
//             None         => { *$self = LexerState::$from($lexer); None }
//             Some((Err(e), span))
//             => { *$self = LexerState::$from($lexer); Some((Err(e), span)) }
//             Some((Ok(tok), span)) => {
//                 *$self = match &tok {
//                     $( $trans => LexerState::$to($lexer.morph()), )+
//                     _ => LexerState::$from($lexer),
//                 };
//                 Some((Ok(Token::$wrap(tok)), span))
//             }
//         }
//     }};
// }


// transforms the current state by pooping a state from the state stack
macro_rules! pop_to {
    ($stack:expr, $lexer:expr) => {
        match $stack.pop().expect("pop on empty mode stack") {
            Mode::Normal  => LexerState::Normal($lexer.morph()),
            Mode::Command => LexerState::Command($lexer.morph()),
        }
    };
}

// Define a match arm,
// Given that we are in starting state $from, wrap tokens in $wrap
// If we find one of $trans tokens, advance to new state $to
// Otherwise stay in the current state
macro_rules! advance {
    ($tok:expr, $lexer:expr,
        state $from:ident, wrap $wrap:ident,
        $( on $trans:pat => $action:tt ),+ $(,)?
    ) => {{
        match $lexer.next().map(|token| (token, $lexer.span())) {
            None         => { $tok.current = LexerState::$from($lexer); None }
            Some((Err(e), span)) =>
            { $tok.current = LexerState::$from($lexer); Some((Err(e), span)) }
            Some((Ok(t), span))  => {
                $tok.current = match &t {
                    $( $trans => advance!(@act $action, $tok.stack, $lexer, $from), )+
                    _ => LexerState::$from($lexer),
                };
                Some((Ok(Token::$wrap(t)), span))
            }
        }
    }};
    (@act (enter $to:ident), $stack:expr, $lexer:expr, $from:ident) => {{
        $stack.push(Mode::$from);
        LexerState::$to($lexer.morph())
    }};
    (@act (leave), $stack:expr, $lexer:expr, $from:ident) => {
        pop_to!($stack, $lexer)
    };
    (@act (switch $to:ident), $stack:expr, $lexer:expr, $from:ident) => {
        LexerState::$to($lexer.morph())
    };
}

impl<'a> Tokenizer<'a> {
    pub fn new(source: &'a str) -> Self {
        Tokenizer{ current: LexerState::Normal(NormalToken::lexer(source))
            , stack: Vec::new() }
    }
}

impl<'a> Iterator for Tokenizer<'a> {
    type Item = (Result<Token<'a>, LexError>, Span);
    // self.lexer.next().map(|token| (token, self.lexer.span()))
    fn next(&mut self) -> Option<(Result<Token<'a>, LexError>, Span)> {
        let lexer = std::mem::take(&mut self.current);
        match lexer {
            LexerState::Normal(mut lexer) => advance!(
                self, lexer,
                state Normal, wrap Normal,
                on NormalToken::CommandStart => (enter Command)
            ),
            LexerState::Command(mut lexer) => advance!(
                self, lexer,
                state Command, wrap Command,
                on CommandToken::CommandEnd => (leave)
            ),
            LexerState::Taken => unreachable!("Taken seen between next calls")
        }
    }
}



#[cfg(test)]
mod test {
    use super::*;
    use insta;
    use std::fs;

    #[test]
    fn golden() {
        eprintln!("{:?}", std::env::current_dir());
        insta::glob!("test/lexer", "*.in", |path| {
            let src = fs::read_to_string(path).expect("Failed to read file");
            let tokens: Vec<(Result<Token, _>, Span)> =
            Tokenizer::new(src.as_str()).collect();

            insta::assert_debug_snapshot!(
                path.file_stem().unwrap().to_str().unwrap(),
                tokens
            );
        });
    }
}
