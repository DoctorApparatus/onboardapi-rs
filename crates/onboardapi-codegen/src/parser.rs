//! Recursive-descent parser for the `.rmodel` IDL subset.

use crate::{
    ast::{ConstValue, Decl, Field, Operation, Param, Primitive, ScopedName, TypeSpec, Variant},
    error::{CodegenError, Result},
    lexer::{Tok, Token, tokenize},
};

pub fn parse(file: &str, src: &str) -> Result<Vec<Decl>> {
    let tokens = tokenize(file, src)?;
    let mut p = Parser {
        file,
        tokens,
        pos: 0,
    };
    let mut decls = Vec::new();
    while !p.at_end() {
        if let Some(d) = p.decl()? {
            decls.push(d);
        }
    }
    Ok(decls)
}

struct Parser<'a> {
    file: &'a str,
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser<'_> {
    fn at_end(&mut self) -> bool {
        // Trailing doc comments without a declaration are ignored.
        while matches!(
            self.tokens.get(self.pos),
            Some(Token {
                tok: Tok::Doc(_),
                ..
            })
        ) {
            self.pos += 1;
        }
        self.pos >= self.tokens.len()
    }

    fn err<T>(&self, msg: impl Into<String>) -> Result<T> {
        let line = self
            .tokens
            .get(self.pos)
            .or(self.tokens.last())
            .map_or(0, |t| t.line);
        Err(CodegenError::Syntax {
            file: self.file.to_owned(),
            line,
            msg: msg.into(),
        })
    }

    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos).map(|t| &t.tok)
    }

    fn next(&mut self) -> Result<Tok> {
        match self.tokens.get(self.pos) {
            Some(t) => {
                self.pos += 1;
                Ok(t.tok.clone())
            }
            None => self.err("unexpected end of file"),
        }
    }

    /// Doc comments directly in front of the next token.
    fn docs(&mut self) -> Vec<String> {
        let mut docs = Vec::new();
        while let Some(Tok::Doc(d)) = self.peek() {
            docs.extend(d.clone());
            self.pos += 1;
        }
        docs
    }

    fn is_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::Punct(x)) if *x == p)
    }

    fn eat_punct(&mut self, p: &str) -> bool {
        if self.is_punct(p) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, p: &str) -> Result<()> {
        if self.eat_punct(p) {
            Ok(())
        } else {
            self.err(format!("expected `{p}`, found {:?}", self.peek()))
        }
    }

    fn is_ident(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(x)) if x == kw)
    }

    fn eat_ident(&mut self, kw: &str) -> bool {
        if self.is_ident(kw) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn ident(&mut self) -> Result<String> {
        match self.next()? {
            Tok::Ident(s) => Ok(s),
            other => self.err(format!("expected identifier, found {other:?}")),
        }
    }

    fn decl(&mut self) -> Result<Option<Decl>> {
        let docs = self.docs();
        if self.eat_punct(";") {
            return Ok(None);
        }
        let kw = self.ident()?;
        let decl = match kw.as_str() {
            "module" => {
                let name = self.ident()?;
                self.expect_punct("{")?;
                let mut decls = Vec::new();
                loop {
                    self.docs_skip_if_closing();
                    if self.eat_punct("}") {
                        break;
                    }
                    if let Some(d) = self.decl()? {
                        decls.push(d);
                    }
                }
                Decl::Module { docs, name, decls }
            }
            "struct" => {
                let name = self.ident()?;
                if self.eat_punct(";") {
                    return Ok(None); // forward declaration
                }
                self.expect_punct("{")?;
                let mut fields = Vec::new();
                loop {
                    let fdocs = self.docs();
                    if self.eat_punct("}") {
                        break;
                    }
                    let ty = self.type_spec()?;
                    let fname = self.ident()?;
                    if self.is_punct("[") {
                        return self.err("array members are not supported");
                    }
                    self.expect_punct(";")?;
                    fields.push(Field {
                        docs: fdocs,
                        name: fname,
                        ty,
                    });
                }
                Decl::Struct { docs, name, fields }
            }
            "enum" => {
                let name = self.ident()?;
                self.expect_punct("{")?;
                let mut variants = Vec::new();
                loop {
                    let vdocs = self.docs();
                    if self.eat_punct("}") {
                        break;
                    }
                    let vname = self.ident()?;
                    if self.is_punct("=") {
                        return self.err("explicit enum values are not supported");
                    }
                    variants.push(Variant {
                        docs: vdocs,
                        name: vname,
                    });
                    if !self.eat_punct(",") {
                        self.docs_skip_if_closing();
                        self.expect_punct("}")?;
                        break;
                    }
                }
                Decl::Enum {
                    docs,
                    name,
                    variants,
                }
            }
            "typedef" => {
                let ty = self.type_spec()?;
                let name = self.ident()?;
                Decl::Typedef { docs, name, ty }
            }
            "const" => {
                let ty = self.type_spec()?;
                let name = self.ident()?;
                self.expect_punct("=")?;
                let value = match self.next()? {
                    Tok::Str(s) => ConstValue::String(s),
                    Tok::Number(n) => ConstValue::Number(n),
                    other => return self.err(format!("unsupported const value {other:?}")),
                };
                Decl::Const {
                    docs,
                    name,
                    ty,
                    value,
                }
            }
            "interface" => {
                let name = self.ident()?;
                let mut bases = Vec::new();
                if self.eat_punct(":") {
                    loop {
                        bases.push(self.scoped_name()?);
                        if !self.eat_punct(",") {
                            break;
                        }
                    }
                }
                self.expect_punct("{")?;
                let mut ops = Vec::new();
                loop {
                    let odocs = self.docs();
                    if self.eat_punct("}") {
                        break;
                    }
                    ops.push(self.operation(odocs)?);
                }
                Decl::Interface {
                    docs,
                    name,
                    bases,
                    ops,
                }
            }
            other => return self.err(format!("unexpected keyword `{other}`")),
        };
        self.expect_punct(";")?;
        Ok(Some(decl))
    }

    /// Doc comments directly before a closing brace belong to nothing.
    fn docs_skip_if_closing(&mut self) {
        let save = self.pos;
        self.docs();
        if !self.is_punct("}") {
            self.pos = save;
        }
    }

    fn operation(&mut self, docs: Vec<String>) -> Result<Operation> {
        if !self.eat_ident("void") {
            return self.err("only `void` operations are supported");
        }
        let name = self.ident()?;
        self.expect_punct("(")?;
        let mut params = Vec::new();
        loop {
            let pdocs = self.docs();
            if self.eat_punct(")") {
                break;
            }
            if !self.eat_ident("in") {
                return self.err("only `in` parameters are supported");
            }
            let ty = self.type_spec()?;
            let pname = self.ident()?;
            params.push(Param {
                docs: pdocs,
                name: pname,
                ty,
            });
            if !self.eat_punct(",") {
                self.docs();
                self.expect_punct(")")?;
                break;
            }
        }
        self.expect_punct(";")?;
        Ok(Operation { docs, name, params })
    }

    fn scoped_name(&mut self) -> Result<ScopedName> {
        self.eat_punct("::");
        let mut parts = vec![self.ident()?];
        while self.eat_punct("::") {
            parts.push(self.ident()?);
        }
        Ok(ScopedName(parts))
    }

    fn type_spec(&mut self) -> Result<TypeSpec> {
        let prim = |p| Ok(TypeSpec::Primitive(p));
        if self.eat_ident("unsigned") {
            return if self.eat_ident("short") {
                prim(Primitive::U16)
            } else if self.eat_ident("long") {
                if self.eat_ident("long") {
                    prim(Primitive::U64)
                } else {
                    prim(Primitive::U32)
                }
            } else {
                self.err("expected `short` or `long` after `unsigned`")
            };
        }
        if self.eat_ident("long") {
            return if self.eat_ident("long") {
                prim(Primitive::I64)
            } else {
                prim(Primitive::I32)
            };
        }
        for (kw, p) in [
            ("boolean", Primitive::Bool),
            ("octet", Primitive::Octet),
            ("char", Primitive::Char),
            ("short", Primitive::I16),
            ("float", Primitive::F32),
            ("double", Primitive::F64),
            ("int8", Primitive::Char),
            ("uint8", Primitive::Octet),
            ("int16", Primitive::I16),
            ("uint16", Primitive::U16),
            ("int32", Primitive::I32),
            ("uint32", Primitive::U32),
            ("int64", Primitive::I64),
            ("uint64", Primitive::U64),
        ] {
            if self.eat_ident(kw) {
                return prim(p);
            }
        }
        if self.eat_ident("string") {
            if self.eat_punct("<") {
                self.next()?; // bound is irrelevant for the wire format
                self.expect_punct(">")?;
            }
            return Ok(TypeSpec::String);
        }
        if self.eat_ident("sequence") {
            self.expect_punct("<")?;
            let element = Box::new(self.type_spec()?);
            let bound = if self.eat_punct(",") {
                match self.next()? {
                    Tok::Number(n) => Some(
                        n.parse()
                            .map_err(|_| CodegenError::Unsupported(format!("bound {n}")))?,
                    ),
                    other => return self.err(format!("expected sequence bound, found {other:?}")),
                }
            } else {
                None
            };
            self.expect_punct(">")?;
            return Ok(TypeSpec::Sequence { element, bound });
        }
        Ok(TypeSpec::Named(self.scoped_name()?))
    }
}
