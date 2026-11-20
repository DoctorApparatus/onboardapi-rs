//! Emits the C++ shim (`extern "C"` vtables over the onboardapi C++ API) and the
//! matching Rust FFI declarations for `onboardapi-sys`.

use std::{collections::BTreeSet, path::Path};

use crate::{
    ast::Primitive,
    error::Result,
    model::{Interface, Model, Module, Ty, TypeKind},
    writer::{HEADER, Src, write_if_changed},
};

pub fn emit(model: &Model, dir: &Path, sys_rs: &Path) -> Result<()> {
    for m in model.modules.values() {
        write_if_changed(
            &dir.join(format!("wire_{}.hpp", m.short())),
            &emit_header(model, m),
        )?;
        write_if_changed(
            &dir.join(format!("{}.cpp", m.short())),
            &emit_source(model, m),
        )?;
    }
    write_if_changed(sys_rs, &emit_sys(model))
}

fn prim(p: Primitive) -> &'static str {
    match p {
        Primitive::Bool => "bool",
        Primitive::Octet => "uint8_t",
        Primitive::Char => "char",
        Primitive::I16 => "int16_t",
        Primitive::U16 => "uint16_t",
        Primitive::I32 => "int32_t",
        Primitive::U32 => "uint32_t",
        Primitive::I64 => "int64_t",
        Primitive::U64 => "uint64_t",
        Primitive::F32 => "float",
        Primitive::F64 => "double",
    }
}

/// Fully expanded C++ type, used for `static_assert`s against the SDK headers.
fn cpp_expanded(model: &Model, ty: &Ty) -> String {
    match ty {
        Ty::Prim(p) => prim(*p).to_owned(),
        Ty::String => "std::string".to_owned(),
        Ty::Seq(t) | Ty::Opt(t) => format!("std::vector<{}>", cpp_expanded(model, t)),
        Ty::Ref(p) => match &model.type_def(p).kind {
            TypeKind::Alias { ty } => cpp_expanded(model, ty),
            _ => format!("::{}", p.join("::")),
        },
    }
}

/// C++ type as spelled in the SDK headers (typedef names kept).
fn cpp_named(ty: &Ty) -> String {
    match ty {
        Ty::Prim(p) => prim(*p).to_owned(),
        Ty::String => "std::string".to_owned(),
        Ty::Seq(t) | Ty::Opt(t) => format!("std::vector<{}>", cpp_named(t)),
        Ty::Ref(p) => format!("::{}", p.join("::")),
    }
}

fn param_decl(model: &Model, ty: &Ty, name: &str) -> String {
    if model.is_by_value(ty) {
        format!("{} {name}", cpp_named(ty))
    } else {
        format!("{} const& {name}", cpp_named(ty))
    }
}

/// Modules whose struct codecs are referenced by `ty`.
fn deps_of(model: &Model, ty: &Ty, out: &mut BTreeSet<String>) {
    match ty {
        Ty::Prim(_) | Ty::String => {}
        Ty::Seq(t) | Ty::Opt(t) => deps_of(model, t, out),
        Ty::Ref(p) => {
            out.insert(p[0].clone());
            if let TypeKind::Alias { ty } = &model.type_def(p).kind {
                deps_of(model, ty, out);
            }
        }
    }
}

fn module_deps(model: &Model, m: &Module) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for t in &m.types {
        match &t.kind {
            TypeKind::Struct { fields } => {
                fields.iter().for_each(|f| deps_of(model, &f.ty, &mut out))
            }
            TypeKind::Alias { ty } => deps_of(model, ty, &mut out),
            TypeKind::Enum { .. } => {}
        }
    }
    for iface in m.service.iter().chain(m.client.iter()) {
        for op in &iface.ops {
            op.params
                .iter()
                .for_each(|p| deps_of(model, &p.ty, &mut out));
        }
    }
    out.remove(&m.name);
    out
}

fn sdk_header(m: &Module) -> String {
    if m.has_api() {
        format!("<onboardapi/api/{}.hpp>", m.short())
    } else {
        format!("<onboardapi/type/{}.hpp>", m.short())
    }
}

fn structs(
    m: &Module,
) -> impl Iterator<Item = (&crate::model::TypeDef, &Vec<crate::model::Member>)> {
    m.types.iter().filter_map(|t| match &t.kind {
        TypeKind::Struct { fields } => Some((t, fields)),
        _ => None,
    })
}

fn emit_header(model: &Model, m: &Module) -> String {
    let mut s = Src::default();
    s.line(HEADER);
    s.line("#pragma once");
    s.blank();
    s.line("#include \"../obapi_wire.hpp\"");
    s.line(format!("#include {}", sdk_header(m)));
    for dep in module_deps(model, m) {
        s.line(format!(
            "#include \"wire_{}.hpp\"",
            dep.strip_prefix("M_").unwrap_or(&dep)
        ));
    }
    s.blank();
    s.open(format!("namespace {} {{", m.name));
    for (t, _) in structs(m) {
        let ty = t.path[1..].join("::");
        s.line(format!(
            "void obapi_put(::obapi::Writer& w, {ty} const& v);"
        ));
        s.line(format!("void obapi_get(::obapi::Reader& r, {ty}& v);"));
    }
    s.close(format!("}} // namespace {}", m.name));
    s.finish()
}

fn emit_source(model: &Model, m: &Module) -> String {
    let mut s = Src::default();
    s.line(HEADER);
    s.line(format!("#include \"wire_{}.hpp\"", m.short()));
    s.line("#include \"../obapi_runtime.hpp\"");
    s.blank();
    s.line("#include <type_traits>");
    s.blank();

    // Layout checks: the Rust side was generated from the .rmodel files, the SDK
    // headers are the ground truth. Any drift fails the build here.
    for t in &m.types {
        let full = format!("::{}", t.path.join("::"));
        match &t.kind {
            TypeKind::Struct { fields } => {
                for f in fields {
                    s.line(format!(
                        "static_assert(std::is_same<decltype({full}::{}), {}>::value, \"{} field {} differs from the data model\");",
                        f.name,
                        cpp_expanded(model, &f.ty),
                        t.path.join("::"),
                        f.name
                    ));
                }
            }
            TypeKind::Alias { ty } => {
                s.line(format!(
                    "static_assert(std::is_same<{full}, {}>::value, \"typedef {} differs from the data model\");",
                    cpp_expanded(model, ty),
                    t.path.join("::")
                ));
            }
            TypeKind::Enum { variants } => {
                let enum_scope = t.path[..t.path.len() - 1].join("::");
                for (i, (_, v)) in variants.iter().enumerate() {
                    s.line(format!(
                        "static_assert(static_cast<uint32_t>(::{enum_scope}::{v}) == {i}u, \"enum value {}::{v} differs from the data model\");",
                        t.path.join("::")
                    ));
                }
            }
        }
    }
    s.blank();

    s.open(format!("namespace {} {{", m.name));
    for (t, fields) in structs(m) {
        let ty = t.path[1..].join("::");
        let (w, r) = if fields.is_empty() {
            ("", "")
        } else {
            (" w", " r")
        };
        let v = if fields.is_empty() { "" } else { " v" };
        s.open(format!(
            "void obapi_put(::obapi::Writer&{w}, {ty} const&{v}) {{"
        ));
        for f in fields {
            s.line(format!("::obapi::put(w, v.{});", f.name));
        }
        s.close("}");
        s.open(format!("void obapi_get(::obapi::Reader&{r}, {ty}&{v}) {{"));
        for f in fields {
            s.line(format!("::obapi::get(r, v.{});", f.name));
        }
        s.close("}");
    }
    s.close(format!("}} // namespace {}", m.name));

    if let (Some(svc), Some(cli)) = (&m.service, &m.client) {
        s.blank();
        s.open("namespace {");
        emit_callbacks(model, m, "ServiceCallbacks", "IService", svc, &mut s);
        emit_callbacks(model, m, "ClientCallbacks", "IClient", cli, &mut s);
        emit_sender(
            "service_send",
            &format!("::{}::Service", m.name),
            "ServiceCallbacks",
            cli,
            &mut s,
        );
        emit_sender(
            "client_send",
            &format!("::{}::Client", m.name),
            "ClientCallbacks",
            svc,
            &mut s,
        );
        s.close("} // namespace");
        s.blank();
        let short = m.short();
        s.line(format!(
            "extern \"C\" const obapi_vtable obapi_{short}_service_vtable = ::obapi::Endpoint<::{0}::Service, ServiceCallbacks, &service_send, false>::vtable(\"{0}\", \"Service\");",
            m.name
        ));
        s.line(format!(
            "extern \"C\" const obapi_vtable obapi_{short}_client_vtable = ::obapi::Endpoint<::{0}::Client, ClientCallbacks, &client_send, true>::vtable(\"{0}\", \"Client\");",
            m.name
        ));
    }
    s.finish()
}

/// Implements the C++ callback interface by encoding the arguments and handing them to Rust.
fn emit_callbacks(
    model: &Model,
    m: &Module,
    name: &str,
    iface_name: &str,
    iface: &Interface,
    s: &mut Src,
) {
    s.open(format!(
        "struct {name} final : ::{}::{iface_name} {{",
        m.name
    ));
    s.line("::obapi::Dispatcher dispatcher;");
    for (i, op) in iface.ops.iter().enumerate() {
        let params: Vec<String> = op
            .params
            .iter()
            .map(|p| param_decl(model, &p.ty, &p.name))
            .collect();
        s.open(format!(
            "void {}({}) override {{",
            op.name,
            params.join(", ")
        ));
        s.line("::obapi::Writer w;");
        for p in &op.params {
            s.line(format!("::obapi::put(w, {});", p.name));
        }
        s.line(format!("dispatcher.dispatch({i}u, w);"));
        s.close("}");
    }
    s.close("};");
    s.blank();
}

/// Decodes an outgoing operation coming from Rust and invokes it on the endpoint.
fn emit_sender(fn_name: &str, endpoint: &str, callbacks: &str, iface: &Interface, s: &mut Src) {
    s.open(format!(
        "void {fn_name}(::obapi::Holder<{endpoint}, {callbacks}>& h, uint32_t op, ::obapi::Reader& r) {{"
    ));
    s.open("switch (op) {");
    for (i, op) in iface.ops.iter().enumerate() {
        s.open(format!("case {i}u: {{"));
        for p in &op.params {
            s.line(format!("{} {}{{}};", cpp_named(&p.ty), p.name));
            s.line(format!("::obapi::get(r, {});", p.name));
        }
        s.line("r.finish();");
        let args: Vec<&str> = op.params.iter().map(|p| p.name.as_str()).collect();
        s.line(format!("h.endpoint->{}({});", op.name, args.join(", ")));
        s.line("return;");
        s.close("}");
    }
    s.line("default: throw ::obapi::UnknownOp(op);");
    s.close("}");
    s.close("}");
    s.blank();
}

fn emit_sys(model: &Model) -> String {
    let mut s = Src::default();
    s.line(HEADER);
    s.line("use crate::obapi_vtable;");
    s.blank();
    s.open("unsafe extern \"C\" {");
    for m in model.modules.values().filter(|m| m.has_api()) {
        for kind in ["service", "client"] {
            s.line(format!(
                "pub static obapi_{}_{kind}_vtable: obapi_vtable;",
                m.short()
            ));
        }
    }
    s.close("}");
    s.blank();
    s.line("/// IDL modules that have a service/client API.");
    s.open("pub const API_MODULES: &[&str] = &[");
    for m in model.modules.values().filter(|m| m.has_api()) {
        s.line(format!("{:?},", m.name));
    }
    s.close("];");
    s.finish()
}
