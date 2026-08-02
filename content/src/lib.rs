pub mod enemy;

pub use enemy::{
    load_all_enemy_templates, load_enemy_template, parse_enemy_template, spawn_enemy, EnemyTemplate,
};

use std::fmt;
use std::path::PathBuf;

/// Shared error type for loading any content file (enemy/item/spell
/// templates) — kept generic here rather than duplicated per content type.
#[derive(Debug)]
pub enum ContentError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: Box<ron::error::SpannedError>,
    },
}

impl fmt::Display for ContentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContentError::Io { path, source } => {
                write!(
                    f,
                    "failed to read content file {}: {source}",
                    path.display()
                )
            }
            ContentError::Parse { path, source } => {
                write!(
                    f,
                    "failed to parse content file {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ContentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ContentError::Io { source, .. } => Some(source),
            ContentError::Parse { source, .. } => Some(source),
        }
    }
}
