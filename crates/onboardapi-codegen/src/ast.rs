//! Syntax tree of a `.rmodel` file (an OMG IDL subset).

/// A (possibly) scoped name such as `M_Common::PositionType`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScopedName(pub Vec<String>);

impl std::fmt::Display for ScopedName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.join("::"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Primitive {
    Bool,
    Octet,
    Char,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeSpec {
    Primitive(Primitive),
    String,
    Sequence {
        element: Box<TypeSpec>,
        bound: Option<u32>,
    },
    Named(ScopedName),
}

#[derive(Debug, Clone)]
pub struct Field {
    pub docs: Vec<String>,
    pub name: String,
    pub ty: TypeSpec,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub docs: Vec<String>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub docs: Vec<String>,
    pub name: String,
    pub ty: TypeSpec,
}

#[derive(Debug, Clone)]
pub struct Operation {
    pub docs: Vec<String>,
    pub name: String,
    pub params: Vec<Param>,
}

#[derive(Debug, Clone)]
pub enum Decl {
    Module {
        docs: Vec<String>,
        name: String,
        decls: Vec<Decl>,
    },
    Struct {
        docs: Vec<String>,
        name: String,
        fields: Vec<Field>,
    },
    Enum {
        docs: Vec<String>,
        name: String,
        variants: Vec<Variant>,
    },
    Typedef {
        docs: Vec<String>,
        name: String,
        ty: TypeSpec,
    },
    Const {
        docs: Vec<String>,
        name: String,
        ty: TypeSpec,
        value: ConstValue,
    },
    Interface {
        docs: Vec<String>,
        name: String,
        bases: Vec<ScopedName>,
        ops: Vec<Operation>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    String(String),
    Number(String),
}
