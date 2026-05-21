use std::borrow::Cow;

use super::error::ParseError;

#[derive(Debug, Clone, PartialEq)]
pub struct Quoted<'a> {
    pub delim: char,
    pub raw: &'a str,
}

impl<'a> Quoted<'a> {
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
    pub fn unquote(&'a self) -> Result<Cow<'_, str>, ParseError> {
        let s = &self.raw;
        debug_assert!(s.starts_with(self.delim) && s.ends_with(self.delim));
        debug_assert!(s.len() >= self.delim.len_utf8() + self.delim.len_utf8());

        let inner = &s[self.delim.len_utf8()..s.len() - self.delim.len_utf8()];

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
                    Some(c) if c == self.delim => out.push(self.delim),
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => {
                        return Err(ParseError::UnknownEscape(other))
                    }
                    None => unreachable!(
                    "lexer bug: trailing backslash inside quoted literal — \
                     it should have escaped the closing delimiter"
                ),
                }
            } else {
                out.push(c);
            }
        }
        Ok(Cow::Owned(out))
    }
}
