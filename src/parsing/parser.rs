use crate::{
    execution::cmd::Jobs,
    parsing::{
        ast::{
            If, Ast, Decl, Expr, Item, Job, Operation
        },
        lexer::{
            LinizedTokens, Token, TokenType, Tokens
        }
    },
};

use std::{
    fmt,
    slice::Iter,
    iter::Peekable
};

pub type LinizedTokensIterator<'a> = Peekable::<Iter::<'a, (usize, Tokens<'a>)>>;

const IFS: &'static [&'static str] = &[
    "ifeq", "ifneq"
];

pub enum ErrorType {
    NoClosingEndif,
    UnexpectedToken,
    JobWithoutTarget,
    ExpectedOnlyOneTokenOnTheLeftSide,
}

impl fmt::Display for ErrorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ErrorType::*;
        match self {
            NoClosingEndif => write!(f, "No closing endif"),
            UnexpectedToken => write!(f, "Unexpected token"),
            JobWithoutTarget => write!(f, "Job without a target"),
            ExpectedOnlyOneTokenOnTheLeftSide => write!(f, "Expected only one token on the left side")
        }
    }
}

pub struct Error {
    ty: ErrorType,
    note: Option::<&'static str>,
}

impl Error {
    #[inline]
    pub fn new(ty: ErrorType,
               note: Option::<&'static str>)
       -> Self
    {
        Self { ty, note }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ty = &self.ty;
        if let Some(note) = self.note {
            write!(f, "{ty}\n\tNOTE: {note}")
        } else {
            write!(f, "{ty}")
        }
    }
}

pub struct Parser<'a> {
    ast: Ast<'a>,
    iter: LinizedTokensIterator<'a>,
    err_token: Option::<&'a Token<'a>>,
}

impl<'a> Parser<'a> {
    #[inline]
    pub fn new(ts: &'a LinizedTokens<'a>) -> Self {
        Self {
            ast: Ast::default(),
            iter: ts.into_iter().peekable(),
            err_token: None
        }
    }

    #[track_caller]
    fn report_err(&mut self, err: Error) -> ! {
        if let Some(errt) = self.err_token {
            panic!("{errt}: [ERROR] {err}")
        } else {
            panic!("[ERROR] {err}")
        }
    }

    // Unexpected First Token error
    #[inline]
    #[track_caller]
    fn uft_err(&mut self, line: &'a Tokens) -> ! {
        self.err_token = line.get(0);
        self.report_err(Error::new(ErrorType::UnexpectedToken, None))
    }

    // To check token that we have only one token on the left side in these kinda situations:
    // ```
    // FLAGS=-f 69
    // ```
    // or here:
    // ```
    // $OUT: main.c
    //     $CC -o $t $FLAGS
    // ```
    #[inline]
    fn check_token_pos(&mut self, pos: usize, token: Option::<&'a Token<'a>>) {
        if pos > 1 {
            self.err_token = token;
            self.report_err(Error::new(ErrorType::ExpectedOnlyOneTokenOnTheLeftSide, None))
        }
    }

    fn parse_eq(first: &'a Token, line: &'a Tokens, eq_idx: usize) -> Item<'a> {
        if let Some(token) = line.get(eq_idx - 1) {
            if token.str.eq("+") {
                let Some(right_side) = line.get(eq_idx + 1) else {
                    panic!("Expected right side after expression")
                };

                let left_side = line.get(eq_idx - 2).unwrap();
                let expr = Expr::new(left_side, Operation::PlusEqual, right_side);
                return Item::Expr(expr)
            } else if token.str.ends_with("+") {
                let Some(right_side) = line.get(eq_idx + 1) else {
                    panic!("Expected right side after expression")
                };

                let expr = Expr::new(token, Operation::PlusEqual, right_side);
                return Item::Expr(expr)
            } else if token.str.eq("-") {
                let Some(right_side) = line.get(eq_idx + 2) else {
                    panic!("Expected right side after expression")
                };

                let left_side = line.get(eq_idx - 2).unwrap();
                let expr = Expr::new(left_side, Operation::MinusEqual, right_side);
                return Item::Expr(expr)
            } else if token.str.ends_with("-") {
                let Some(right_side) = line.get(eq_idx + 1) else {
                    panic!("Expected right side after expression")
                };

                let expr = Expr::new(token, Operation::MinusEqual, right_side);
                return Item::Expr(expr)
            }
        }

        // self.check_token_pos(eq_idx, Some(first));

        let left_side = first;
        let right_side = line[eq_idx + 1..].into_iter().collect::<Vec::<_>>();
        let decl = Decl::new(left_side, right_side);
        Item::Decl(decl)
    }

    fn parse_line(&mut self, _: &usize, line: &'a Tokens) {
        use {
            ErrorType::*,
            TokenType::*
        };

        let mut iter = line.into_iter().peekable();
        let Some(first) = iter.peek() else { return };
        if first.str.eq("endif") { return };
        match first.typ {
            Literal => if IFS.contains(&first.str) {
                let mut endif = false;
                let mut body = Vec::new();
                let (mut else_body, mut else_flag) = (Vec::new(), false);
                while let Some((_, line)) = self.iter.next() {
                    if line.iter().find(|t| t.str.eq("else")).is_some() {
                        else_flag = true;
                    } else {
                        if line.iter().any(|t| t.str.eq("endif")) {
                            endif = true;
                            break
                        }

                        let Some(eq_idx) = line.iter().position(|x| matches!(x.typ, Equal)) else { continue };
                        let Some(first) = line.get(eq_idx - 1) else { continue };
                        let item = Self::parse_eq(first, line, eq_idx);
                        if else_flag {
                            else_body.push(item);
                        } else {
                            body.push(item);
                        }
                    }
                }

                if !endif {
                    self.err_token = Some(first);
                    self.report_err(Error::new(NoClosingEndif, None));
                }

                let rev = if first.str.eq("ifeq") { false } else { true };

                // Skip the if keyword
                iter.next();

                let Some(left_side) = iter.next() else {
                    panic!("If without a left_side")
                };

                let Some(right_side) = iter.next() else {
                    panic!("If without a right_side")
                };

                let r#if = If::new(rev, left_side, right_side, body, else_body);
                self.ast.items.push(Item::If(r#if));
            } else if let Some(eq_idx) = line.iter().position(|x| matches!(x.typ, Equal)) {
                let item = Self::parse_eq(first, line, eq_idx);
                self.ast.items.push(item);
            } else if let Some(colon_idx) = line.iter().position(|x| matches!(x.typ, Colon)) {
                self.check_token_pos(colon_idx, Some(first));

                let target = first;
                let dependencies = &line[colon_idx + 1..];
                let mut body = Vec::with_capacity(line.len());
                while let Some((wc, line)) = self.iter.peek() {
                    if wc.eq(&0) { break }
                    body.push(line);
                    self.iter.next();
                }

                let job = Job::new(target, dependencies, body);
                self.ast.items.push(Item::Job(job));
            } else {
                self.uft_err(line);
            },
            Colon => {
                self.err_token = Some(first);
                let err = Error::new(JobWithoutTarget, Some("Jobs without targets are not allowed here!"));
                self.report_err(err);
            }
            _ => self.uft_err(line)
        };
    }

    pub fn parse(&mut self) -> Result::<Jobs, crate::parsing::ast::Error> {
        while let Some((wc, line)) = self.iter.next() {
            self.parse_line(wc, line);
        }
        self.ast.parse()
    }
}
