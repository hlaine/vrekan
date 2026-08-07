pub mod destructible;
pub mod economy;
pub mod enemy;
pub mod interact;
pub mod item;
pub mod skill;

pub use destructible::{
    load_all_destructible_templates, load_destructible_template, parse_destructible_template,
    spawn_destructible, DestructibleTemplate,
};
pub use economy::{
    load_all_vendor_templates, load_vendor_template, parse_vendor_template, VendorListingTemplate,
    VendorTemplate,
};
pub use enemy::{
    load_all_enemy_templates, load_enemy_template, parse_enemy_template, spawn_enemy,
    EnemyTemplate, LootEntryTemplate, LootKindTemplate,
};
pub use interact::{
    load_all_interactable_templates, load_interactable_template, parse_interactable_template,
    InteractableTemplate,
};
pub use item::{
    load_all_item_templates, load_all_rune_templates, load_item_template, load_rune_template,
    parse_item_template, parse_rune_template, ItemTemplate, RuneTemplate,
};
pub use skill::{
    load_all_skill_templates, load_skill_template, parse_skill_template, SkillKindTemplate,
    SkillTemplate,
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
    /// The file parsed as valid RON but violates a cross-field schema rule
    /// (e.g. a `Weapon`-slot item with no `weapon` stats) — distinct from
    /// `Parse`, which is a RON *syntax* error. Same "malformed content fails
    /// loudly" convention, just a semantic check `ron::from_str` itself
    /// can't express.
    Validation { path: PathBuf, message: String },
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
            ContentError::Validation { path, message } => {
                write!(f, "invalid content file {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for ContentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ContentError::Io { source, .. } => Some(source),
            ContentError::Parse { source, .. } => Some(source),
            ContentError::Validation { .. } => None,
        }
    }
}
