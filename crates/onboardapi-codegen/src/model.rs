//! Resolved, flattened view of all `.rmodel` files.
//!
//! Top-level IDL modules (`M_CoordinateFrame`, ...) become [`Module`]s. Nested modules
//! (the `module XKind { enum Type {...} }` idiom) are folded into their top-level module,
//! with the enum named after the nested module.

use std::collections::{BTreeMap, HashMap};

use heck::ToSnakeCase;

use crate::{
    ast::{ConstValue, Decl, Primitive, ScopedName, TypeSpec},
    error::{CodegenError, Result},
};

pub type Path = Vec<String>;

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Prim(Primitive),
    String,
    Seq(Box<Ty>),
    Opt(Box<Ty>),
    Ref(Path),
}

#[derive(Debug, Clone)]
pub struct Member {
    pub docs: Vec<String>,
    pub name: String,
    pub ty: Ty,
}

#[derive(Debug, Clone)]
pub enum TypeKind {
    Struct {
        fields: Vec<Member>,
    },
    Enum {
        variants: Vec<(Vec<String>, String)>,
    },
    Alias {
        ty: Ty,
    },
}

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub path: Path,
    pub rust_name: String,
    pub docs: Vec<String>,
    pub kind: TypeKind,
}

#[derive(Debug, Clone)]
pub struct Op {
    pub docs: Vec<String>,
    pub name: String,
    pub params: Vec<Member>,
}

#[derive(Debug, Clone, Default)]
pub struct Interface {
    pub docs: Vec<String>,
    /// Fully qualified C++ names of the direct base interfaces.
    pub bases: Vec<Path>,
    /// All operations, inherited ones first. The index is the wire operation id.
    pub ops: Vec<Op>,
}

#[derive(Debug, Clone)]
pub struct Const {
    pub docs: Vec<String>,
    pub name: String,
    pub ty: Ty,
    pub value: ConstValue,
}

#[derive(Debug, Clone, Default)]
pub struct Module {
    /// IDL name, e.g. `M_CoordinateFrame`.
    pub name: String,
    pub docs: Vec<String>,
    pub consts: Vec<Const>,
    pub types: Vec<TypeDef>,
    pub service: Option<Interface>,
    pub client: Option<Interface>,
}

impl Module {
    /// `M_CoordinateFrame` -> `CoordinateFrame`
    pub fn short(&self) -> &str {
        self.name.strip_prefix("M_").unwrap_or(&self.name)
    }

    /// `M_CoordinateFrame` -> `coordinate_frame`
    pub fn rust_mod(&self) -> String {
        rust_mod_name(&self.name)
    }

    pub fn has_api(&self) -> bool {
        self.service.is_some() && self.client.is_some()
    }
}

pub fn rust_mod_name(idl: &str) -> String {
    idl.strip_prefix("M_").unwrap_or(idl).to_snake_case()
}

#[derive(Debug, Default)]
pub struct Model {
    pub modules: BTreeMap<String, Module>,
    types: HashMap<Path, TypeDef>,
}

impl Model {
    pub fn type_def(&self, path: &Path) -> &TypeDef {
        &self.types[path]
    }

    /// Follows aliases until a non-alias type is reached.
    pub fn resolve<'a>(&'a self, ty: &'a Ty) -> &'a Ty {
        match ty {
            Ty::Ref(p) => match &self.types[p].kind {
                TypeKind::Alias { ty } => self.resolve(ty),
                _ => ty,
            },
            _ => ty,
        }
    }

    /// Primitives and enums are passed by value in the C++ API, everything else by `const&`.
    pub fn is_by_value(&self, ty: &Ty) -> bool {
        match self.resolve(ty) {
            Ty::Prim(_) => true,
            Ty::Ref(p) => matches!(self.types[p].kind, TypeKind::Enum { .. }),
            _ => false,
        }
    }

    /// `Copy` on the Rust side (primitives and enums).
    pub fn is_copy(&self, ty: &Ty) -> bool {
        self.is_by_value(ty)
    }

    pub fn is_string(&self, ty: &Ty) -> bool {
        matches!(self.resolve(ty), Ty::String)
    }

    /// True if the type (transitively) contains a floating point number, so it cannot be `Eq`.
    pub fn contains_float(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Prim(p) => matches!(p, Primitive::F32 | Primitive::F64),
            Ty::String => false,
            Ty::Seq(t) | Ty::Opt(t) => self.contains_float(t),
            Ty::Ref(p) => match &self.types[p].kind {
                TypeKind::Alias { ty } => self.contains_float(ty),
                TypeKind::Enum { .. } => false,
                TypeKind::Struct { fields } => fields.iter().any(|f| self.contains_float(&f.ty)),
            },
        }
    }
}

/// Builds the model from `(file name, declarations)` pairs.
pub fn build(files: Vec<(String, Vec<Decl>)>) -> Result<Model> {
    let mut b = Builder::default();
    for (_, decls) in &files {
        b.collect(&[], decls)?;
    }
    for (_, decls) in &files {
        b.lower(&[], decls)?;
    }
    b.finish()
}

/// Interface as declared, before inherited operations are flattened in.
#[derive(Clone)]
struct RawInterface {
    docs: Vec<String>,
    bases: Vec<Path>,
    ops: Vec<Op>,
}

#[derive(Default)]
struct Builder {
    /// Every named type, keyed by its full IDL path.
    names: HashMap<Path, ()>,
    interfaces_raw: HashMap<Path, RawInterface>,
    model: Model,
}

impl Builder {
    fn collect(&mut self, scope: &[String], decls: &[Decl]) -> Result<()> {
        for d in decls {
            match d {
                Decl::Module { name, decls, .. } => {
                    let mut s = scope.to_vec();
                    s.push(name.clone());
                    self.collect(&s, decls)?;
                }
                Decl::Struct { name, .. }
                | Decl::Enum { name, .. }
                | Decl::Typedef { name, .. } => {
                    let mut p = scope.to_vec();
                    p.push(name.clone());
                    self.names.insert(p, ());
                }
                Decl::Const { .. } | Decl::Interface { .. } => {}
            }
        }
        Ok(())
    }

    fn lookup(&self, scope: &[String], name: &ScopedName) -> Result<Path> {
        for i in (0..=scope.len()).rev() {
            let mut cand = scope[..i].to_vec();
            cand.extend(name.0.iter().cloned());
            if self.names.contains_key(&cand) {
                return Ok(cand);
            }
        }
        Err(CodegenError::Unresolved {
            name: name.to_string(),
            scope: scope.join("::"),
        })
    }

    fn ty(&self, scope: &[String], spec: &TypeSpec) -> Result<Ty> {
        Ok(match spec {
            TypeSpec::Primitive(p) => Ty::Prim(*p),
            TypeSpec::String => Ty::String,
            TypeSpec::Sequence {
                element,
                bound: Some(1),
            } => Ty::Opt(Box::new(self.ty(scope, element)?)),
            TypeSpec::Sequence { element, .. } => Ty::Seq(Box::new(self.ty(scope, element)?)),
            TypeSpec::Named(n) => Ty::Ref(self.lookup(scope, n)?),
        })
    }

    fn module(&mut self, scope: &[String]) -> &mut Module {
        let top = scope[0].clone();
        self.model
            .modules
            .entry(top.clone())
            .or_insert_with(|| Module {
                name: top,
                ..Default::default()
            })
    }

    fn lower(&mut self, scope: &[String], decls: &[Decl]) -> Result<()> {
        for d in decls {
            match d {
                Decl::Module { name, decls, docs } => {
                    let mut s = scope.to_vec();
                    s.push(name.clone());
                    if scope.is_empty() {
                        let m = self.module(&s);
                        if m.docs.is_empty() {
                            m.docs = docs.clone();
                        }
                    }
                    self.lower(&s, decls)?;
                }
                Decl::Struct { docs, name, fields } => {
                    let fields = fields
                        .iter()
                        .map(|f| {
                            Ok(Member {
                                docs: f.docs.clone(),
                                name: f.name.clone(),
                                ty: self.ty(scope, &f.ty)?,
                            })
                        })
                        .collect::<Result<_>>()?;
                    self.add_type(scope, name, docs, TypeKind::Struct { fields })?;
                }
                Decl::Enum {
                    docs,
                    name,
                    variants,
                } => {
                    let variants = variants
                        .iter()
                        .map(|v| (v.docs.clone(), v.name.clone()))
                        .collect();
                    self.add_type(scope, name, docs, TypeKind::Enum { variants })?;
                }
                Decl::Typedef { docs, name, ty } => {
                    let ty = self.ty(scope, ty)?;
                    self.add_type(scope, name, docs, TypeKind::Alias { ty })?;
                }
                Decl::Const {
                    docs,
                    name,
                    ty,
                    value,
                } => {
                    if scope.len() != 1 {
                        return Err(CodegenError::Unsupported(format!("nested const {name}")));
                    }
                    let ty = self.ty(scope, ty)?;
                    let c = Const {
                        docs: docs.clone(),
                        name: name.clone(),
                        ty,
                        value: value.clone(),
                    };
                    self.module(scope).consts.push(c);
                }
                Decl::Interface {
                    docs,
                    name,
                    bases,
                    ops,
                } => {
                    if scope.len() != 1 {
                        return Err(CodegenError::Unsupported(format!(
                            "nested interface {name}"
                        )));
                    }
                    let bases = bases
                        .iter()
                        .map(|b| {
                            let mut full = if b.0.len() == 1 {
                                scope.to_vec()
                            } else {
                                vec![]
                            };
                            full.extend(b.0.iter().cloned());
                            full
                        })
                        .collect();
                    let ops = ops
                        .iter()
                        .map(|o| {
                            let params = o
                                .params
                                .iter()
                                .map(|p| {
                                    Ok(Member {
                                        docs: p.docs.clone(),
                                        name: p.name.clone(),
                                        ty: self.ty(scope, &p.ty)?,
                                    })
                                })
                                .collect::<Result<_>>()?;
                            Ok(Op {
                                docs: o.docs.clone(),
                                name: o.name.clone(),
                                params,
                            })
                        })
                        .collect::<Result<_>>()?;
                    let mut p = scope.to_vec();
                    p.push(name.clone());
                    self.interfaces_raw.insert(
                        p,
                        RawInterface {
                            docs: docs.clone(),
                            bases,
                            ops,
                        },
                    );
                }
            }
        }
        Ok(())
    }

    fn add_type(
        &mut self,
        scope: &[String],
        name: &str,
        docs: &[String],
        kind: TypeKind,
    ) -> Result<()> {
        let mut path = scope.to_vec();
        path.push(name.to_owned());
        if scope.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "type {name} outside of a module"
            )));
        }
        let mut parts: Vec<&str> = path[1..].iter().map(String::as_str).collect();
        if parts.len() > 1 && parts.last() == Some(&"Type") {
            parts.pop();
        }
        let rust_name = parts.concat();
        let def = TypeDef {
            path: path.clone(),
            rust_name,
            docs: docs.to_vec(),
            kind,
        };
        self.module(scope).types.push(def.clone());
        self.model.types.insert(path, def);
        Ok(())
    }

    fn flatten(&self, path: &Path, out: &mut Vec<Op>) -> Result<()> {
        let raw = self
            .interfaces_raw
            .get(path)
            .ok_or_else(|| CodegenError::Unresolved {
                name: path.join("::"),
                scope: "interfaces".into(),
            })?;
        for b in &raw.bases {
            self.flatten(b, out)?;
        }
        for op in &raw.ops {
            if !out.iter().any(|o| o.name == op.name) {
                out.push(op.clone());
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Model> {
        let mut paths: Vec<_> = self.interfaces_raw.keys().cloned().collect();
        paths.sort();
        for path in paths {
            let mut ops = Vec::new();
            self.flatten(&path, &mut ops)?;
            let RawInterface { docs, bases, .. } = self.interfaces_raw[&path].clone();
            let iface = Interface { docs, bases, ops };
            let m = self.model.modules.get_mut(&path[0]).expect("module exists");
            match path[1].as_str() {
                "IService" => m.service = Some(iface),
                "IClient" => m.client = Some(iface),
                other => return Err(CodegenError::Unsupported(format!("interface name {other}"))),
            }
        }
        // Detect Rust name clashes inside a module.
        for m in self.model.modules.values() {
            let mut seen = HashMap::new();
            for t in &m.types {
                if let Some(prev) = seen.insert(t.rust_name.clone(), t.path.join("::")) {
                    return Err(CodegenError::Unsupported(format!(
                        "name clash in {}: {} and {}",
                        m.name,
                        prev,
                        t.path.join("::")
                    )));
                }
            }
        }
        Ok(self.model)
    }
}
