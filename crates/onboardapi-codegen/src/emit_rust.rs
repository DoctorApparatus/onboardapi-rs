//! Emits the Rust side: one module per IDL module plus `generated/mod.rs`.

use std::{collections::HashSet, path::Path};

use heck::{ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};

use crate::{
    ast::{ConstValue, Primitive},
    error::Result,
    model::{Interface, Model, Module, Op, Ty, TypeDef, TypeKind, rust_mod_name},
    writer::{HEADER, Src, write_if_changed},
};

pub fn emit(model: &Model, dir: &Path) -> Result<()> {
    let mut root = Src::default();
    root.line(HEADER.replace("// ", "//! ").replace("//! SPDX", "// SPDX"));
    root.line("//! Generated bindings, one module per onboardapi interface module.");
    root.blank();
    for m in model.modules.values() {
        let file = emit_module(model, m);
        write_if_changed(&dir.join(format!("{}.rs", m.rust_mod())), &file)?;
        for l in docs_summary(&m.docs, m.short()) {
            root.line(format!("/// {l}"));
        }
        root.line("#[allow(clippy::all, clippy::pedantic, rustdoc::all, unused_imports, unused_variables, unreachable_code, non_camel_case_types)]");
        root.line(format!("pub mod {};", m.rust_mod()));
    }
    write_if_changed(&dir.join("mod.rs"), &root.finish())?;
    let tests = dir.join("../../tests/all_operations.rs");
    write_if_changed(&tests, &emit_ops_test(model))
}

/// Integration test sending every operation of every module once in each direction.
fn emit_ops_test(model: &Model) -> String {
    let mut s = Src::default();
    s.line(HEADER);
    s.line("//! Sends every operation with default arguments through DDS and checks it arrives unchanged.");
    s.line("//! Operations travel on separate topics, so arrival order is not checked.");
    s.blank();
    s.line("mod support;");
    s.blank();
    s.line("#[rustfmt::skip]");
    s.open("mod ops {");
    s.line("use super::support;");
    s.blank();
    for m in model.modules.values().filter(|m| m.has_api()) {
        let r = m.rust_mod();
        s.line("#[tokio::test(flavor = \"multi_thread\")]");
        s.open(format!("async fn {r}() -> anyhow::Result<()> {{"));
        s.line(format!(
            "use onboardapi::{r}::{{Client, ClientMessage, Service, ServiceMessage}};"
        ));
        s.line(format!("let name = support::unique({r:?});"));
        s.line("let (service, mut service_inbox) = Service::create(support::domain(), &name)?;");
        s.line("let (client, mut client_inbox) = Client::create(support::domain(), &name)?;");
        s.line("support::connect(&client).await?;");
        for (side, msg, sender, inbox) in [
            (
                m.service.as_ref().expect("api"),
                "ServiceMessage",
                "client",
                "service_inbox",
            ),
            (
                m.client.as_ref().expect("api"),
                "ClientMessage",
                "service",
                "client_inbox",
            ),
        ] {
            s.open(format!("let expected: Vec<{msg}> = vec!["));
            for op in &side.ops {
                if op.params.is_empty() {
                    s.line(format!("{msg}::{},", op.name));
                } else {
                    let fields: Vec<String> = op
                        .params
                        .iter()
                        .map(|p| format!("{}: Default::default()", field_ident(&p.name)))
                        .collect();
                    s.line(format!("{msg}::{} {{ {} }},", op.name, fields.join(", ")));
                }
            }
            s.close("];");
            s.line(format!(
                "support::exchange(&{sender}, &mut {inbox}, expected).await?;"
            ));
        }
        s.line("drop((service, client));");
        s.line("Ok(())");
        s.close("}");
        s.blank();
    }
    s.close("}");
    s.finish()
}

/// First paragraph of the module documentation (no diagrams).
fn docs_summary(docs: &[String], fallback: &str) -> Vec<String> {
    let lines: Vec<String> = docs
        .iter()
        .take_while(|l| !l.trim().is_empty() && !l.trim_start().starts_with('@'))
        .map(|l| escape_doc(l.trim()))
        .collect();
    if lines.is_empty() {
        vec![format!("`M_{fallback}`")]
    } else {
        lines
    }
}

/// Keeps rustdoc from reading prose as markup it does not understand.
fn escape_doc(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    for c in line.chars() {
        match c {
            '`' => {
                in_code = !in_code;
                out.push(c);
            }
            '[' | ']' if !in_code => {
                out.push('\\');
                out.push(c);
            }
            '<' if !in_code => out.push_str("&lt;"),
            '>' if !in_code => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Rustdoc lines. PlantUML diagrams are fenced as `text` so they are neither
/// rendered as markdown nor compiled as doctests.
fn doc_lines(docs: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_uml = false;
    for l in docs {
        let t = l.trim();
        if t.starts_with("@startuml") {
            in_uml = true;
            out.push("```text".to_owned());
            out.push(l.clone());
            continue;
        }
        if in_uml {
            out.push(l.clone());
            if t.starts_with("@enduml") {
                in_uml = false;
                out.push("```".to_owned());
            }
            continue;
        }
        out.push(escape_doc(t));
    }
    if in_uml {
        out.push("```".to_owned());
    }
    out
}

fn docs(src: &mut Src, docs: &[String]) {
    for l in doc_lines(docs) {
        src.line(if l.is_empty() {
            "///".to_owned()
        } else {
            format!("/// {l}")
        });
    }
}

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
    "pub", "ref", "return", "self", "static", "struct", "super", "trait", "true", "type", "unsafe",
    "use", "where", "while", "abstract", "become", "box", "do", "final", "macro", "override",
    "priv", "try", "typeof", "unsized", "virtual", "yield",
];

pub fn field_ident(name: &str) -> String {
    let s = name.to_snake_case();
    if KEYWORDS.contains(&s.as_str()) {
        format!("r#{s}")
    } else {
        s
    }
}

fn bare(ident: &str) -> &str {
    ident.strip_prefix("r#").unwrap_or(ident)
}

fn prim(p: Primitive) -> &'static str {
    match p {
        Primitive::Bool => "bool",
        Primitive::Octet | Primitive::Char => "u8",
        Primitive::I16 => "i16",
        Primitive::U16 => "u16",
        Primitive::I32 => "i32",
        Primitive::U32 => "u32",
        Primitive::I64 => "i64",
        Primitive::U64 => "u64",
        Primitive::F32 => "f32",
        Primitive::F64 => "f64",
    }
}

fn rust_ty(model: &Model, cur: &Module, ty: &Ty) -> String {
    match ty {
        Ty::Prim(p) => prim(*p).to_owned(),
        Ty::String => "String".to_owned(),
        Ty::Seq(t) => format!("Vec<{}>", rust_ty(model, cur, t)),
        Ty::Opt(t) => format!("Option<{}>", rust_ty(model, cur, t)),
        Ty::Ref(p) => {
            let def = model.type_def(p);
            if p[0] == cur.name {
                def.rust_name.clone()
            } else {
                format!("crate::{}::{}", rust_mod_name(&p[0]), def.rust_name)
            }
        }
    }
}

/// Argument type of a send helper: by value for `Copy`, `&str` for strings, `&T` otherwise.
fn arg_ty(model: &Model, cur: &Module, ty: &Ty) -> String {
    if model.is_copy(ty) {
        rust_ty(model, cur, ty)
    } else if model.is_string(ty) {
        "&str".to_owned()
    } else {
        format!("&{}", rust_ty(model, cur, ty))
    }
}

fn variant_idents(variants: &[(Vec<String>, String)]) -> Vec<String> {
    let mut idents: Vec<String> = variants
        .iter()
        .map(|(_, v)| {
            let c = v.to_upper_camel_case();
            if c.starts_with(|ch: char| ch.is_ascii_digit()) {
                format!("V{c}")
            } else {
                c
            }
        })
        .collect();
    let unique: HashSet<_> = idents.iter().collect();
    if unique.len() != idents.len() {
        // Case conversion collapsed two variants; keep the IDL spelling.
        idents = variants.iter().map(|(_, v)| v.clone()).collect();
    }
    idents
}

fn emit_module(model: &Model, m: &Module) -> String {
    let mut s = Src::default();
    s.line(HEADER);
    if m.docs.is_empty() {
        s.line(format!("//! `{}`", m.name));
    } else {
        for l in doc_lines(&m.docs) {
            s.line(if l.is_empty() {
                "//!".to_owned()
            } else {
                format!("//! {l}")
            });
        }
    }
    s.blank();
    s.line("use schemars::JsonSchema;");
    s.line("use serde::{Deserialize, Serialize};");
    s.line("use typed_builder::TypedBuilder;");
    s.blank();
    s.line("use crate::wire::{Decode, Encode, Reader, WireError, Writer};");
    s.blank();
    s.line(format!(
        "/// IDL module name.\npub const MODULE: &str = \"{}\";",
        m.name
    ));
    s.blank();

    for c in &m.consts {
        docs(&mut s, &c.docs);
        let name = c.name.to_shouty_snake_case();
        match &c.value {
            ConstValue::String(v) => s.line(format!("pub const {name}: &str = {v:?};")),
            ConstValue::Number(v) => s.line(format!(
                "pub const {name}: {} = {v};",
                rust_ty(model, m, &c.ty)
            )),
        };
        s.blank();
    }

    for t in &m.types {
        emit_type(model, m, t, &mut s);
        s.blank();
    }

    if let (Some(svc), Some(cli)) = (&m.service, &m.client) {
        emit_api(model, m, svc, cli, &mut s);
    }
    s.finish()
}

fn emit_type(model: &Model, m: &Module, t: &TypeDef, s: &mut Src) {
    let name = &t.rust_name;
    let idl = t.path.join("::");
    match &t.kind {
        TypeKind::Alias { ty } => {
            docs(s, &t.docs);
            s.line(format!("pub type {name} = {};", rust_ty(model, m, ty)));
        }
        TypeKind::Enum { variants } => {
            docs(s, &t.docs);
            s.line(format!("///\n/// IDL: `{idl}`"));
            s.line("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]");
            s.line("#[derive(Serialize, Deserialize, JsonSchema)]");
            s.line("#[derive(strum::Display, strum::EnumString, strum::EnumIter, strum::IntoStaticStr, strum::FromRepr, strum::VariantArray)]");
            s.line("#[repr(u32)]");
            s.open(format!("pub enum {name} {{"));
            for (i, ((vdocs, orig), ident)) in
                variants.iter().zip(variant_idents(variants)).enumerate()
            {
                docs(s, vdocs);
                if i == 0 {
                    s.line("#[default]");
                }
                s.line(format!("#[serde(rename = {orig:?})]"));
                s.line(format!("#[strum(serialize = {orig:?})]"));
                s.line(format!("{ident} = {i},"));
            }
            s.close("}");
            s.blank();
            s.open(format!("impl Encode for {name} {{"));
            s.open("fn encode(&self, w: &mut Writer) {");
            s.line("(*self as u32).encode(w);");
            s.close("}");
            s.close("}");
            s.blank();
            s.open(format!("impl Decode for {name} {{"));
            s.open("fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {");
            s.line("let value = u32::decode(r)?;");
            s.line(format!(
                "Self::from_repr(value).ok_or(WireError::InvalidEnum {{ ty: {idl:?}, value }})"
            ));
            s.close("}");
            s.close("}");
        }
        TypeKind::Struct { fields } => {
            let eq = if fields.iter().any(|f| model.contains_float(&f.ty)) {
                ""
            } else {
                ", Eq"
            };
            docs(s, &t.docs);
            s.line(format!("///\n/// IDL: `{idl}`"));
            s.line(format!("#[derive(Debug, Clone, PartialEq{eq}, Default, Serialize, Deserialize, JsonSchema, TypedBuilder)]"));
            s.line("#[serde(default)]");
            s.line("#[builder(field_defaults(default))]");
            s.open(format!("pub struct {name} {{"));
            for f in fields {
                let ident = field_ident(&f.name);
                docs(s, &f.docs);
                s.line(format!("#[serde(rename = {:?})]", f.name));
                // typed-builder only recognizes a literal `Option<..>`, so optional
                // aliases are expanded for struct fields.
                let ty = match model.resolve(&f.ty) {
                    Ty::String => {
                        s.line("#[builder(setter(into))]");
                        rust_ty(model, m, &f.ty)
                    }
                    Ty::Opt(inner) => {
                        s.line(format!(
                            "#[builder(setter(strip_option(fallback = {}_opt)))]",
                            bare(&ident)
                        ));
                        format!("Option<{}>", rust_ty(model, m, inner))
                    }
                    _ => rust_ty(model, m, &f.ty),
                };
                s.line(format!("pub {ident}: {ty},"));
            }
            s.close("}");
            s.blank();
            s.open(format!("impl Encode for {name} {{"));
            let w = if fields.is_empty() { "_w" } else { "w" };
            s.open(format!("fn encode(&self, {w}: &mut Writer) {{"));
            for f in fields {
                s.line(format!("self.{}.encode(w);", field_ident(&f.name)));
            }
            s.close("}");
            s.close("}");
            s.blank();
            s.open(format!("impl Decode for {name} {{"));
            let r = if fields.is_empty() { "_r" } else { "r" };
            s.open(format!(
                "fn decode({r}: &mut Reader<'_>) -> Result<Self, WireError> {{"
            ));
            s.open("Ok(Self {");
            for f in fields {
                s.line(format!("{}: Decode::decode(r)?,", field_ident(&f.name)));
            }
            s.close("})");
            s.close("}");
            s.close("}");
        }
    }
}

struct Side<'a> {
    iface: &'a Interface,
    /// `ServiceMessage` / `ClientMessage`
    message: &'static str,
    handler: &'static str,
    idl: String,
}

fn emit_api(model: &Model, m: &Module, svc: &Interface, cli: &Interface, s: &mut Src) {
    let service_side = Side {
        iface: svc,
        message: "ServiceMessage",
        handler: "ServiceHandler",
        idl: format!("{}::IService", m.name),
    };
    let client_side = Side {
        iface: cli,
        message: "ClientMessage",
        handler: "ClientHandler",
        idl: format!("{}::IClient", m.name),
    };

    s.line("// ---------------------------------------------------------------------------------------------");
    s.line("// Interfaces");
    s.line("// ---------------------------------------------------------------------------------------------");
    s.blank();
    for side in [&service_side, &client_side] {
        emit_message(model, m, side, s);
        emit_handler(model, m, side, s);
    }

    let short = m.short();
    for (role, kind, rx, tx, sends) in [
        ("ServiceRole", "service", &service_side, &client_side, cli),
        ("ClientRole", "client", &client_side, &service_side, svc),
    ] {
        let alias = if kind == "service" {
            "Service"
        } else {
            "Client"
        };
        s.line(format!(
            "/// Marker type for the {kind} side of `{}`.",
            m.name
        ));
        s.line("#[derive(Debug, Clone, Copy)]");
        s.line(format!("pub enum {role} {{}}"));
        s.blank();
        s.open(format!("impl crate::Role for {role} {{"));
        s.line(format!("type Incoming = {};", rx.message));
        s.line(format!("type Outgoing = {};", tx.message));
        s.line(format!(
            "const NAME: &'static str = \"{}::{alias}\";",
            m.name
        ));
        s.blank();
        s.open("fn vtable() -> &'static onboardapi_sys::obapi_vtable {");
        s.line("// SAFETY: the vtable is an immutable static defined by the C++ shim.");
        s.line(format!(
            "unsafe {{ &*(&raw const onboardapi_sys::obapi_{short}_{kind}_vtable) }}"
        ));
        s.close("}");
        s.close("}");
        s.blank();
        if kind == "client" {
            s.line(format!("impl crate::ClientSide for {role} {{}}"));
            s.blank();
        }
        s.line(format!(
            "/// `{}::{alias}`: receives [`{}`], sends [`{}`].",
            m.name, rx.message, tx.message
        ));
        s.line(format!("pub type {alias} = crate::Endpoint<{role}>;"));
        s.blank();
        emit_send_helpers(model, m, role, sends, s);
    }
}

fn op_fields(model: &Model, m: &Module, op: &Op) -> Vec<(String, String)> {
    op.params
        .iter()
        .map(|p| (field_ident(&p.name), rust_ty(model, m, &p.ty)))
        .collect()
}

fn emit_message(model: &Model, m: &Module, side: &Side<'_>, s: &mut Src) {
    let msg = side.message;
    docs(s, &side.iface.docs);
    if !side.iface.docs.is_empty() {
        s.line("///");
    }
    s.line(format!(
        "/// Operations of `{}`, one variant per operation.",
        side.idl
    ));
    if !side.iface.bases.is_empty() {
        let bases: Vec<String> = side
            .iface
            .bases
            .iter()
            .map(|b| format!("`{}`", b.join("::")))
            .collect();
        s.line(format!(
            "/// Includes the operations inherited from {}.",
            bases.join(", ")
        ));
    }
    if side.iface.ops.is_empty() {
        s.line("#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]");
    } else {
        s.line("#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, strum::IntoStaticStr)]");
    }
    s.open(format!("pub enum {msg} {{"));
    for op in &side.iface.ops {
        docs(s, &op.docs);
        if op.params.is_empty() {
            s.line(format!("{},", op.name));
            continue;
        }
        s.open(format!("{} {{", op.name));
        for (p, (ident, ty)) in op.params.iter().zip(op_fields(model, m, op)) {
            docs(s, &p.docs);
            s.line(format!("#[serde(rename = {:?})]", p.name));
            s.line(format!("{ident}: {ty},"));
        }
        s.close("},");
    }
    s.close("}");
    s.blank();

    s.open(format!("impl crate::Message for {msg} {{"));
    s.line(format!("const INTERFACE: &'static str = {:?};", side.idl));
    s.blank();
    if side.iface.ops.is_empty() {
        // Interface without operations: the enum is uninhabited.
        s.line("fn op_id(&self) -> u32 { match *self {} }");
        s.line("fn op_name(&self) -> &'static str { match *self {} }");
        s.line("fn encode_args(&self, _w: &mut Writer) { match *self {} }");
        s.open("fn decode(op: u32, _r: &mut Reader<'_>) -> Result<Self, WireError> {");
        s.line(format!(
            "Err(WireError::UnknownOp {{ interface: {:?}, op }})",
            side.idl
        ));
        s.close("}");
        s.close("}");
        s.blank();
        return;
    }
    s.open("fn op_id(&self) -> u32 {");
    s.open("match self {");
    for (i, op) in side.iface.ops.iter().enumerate() {
        let pat = if op.params.is_empty() {
            String::new()
        } else {
            " { .. }".to_owned()
        };
        s.line(format!("Self::{}{pat} => {i},", op.name));
    }
    s.close("}");
    s.close("}");
    s.blank();
    s.open("fn op_name(&self) -> &'static str {");
    s.line("self.into()");
    s.close("}");
    s.blank();
    s.open("fn encode_args(&self, w: &mut Writer) {");
    s.line("#[allow(unused_variables)]");
    s.open("match self {");
    for op in &side.iface.ops {
        if op.params.is_empty() {
            s.line(format!("Self::{} => {{}}", op.name));
            continue;
        }
        let names: Vec<String> = op.params.iter().map(|p| field_ident(&p.name)).collect();
        s.open(format!(
            "Self::{} {{ {} }} => {{",
            op.name,
            names.join(", ")
        ));
        for n in &names {
            s.line(format!("{n}.encode(w);"));
        }
        s.close("}");
    }
    s.close("}");
    s.close("}");
    s.blank();
    s.open("fn decode(op: u32, r: &mut Reader<'_>) -> Result<Self, WireError> {");
    s.open("Ok(match op {");
    for (i, op) in side.iface.ops.iter().enumerate() {
        if op.params.is_empty() {
            s.line(format!("{i} => Self::{},", op.name));
            continue;
        }
        s.open(format!("{i} => Self::{} {{", op.name));
        for p in &op.params {
            s.line(format!("{}: Decode::decode(r)?,", field_ident(&p.name)));
        }
        s.close("},");
    }
    s.line(format!(
        "_ => return Err(WireError::UnknownOp {{ interface: {:?}, op }}),",
        side.idl
    ));
    s.close("})");
    s.close("}");
    s.close("}");
    s.blank();
}

fn emit_handler(model: &Model, m: &Module, side: &Side<'_>, s: &mut Src) {
    let (msg, handler) = (side.message, side.handler);
    s.line(format!(
        "/// Async handler for [`{msg}`]. Every method defaults to a no-op."
    ));
    s.line("#[async_trait::async_trait]");
    s.open(format!("pub trait {handler}: Send {{"));
    for op in &side.iface.ops {
        docs(s, &op.docs);
        let fields = op_fields(model, m, op);
        let args: String = fields.iter().map(|(n, t)| format!(", {n}: {t}")).collect();
        let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
        s.open(format!(
            "async fn {}(&mut self{args}) -> anyhow::Result<()> {{",
            field_ident(&op.name)
        ));
        match names.as_slice() {
            [] => {}
            [one] => {
                s.line(format!("let _ = {one};"));
            }
            many => {
                s.line(format!("let _ = ({});", many.join(", ")));
            }
        }
        s.line("Ok(())");
        s.close("}");
    }
    s.close("}");
    s.blank();
    s.line("#[async_trait::async_trait]");
    s.open(format!(
        "impl<H: {handler} + ?Sized> crate::Dispatch<H> for {msg} {{"
    ));
    s.open("async fn dispatch(self, handler: &mut H) -> anyhow::Result<()> {");
    if side.iface.ops.is_empty() {
        s.line("let _ = handler;");
    }
    s.open("match self {");
    for op in &side.iface.ops {
        let names: Vec<String> = op.params.iter().map(|p| field_ident(&p.name)).collect();
        let pat = if names.is_empty() {
            String::new()
        } else {
            format!(" {{ {} }}", names.join(", "))
        };
        s.line(format!(
            "Self::{}{pat} => handler.{}({}).await,",
            op.name,
            field_ident(&op.name),
            names.join(", ")
        ));
    }
    s.close("}");
    s.close("}");
    s.close("}");
    s.blank();
}

fn emit_send_helpers(model: &Model, m: &Module, role: &str, sends: &Interface, s: &mut Src) {
    s.line(
        "/// Typed senders, one per outgoing operation. They encode straight from borrowed data.",
    );
    s.open(format!("impl crate::Endpoint<{role}> {{"));
    for (i, op) in sends.ops.iter().enumerate() {
        docs(s, &op.docs);
        let args: String = op
            .params
            .iter()
            .map(|p| format!(", {}: {}", field_ident(&p.name), arg_ty(model, m, &p.ty)))
            .collect();
        s.open(format!(
            "pub fn {}(&self{args}) -> crate::Result<()> {{",
            field_ident(&op.name)
        ));
        let mutw = if op.params.is_empty() { "" } else { "mut " };
        s.line(format!("let {mutw}w = Writer::default();"));
        for p in &op.params {
            s.line(format!("{}.encode(&mut w);", field_ident(&p.name)));
        }
        s.line(format!("self.send_raw({i}, w.as_bytes())"));
        s.close("}");
        s.blank();
    }
    s.close("}");
    s.blank();
}
