use litrs;
use std::{borrow::Cow, fmt};

use logos::{Lexer, Logos};

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Clone, Default)]
pub enum LexError {
    #[error("unknown escape sequence: \\{0}")]
    UnknownEscape(char),
    #[default]
    #[error("Other lexing error")]
    Other,
}

/// Strips the surrounding `open`/`close` quotes from a lexer-produced
/// literal and resolves `\\` and `\<close>`.
///
/// `open` and `close` may differ (e.g. `«` / `»`) or be the same (`"` / `"`).
/// The escape character is hard-coded as `\`; neither delimiter may be `\`.
///
/// # Lexer invariants
///
/// * `s` starts with `open` and ends with `close`, with length at least
///   `open.len_utf8() + close.len_utf8()`. `debug_assert!`.
/// * No unescaped `close` appears between the delimiters — it would have
///   terminated the literal. **Not verified**; a violation silently copies
///   the stray delimiter through.
/// * The byte before the closing `close` is not a lone `\`, since that
///   backslash would have escaped the closing quote. Caught by `unreachable!`.
///
/// Stray `open` characters in the interior (when `open != close`) are not
/// the lexer's problem and are copied as ordinary text.
///
/// # Errors
///
/// [`UnquoteError::UnknownEscape`] if `\` is followed by anything other
/// than `\` or `close`.
pub fn unquote_string(
    open: char,
    close: char,
    s: &str,
) -> Result<Cow<'_, str>, LexError> {
    debug_assert!(s.starts_with(open) && s.ends_with(close));
    debug_assert!(s.len() >= open.len_utf8() + close.len_utf8());

    let inner = &s[open.len_utf8()..s.len() - close.len_utf8()];

    let Some(first) = inner.find('\\') else {
        return Ok(Cow::Borrowed(inner));
    };

    let (head, tail) = inner.split_at(first);
    let mut out = String::with_capacity(inner.len());
    out.push_str(head);

    let mut chars = tail.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some(c) if c == close => out.push(close),
                Some('\n') => out.push('\n'),
                Some('\t') => out.push('\t'),
                Some(other) => return Err(LexError::UnknownEscape(other)),
                None => unreachable!(
                    "lexer bug: trailing backslash inside quoted literal — \
                     it should have escaped the closing `{close}`"
                ),
            }
        } else {
            out.push(c);
        }
    }
    Ok(Cow::Owned(out))
}

#[derive(Debug, Logos, PartialEq, Clone)]
#[logos(error(LexError))]
#[logos(skip r"[ \t\r\n\f]+")]
pub enum Token<'a> {
    #[regex(r"[a-zA-Z][a-zA-Z/-]*", |lex| lex.slice())]
    Ident(&'a str),

    #[regex(r"@[.a-zA-Z][.a-zA-Z/-]*", |lex| lex.slice())]
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
    #[regex(r#""([^"\\\x00-\x1F]|\\(["\\bnfrt/]|u[a-fA-F0-9]{4}))*""#
        , |lex| unquote_string('"', '"', lex.slice()))]
    StringLit(Cow<'a, str>),
    // Taken from https://logos.maciej.codes/examples/json.html
    #[regex(r#"`([^`\\\x00-\x1F]|\\([`\\bnfrt/]|u[a-fA-F0-9]{4}))*`"#
        , |lex| unquote_string('`', '`', lex.slice()))]
    BackticksLit(Cow<'a, str>),

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
            Self::ParenOpen => write!(f, ""),
            Self::ParenClose => write!(f, ""),
            Self::BraceOpen => write!(f, ""),
            Self::BraceClose => write!(f, ""),
            Self::Semicolon => write!(f, ""),
            Self::Comma => write!(f, ""),
            Self::Equals => write!(f, ""),
            Self::StringLit(s) => write!(f, "{s}"),
            Self::BackticksLit(s) => write!(f, "{s}"),
            Self::Let => write!(f, ""),
            Self::Comment => write!(f, ""),
            Self::Error => write!(f, ""),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::fs;

    #[test]
    fn test() {
        let filename = "examples/test1.build";
        let src = fs::read_to_string(filename).expect("Failed to read file");
        let tokens: Vec<Result<Token, _>> =
            Token::lexer(src.as_str()).collect();

        // for t in tokens {
        //     println!("{:?}", t);
        // }

        assert_eq!(
            tokens,
            [
                Ok(Token::AtIdent("@dist/exe")),
                Ok(Token::BraceOpen),
                Ok(Token::Let),
                Ok(Token::Ident("exe")),
                Ok(Token::Equals),
                Ok(Token::StringLit(Cow::Borrowed("target/\"build\"/exe"))),
                Ok(Token::Semicolon),
                Ok(Token::Ident("need")),
                Ok(Token::ParenOpen),
                Ok(Token::StringLit(Cow::Borrowed("Cargo.toml"))),
                Ok(Token::Comma),
                Ok(Token::StringLit(Cow::Borrowed("Cargo.lock"))),
                Ok(Token::ParenClose),
                Ok(Token::Semicolon),
                Ok(Token::Ident("need")),
                Ok(Token::ParenOpen),
                Ok(Token::Ident("glob")),
                Ok(Token::ParenOpen),
                Ok(Token::StringLit(Cow::Borrowed("src/**/*"))),
                Ok(Token::ParenClose),
                Ok(Token::ParenClose),
                Ok(Token::Semicolon),
                Ok(Token::BackticksLit(Cow::Borrowed("cargo build"))),
                Ok(Token::BraceClose)
            ]
        );
    }
}
