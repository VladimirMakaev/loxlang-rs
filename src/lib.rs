//! Lox language interpreter implemented in Rust
//!
//! This library provides a bytecode virtual machine for the Lox language
//! as described in "Crafting Interpreters" by Robert Nystrom.

pub mod byte_code;
pub mod codemap;
pub mod interner;
pub mod lexer;
pub mod parser;
pub mod value;
pub mod vm;
