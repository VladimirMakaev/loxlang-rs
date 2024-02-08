

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
    LEFT_PAREN,
    #[token(")")]
    RIGHT_PAREN,
    #[token("{")]
    LEFT_BRACE,
    #[token("}")]
    RIGHT_BRACE,
    #[token(",")]
    COMMA,
    #[token(".")]
    DOT,
    #[token("-")]
    MINUS,
    #[token("+")]
    PLUS,
    #[token(";")]
    SEMICOLON,
    #[token("\\")]
    SLASH,
    #[token("*")]
    STAR,
    #[token("!")]
    BANG,
    #[token("!=")]
    BANG_EQUAL,
    #[token("=")]
    EQUAL,
    #[token("==")]
    EQUAL_EQUAL,
    #[token(">")]
    GREATER,
    #[token(">=")]
    GREATER_EQUAL,
    #[token("<")]
    LESS,
    #[token("<=")]
    LESS_EQUAL,
    #[regex(r"[a-zA-Z][a-zA-Z0-9]*")]
    IDENTIFIER,
    #[regex("\"[^\"]*\"")]
    STRING,
    #[regex(r"\d+(\.\d+)?")]
    NUMBER,
    #[token("and")]
    AND,
    #[token("class")]
    CLASS,
    #[token("else")]
    ELSE,
    #[token("false")]
    FALSE,
    #[token("for")]
    FOR,
    #[token("fun")]
    FUN,
    #[token("if")]
    IF,
    #[token("nil")]
    NIL,
    #[token("or")]
    OR,
    #[token("print")]
    PRINT,
    #[token("return")]
    RETURN,
    #[token("super")]
    SUPER,
    #[token("this")]
    THIS,
    #[token("true")]
    TRUE,
    #[token("var")]
    VAR,
    #[token("while")]
    WHILE,
    #[regex(r"\n", newline_callback)]
    NEWLINE,
}

fn on_error(_lex: &mut logos::Lexer<TokenType>) -> logos::Skip {
    Skip
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

    pub fn line(&self) -> usize {
        self.line
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
    current: Option<Token>,
}

impl<'source> Iterator for Lexer<'source> {
    type Item = Result<Token, LexerError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next()
    }
}

impl<'source> Lexer<'source> {
    pub fn new(source: &'source str) -> Self {
        Self {
            logos_lexer: logos::Lexer::new(source),
            current: None,
        }
    }

    pub fn next(&mut self) -> Option<Result<Token, LexerError>> {
        match self.logos_lexer.next() {
            Some(Ok(t)) => Some(Ok(Token {
                start: self.logos_lexer.span().start,
                end: self.logos_lexer.span().end,
                token_type: t,
                line: self.logos_lexer.extras,
            })),
            Some(Err(err)) => match err {
                LexerError::InvalidToken => Some(Err(LexerError::InvalidTokenAt {
                    start: self.logos_lexer.span().start,
                    end: self.logos_lexer.span().end,
                })),
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

    #[test_case("501+3", &[TokenType::NUMBER, TokenType::PLUS, TokenType::NUMBER], &["501", "+", "3"], &[]; "test1")]
    #[test_case("var test = 234.1", &[TokenType::VAR, TokenType::IDENTIFIER, TokenType::EQUAL, TokenType::NUMBER], &["var", "test", "=", "234.1"], &[]; "test 2")]
    #[test_case("var x = \"test\"", &[TokenType::VAR, TokenType::IDENTIFIER, TokenType::EQUAL, TokenType::STRING], &["var", "x", "=", "\"test\""], &[0, 0, 0, 0]; "test3")]
    #[test_case("1\n2+3\n4", &[TokenType::NUMBER, TokenType::NUMBER, TokenType::PLUS, TokenType::NUMBER, TokenType::NUMBER], &["1", "2", "+", "3", "4"], &[0, 1, 1, 1, 2]; "test_multiline")]
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

        while let Some(Ok(t)) = l.next() {
            tokens.push(t.ty());
            values.push(t.slice(str));
            lines.push(t.line());
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
        let result = [l.next(), l.next(), l.next()];
        assert_eq!(
            result,
            [
                Some(Ok(Token {
                    token_type: TokenType::PLUS,
                    start: 0,
                    end: 1,
                    line: 0
                })),
                Some(Result::Err(LexerError::InvalidTokenAt { start: 1, end: 5 })),
                None
            ]
        );
    }
}
