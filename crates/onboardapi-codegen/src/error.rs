use thiserror::Error;

#[derive(Debug, Error)]
pub enum CodegenError {
    #[error("{file}:{line}: {msg}")]
    Syntax {
        file: String,
        line: usize,
        msg: String,
    },

    #[error("unresolved type `{name}` in scope `{scope}`")]
    Unresolved { name: String, scope: String },

    #[error("unsupported construct: {0}")]
    Unsupported(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = CodegenError> = std::result::Result<T, E>;
