use std::{iter::Peekable, num::ParseFloatError};

use strum::EnumIter;
use thiserror::Error;

use crate::lexer::{Lexer, LexerError, PosIdx, Token, TokenType};

#[derive(Error, Debug)]
#[error("{errors:?}")]
pub struct StmtError {
    pub errors: Vec<ParseError>,
}

impl From<ParseError> for StmtError {
    fn from(value: ParseError) -> Self {
        Self {
            errors: vec![value],
        }
    }
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Already a variable with this name in this scope.")]
    VariableAlreadyDeclared { span: Span },

    #[error("{expectation}")]
    UnexpectedToken { span: Span, expectation: String },

    #[error("Unexpected end of file.")]
    UnexpectedEof { last_position: usize },
    #[error("{message}")]
    UnexpectedEofWithMessage { last_position: usize, message: String },
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
    #[error("Expect expression.")]
    ExpectedExpression { span: Span },
    #[error("Expected unary operator.")]
    UnexpectedUnaryOperator { span: Span },
    #[error("Expected boolean literal.")]
    UnsatisfiedBoolLiteral { span: Span },
    #[error("Expect variable name")]
    InvalidVariableName { span: Span },
    #[error("Invalid assignment target.")]
    InvalidAssignmentTarget { span: Span },
    #[error("Can't return from top-level code.")]
    InvalidTopLevelReturn { span: Span },
    #[error("Can't have more than 255 arguments.")]
    MaxFunCallArguments { span: Span },
    #[error("Can't have more than 255 parameters.")]
    MaxFunDeclarationParameters { span: Span },
    #[error("Can't return a value from an initializer.")]
    InitializerReturnValue { span: Span },
    #[error("A class can't inherit from itself.")]
    InheritFromSelf { span: Span },
    #[error("Can't use 'super' outside of a class.")]
    SuperOutsideClass { span: Span },
    #[error("Can't use 'super' in a class with no superclass.")]
    SuperWithoutSuperclass { span: Span },
    #[error("Too many local variables in function.")]
    TooManyLocals { span: Span },
    #[error("Too many constants in one chunk.")]
    TooManyConstants { span: Span },
    #[error("Too many closure variables in function.")]
    TooManyUpvalues { span: Span },
    #[error("Loop body too large.")]
    LoopBodyTooLarge { span: Span },
    #[error("Can't read local variable in its own initializer.")]
    UninitializedLocal { span: Span },
}

impl Into<Vec<ParseError>> for ParseError {
    fn into(self) -> Vec<ParseError> {
        vec![self]
    }
}

pub trait Ast: Sized {
    fn ast(self, span: Span) -> Spanned<Self> {
        Spanned { node: self, span }
    }
}

impl<T: Sized> Ast for T {}

#[derive(Debug, Clone, Copy)]
pub struct Span {
    start: PosIdx,
    end: PosIdx,
}

impl Span {
    pub fn start(&self) -> PosIdx {
        self.start
    }

    pub fn new(start: PosIdx, end: PosIdx) -> Self {
        Self { start, end }
    }

    pub fn slice<'a, 'b>(&'a self, code: &'b str) -> &'b str {
        &code[self.start..self.end]
    }
}

#[derive(Debug)]
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
pub type AstIdent = Spanned<String>;
pub type AstReturn = Spanned<String>;

#[derive(Debug, strum::Display)]
pub enum AstLiteral {
    NumberLiteral(f64),
    NilLiteral,
    BoolLiteral(bool),
    StringLiteral(String),
}

#[derive(Debug, strum::Display)]
pub enum LogicalExpression {
    Greater {
        left: Box<AstExpression>,
        right: Box<AstExpression>,
    },
    GreaterEqual {
        left: Box<AstExpression>,
        right: Box<AstExpression>,
    },
    Less {
        left: Box<AstExpression>,
        right: Box<AstExpression>,
    },
    LessEqual {
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

#[derive(Debug, strum::Display)]
pub enum Expression {
    Assignment {
        lvalue: Box<AstExpression>,
        rvalue: Box<AstExpression>,
    },
    Identier(String),
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
    Call {
        calee: Box<AstExpression>,
        arguments: Vec<AstExpression>,
    },
    GetProperty {
        object: Box<AstExpression>,
        name: String,
    },
    SetProperty {
        object: Box<AstExpression>,
        name: String,
        value: Box<AstExpression>,
    },
    This,
    Super { method: String },  // super.method - stores the method name
}

#[derive(Debug, strum::Display, strum::EnumTryAs)]
pub enum Stmt {
    Print(AstExpression),
    Declarations(StmtDeclaration),
    If(IfStmt),
    While(WhileStmt),
    For(ForStmt),
    Block(BlockStmt),
    Return(AstReturn, Option<AstExpression>),
    Expression(AstExpression),
}

#[derive(Debug)]
pub struct BlockStmt(pub(crate) Vec<AstStmt>);

#[derive(Debug)]
pub struct IfStmt {
    pub(crate) condition: AstExpression,
    pub(crate) then_block: Box<AstStmt>,
    pub(crate) else_block: Option<Box<AstStmt>>,
}

#[derive(Debug)]
pub struct WhileStmt {
    pub(crate) condition: AstExpression,
    pub(crate) loop_block: Box<AstStmt>,
}

#[derive(Debug)]
pub struct ForStmt {
    pub(crate) condition: Option<Box<AstExpression>>,
    pub(crate) initializer: Option<Box<AstStmt>>,
    pub(crate) increment: Option<Box<AstStmt>>,
    pub(crate) block: Box<AstStmt>,
}

#[derive(Debug, strum::Display)]
pub enum StmtDeclaration {
    Variable {
        ident: AstIdent,
        expr: Option<AstExpression>,
    },
    Function(FunDeclaration),
    Class(ClassDeclaration),
}

#[derive(Debug)]
pub struct FunDeclaration {
    pub(crate) name: AstIdent,
    pub(crate) params: Vec<AstIdent>,
    pub(crate) body: Box<AstStmt>,
}

#[derive(Debug)]
pub struct ClassDeclaration {
    pub(crate) name: AstIdent,
    pub(crate) superclass: Option<AstIdent>,
    pub(crate) methods: Vec<MethodDeclaration>,
}

#[derive(Debug)]
pub struct MethodDeclaration {
    pub(crate) name: AstIdent,
    pub(crate) params: Vec<AstIdent>,
    pub(crate) body: Box<AstStmt>,
}

pub struct Parser<'source> {
    code: &'source str,
    lexer: Peekable<Lexer<'source>>,
    errors: Vec<ParseError>,
    last_position: usize,
}

type ExprResult = Result<AstExpression, ParseError>;
type StmtResult = Result<AstStmt, StmtError>;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, EnumIter)]
enum Precedence {
    NONE = 0,
    LOWEST,
    ASSIGNMENT,
    OR,
    AND,
    COMPARISON,
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
    pub fn had_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn into_errors(self) -> Vec<ParseError> {
        self.errors
    }

    pub fn new(code: &'source str) -> Self {
        Self {
            code,
            lexer: Lexer::new(code).peekable(),
            errors: Default::default(),
            last_position: 0,
        }
    }

    pub fn parse(&mut self) -> Vec<AstStmt> {
        let mut result = Vec::new();
        while let Some(next_stmt_result) = self.next_declaration() {
            match next_stmt_result {
                Ok(stmt) => result.push(stmt),
                Err(e) => {
                    self.errors.extend(e.errors);
                    self.syncronize();
                }
            }
        }
        result
    }

    fn syncronize(&mut self) {
        loop {
            match self.lexer.next() {
                Some(Ok(t)) if t.ty() == TokenType::Semicolon => break,
                Some(Ok(t)) if t.ty() == TokenType::RightBrace => break,
                Some(Ok(_)) => {
                    continue;
                }
                Some(Err(_)) => {
                    continue;
                }
                None => {
                    break;
                }
            }
        }
    }

    fn consume_or_else(
        &mut self,
        token_type: TokenType,
        f: impl FnOnce(&Token) -> ParseError,
    ) -> Result<Token, ParseError> {
        match self.lexer.peek() {
            Some(Ok(t)) => {
                if t.ty() == token_type {
                    let token = self.lexer.next().unwrap()?;
                    self.last_position = token.end();
                    Ok(token)
                } else {
                    Err(f(t))
                }
            }
            Some(Err(_)) => {
                let token = self.lexer.next().unwrap()?;
                self.last_position = token.end();
                Ok(token)
            }
            None => Err(ParseError::UnexpectedEof { last_position: self.last_position }),
        }
    }

    fn consume(&mut self, token_type: TokenType) -> Result<Token, ParseError> {
        self.consume_or_else(token_type, |t| ParseError::UnexpectedToken {
            span: t.span(),
            expectation: format!("Expected '{:?}' got '{}'", token_type, t.slice(self.code)),
        })
    }

    fn next_declaration(&mut self) -> Option<StmtResult> {
        match self.lexer.peek() {
            Some(_) => Some(self.declaration()),
            None => None,
        }
    }

    fn declaration(&mut self) -> StmtResult {
        if self.check_token(TokenType::VAR)? {
            return self.var_declaration();
        }

        if self.check_token(TokenType::FUN)? {
            return self.fun_declaration();
        }

        if self.check_token(TokenType::CLASS)? {
            return self.class_declaration();
        }

        self.statement()
    }

    fn fun_declaration(&mut self) -> StmtResult {
        let fun_token = self.consume(TokenType::FUN)?;
        let name = self.consume(TokenType::IDENTIFIER)?;
        let name = name.slice(self.code).to_owned().ast(name.span());
        let mut params = Vec::new();
        self.consume(TokenType::LeftParen)?;

        if !self.check_token(TokenType::RightParen)? {
            loop {
                if let Spanned {
                    node: Expression::Identier(x),
                    span,
                } = self.identifier_contant()?
                {
                    if params.len() == u8::MAX as usize {
                        return Err(ParseError::MaxFunDeclarationParameters { span: span }.into());
                    }

                    // Check for duplicate parameter name
                    if params.iter().any(|p: &AstIdent| p.node == x) {
                        return Err(ParseError::VariableAlreadyDeclared { span }.into());
                    }

                    params.push(x.ast(span));
                } else {
                    unreachable!()
                }

                if !self.check_token(TokenType::RightParen)? {
                    self.consume_or_else(TokenType::Comma, |t| ParseError::UnexpectedToken {
                        span: t.span(),
                        expectation: "Expect ')' after parameters.".to_owned(),
                    })?;
                } else {
                    break;
                }
            }
        }
        self.consume(TokenType::RightParen)?;

        if !self.check_token(TokenType::LeftBrace)? {
            return Err(ParseError::UnexpectedToken {
                span: self.consume_next()?.span(),
                expectation: "Expect '{' before function body.".to_owned(),
            }
            .into());
        }

        let block = self.block_statement()?;

        let span = Span::new(fun_token.start(), block.end());

        Ok(
            Stmt::Declarations(StmtDeclaration::Function(FunDeclaration {
                name,
                params: params,
                body: Box::new(block),
            }))
            .ast(span),
        )
    }

    fn class_declaration(&mut self) -> StmtResult {
        let class_token = self.consume(TokenType::CLASS)?;
        let name = self.consume(TokenType::IDENTIFIER)?;
        let name = name.slice(self.code).to_owned().ast(name.span());

        // Parse optional superclass
        let superclass = if self.check_token(TokenType::LESS)? {
            self.consume(TokenType::LESS)?;
            let super_name = self.consume_or_else(TokenType::IDENTIFIER, |t| {
                ParseError::UnexpectedToken {
                    span: t.span(),
                    expectation: "Expect superclass name.".to_owned(),
                }
            })?;
            let super_ident = super_name.slice(self.code).to_owned().ast(super_name.span());

            // Check for self-inheritance
            if super_ident.node == name.node {
                return Err(ParseError::InheritFromSelf { span: super_name.span() }.into());
            }

            Some(super_ident)
        } else {
            None
        };

        self.consume(TokenType::LeftBrace)?;

        let mut methods = Vec::new();
        while !self.check_token(TokenType::RightBrace)? {
            methods.push(self.method_declaration()?);
        }

        let right_brace = self.consume(TokenType::RightBrace)?;
        let span = Span::new(class_token.start(), right_brace.end());

        Ok(
            Stmt::Declarations(StmtDeclaration::Class(ClassDeclaration {
                name,
                superclass,
                methods,
            }))
            .ast(span),
        )
    }

    fn method_declaration(&mut self) -> Result<MethodDeclaration, StmtError> {
        let name_token = self.consume(TokenType::IDENTIFIER)?;
        let name = name_token.slice(self.code).to_owned().ast(name_token.span());

        self.consume(TokenType::LeftParen)?;

        let mut params = Vec::new();
        if !self.check_token(TokenType::RightParen)? {
            loop {
                if let Spanned {
                    node: Expression::Identier(x),
                    span,
                } = self.identifier_contant()?
                {
                    if params.len() == u8::MAX as usize {
                        return Err(ParseError::MaxFunDeclarationParameters { span: span }.into());
                    }

                    // Check for duplicate parameter name
                    if params.iter().any(|p: &AstIdent| p.node == x) {
                        return Err(ParseError::VariableAlreadyDeclared { span }.into());
                    }

                    params.push(x.ast(span));
                } else {
                    unreachable!()
                }

                if !self.check_token(TokenType::RightParen)? {
                    self.consume_or_else(TokenType::Comma, |t| ParseError::UnexpectedToken {
                        span: t.span(),
                        expectation: "Expect ')' after parameters.".to_owned(),
                    })?;
                } else {
                    break;
                }
            }
        }
        self.consume(TokenType::RightParen)?;

        if !self.check_token(TokenType::LeftBrace)? {
            return Err(ParseError::UnexpectedToken {
                span: self.consume_next()?.span(),
                expectation: "Expect '{' before method body.".to_owned(),
            }
            .into());
        }

        let body = self.block_statement()?;

        Ok(MethodDeclaration {
            name,
            params,
            body: Box::new(body),
        })
    }

    fn var_declaration(&mut self) -> StmtResult {
        let decl_token = self.consume(TokenType::VAR)?;
        // Use consume_or_else to produce "Expect variable name." error for reserved words
        let ident_token = self.consume_or_else(TokenType::IDENTIFIER, |t| ParseError::UnexpectedToken {
            span: t.span(),
            expectation: "Expect variable name.".to_owned(),
        })?;
        let var_ident = ident_token.slice(self.code).to_owned().ast(ident_token.span());
        let mut expr = None;
        if let Some(_) = self.match_token(TokenType::EQUAL)? {
            expr = Some(self.expression()?);
        }

        self.consume(TokenType::Semicolon)?;

        let span = Span::new(
            decl_token.start(),
            expr.as_ref().map(|e| e.end()).unwrap_or(var_ident.end()),
        );

        Ok(Stmt::Declarations(StmtDeclaration::Variable {
            ident: var_ident,
            expr: expr,
        })
        .ast(span))
    }

    fn identifier_contant(&mut self) -> ExprResult {
        let ident = self.consume(TokenType::IDENTIFIER)?;
        Ok(Expression::Identier(ident.slice(self.code).into()).ast(ident.span()))
    }

    fn statement(&mut self) -> StmtResult {
        if self.check_token(TokenType::PRINT)? {
            return self.print_statement();
        }

        if self.check_token(TokenType::RETURN)? {
            return self.return_statement();
        }

        if self.check_token(TokenType::IF)? {
            return self.if_statement();
        }

        if self.check_token(TokenType::WHILE)? {
            return self.while_statement();
        }

        if self.check_token(TokenType::FOR)? {
            return self.for_statement();
        }

        if self.check_token(TokenType::LeftBrace)? {
            return self.block_statement();
        }

        let result = self.expression()?;
        let t = self.consume(TokenType::Semicolon)?;
        let span = Span::new(result.start(), t.end());
        Ok(Stmt::Expression(result).ast(span))
    }

    fn print_statement(&mut self) -> StmtResult {
        let print_token = self.consume(TokenType::PRINT)?;
        let expr = self.expression()?;
        let span = Span::new(print_token.start(), expr.end());
        self.consume(TokenType::Semicolon)?;
        Ok(Stmt::Print(expr).ast(span))
    }

    fn return_statement(&mut self) -> StmtResult {
        let return_token = self.consume(TokenType::RETURN)?;
        if let Some(semi) = self.match_token(TokenType::Semicolon)? {
            let span = Span::new(return_token.start(), semi.end());
            return Ok(Stmt::Return(
                return_token
                    .slice(self.code)
                    .to_owned()
                    .ast(return_token.span()),
                None,
            )
            .ast(span));
        }
        let expr = self.expression()?;
        let span = Span::new(return_token.start(), expr.end());
        self.consume(TokenType::Semicolon)?;
        Ok(Stmt::Return(
            return_token
                .slice(self.code)
                .to_owned()
                .ast(return_token.span()),
            Some(expr),
        )
        .ast(span))
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

    // fn match_tokens<const N: usize>(
    //     &mut self,
    //     token_types: &[TokenType; N],
    // ) -> Result<Option<Token>, ParseError> {
    //     match self.lexer.peek() {
    //         Some(Ok(t)) if token_types.iter().find(|x| t.ty().eq(x)).is_some() => {
    //             self.lexer.next().map_or(Ok(None), |r| {
    //                 r.map(Some)
    //                     .map_err(|err| ParseError::LexerError { source: err })
    //             })
    //         }
    //         Some(Err(e)) => Err(ParseError::LexerError { source: e.clone() }),
    //         _ => Ok(None),
    //     }
    // }

    fn consume_next(&mut self) -> Result<Token, ParseError> {
        if let Some(t) = self.lexer.next() {
            let token = t?;
            self.last_position = token.end();
            Ok(token)
        } else {
            Err(ParseError::UnexpectedEof { last_position: self.last_position })
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
            } else {
                return Err(ParseError::ExpectedExpression { span: token.span() });
            }
        } else {
            return Err(ParseError::UnexpectedEof { last_position: self.last_position });
        }
    }

    fn grouping(&mut self) -> ExprResult {
        let l = self.consume(TokenType::LeftParen)?;
        let result = self.expression()?;
        let r = self.consume(TokenType::RightParen)?;
        return Ok(Expression::Grouping {
            expr: Box::new(result),
        }
        .ast(Span::new(l.start(), r.end())));
    }

    fn call(&mut self, left: AstExpression) -> ExprResult {
        self.consume(TokenType::LeftParen)?;

        match self.match_token(TokenType::RightParen)? {
            Some(t) => {
                let span = Span::new(left.start(), t.end());
                return Ok(Expression::Call {
                    calee: Box::new(left),
                    arguments: Default::default(),
                }
                .ast(span));
            }
            None => {
                let mut params = Vec::new();
                loop {
                    if params.len() as u8 == u8::MAX {
                        return Err(ParseError::MaxFunCallArguments {
                            span: self.consume_next()?.span(),
                        });
                    }
                    params.push(self.expression()?);
                    if self.match_token(TokenType::Comma)?.is_some() {
                        continue;
                    }
                    let t = self.consume(TokenType::RightParen)?;
                    let span = Span::new(left.start(), t.end());
                    return Ok(Expression::Call {
                        calee: Box::new(left),
                        arguments: params,
                    }
                    .ast(span));
                }
            }
        }
    }

    fn dot(&mut self, left: AstExpression) -> ExprResult {
        self.consume(TokenType::Dot)?;
        // Check for EOF before trying to consume identifier
        if self.lexer.peek().is_none() {
            return Err(ParseError::UnexpectedEofWithMessage {
                last_position: self.last_position,
                message: "Expect property name after '.'.".to_owned(),
            });
        }
        let name_token = self.consume_or_else(TokenType::IDENTIFIER, |t| {
            ParseError::UnexpectedToken {
                span: t.span(),
                expectation: "Expect property name after '.'.".to_owned(),
            }
        })?;
        let name = name_token.span().slice(self.code).to_string();
        let span = Span::new(left.start(), name_token.end());
        Ok(Expression::GetProperty {
            object: Box::new(left),
            name,
        }
        .ast(span))
    }

    fn unary(&mut self) -> ExprResult {
        let operator = self.consume_next()?;
        let expression = self.parse_by_precedence(Precedence::UNARY)?;
        let span = Span::new(operator.start(), expression.end());
        match operator.ty() {
            TokenType::Minus => Ok(Expression::UnaryNegation {
                expr: Box::new(expression),
            }
            .ast(span)),
            TokenType::Bang => Ok(Expression::UnaryNot {
                expr: Box::new(expression),
            }
            .ast(span)),
            _ => Err(ParseError::UnexpectedUnaryOperator {
                span: operator.span(),
            }),
        }
    }

    fn logical_and(&mut self, left: AstExpression) -> ExprResult {
        let _ = self.consume(TokenType::AND)?;
        let right = self.parse_by_precedence(Precedence::AND)?;
        let span = Span::new(left.start(), right.end());
        Ok(Expression::Logical(LogicalExpression::And {
            left: Box::new(left),
            right: Box::new(right),
        })
        .ast(span))
    }

    fn logical_or(&mut self, left: AstExpression) -> ExprResult {
        let _ = self.consume(TokenType::OR)?;
        let right = self.parse_by_precedence(Precedence::OR)?;
        let span = Span::new(left.start(), right.end());
        Ok(Expression::Logical(LogicalExpression::Or {
            left: Box::new(left),
            right: Box::new(right),
        })
        .ast(span))
    }

    fn logical(&mut self, left: AstExpression) -> ExprResult {
        let operator = self.consume_next()?;
        let right = self.parse_by_precedence(Precedence::COMPARISON)?;
        let span = Span::new(left.start(), right.end());
        match operator.ty() {
            TokenType::BantEqual => Ok(Expression::Logical(LogicalExpression::NotEqual {
                left: Box::new(left),
                right: Box::new(right),
            })
            .ast(span)),
            TokenType::EqualEqual => Ok(Expression::Logical(LogicalExpression::Equal {
                left: Box::new(left),
                right: Box::new(right),
            })
            .ast(span)),
            TokenType::OR => Ok(Expression::Logical(LogicalExpression::Or {
                left: Box::new(left),
                right: Box::new(right),
            })
            .ast(span)),
            TokenType::GreaterEqual => Ok(Expression::Logical(LogicalExpression::GreaterEqual {
                left: Box::new(left),
                right: Box::new(right),
            })
            .ast(span)),
            TokenType::LessEqual => Ok(Expression::Logical(LogicalExpression::LessEqual {
                left: Box::new(left),
                right: Box::new(right),
            })
            .ast(span)),
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
                span: next.span(),
                expectation: format!(
                    "Expected either {:?} or {:?}. Got {}",
                    TokenType::NUMBER,
                    TokenType::Star,
                    next.slice(self.code)
                ),
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
            _ => Err(ParseError::UnsatisfiedBoolLiteral { span: bool.span() }),
        }
    }

    fn binary(&mut self, left: AstExpression) -> ExprResult {
        let operator = self.lexer.next().unwrap()?;
        let (_, _, prec) = Self::precedence(operator.ty());
        let right = self.parse_by_precedence(prec.next())?;
        let span = Span::new(left.start(), right.end());
        match operator.ty() {
            TokenType::Minus => Ok(Expression::Subtract {
                left: Box::new(left),
                right: Box::new(right),
            }
            .ast(span)),
            TokenType::PLUS => Ok(Expression::Add {
                left: Box::new(left),
                right: Box::new(right),
            }
            .ast(span)),
            TokenType::Star => Ok(Expression::Multiply {
                left: Box::new(left),
                right: Box::new(right),
            }
            .ast(span)),
            TokenType::Slash => Ok(Expression::Divide {
                left: Box::new(left),
                right: Box::new(right),
            }
            .ast(span)),
            _ => todo!(),
        }
    }

    fn assignment(&mut self, left: AstExpression) -> ExprResult {
        let equal_span = self
            .lexer
            .peek()
            .and_then(|r| r.as_ref().ok())
            .map(|t| t.span())
            .unwrap_or(left.span);

        self.consume(TokenType::EQUAL)?;
        let rvalue = self.expression()?;

        match left {
            Spanned {
                node: Expression::Identier(name),
                span,
            } => {
                let lvalue = Expression::Identier(name).ast(span);
                let result_span = Span::new(lvalue.start(), rvalue.end());
                Ok(Expression::Assignment {
                    lvalue: Box::new(lvalue),
                    rvalue: Box::new(rvalue),
                }
                .ast(result_span))
            }
            Spanned {
                node: Expression::GetProperty { object, name },
                span: _,
            } => {
                let result_span = Span::new(object.start(), rvalue.end());
                Ok(Expression::SetProperty {
                    object,
                    name,
                    value: Box::new(rvalue),
                }
                .ast(result_span))
            }
            _ => Err(ParseError::InvalidAssignmentTarget { span: equal_span }),
        }
    }

    fn block_statement(&mut self) -> StmtResult {
        let left = self.consume(TokenType::LeftBrace)?;
        let mut statements = Vec::new();
        if !self.check_token(TokenType::RightBrace)? {
            while let Some(next) = self.next_declaration() {
                match next {
                    Ok(s) => statements.push(s),
                    Err(e) => {
                        self.syncronize();
                        return Err(e);
                    }
                }

                if self.check_token(TokenType::RightBrace)? {
                    break;
                }
            }
        }
        let right = self.consume(TokenType::RightBrace)?;
        Ok(Stmt::Block(BlockStmt(statements)).ast(Span::new(left.start(), right.end())))
    }

    fn while_statement(&mut self) -> StmtResult {
        let while_token = self.consume(TokenType::WHILE)?;
        self.consume(TokenType::LeftParen)?;
        let condition = self.expression()?;
        self.consume(TokenType::RightParen)?;
        let loop_block = self.statement()?;
        let span = Span::new(while_token.start(), loop_block.end());
        Ok(Stmt::While(WhileStmt {
            condition,
            loop_block: Box::new(loop_block),
        })
        .ast(span))
    }

    fn for_statement(&mut self) -> StmtResult {
        let for_token = self.consume(TokenType::FOR)?;

        self.consume(TokenType::LeftParen)?;

        let mut collected_errors: Vec<ParseError> = Vec::new();

        // Parse initializer
        let init_stmt = if self.check_token(TokenType::VAR)? {
            Some(Box::new(self.var_declaration()?))
        } else {
            if self.check_token(TokenType::Semicolon)? {
                self.consume(TokenType::Semicolon)?;
                None
            } else {
                match self.expression() {
                    Ok(expression) => {
                        self.consume_or_else(TokenType::Semicolon, |t| ParseError::UnexpectedToken {
                            span: t.span(),
                            expectation: "Expect ';' after expression.".to_owned(),
                        })?;
                        let span = Span::new(expression.start(), expression.end());
                        Some(Box::new(Stmt::Expression(expression).ast(span)))
                    }
                    Err(err) => {
                        collected_errors.push(err);
                        // Skip until we find right paren (not semicolon) to match clox behavior
                        while !self.check_token(TokenType::RightParen)? {
                            if self.lexer.next().is_none() {
                                break;
                            }
                        }
                        // Report "Expect ';' after expression" at the )
                        if let Some(Ok(t)) = self.lexer.peek() {
                            collected_errors.push(ParseError::UnexpectedToken {
                                span: t.span(),
                                expectation: "Expect ';' after expression.".to_owned(),
                            });
                        }
                        None
                    }
                }
            }
        };

        // Parse condition - only if no errors yet from initializer
        let condition = if collected_errors.is_empty() {
            if let Some(_) = self.match_token(TokenType::Semicolon)? {
                None
            } else if self.check_token(TokenType::RightParen)? {
                // No condition and no semicolon - already at end
                None
            } else {
                match self.expression() {
                    Ok(expression) => {
                        self.consume_or_else(TokenType::Semicolon, |t| ParseError::UnexpectedToken {
                            span: t.span(),
                            expectation: "Expect ';' after expression.".to_owned(),
                        })?;
                        Some(Box::new(expression))
                    }
                    Err(err) => {
                        collected_errors.push(err);
                        // Skip until we find right paren
                        while !self.check_token(TokenType::RightParen)? {
                            if self.lexer.next().is_none() {
                                break;
                            }
                        }
                        // Report "Expect ';' after expression" at the )
                        if let Some(Ok(t)) = self.lexer.peek() {
                            collected_errors.push(ParseError::UnexpectedToken {
                                span: t.span(),
                                expectation: "Expect ';' after expression.".to_owned(),
                            });
                        }
                        None
                    }
                }
            }
        } else {
            None
        };

        // If we have collected errors, return them now
        if !collected_errors.is_empty() {
            return Err(StmtError { errors: collected_errors }.into());
        }

        let increment_stmt = if self.check_token(TokenType::RightParen)? {
            None
        } else {
            let expression = self.expression()?;
            let span = Span::new(expression.start(), expression.end());
            Some(Box::new(Stmt::Expression(expression).ast(span)))
        };

        self.consume(TokenType::RightParen)?;

        let block = self.statement()?;

        let span = Span::new(for_token.start(), block.end());
        Ok(Stmt::For(ForStmt {
            condition,
            initializer: init_stmt,
            increment: increment_stmt,
            block: Box::new(block),
        })
        .ast(span))
    }

    fn if_statement(&mut self) -> StmtResult {
        let if_token = self.consume(TokenType::IF)?;
        self.consume(TokenType::LeftParen)?;
        let condition = self.expression()?;
        self.consume(TokenType::RightParen)?;

        let then_block = self.statement()?;
        let mut else_block = Option::None;

        if self.check_token(TokenType::ELSE)? {
            self.consume(TokenType::ELSE)?;
            else_block = Some(Box::new(self.statement()?));
        }

        let span = Span::new(
            if_token.start(),
            else_block.as_ref().map_or(then_block.end(), |b| b.end()),
        );

        Ok(Stmt::If(IfStmt {
            condition: condition,
            then_block: Box::new(then_block),
            else_block,
        })
        .ast(span))
    }

    fn this_expression(&mut self) -> ExprResult {
        let token = self.consume(TokenType::THIS)?;
        Ok(Expression::This.ast(token.span()))
    }

    fn super_expression(&mut self) -> ExprResult {
        let super_token = self.consume(TokenType::SUPER)?;

        // Require '.' after super
        self.consume_or_else(TokenType::Dot, |t| ParseError::UnexpectedToken {
            span: t.span(),
            expectation: "Expect '.' after 'super'.".to_owned(),
        })?;

        // Require method name
        let method_token = self.consume_or_else(TokenType::IDENTIFIER, |t| {
            ParseError::UnexpectedToken {
                span: t.span(),
                expectation: "Expect superclass method name.".to_owned(),
            }
        })?;

        let method = method_token.slice(self.code).to_string();
        let span = Span::new(super_token.start(), method_token.end());

        Ok(Expression::Super { method }.ast(span))
    }

    fn precedence(
        token: TokenType,
    ) -> (
        Option<Box<dyn Fn(&mut Self) -> ExprResult>>,
        Option<Box<dyn Fn(&mut Self, AstExpression) -> ExprResult>>,
        Precedence,
    ) {
        match token {
            TokenType::LeftParen => (
                Some(Box::new(Self::grouping)),
                Some(Box::new(Self::call)),
                Precedence::HIGHEST,
            ),
            TokenType::RightParen => (None, None, Precedence::NONE),
            TokenType::NIL => (Some(Box::new(Self::nil)), None, Precedence::NONE),
            TokenType::Bang => (Some(Box::new(Self::unary)), None, Precedence::UNARY),
            TokenType::TRUE | TokenType::FALSE => {
                (Some(Box::new(Self::bool)), None, Precedence::NONE)
            }
            TokenType::EqualEqual
            | TokenType::BantEqual
            | TokenType::GREATER
            | TokenType::GreaterEqual
            | TokenType::LESS
            | TokenType::LessEqual => (None, Some(Box::new(Self::logical)), Precedence::COMPARISON),
            TokenType::Minus => (
                Some(Box::new(Self::unary)),
                Some(Box::new(Self::binary)),
                Precedence::SUM,
            ),
            TokenType::PLUS => (
                Some(Box::new(Self::unary)),
                Some(Box::new(Self::binary)),
                Precedence::SUM,
            ),
            TokenType::Star => (
                Some(Box::new(Self::unary)),
                Some(Box::new(Self::binary)),
                Precedence::MULT,
            ),
            TokenType::Slash => (
                Some(Box::new(Self::unary)),
                Some(Box::new(Self::binary)),
                Precedence::MULT,
            ),
            TokenType::NUMBER => (Some(Box::new(Self::literal)), None, Precedence::NONE),
            TokenType::STRING => (Some(Box::new(Self::literal)), None, Precedence::NONE),
            TokenType::PRINT => (None, None, Precedence::NONE),
            TokenType::Semicolon => (None, None, Precedence::NONE),
            TokenType::VAR => (None, None, Precedence::NONE),
            TokenType::IDENTIFIER => (
                Some(Box::new(Self::identifier_contant)),
                None,
                Precedence::NONE,
            ),
            TokenType::EQUAL => (
                None,
                Some(Box::new(Self::assignment)),
                Precedence::ASSIGNMENT,
            ),
            TokenType::LeftBrace | TokenType::RightBrace => (None, None, Precedence::NONE),
            TokenType::IF | TokenType::ELSE => (None, None, Precedence::NONE),
            TokenType::WHILE | TokenType::FOR => (None, None, Precedence::NONE),
            TokenType::AND => (None, Some(Box::new(Self::logical_and)), Precedence::AND),
            TokenType::OR => (None, Some(Box::new(Self::logical_or)), Precedence::AND),
            TokenType::FUN | TokenType::Comma | TokenType::RETURN => (None, None, Precedence::NONE),
            TokenType::Dot => (None, Some(Box::new(Self::dot)), Precedence::HIGHEST),
            TokenType::CLASS => (None, None, Precedence::NONE),
            TokenType::THIS => (Some(Box::new(Self::this_expression)), None, Precedence::NONE),
            TokenType::SUPER => (Some(Box::new(Self::super_expression)), None, Precedence::NONE),
            _ => panic!("Not implemented token: {:?}", token),
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::bail;
    use test_case::test_case;

    use crate::parser::Expression;

    use super::{AstExpression, AstLiteral};
    use super::{LogicalExpression, Parser};

    #[test_case("4", 4.0; "test1")]
    #[test_case("4+1", 5.0; "test2")]
    #[test_case("2*(1+1)", 4.0; "test3")]
    #[test_case("(5+2)+10", 17.0; "test4")]
    #[test_case("10-5*2-5", -5.0; "test5")]
    #[test_case("1+1+1", 3.0; "test6")]
    #[test_case("2*(2*2+2*(2+3))", 28.0; "test7")]
    #[test_case("-1*2-2", -4.0; "test8")]
    fn test_calculator(code: &str, expected: f64) {
        assert_eq!(eval(&parse_expression(code).unwrap()), expected);
    }

    #[test_case("true", true; "bool1")]
    #[test_case("false", false; "bool2")]
    #[test_case("!false", true; "bool3")]
    #[test_case("!!false", false; "bool4")]
    #[test_case("!(5 > 4)", false; "bool5")]
    #[test_case("!(5 > 4*2)", true; "bool6")]
    #[test_case("!(5 < 4*2)", false; "bool7")]
    fn test_booleans(code: &str, expected: bool) {
        assert_eq!(eval_bool(&parse_expression(code).unwrap()), expected);
    }

    fn parse_expression(code: &str) -> anyhow::Result<AstExpression> {
        let with_semi = if code.ends_with(";") {
            code.into()
        } else {
            String::from(code) + ";"
        };
        let mut parser = Parser::new(&with_semi);
        let mut statements = parser.parse();
        let ast = statements.swap_remove(0);
        match ast.node {
            super::Stmt::Expression(expr) => Ok(expr),
            _ => bail!("input should contain single expression"),
        }
    }

    #[cfg(test)]
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
            Expression::Logical(LogicalExpression::GreaterEqual { left, right }) => {
                eval(&left) >= eval(&right)
            }
            Expression::Logical(LogicalExpression::Less { left, right }) => {
                eval(&left) < eval(&right)
            }
            Expression::Logical(LogicalExpression::LessEqual { left, right }) => {
                eval(&left) <= eval(&right)
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
            Expression::Identier(_) => todo!(),
            Expression::Assignment { .. } => todo!(),
            Expression::Call { .. } => todo!(),
            Expression::GetProperty { .. } => todo!(),
            Expression::SetProperty { .. } => todo!(),
            Expression::This => unimplemented!(),
            Expression::Super { .. } => unimplemented!(),
        }
    }

    #[cfg(test)]
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

    #[test]
    fn test_get_property_simple() {
        let expr = parse_expression("obj.field").unwrap();
        match expr.node {
            Expression::GetProperty { object, name } => {
                assert_eq!(name, "field");
                match object.node {
                    Expression::Identier(obj_name) => assert_eq!(obj_name, "obj"),
                    _ => panic!("Expected identifier for object"),
                }
            }
            _ => panic!("Expected GetProperty expression"),
        }
    }

    #[test]
    fn test_get_property_chained() {
        let expr = parse_expression("obj.a.b.c").unwrap();
        match expr.node {
            Expression::GetProperty { object, name } => {
                assert_eq!(name, "c");
                match object.node {
                    Expression::GetProperty { object: inner, name } => {
                        assert_eq!(name, "b");
                        match inner.node {
                            Expression::GetProperty { object: innermost, name } => {
                                assert_eq!(name, "a");
                                match innermost.node {
                                    Expression::Identier(obj_name) => assert_eq!(obj_name, "obj"),
                                    _ => panic!("Expected identifier"),
                                }
                            }
                            _ => panic!("Expected GetProperty"),
                        }
                    }
                    _ => panic!("Expected GetProperty"),
                }
            }
            _ => panic!("Expected GetProperty expression"),
        }
    }

    #[test]
    fn test_set_property_simple() {
        let expr = parse_expression("obj.field = value").unwrap();
        match expr.node {
            Expression::SetProperty { object, name, value } => {
                assert_eq!(name, "field");
                match object.node {
                    Expression::Identier(obj_name) => assert_eq!(obj_name, "obj"),
                    _ => panic!("Expected identifier for object"),
                }
                match value.node {
                    Expression::Identier(val_name) => assert_eq!(val_name, "value"),
                    _ => panic!("Expected identifier for value"),
                }
            }
            _ => panic!("Expected SetProperty expression"),
        }
    }

    #[test]
    fn test_set_property_chained() {
        let expr = parse_expression("obj.a.b = value").unwrap();
        match expr.node {
            Expression::SetProperty { object, name, value } => {
                assert_eq!(name, "b");
                match object.node {
                    Expression::GetProperty { object: inner, name } => {
                        assert_eq!(name, "a");
                        match inner.node {
                            Expression::Identier(obj_name) => assert_eq!(obj_name, "obj"),
                            _ => panic!("Expected identifier"),
                        }
                    }
                    _ => panic!("Expected GetProperty for object"),
                }
                match value.node {
                    Expression::Identier(val_name) => assert_eq!(val_name, "value"),
                    _ => panic!("Expected identifier for value"),
                }
            }
            _ => panic!("Expected SetProperty expression"),
        }
    }

    #[test]
    fn test_property_with_call() {
        let expr = parse_expression("obj.method()").unwrap();
        match expr.node {
            Expression::Call { calee, arguments } => {
                assert!(arguments.is_empty());
                match calee.node {
                    Expression::GetProperty { object, name } => {
                        assert_eq!(name, "method");
                        match object.node {
                            Expression::Identier(obj_name) => assert_eq!(obj_name, "obj"),
                            _ => panic!("Expected identifier for object"),
                        }
                    }
                    _ => panic!("Expected GetProperty for calee"),
                }
            }
            _ => panic!("Expected Call expression"),
        }
    }
}
