use std::{iter::Peekable, num::ParseFloatError};

use strum::EnumIter;
use thiserror::Error;

use crate::lexer::{Lexer, LexerError, PosIdx, Token, TokenType};

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Unexpected token {found:?} at {line}:{span:?}. Expected {expected:?}")]
    UnexpectedToken {
        expected: Vec<TokenType>,
        line: usize,
        span: Span,
        found: TokenType,
    },
    #[error("Expected token {expected:?} but reached eof")]
    UnexpectedEof { expected: Option<TokenType> },
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
    #[error("Expected expression")]
    ExpectedExpression,
    #[error("Expected unary operator. Found {found:?}")]
    UnexpectedUnaryOperator { found: TokenType },
    #[error("Expected boolean literal. Found {found:?}")]
    UnsatisfiedBoolLiteral { found: TokenType },
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
pub type AstStmt = Spanned<Stmt>;

pub enum AstLiteral {
    NumberLiteral(f64),
    NilLiteral,
    BoolLiteral(bool),
    StringLiteral(String),
}

pub enum LogicalExpression {
    Greater {
        left: Box<AstExpression>,
        right: Box<AstExpression>,
    },
    Less {
        left: Box<AstExpression>,
        right: Box<AstExpression>,
    },
    Equal {
        left: Box<AstExpression>,
        right: Box<AstExpression>,
    },
    NotEqual {
        left: Box<AstExpression>,
        right: Box<AstExpression>,
    },
    And {
        left: Box<AstExpression>,
        right: Box<AstExpression>,
    },
    Or {
        left: Box<AstExpression>,
        right: Box<AstExpression>,
    },
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
    UnaryNot {
        expr: Box<AstExpression>,
    },
    Grouping {
        expr: Box<AstExpression>,
    },
    Logical(LogicalExpression),
}

pub enum Stmt {
    Print(AstExpression),
    Expression(AstExpression),
}

pub fn parse(code: &str) -> Result<Vec<AstStmt>, ParseError> {
    let lexer = Lexer::new(code);
    let mut parser = Parser {
        lexer: lexer.peekable(),
        code: code,
        had_errors: false,
        panic_mode: false,
    };

    let mut result = Vec::new();
    while let Some(stmt) = parser.next_declaration() {
        result.push(stmt?);
    }
    Ok(result)
}

pub struct Parser<'source> {
    code: &'source str,
    lexer: Peekable<Lexer<'source>>,
    had_errors: bool,
    panic_mode: bool,
}

type ExprResult = Result<AstExpression, ParseError>;
type StmtResult = Result<AstStmt, ParseError>;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, EnumIter)]
enum Precedence {
    NONE = 0,
    LOWEST,
    LOGICAL,
    SUM,
    MULT,
    UNARY,
    HIGHEST,
}

impl Precedence {
    fn next(self) -> Self {
        if self == Precedence::HIGHEST {
            return self;
        } else {
            let value: u8 = unsafe { std::mem::transmute(self) };
            unsafe { std::mem::transmute(value + 1) }
        }
    }
}

impl Into<u8> for Precedence {
    fn into(self) -> u8 {
        unsafe { std::mem::transmute(self) }
    }
}

impl<'source> Parser<'source> {
    fn consume(&mut self, token_type: TokenType) -> Result<Token, ParseError> {
        match self.lexer.peek() {
            Some(Ok(t)) => {
                if t.ty() == token_type {
                    Ok(self.lexer.next().unwrap()?)
                } else {
                    Err(ParseError::UnexpectedToken {
                        expected: vec![token_type],
                        line: t.line(),
                        span: t.span(),
                        found: t.ty(),
                    })
                }
            }
            Some(Err(_)) => Ok(self.lexer.next().unwrap()?),
            None => Err(ParseError::UnexpectedEof {
                expected: Some(token_type),
            }),
        }
    }

    fn next_declaration(&mut self) -> Option<StmtResult> {
        match self.lexer.peek() {
            Some(_) => Some(self.declaration()),
            None => None,
        }
    }

    fn declaration(&mut self) -> StmtResult {
        self.statement()
    }

    fn statement(&mut self) -> StmtResult {
        if self.check_token(TokenType::PRINT)? {
            return self.print_statement();
        }
        todo!()
    }

    fn print_statement(&mut self) -> StmtResult {
        let print_token = self.consume(TokenType::PRINT)?;
        let expr = self.expression()?;
        let span = Span::new(print_token.start(), expr.end());
        self.consume(TokenType::SEMICOLON)?;
        Ok(Stmt::Print(expr).ast(span))
    }

    fn check_token(&mut self, token_type: TokenType) -> Result<bool, ParseError> {
        match self.lexer.peek() {
            Some(Ok(t)) => Ok(t.ty() == token_type),
            Some(Err(e)) => Err(ParseError::LexerError { source: e.clone() }),
            None => Ok(false),
        }
    }

    fn match_token(&mut self, token_type: TokenType) -> Result<Option<Token>, ParseError> {
        match self.lexer.peek() {
            Some(Ok(t)) if t.ty() == token_type => self.lexer.next().map_or(Ok(None), |r| {
                r.map(Some)
                    .map_err(|err| ParseError::LexerError { source: err })
            }),
            Some(Err(e)) => Err(ParseError::LexerError { source: e.clone() }),
            _ => Ok(None),
        }
    }

    fn consume_next(&mut self) -> Result<Token, ParseError> {
        if let Some(t) = self.lexer.next() {
            Ok(t?)
        } else {
            Err(ParseError::UnexpectedEof { expected: None })
        }
    }

    pub fn expression(&mut self) -> ExprResult {
        self.parse_by_precedence(Precedence::LOWEST)
    }

    fn parse_by_precedence(&mut self, precedence: Precedence) -> ExprResult {
        // 1 + 1 + 1
        if let Some(Ok(token)) = self.lexer.peek() {
            let (prefix_fn, _, _) = Self::precedence(token.ty());

            if let Some(prefix_fn) = prefix_fn {
                let mut result = prefix_fn(self)?;

                while let Some(Ok(next_token)) = self.lexer.peek() {
                    if let (_, Some(infix_fn), infix_prec) = Self::precedence(next_token.ty()) {
                        if precedence <= infix_prec {
                            result = infix_fn(self, result)?;
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }

                return Ok(result);
            }
        }
        return Err(ParseError::ExpectedExpression);
    }

    fn grouping(&mut self) -> ExprResult {
        let l = self.consume(TokenType::LEFT_PAREN)?;
        let result = self.expression()?;
        let r = self.consume(TokenType::RIGHT_PAREN)?;
        return Ok(Expression::Grouping {
            expr: Box::new(result),
        }
        .ast(Span::new(l.start(), r.end())));
    }

    fn unary(&mut self) -> ExprResult {
        let operator = self.consume_next()?;
        let expression = self.parse_by_precedence(Precedence::UNARY)?;
        let span = Span::new(operator.start(), expression.end());
        match operator.ty() {
            TokenType::MINUS => Ok(Expression::UnaryNegation {
                expr: Box::new(expression),
            }
            .ast(span)),
            TokenType::BANG => Ok(Expression::UnaryNot {
                expr: Box::new(expression),
            }
            .ast(span)),
            _ => Err(ParseError::UnexpectedUnaryOperator {
                found: operator.ty(),
            }),
        }
    }

    fn logical(&mut self, left: AstExpression) -> ExprResult {
        let operator = self.consume_next()?;
        let right = self.parse_by_precedence(Precedence::LOGICAL)?;
        let span = Span::new(left.start(), right.end());
        match operator.ty() {
            TokenType::BANG_EQUAL => Ok(Expression::Logical(LogicalExpression::NotEqual {
                left: Box::new(left),
                right: Box::new(right),
            })
            .ast(span)),
            TokenType::EQUAL_EQUAL => Ok(Expression::Logical(LogicalExpression::Equal {
                left: Box::new(left),
                right: Box::new(right),
            })
            .ast(span)),
            TokenType::AND => Ok(Expression::Logical(LogicalExpression::And {
                left: Box::new(left),
                right: Box::new(right),
            })
            .ast(span)),
            TokenType::OR => Ok(Expression::Logical(LogicalExpression::Or {
                left: Box::new(left),
                right: Box::new(right),
            })
            .ast(span)),
            TokenType::GREATER_EQUAL => todo!(),
            TokenType::LESS_EQUAL => todo!(),
            TokenType::GREATER => Ok(Expression::Logical(LogicalExpression::Greater {
                left: Box::new(left),
                right: Box::new(right),
            })
            .ast(span)),
            TokenType::LESS => Ok(Expression::Logical(LogicalExpression::Less {
                left: Box::new(left),
                right: Box::new(right),
            })
            .ast(span)),
            _ => todo!(),
        }
    }

    fn literal(&mut self) -> ExprResult {
        let next = self.consume_next()?;
        match next.ty() {
            TokenType::NUMBER => Ok(Expression::Literal(AstLiteral::NumberLiteral(
                next.slice(self.code).parse::<f64>()?,
            ))
            .ast(next.span())),
            TokenType::STRING => {
                Ok(Expression::Literal(AstLiteral::StringLiteral(String::from({
                    &self.code[next.start() + 1..next.end() - 1]
                })))
                .ast(next.span()))
            }
            _ => Err(ParseError::UnexpectedToken {
                expected: vec![TokenType::NUMBER, TokenType::STRING],
                line: next.line(),
                span: next.span(),
                found: next.ty(),
            }),
        }
    }

    fn nil(&mut self) -> ExprResult {
        let number = self.consume(TokenType::NIL)?;
        Ok(Expression::Literal(AstLiteral::NilLiteral).ast(number.span()))
    }

    fn bool(&mut self) -> ExprResult {
        let bool = self.consume_next()?;
        match bool.ty() {
            TokenType::TRUE => {
                Ok(Expression::Literal(AstLiteral::BoolLiteral(true)).ast(bool.span()))
            }
            TokenType::FALSE => {
                Ok(Expression::Literal(AstLiteral::BoolLiteral(false)).ast(bool.span()))
            }
            _ => Err(ParseError::UnsatisfiedBoolLiteral { found: bool.ty() }),
        }
    }

    fn binary(&mut self, left: AstExpression) -> ExprResult {
        let operator = self.lexer.next().unwrap()?;
        let (_, _, prec) = Self::precedence(operator.ty());
        let right = self.parse_by_precedence(prec.next())?;
        let span = Span::new(left.start(), right.end());
        match operator.ty() {
            TokenType::MINUS => Ok(Expression::Subtract {
                left: Box::new(left),
                right: Box::new(right),
            }
            .ast(span)),
            TokenType::PLUS => Ok(Expression::Add {
                left: Box::new(left),
                right: Box::new(right),
            }
            .ast(span)),
            TokenType::STAR => Ok(Expression::Multiply {
                left: Box::new(left),
                right: Box::new(right),
            }
            .ast(span)),
            TokenType::SLASH => Ok(Expression::Divide {
                left: Box::new(left),
                right: Box::new(right),
            }
            .ast(span)),
            _ => todo!(),
        }
    }

    fn precedence(
        token: TokenType,
    ) -> (
        Option<Box<dyn Fn(&mut Self) -> ExprResult>>,
        Option<Box<dyn Fn(&mut Self, AstExpression) -> ExprResult>>,
        Precedence,
    ) {
        match token {
            TokenType::LEFT_PAREN => (Some(Box::new(Self::grouping)), None, Precedence::LOWEST),
            TokenType::RIGHT_PAREN => (None, None, Precedence::NONE),
            TokenType::NIL => (Some(Box::new(Self::nil)), None, Precedence::NONE),
            TokenType::BANG => (Some(Box::new(Self::unary)), None, Precedence::UNARY),
            TokenType::TRUE | TokenType::FALSE => {
                (Some(Box::new(Self::bool)), None, Precedence::NONE)
            }
            TokenType::EQUAL_EQUAL
            | TokenType::BANG_EQUAL
            | TokenType::GREATER
            | TokenType::GREATER_EQUAL
            | TokenType::LESS
            | TokenType::LESS_EQUAL => (None, Some(Box::new(Self::logical)), Precedence::LOGICAL),
            TokenType::MINUS => (
                Some(Box::new(Self::unary)),
                Some(Box::new(Self::binary)),
                Precedence::SUM,
            ),
            TokenType::PLUS => (
                Some(Box::new(Self::unary)),
                Some(Box::new(Self::binary)),
                Precedence::SUM,
            ),
            TokenType::STAR => (
                Some(Box::new(Self::unary)),
                Some(Box::new(Self::binary)),
                Precedence::MULT,
            ),
            TokenType::SLASH => (
                Some(Box::new(Self::unary)),
                Some(Box::new(Self::binary)),
                Precedence::MULT,
            ),
            TokenType::NUMBER => (Some(Box::new(Self::literal)), None, Precedence::NONE),
            TokenType::STRING => (Some(Box::new(Self::literal)), None, Precedence::NONE),
            TokenType::PRINT => (None, None, Precedence::NONE),
            TokenType::SEMICOLON => (None, None, Precedence::NONE),
            _ => todo!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use crate::parser::Expression;

    use super::{parse, LogicalExpression};
    use super::{AstExpression, AstLiteral};

    #[test_case("4", 4.0; "test1")]
    #[test_case("4+1", 5.0; "test2")]
    #[test_case("2*(1+1)", 4.0; "test3")]
    #[test_case("(5+2)+10", 17.0; "test4")]
    #[test_case("10-5*2-5", -5.0; "test5")]
    #[test_case("1+1+1", 3.0; "test6")]
    #[test_case("2*(2*2+2*(2+3))", 28.0; "test7")]
    #[test_case("-1*2-2", -4.0; "test8")]
    fn test_calculator(code: &str, expected: f64) {
        //assert_eq!(eval(&parse(code).unwrap()), expected);
    }

    #[test_case("true", true; "bool1")]
    #[test_case("false", false; "bool2")]
    #[test_case("!false", true; "bool3")]
    #[test_case("!!false", false; "bool4")]
    #[test_case("!(5 > 4)", false; "bool5")]
    #[test_case("!(5 > 4*2)", true; "bool6")]
    #[test_case("!(5 < 4*2)", false; "bool7")]
    fn test_booleans(code: &str, expected: bool) {
        //assert_eq!(eval_bool(&parse(code).unwrap()), expected);
    }

    fn eval_bool(expression: &AstExpression) -> bool {
        match &expression.node {
            Expression::Literal(AstLiteral::BoolLiteral(x)) => *x,
            Expression::Literal(_) => unimplemented!(),
            Expression::Multiply { left: _, right: _ } => todo!(),
            Expression::Divide { left: _, right: _ } => todo!(),
            Expression::Add { left: _, right: _ } => todo!(),
            Expression::Subtract { left: _, right: _ } => todo!(),
            Expression::UnaryNegation { expr: _ } => todo!(),
            Expression::Grouping { expr } => eval_bool(&expr),
            Expression::UnaryNot { expr } => !eval_bool(expr),
            Expression::Logical(LogicalExpression::Greater { left, right }) => {
                eval(&left) > eval(&right)
            }
            Expression::Logical(LogicalExpression::Less { left, right }) => {
                eval(&left) < eval(&right)
            }
            Expression::Logical(LogicalExpression::And { left, right }) => {
                eval_bool(&left) && eval_bool(&right)
            }
            Expression::Logical(LogicalExpression::Or { left, right }) => {
                eval_bool(&left) || eval_bool(&right)
            }
            Expression::Logical(LogicalExpression::Equal { left, right }) => {
                eval(&left) == eval(&right)
            }
            Expression::Logical(LogicalExpression::NotEqual { left, right }) => {
                eval(&left) != eval(&right)
            }
        }
    }

    fn eval(expression: &AstExpression) -> f64 {
        match &expression.node {
            Expression::Literal(AstLiteral::NumberLiteral(x)) => *x,
            Expression::Literal(_) => unimplemented!(),
            Expression::Add { left, right } => {
                let x = eval(left.as_ref());
                let y = eval(right.as_ref());
                return x + y;
            }
            Expression::Multiply { left, right } => {
                let x = eval(left.as_ref());
                let y = eval(right.as_ref());
                return x * y;
            }
            Expression::Divide { left: _, right: _ } => todo!(),
            Expression::Subtract { left, right } => {
                let x = eval(left.as_ref());
                let y = eval(right.as_ref());
                return x - y;
            }
            Expression::UnaryNegation { expr } => -1.0 * eval(&expr),
            Expression::Grouping { expr } => eval(&expr),
            _ => unimplemented!(),
        }
    }
}
