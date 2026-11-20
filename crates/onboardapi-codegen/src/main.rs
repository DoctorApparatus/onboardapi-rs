//! `onboardapi-codegen`: reads the `.rmodel` data model and writes
//!
//! * `crates/onboardapi/src/generated/*.rs`: Rust types, messages and handler traits
//! * `crates/onboardapi-sys/shim/generated/*.cpp`: `extern "C"` shim over the C++ API
//! * `crates/onboardapi-sys/src/generated.rs`: the matching FFI declarations

mod ast;
mod emit_cpp;
mod emit_rust;
mod error;
mod lexer;
mod model;
mod parser;
mod writer;

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use tracing::info;

#[derive(Debug, Parser)]
#[command(about = "Generate the onboardapi Rust wrapper from the .rmodel data model")]
struct Args {
    /// Directory containing the `.rmodel` files.
    #[arg(long, default_value = "datamodel")]
    datamodel: PathBuf,

    /// Workspace root; output paths are relative to it.
    #[arg(long, default_value = ".")]
    workspace: PathBuf,

    /// Only parse and print a summary.
    #[arg(long)]
    check: bool,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();

    let mut entries: Vec<_> = std::fs::read_dir(&args.datamodel)
        .with_context(|| format!("reading {}", args.datamodel.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "rmodel"))
        .collect();
    entries.sort();

    let mut files = Vec::new();
    for path in &entries {
        let src = std::fs::read_to_string(path)?;
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let decls = parser::parse(&name, &src)?;
        files.push((name, decls));
    }
    let model = model::build(files)?;

    let api = model.modules.values().filter(|m| m.has_api()).count();
    let types: usize = model.modules.values().map(|m| m.types.len()).sum();
    let ops: usize = model
        .modules
        .values()
        .flat_map(|m| m.service.iter().chain(m.client.iter()))
        .map(|i| i.ops.len())
        .sum();
    info!(
        files = entries.len(),
        modules = model.modules.len(),
        api,
        types,
        ops,
        "parsed data model"
    );

    if args.check {
        return Ok(());
    }

    let rust_dir = args.workspace.join("crates/onboardapi/src/generated");
    let cpp_dir = args.workspace.join("crates/onboardapi-sys/shim/generated");
    let sys_rs = args
        .workspace
        .join("crates/onboardapi-sys/src/generated.rs");

    emit_rust::emit(&model, &rust_dir)?;
    emit_cpp::emit(&model, &cpp_dir, &sys_rs)?;
    info!(rust = %rust_dir.display(), cpp = %cpp_dir.display(), "generated wrapper");
    Ok(())
}
