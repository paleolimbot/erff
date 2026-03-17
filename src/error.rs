use thiserror::Error;

#[derive(Error, Debug)]
pub enum ErffError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid magic bytes — not an ERFF file")]
    InvalidMagic,

    #[error("Unsupported version {major}.{minor}")]
    UnsupportedVersion { major: u8, minor: u8 },

    #[error("Invalid geometry type: {0}")]
    InvalidGeometryType(u8),

    #[error("Invalid coordinate type: {0}")]
    InvalidCoordType(u8),

    #[error("Invalid column type: {0}")]
    InvalidColumnType(u8),

    #[error("Schema mismatch: expected {expected} columns, got {got}")]
    SchemaMismatch { expected: usize, got: usize },

    #[error("Invalid WKB: {0}")]
    InvalidWkb(String),

    #[error("Feature index {0} out of range (count: {1})")]
    FeatureOutOfRange(u64, u64),

    #[error("No spatial index present")]
    NoSpatialIndex,

    #[error("Invalid UTF-8 in string field")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),

    #[error("Writer already finalized")]
    AlreadyFinalized,
}

pub type Result<T> = std::result::Result<T, ErffError>;
