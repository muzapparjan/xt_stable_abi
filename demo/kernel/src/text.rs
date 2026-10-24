//! Splitting text into tokens that borrow the input.

use core::fmt;
use core::iter::Peekable;
use core::str::CharIndices;
use std::cell::Cell;

/// The kind of a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TokenKind {
    /// Letters, digits and underscores.
    Word,
    /// A run of digits, possibly with dots.
    Number,
    /// Anything else that is not whitespace.
    Punctuation,
}

/// One token, borrowing the text it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Token<'a> {
    /// The bytes of the token, as they appear in the input.
    pub text: &'a str,
    /// What the token is.
    pub kind: TokenKind,
    /// Where the token starts, in bytes.
    pub offset: usize,
}

impl<'a> Token<'a> {
    /// The token as the lowercase form a counter keys on.
    pub fn folded(&self) -> String {
        match self.kind {
            TokenKind::Word | TokenKind::Number => self.text.to_lowercase(),
            TokenKind::Punctuation => self.text.to_owned(),
        }
    }
}

impl fmt::Display for Token<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.text)
    }
}

/// An iterator over the tokens of a text.
///
/// The iterator borrows the input; the tokens borrow it too. The number of
/// characters it has looked at is recorded in a [`Cell`], which is the part of
/// the iterator that changes without a `&mut` borrow.
pub struct Tokenizer<'a> {
    text: &'a str,
    chars: Peekable<CharIndices<'a>>,
    scanned: Cell<usize>,
}

impl<'a> Tokenizer<'a> {
    /// The number of characters the iterator has consumed.
    pub fn scanned(&self) -> usize {
        self.scanned.get()
    }
}

impl<'a> Iterator for Tokenizer<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            let (offset, first) = self.chars.next()?;
            self.scanned.set(self.scanned.get() + 1);
            if first.is_whitespace() {
                continue;
            }
            let kind = if first.is_numeric() {
                TokenKind::Number
            } else if first.is_alphanumeric() || first == '_' {
                TokenKind::Word
            } else {
                TokenKind::Punctuation
            };
            let mut end = offset + first.len_utf8();
            while let Some((next_offset, next)) = self.chars.peek().copied() {
                let continues = match kind {
                    TokenKind::Word => next.is_alphanumeric() || next == '_',
                    TokenKind::Number => next.is_numeric() || next == '.',
                    TokenKind::Punctuation => {
                        !next.is_whitespace() && !next.is_alphanumeric() && next != '_'
                    }
                };
                if !continues {
                    break;
                }
                self.chars.next();
                self.scanned.set(self.scanned.get() + 1);
                end = next_offset + next.len_utf8();
            }
            return Some(Token {
                text: &self.text[offset..end],
                kind,
                offset,
            });
        }
    }
}

impl fmt::Debug for Tokenizer<'_> {
    // The text can be content a log must not repeat, so the debug form reports
    // how far the iterator has read.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Tokenizer")
            .field("scanned", &self.scanned())
            .finish()
    }
}

/// Splits `text` into tokens.
pub fn tokenize(text: &str) -> Tokenizer<'_> {
    Tokenizer {
        text,
        chars: text.char_indices().peekable(),
        scanned: Cell::new(0),
    }
}

/// The words of `text`, in order, borrowing the input.
pub fn words(text: &str) -> impl Iterator<Item = &str> {
    tokenize(text)
        .filter(|token| token.kind == TokenKind::Word)
        .map(|token| token.text)
}

/// Folds case and whitespace.
///
/// Every run of whitespace becomes one space, leading and trailing whitespace is
/// removed, and each character is lowercased with its own rules. The result is
/// what the counters compare, so two spellings of one word count as one term.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        for lower in character.to_lowercase() {
            out.push(lower);
        }
    }
    out
}

/// The number of single-character edits that turn `left` into `right`.
///
/// The table is two rows wide, because a row is all the recurrence needs.
pub fn levenshtein(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    if left.is_empty() {
        return right.len();
    }
    if right.is_empty() {
        return left.len();
    }
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0usize; right.len() + 1];
    for (row, left_char) in left.iter().enumerate() {
        current[0] = row + 1;
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != right_char);
            let deletion = previous[column + 1] + 1;
            let insertion = current[column] + 1;
            current[column + 1] = substitution.min(deletion).min(insertion);
        }
        core::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

/// The longest prefix of `text` that fits `max_bytes` and ends on a character
/// boundary.
pub fn truncate_at_boundary(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_borrow_the_input_and_keep_their_offsets() {
        let text = "Hello, world 42!";
        let tokens: Vec<_> = tokenize(text).collect();
        assert_eq!(tokens.len(), 5);
        assert_eq!(tokens[0].text, "Hello");
        assert_eq!(tokens[0].offset, 0);
        assert_eq!(tokens[1].text, ",");
        assert_eq!(tokens[1].offset, 5);
        assert_eq!(tokens[2].offset, 7);
        assert_eq!(tokens[4].kind, TokenKind::Punctuation);
        // The token points into the same allocation as the input.
        assert!(core::ptr::eq(tokens[0].text.as_ptr(), text.as_ptr()));
    }

    #[test]
    fn folding_and_normalising_agree() {
        assert_eq!(normalize("  Hello   WORLD\tgoodbye "), "hello world goodbye");
        assert_eq!(
            tokenize("ÄRGER").next().unwrap().folded(),
            "ärger".to_owned()
        );
    }

    #[test]
    fn the_iterator_counts_what_it_looked_at() {
        let mut tokenizer = tokenize("ab cd");
        assert_eq!(tokenizer.next().unwrap().text, "ab");
        assert_eq!(tokenizer.next().unwrap().text, "cd");
        assert!(tokenizer.next().is_none());
        assert_eq!(tokenizer.scanned(), 5);
    }

    #[test]
    fn edit_distance_handles_unicode_and_empty_inputs() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "drei"), 4);
        assert_eq!(levenshtein("grüß", "gruss"), 3);
    }

    #[test]
    fn truncation_never_splits_a_character() {
        assert_eq!(truncate_at_boundary("äbc", 2), "ä");
        assert_eq!(truncate_at_boundary("abc", 10), "abc");
    }
}
