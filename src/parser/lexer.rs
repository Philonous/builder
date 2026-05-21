use std::fmt;

use logos::{Lexer, Logos};

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
pub enum Token<'a> {
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

    // Command blocks
    #[token("`" , command_block)]
    CommandStart,
    CommandUnquoted(&'a str),
    CommandQuoted(&'a str),
    CommendEnd,

    // Keywords
    #[token("let")]
    Let, // let

    // Comment
    #[regex(r"#[^\n]*", logos::skip, allow_greedy = true)]
    Comment,

    // Error token, never produced directly. We synthesize it as a
    // marker for chumsky to consume when lexing fails
    Error,
}

impl fmt::Display for Token<'_> {
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
            // Self::Command(s) => {
            //     write!(f, "`")?;
            //     let mut iter = s.iter();
            //     if let Some(first) = iter.next() {
            //         write!(f, "{first}")?;
            //         for x in iter {
            //             write!(f, " {x}")?;
            //         }
            //     }
            //     write!(f, "`")?;
            //     Ok(())

            // }
            Self::Let => write!(f, "let"),
            Self::Comment => write!(f, "#"),
            Self::Error => write!(f, ""),
        }
    }
}

// Command tokens


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



fn command_block<'a>(lex: &mut Lexer<'a, Token<'a>>) -> Option<Vec<CommandToken<'a>>> {
    let mut command_lexer = lex.clone().morph::<CommandToken<'a>>();
    let mut tokens = Vec::new();
    while let Some(token) = command_lexer.next() {
        match token {
            Ok(CommandToken::CommandEnd) => break,
            Err(_) => return None,
            Ok(tok) => tokens.push(tok),
        }
    }
    *lex = command_lexer.morph();
    Some(tokens)
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
            let tokens: Vec<Result<Token, _>> =
                Token::lexer(src.as_str()).collect();

            insta::assert_debug_snapshot!(
                path.file_stem().unwrap().to_str().unwrap(),
                tokens
            );
        });
    }
}
