use std::fmt;

use logos::{Lexer, Logos, Span};

use super::string::Quoted;
use thiserror::Error;

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
    #[regex(r"[a-zA-Z][a-zA-Z/_-]*", |lex| lex.slice())]
    Ident(&'a str),

    #[regex(r"@[.-/a-zA-Z]+", |lex| &lex.slice()[1..])]
    AtString(&'a str),

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

    #[token("_")]
    Underscore,

    // Literals
    #[regex(r#"""#)]
    StringStart,

    #[token("true", |_| true)]
    #[token("false", |_| false)]
    Bool(bool),

    // Keywords
    #[token("let")]
    Let,

    #[token("need")]
    Need,

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
            Self::AtString(s) => write!(f, "@{s}"),
            Self::ParenOpen => write!(f, "open parenthesis"),
            Self::ParenClose => write!(f, "closing parenthesis"),
            Self::BraceOpen => write!(f, "open brace"),
            Self::BraceClose => write!(f, "closing brace"),
            Self::Semicolon => write!(f, "semicolon"),
            Self::Comma => write!(f, "comma"),
            Self::Equals => write!(f, "equals"),
            Self::Underscore => write!(f, "underscore"),
            Self::StringStart => write!(f, "quotation mark"),
            Self::Bool(b) => write!(f, "{}", if *b { "true" } else { "false" }),
            Self::CommandStart => write!(f, "backtick"),
            Self::Let => write!(f, "let"),
            Self::Need => write!(f, "need"),
            Self::Comment => write!(f, "#"),
        }
    }
}

#[derive(Debug, Logos, PartialEq, Clone)]
#[logos(error(LexError))]
#[logos(skip r"[ \t\r\n\f]+")]
pub enum CommandToken<'a> {
    #[regex(r#"[^ `\"$\t\r\n\f]+"#)]
    UnquotedString(&'a str),

    #[regex(r#"\{[[:space:]]*[a-zA-Z][a-zA-Z/_-]+[[:space:]]*\}"#, |lex| splice_name(lex.slice()))]
    SpliceIdent(&'a str),

    #[regex(r#"""#)]
    StringStart,

    #[token("`")]
    CommandEnd,
}

impl fmt::Display for CommandToken<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::UnquotedString(s) => write!(f, "{s}"),

            Self::SpliceIdent(s) => {
                write!(f, "{{{s}}}")
            }
            Self::StringStart => write!(f, "quotation mark"),
            Self::CommandEnd => Ok(()),
        }
    }
}

// Extract the variable of a splice, e.g. "{ my_var }" => "my_var"
fn splice_name(s: &str) -> Option<&str> {
    // Expect sigil followed by identifier wrapped in braces with optional spaces
    // e.g. ${ my_variable } or #{testing-testing}
    let re = regex::Regex::new(
        r#"\{[[:space:]]*([a-zA-Z_][a-zA-Z/-_]*)[[:space:]]*\}"#,
    )
    .expect("parsing regex");
    Some(re.captures(s)?.get(1)?.as_str())
}

#[derive(Debug, Logos, PartialEq, Clone)]
#[logos(error(LexError))]
#[logos(subpattern char = r#"[^{}"\\\x00-\x1F]"#)]
#[logos(subpattern escape = r#"\\["\\nrt{}]"#)]
pub enum StringToken<'a> {
    // #[regex(r#"[^ `\"$\t\r\n\f]+"#, |lex| lex.slice())]
    #[regex(r#"((?&char)|(?&escape))+"#, |lex| Quoted{delim: '"', raw: lex.slice()})]
    LitPart(Quoted<'a>),
    #[regex(r#"\{[[:space:]]*[a-zA-Z][a-zA-Z/_-]+[[:space:]]*\}"#, |lex| splice_name(lex.slice()))]
    SpliceIdent(&'a str),
    #[token(r#"""#)]
    StringEnd,
}

impl fmt::Display for StringToken<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            StringToken::LitPart(s) => write!(f, "{}", s),
            StringToken::SpliceIdent(s) => write!(f, "{{{}}}", s),
            StringToken::StringEnd => write!(f, "\""),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Token<'a> {
    Normal(NormalToken<'a>),
    String(StringToken<'a>),
    Command(CommandToken<'a>),
    // Error token, never produced directly. We synthesize it as a
    // marker for chumsky to consume when lexing fails
    Error,
}

impl fmt::Display for Token<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Normal(s) => write!(f, "{s}"),
            Self::String(s) => write!(f, "{s}"),
            Self::Command(s) => write!(f, "{s}"),
            Self::Error => write!(f, "unexpected token"),
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

pub(crate) use normal;

macro_rules! string {
    // string!(Bare)
    ($variant:ident) => {
        Token::String(StringToken::$variant)
    };
    // string!(String(str)), string!(Pair(a, b)), etc.
    ($variant:ident( $($inner:tt)* )) => {
        Token::String(StringToken::$variant( $($inner)* ))
    };
}

pub(crate) use string;

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

pub(crate) use command;

#[derive(Clone, Copy, Debug)]
enum Mode {
    Normal,
    String,
    Command,
}

#[derive(Debug, Default)]
enum LexerState<'a> {
    Normal(Lexer<'a, NormalToken<'a>>),
    String(Lexer<'a, StringToken<'a>>),
    Command(Lexer<'a, CommandToken<'a>>),
    #[default]
    Taken,
}

pub struct Tokenizer<'a> {
    current: LexerState<'a>,
    stack: Vec<Mode>, // modes to return to, deepest first
}

// transforms the current state by pooping a state from the state stack
macro_rules! pop_to {
    ($stack:expr, $lexer:expr) => {
        match $stack.pop().expect("pop on empty mode stack") {
            Mode::Normal => LexerState::Normal($lexer.morph()),
            Mode::String => LexerState::String($lexer.morph()),
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
        Tokenizer {
            current: LexerState::Normal(NormalToken::lexer(source)),
            stack: Vec::new(),
        }
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
                on NormalToken::CommandStart => (enter Command),
                on NormalToken::StringStart => (enter String)
            ),
            LexerState::String(mut lexer) => advance!(
                self, lexer,
                state String, wrap String,
                on StringToken::StringEnd => (leave)
            ),
            LexerState::Command(mut lexer) => advance!(
                self, lexer,
                state Command, wrap Command,
                on CommandToken::StringStart => (enter String),
                on CommandToken::CommandEnd => (leave)
            ),
            LexerState::Taken => unreachable!("Taken seen between next calls"),
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
