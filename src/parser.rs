use std::{iter::Peekable, num::ParseFloatError};

use thiserror::Error;

use crate::lexer::{Lexer, LexerError, PosIdx, Token, TokenType};

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Unexpected token {found:?} at {line}:{span:?}. Expected {expected:?}")]
    UnexpectedToken {
        expected: TokenType,
        line: usize,
        span: Span,
        found: TokenType,
    },
    #[error("Expected token {expected:?} but reached eof")]
    UnexpectedEof { expected: TokenType },
    #[error("Number literal can't be converted to number")]
    InvalidNumberLiteral {
        #[from]
        source: ParseFloatError,
    },
    #[error("Lexer error: {source:?}")]
    LexerError {
        #[from]
        source: LexerError,
    },
}

pub trait Ast: Sized {
    fn ast(self, span: Span) -> Spanned<Self> {
        Spanned { node: self, span }
    }
}

impl<T: Sized> Ast for T {}

#[derive(Debug)]
pub struct Span {
    start: PosIdx,
    end: PosIdx,
}

impl Span {
    pub fn new(start: PosIdx, end: PosIdx) -> Self {
        Self { start, end }
    }
}

pub struct Spanned<TNode> {
    pub(crate) node: TNode,
    pub(crate) span: Span,
}

impl<T> Spanned<T> {
    pub fn start(&self) -> usize {
        self.span.start
    }

    pub fn end(&self) -> usize {
        self.span.end
    }

    pub fn node(&self) -> &T {
        &self.node
    }
}

pub type AstExpression = Spanned<Expression>;
pub type AstNumber = Spanned<f64>;
pub type AstStmt = Spanned<Stmt>;

pub enum AstLiteral {
    NumberLiteral(AstNumber),
    //StringLiteral(AstString),
    //TrueLiteral,
    //FalseLiteral,
}

pub enum Expression {
    Literal(AstLiteral),
    Multiply {
        left: Box<AstExpression>,
        right: Box<AstExpression>,
    },
    Divide {
        left: Box<AstExpression>,
        right: Box<AstExpression>,
    },
    Add {
        left: Box<AstExpression>,
        right: Box<AstExpression>,
    },
    Subtract {
        left: Box<AstExpression>,
        right: Box<AstExpression>,
    },
    UnaryNegation {
        expr: Box<AstExpression>,
    },
    Grouping {
        expr: Box<AstExpression>,
    },
}

pub enum Stmt {
    Expression(AstExpression),
}

pub struct Parser<'source> {
    lexer: Peekable<Lexer<'source>>,
    source: &'source str,
    had_errors: bool,
    panic_mode: bool,
}

impl<'source> Parser<'source> {
    pub fn expression(&mut self) -> Result<AstExpression, ParseError> {
        self.term()
    }

    fn check(&mut self, ty: TokenType) -> Result<bool, ParseError> {
        match self.lexer.peek() {
            Some(Ok(t)) => Ok(t.ty() == ty),
            _ => Ok(false),
        }
    }

    fn consume(&mut self, ty: TokenType) -> Result<Token, ParseError> {
        match self.lexer.next() {
            Some(Ok(t)) => Ok(t),
            Some(Err(error)) => Err(error.into()),
            None => Err(ParseError::UnexpectedEof { expected: ty }),
        }
    }

    pub fn term(&mut self) -> Result<AstExpression, ParseError> {
        let mut left = self.factor()?;
        while self.check(TokenType::PLUS)? {
            self.consume(TokenType::PLUS)?;
            let right = self.factor()?;
            let ast_span = Span::new(left.start(), right.end());
            left = Expression::Add {
                left: Box::new(left),
                right: Box::new(right),
            }
            .ast(ast_span);
        }
        Ok(left)
    }

    pub fn factor(&mut self) -> Result<AstExpression, ParseError> {
        self.unary()
    }

    fn grouping(&mut self) -> Result<AstExpression, ParseError> {
        let left_token = self.consume(TokenType::LEFT_PAREN)?;
        let expression = self.expression()?;
        let right_token = self.consume(TokenType::RIGHT_PAREN)?;
        Ok(Expression::Grouping {
            expr: Box::new(expression),
        }
        .ast(Span::new(left_token.start(), right_token.end())))
    }

    fn unary(&mut self) -> Result<AstExpression, ParseError> {
        if self.check(TokenType::MINUS)? {
            let token = self.consume(TokenType::MINUS)?;
            let expression = self.expression()?;
            let end = expression.end();
            return Ok(Expression::UnaryNegation {
                expr: Box::new(expression),
            }
            .ast(Span::new(token.start(), end)));
        }
        return self.number();
    }

    fn number(&mut self) -> Result<AstExpression, ParseError> {
        let token = self.consume(TokenType::NUMBER)?;
        let val = token.slice(self.source);
        let number_literal = AstLiteral::NumberLiteral(val.parse::<f64>()?.ast(token.span()));
        Ok(Expression::Literal(number_literal).ast(token.span()))
    }
}

pub fn parse(code: &str) -> Result<AstExpression, ParseError> {
    let lexer = Lexer::new(code);
    let mut parser = Parser {
        lexer: lexer.peekable(),
        source: code,
        had_errors: false,
        panic_mode: false,
    };
    parser.expression()
}
