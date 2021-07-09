//! Block statement parsing.
//!
//! More information:
//!  - [MDN documentation][mdn]
//!  - [ECMAScript specification][spec]
//!
//! [mdn]: https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Statements/block
//! [spec]: https://tc39.es/ecma262/#sec-block

#[cfg(test)]
mod tests;

use super::StatementList;

use crate::syntax::lexer::TokenKind;
use crate::{
    profiler::BoaProfiler,
    syntax::{
        ast::{node, Punctuator},
        parser::{
            AllowAwait, AllowReturn, AllowYield, Cursor, DeclaredNames, ParseError, TokenParser,
        },
    },
};

use std::io::Read;

/// The possible TokenKind which indicate the end of a block statement.
const BLOCK_BREAK_TOKENS: [TokenKind; 1] = [TokenKind::Punctuator(Punctuator::CloseBlock)];

/// A `BlockStatement` is equivalent to a `Block`.
///
/// More information:
///  - [ECMAScript specification][spec]
///
/// [spec]: https://tc39.es/ecma262/#prod-BlockStatement
pub(super) type BlockStatement = Block;

/// Variable declaration list parsing.
///
/// More information:
///  - [MDN documentation][mdn]
///  - [ECMAScript specification][spec]
///
/// [mdn]: https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Statements/block
/// [spec]: https://tc39.es/ecma262/#prod-Block
#[derive(Debug, Clone, Copy)]
pub(super) struct Block {
    allow_yield: AllowYield,
    allow_await: AllowAwait,
    allow_return: AllowReturn,
}

impl Block {
    /// Creates a new `Block` parser.
    pub(super) fn new<Y, A, R>(allow_yield: Y, allow_await: A, allow_return: R) -> Self
    where
        Y: Into<AllowYield>,
        A: Into<AllowAwait>,
        R: Into<AllowReturn>,
    {
        Self {
            allow_yield: allow_yield.into(),
            allow_await: allow_await.into(),
            allow_return: allow_return.into(),
        }
    }
}

impl<R> TokenParser<R> for Block
where
    R: Read,
{
    type Output = node::Block;

    fn parse(
        self,
        cursor: &mut Cursor<R>,
        env: &mut DeclaredNames,
    ) -> Result<Self::Output, ParseError> {
        let _timer = BoaProfiler::global().start_event("Block", "Parsing");

        // Even if this is part of a for loop, the initializer of the for loop is considered in its
        // own scope from the values within the for loop. So this is valid:
        // for (let a; ...) {
        //   let a;
        // }
        env.push_stack();
        cursor.expect(Punctuator::OpenBlock, "block")?;
        if let Some(tk) = cursor.peek(0)? {
            if tk.kind() == &TokenKind::Punctuator(Punctuator::CloseBlock) {
                cursor.next()?.expect("} token vanished");
                return Ok(node::Block::from(vec![]));
            }
        }

        let statement_list = StatementList::new(
            self.allow_yield,
            self.allow_await,
            self.allow_return,
            true,
            &BLOCK_BREAK_TOKENS,
        )
        .parse(cursor, env)
        .map(node::Block::from)?;
        cursor.expect(Punctuator::CloseBlock, "block")?;
        env.pop_stack()?;

        Ok(statement_list)
    }
}
