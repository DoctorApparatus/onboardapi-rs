//! Tokenizer for `.rmodel` files.
//!
//! Plain comments and preprocessor lines are dropped. Doc comments (`///` and `/** */`)
//! are kept as tokens so the parser can attach them to the following declaration.

use crate::error::{CodegenError, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Number(String),
    Str(String),
    Doc(Vec<String>),
    Punct(&'static str),
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
}

const PUNCT: &[&str] = &[
    "::", "{", "}", "(", ")", "<", ">", ",", ";", ":", "=", "[", "]",
];

pub fn tokenize(file: &str, src: &str) -> Result<Vec<Token>> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let mut at_line_start = true;

    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'\n' => {
                line += 1;
                at_line_start = true;
                i += 1;
                continue;
            }
            c if c.is_ascii_whitespace() => {
                i += 1;
                continue;
            }
            b'#' if at_line_start => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            _ => {}
        }
        at_line_start = false;

        if src[i..].starts_with("///") {
            let end = src[i..].find('\n').map_or(bytes.len(), |e| i + e);
            let text = src[i + 3..end]
                .strip_prefix(' ')
                .unwrap_or(&src[i + 3..end]);
            push_doc(&mut out, vec![text.trim_end().to_owned()], line);
            i = end;
            continue;
        }
        if src[i..].starts_with("//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if src[i..].starts_with("/*") {
            let is_doc = src[i..].starts_with("/**") && !src[i..].starts_with("/**/");
            let end =
                src[i + 2..]
                    .find("*/")
                    .map(|e| i + 2 + e)
                    .ok_or_else(|| CodegenError::Syntax {
                        file: file.to_owned(),
                        line,
                        msg: "unterminated block comment".into(),
                    })?;
            let body = &src[i + if is_doc { 3 } else { 2 }..end];
            if is_doc {
                push_doc(&mut out, clean_block_doc(body), line);
            }
            line += body.matches('\n').count();
            i = end + 2;
            continue;
        }
        if c == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'"' {
                j += 1;
            }
            out.push(Token {
                tok: Tok::Str(src[start..j].to_owned()),
                line,
            });
            i = j + 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Ident(src[start..i].to_owned()),
                line,
            });
            continue;
        }
        if c.is_ascii_digit() || (c == b'-' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Number(src[start..i].to_owned()),
                line,
            });
            continue;
        }
        if let Some(p) = PUNCT.iter().find(|p| src[i..].starts_with(**p)) {
            out.push(Token {
                tok: Tok::Punct(p),
                line,
            });
            i += p.len();
            continue;
        }
        return Err(CodegenError::Syntax {
            file: file.to_owned(),
            line,
            msg: format!("unexpected character {:?}", c as char),
        });
    }
    Ok(out)
}

/// Consecutive `///` lines form one doc block.
fn push_doc(out: &mut Vec<Token>, lines: Vec<String>, line: usize) {
    if let Some(Token {
        tok: Tok::Doc(prev),
        ..
    }) = out.last_mut()
    {
        prev.extend(lines);
        return;
    }
    out.push(Token {
        tok: Tok::Doc(lines),
        line,
    });
}

fn clean_block_doc(body: &str) -> Vec<String> {
    let mut lines: Vec<String> = body
        .lines()
        .map(|l| {
            let t = l.trim_start();
            let t = t
                .strip_prefix('*')
                .map_or(t, |r| r.strip_prefix(' ').unwrap_or(r));
            t.trim_end().to_owned()
        })
        .collect();
    while lines.first().is_some_and(String::is_empty) {
        lines.remove(0);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}
