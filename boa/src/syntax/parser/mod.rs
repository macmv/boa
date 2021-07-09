//! Boa parser implementation.

mod cursor;
pub mod error;
mod expression;
mod function;
mod statement;
#[cfg(test)]
mod tests;

pub use self::error::{ParseError, ParseResult};
use crate::syntax::{
    ast::node::StatementList,
    lexer::{Error as LexError, Position, TokenKind},
};

use cursor::Cursor;

use std::{collections::HashMap, io::Read, mem};

/// Trait implemented by parsers.
///
/// This makes it possible to abstract over the underlying implementation of a parser.
trait TokenParser<R>: Sized
where
    R: Read,
{
    /// Output type for the parser.
    type Output; // = Node; waiting for https://github.com/rust-lang/rust/issues/29661

    /// Parses the token stream using the current parser.
    ///
    /// This method needs to be provided by the implementor type.
    fn parse(
        self,
        cursor: &mut Cursor<R>,
        env: &mut DeclaredNames,
    ) -> Result<Self::Output, ParseError>;
}

/// Boolean representing if the parser should allow a `yield` keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AllowYield(bool);

impl From<bool> for AllowYield {
    fn from(allow: bool) -> Self {
        Self(allow)
    }
}

/// Boolean representing if the parser should allow a `await` keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AllowAwait(bool);

impl From<bool> for AllowAwait {
    fn from(allow: bool) -> Self {
        Self(allow)
    }
}

/// Boolean representing if the parser should allow a `in` keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AllowIn(bool);

impl From<bool> for AllowIn {
    fn from(allow: bool) -> Self {
        Self(allow)
    }
}

/// Boolean representing if the parser should allow a `return` keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AllowReturn(bool);

impl From<bool> for AllowReturn {
    fn from(allow: bool) -> Self {
        Self(allow)
    }
}

/// Boolean representing if the parser should allow a `default` keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AllowDefault(bool);

impl From<bool> for AllowDefault {
    fn from(allow: bool) -> Self {
        Self(allow)
    }
}

/// Tracks all of the declared names during parsing. This is a small wrapper over two
/// `HashSet`s, which store the var delcared names, and the lexically declared names.
#[derive(Debug, Clone)]
pub struct DeclaredNames {
    lex: HashMap<Box<str>, Position>,
    var: HashMap<Box<str>, Position>,
    stack: Vec<(HashMap<Box<str>, Position>, HashMap<Box<str>, Position>)>,
}

impl Default for DeclaredNames {
    fn default() -> Self {
        DeclaredNames {
            lex: HashMap::new(),
            var: HashMap::new(),
            stack: vec![],
        }
    }
}

impl DeclaredNames {
    /// Inserts a new variable name. If the variable name already exists, this will return
    /// an error. The pos argument is used to generate an error message.
    pub fn insert_var_name(&mut self, name: &str, pos: Position) -> Result<(), ParseError> {
        // This only checks for lexically declared names that have already been defined. It
        // does not check for situations like `{ let a; { var a; } }`, because the var is valid
        // at the point when this function is called.
        if self.check_any_lex(name) {
            Err(ParseError::lex(LexError::Syntax(
                format!("Redeclaration of variable `{}`", name).into(),
                pos,
            )))
        } else {
            self.var.insert(name.into(), pos);
            Ok(())
        }
    }
    /// Inserts a lexically declared name. Returns an error if the var name or the lexically
    /// declared name already exists.
    pub fn insert_lex_name(&mut self, name: &str, pos: Position) -> Result<(), ParseError> {
        // This only cares about the current lex level. Lexically declared names that are
        // outside the current scope are not checked here (see `pop_stack`).
        if self.var.contains_key(name) || self.lex.insert(name.into(), pos).is_some() {
            Err(ParseError::lex(LexError::Syntax(
                format!("Redeclaration of variable `{}`", name).into(),
                pos,
            )))
        } else {
            Ok(())
        }
    }
    /// This adds an element to the lexical names restore list. If
    /// [`pop_stack`](Self::pop_stack) is called, then the current
    /// copy of lexically declared names will be restored.
    ///
    /// This works like a stack:
    ///
    /// ```
    /// # use boa::syntax::lexer::Position;
    /// use boa::syntax::parser::DeclaredNames;
    ///
    /// let mut env = DeclaredNames::default();
    ///
    /// env.insert_lex_name("hello", Position::new(1, 1));
    /// env.insert_lex_name("world", Position::new(1, 1));
    /// env.push_stack(); // Env is now empty again
    /// env.insert_lex_name("second", Position::new(1, 1));
    /// env.insert_lex_name("level", Position::new(1, 1)); // Env now has two lexically declared names.
    /// env.push_stack(); // Env is empty again
    ///
    /// assert!(env.pop_stack().is_ok()); // Env now has two lexically declared names ("second" and "level").
    /// assert!(env.pop_stack().is_ok()); // Env now has two lexically declared names ("hello" and "world").
    ///
    /// // env.pop_lex_restore(); Will panic
    /// ```
    ///
    /// For variables (not lexically declared names) there is slightly different behavior:
    /// ```
    /// # use boa::syntax::lexer::Position;
    /// use boa::syntax::parser::DeclaredNames;
    ///
    /// let mut env = DeclaredNames::default();
    ///
    /// env.insert_var_name("hello", Position::new(1, 1));
    /// env.insert_var_name("world", Position::new(1, 1));
    /// env.push_stack(); // Env is now empty again
    /// env.insert_var_name("second", Position::new(1, 1));
    /// env.insert_var_name("level", Position::new(1, 1)); // Env now has two lexically declared names.
    /// env.push_stack(); // Env is empty again
    ///
    /// assert!(env.pop_stack().is_ok()); // Env now has two lexically declared names ("second" and "level").
    /// assert!(env.pop_stack().is_ok()); // Env now has all of the lexically declared names.
    ///
    /// // env.pop_lex_restore(); Will panic
    /// ```
    ///
    /// The reason these act differently is a matter of scope. A `let` or `const` statement only lives
    /// within the current block, so `pop_stack` should remove the value from scope. However, `var` lives
    /// within the scope of the function. Therefore, `pop_stack` only merges the inner `var` statements
    /// and the outer `var` list each time you call `pop_stack`.
    pub fn push_stack(&mut self) {
        // When moving to a new stack level, we clear all declared variables. This is because
        // variable declarations are parsed the same way no matter what order the inner statements
        // are in; if there is a nested block before/after a let statement, we should get the
        // same result. So, we do all of the handling for those errors in `pop_stack`.
        let mut lex_old = HashMap::new();
        let mut var_old = HashMap::new();
        mem::swap(&mut self.lex, &mut lex_old);
        mem::swap(&mut self.var, &mut var_old);
        self.stack.push((lex_old, var_old));
    }
    /// See the documentation on [`push_stack`](Self::push_stack).
    ///
    /// This will return true if there was something to pop, false if otherwise.
    /// In normal usage, this should never return false.
    ///
    /// This will also check for any redeclaration errors. Since this is called at
    /// the end of a block, it will check for errors like `{ let a; { var a; } }`.
    /// After the inner block is parsed, the `a` in lexically declared names will
    /// be restored. And then there will be a collision in vars and lex.
    pub fn pop_stack(&mut self) -> Result<(), ParseError> {
        if let Some(outer) = self.stack.pop() {
            // When you leave a stack level, var declarations stay the same, but lexical
            // variables get restored to their outer enfironment.
            self.lex = outer.0;
            self.var.extend(outer.1);
            for (name, pos) in self.var.iter() {
                if self.check_any_lex(name) {
                    // We want to use the `var` position here, as that is the declaration
                    // that is causing this error.
                    dbg!("error in pop inner");
                    return Err(ParseError::lex(LexError::Syntax(
                        format!("Redeclaration of variable (in pop) `{}`", name).into(),
                        *pos,
                    )));
                }
            }
            Ok(())
        } else {
            // Might not want to panic here, but if we are here, then something
            // has definitly gone wrong.
            unreachable!("Called pop without any lex restore to pop!");
        }
    }

    // Checks if the given name exists in either the current or outer stack levels.
    fn check_any_lex(&self, name: &str) -> bool {
        if self.lex.contains_key(name) {
            return true;
        }
        for level in &self.stack {
            if level.0.contains_key(name) {
                return true;
            }
        }
        false
    }
}

#[derive(Debug)]
pub struct Parser<R> {
    /// Cursor of the parser, pointing to the lexer and used to get tokens for the parser.
    cursor: Cursor<R>,
}

impl<R> Parser<R> {
    pub fn new(reader: R, strict_mode: bool) -> Self
    where
        R: Read,
    {
        let mut cursor = Cursor::new(reader);
        cursor.set_strict_mode(strict_mode);

        Self { cursor }
    }

    pub fn parse_all(&mut self) -> Result<StatementList, ParseError>
    where
        R: Read,
    {
        Script.parse(&mut self.cursor, &mut DeclaredNames::default())
    }
}

/// Parses a full script.
///
/// More information:
///  - [ECMAScript specification][spec]
///
/// [spec]: https://tc39.es/ecma262/#prod-Script
#[derive(Debug, Clone, Copy)]
pub struct Script;

impl<R> TokenParser<R> for Script
where
    R: Read,
{
    type Output = StatementList;

    fn parse(
        self,
        cursor: &mut Cursor<R>,
        env: &mut DeclaredNames,
    ) -> Result<Self::Output, ParseError> {
        match cursor.peek(0)? {
            Some(tok) => {
                match tok.kind() {
                    TokenKind::StringLiteral(string) if string.as_ref() == "use strict" => {
                        cursor.set_strict_mode(true);
                    }
                    _ => {}
                }
                ScriptBody.parse(cursor, env)
            }
            None => Ok(StatementList::from(Vec::new())),
        }
    }
}

/// Parses a script body.
///
/// More information:
///  - [ECMAScript specification][spec]
///
/// [spec]: https://tc39.es/ecma262/#prod-ScriptBody
#[derive(Debug, Clone, Copy)]
pub struct ScriptBody;

impl<R> TokenParser<R> for ScriptBody
where
    R: Read,
{
    type Output = StatementList;

    fn parse(
        self,
        cursor: &mut Cursor<R>,
        env: &mut DeclaredNames,
    ) -> Result<Self::Output, ParseError> {
        self::statement::StatementList::new(false, false, false, false, &[]).parse(cursor, env)
    }
}
