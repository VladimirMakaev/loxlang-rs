use logos::{Logos, Skip};
use thiserror::Error;

use crate::parser::Span;

pub type PosIdx = usize;

#[derive(Logos, Debug, PartialEq, Clone, Copy)]
#[logos(error = LexerError, extras = usize)]
pub enum TokenType {
    #[regex(r"//.*\n?", logos::skip)]
    #[regex(r"[ \t\n\f]+", logos::skip)]
    Error,
    #[token("(")]
    LeftParen,
    #[token(")")]
    RightParen,
    #[token("{")]
    LeftBrace,
    #[token("}")]
    RightBrace,
    #[token(",")]
    Comma,
    #[token(".")]
    Dot,
    #[token("-")]
    Minus,
    #[token("+")]
    Plus,
    #[token(";")]
    Semicolon,
    #[token("/")]
    Slash,
    #[token("*")]
    Star,
    #[token("!")]
    Bang,
    #[token("!=")]
    BantEqual,
    #[token("=")]
    Equal,
    #[token("==")]
    EqualEqual,
    #[token(">")]
    Greater,
    #[token(">=")]
    GreaterEqual,
    #[token("<")]
    Less,
    #[token("<=")]
    LessEqual,
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*")]
    Identifier,
    #[regex("\"[^\"]*\"")]
    String,
    #[regex(r"\d+(\.\d+)?")]
    Number,
    #[token("and")]
    And,
    #[token("class")]
    Class,
    #[token("else")]
    Else,
    #[token("false")]
    False,
    #[token("for")]
    For,
    #[token("fun")]
    Fun,
    #[token("if")]
    If,
    #[token("nil")]
    Nil,
    #[token("or")]
    Or,
    #[token("print")]
    Print,
    #[token("return")]
    Return,
    #[token("super")]
    Super,
    #[token("this")]
    This,
    #[token("true")]
    True,
    #[token("var")]
    Var,
    #[token("while")]
    While,
    #[regex(r"\n", newline_callback)]
    Newline,
}

fn newline_callback(lex: &mut logos::Lexer<TokenType>) -> logos::Skip {
    lex.extras += 1;
    Skip
}

#[derive(Error, Debug, Default, Clone, PartialEq)]
pub enum LexerError {
    #[default]
    #[error("Invalid token found")]
    InvalidToken,
    #[error("Invalid token found at {start:?}..{end:?}")]
    InvalidTokenAt { start: PosIdx, end: PosIdx },
    #[error("Unterminated string")]
    UnterminatedString { line: usize },
    #[error("Unexpected character")]
    UnexpectedCharacter { line: usize },
}

#[derive(Debug, PartialEq)]
pub struct Token {
    token_type: TokenType,
    start: PosIdx,
    end: PosIdx,
    line: usize,
}

impl Token {
    pub fn slice<'a>(&self, v: &'a str) -> &'a str {
        &v[self.start..self.end]
    }

    pub fn ty(&self) -> TokenType {
        self.token_type
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn span(&self) -> Span {
        Span::new(self.start, self.end)
    }
}

pub struct Lexer<'source> {
    logos_lexer: logos::Lexer<'source, TokenType>,
}

impl<'source> Iterator for Lexer<'source> {
    type Item = Result<Token, LexerError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_token()
    }
}

impl<'source> Lexer<'source> {
    pub fn new(source: &'source str) -> Self {
        Self {
            logos_lexer: logos::Lexer::new(source),
        }
    }

    pub fn next_token(&mut self) -> Option<Result<Token, LexerError>> {
        match self.logos_lexer.next() {
            Some(Ok(t)) => Some(Ok(Token {
                start: self.logos_lexer.span().start,
                end: self.logos_lexer.span().end,
                token_type: t,
                line: self.logos_lexer.extras,
            })),
            Some(Err(err)) => match err {
                LexerError::InvalidToken => {
                    let span = self.logos_lexer.span();
                    let source = self.logos_lexer.source();

                    // Count newlines up to error position to get correct line number
                    let line = source[..span.start].chars().filter(|&c| c == '\n').count() + 1;

                    // Check if this is an unterminated string (starts with ")
                    if source[span.start..].starts_with('"') {
                        Some(Err(LexerError::UnterminatedString { line }))
                    } else {
                        Some(Err(LexerError::UnexpectedCharacter { line }))
                    }
                }
                e => Some(Err(e)),
            },
            None => None,
        }
    }
}
#[cfg(test)]
mod tests {
    use crate::lexer::{LexerError, Token, TokenType};

    use super::Lexer;
    use pretty_assertions::assert_eq;
    use test_case::test_case;

    #[test_case("501+3", &[TokenType::Number, TokenType::Plus, TokenType::Number], &["501", "+", "3"], &[]; "test1")]
    #[test_case("var test = 234.1", &[TokenType::Var, TokenType::Identifier, TokenType::Equal, TokenType::Number], &["var", "test", "=", "234.1"], &[]; "test 2")]
    #[test_case("var x = \"test\"", &[TokenType::Var, TokenType::Identifier, TokenType::Equal, TokenType::String], &["var", "x", "=", "\"test\""], &[0, 0, 0, 0]; "test3")]
    #[test_case("1\n2+3\n4", &[TokenType::Number, TokenType::Number, TokenType::Plus, TokenType::Number, TokenType::Number], &["1", "2", "+", "3", "4"], &[0, 1, 1, 1, 2]; "test_multiline")]
    #[test_case("print 1+2", &[TokenType::Print, TokenType::Number, TokenType::Plus, TokenType::Number], &["print", "1", "+", "2"], &[]; "test print" )]
    pub fn test1(
        str: &str,
        expected_token: &[TokenType],
        expected_values: &[&str],
        expected_lines: &[usize],
    ) -> anyhow::Result<()> {
        let mut l = Lexer::new(str);
        let mut tokens = Vec::new();
        let mut values = Vec::new();
        let mut lines = Vec::new();

        while let Some(Ok(t)) = l.next_token() {
            tokens.push(t.ty());
            values.push(t.slice(str));
            lines.push(t.line);
        }
        assert_eq!(expected_token, tokens.as_slice());

        if !expected_values.is_empty() {
            assert_eq!(expected_values, values.as_slice());
        }

        if !expected_lines.is_empty() {
            assert_eq!(expected_lines, lines.as_slice());
        }

        Ok(())
    }

    #[test]
    pub fn test_error() {
        let mut l = Lexer::new("+\"sdf");
        let result = [l.next_token(), l.next_token(), l.next_token()];
        assert_eq!(
            result,
            [
                Some(Ok(Token {
                    token_type: TokenType::Plus,
                    start: 0,
                    end: 1,
                    line: 0
                })),
                Some(Result::Err(LexerError::UnterminatedString { line: 1 })),
                None
            ]
        );
    }
}
