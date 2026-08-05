use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use game_core::{
    Currency, Equipment, Inventory, KnownSkills, Level, RuneInventory, Stats, UnspentSkillPoints,
    UnspentStatPoints,
};
use serde::{Deserialize, Serialize};

/// Per-game save data — currently just the shared password a connecting
/// client's `ConnectAuth` is checked against. There's no durable world
/// state to persist (see DESIGN.md's World & session structure section:
/// enemies simply respawn, the overworld remembers nothing), so this file
/// stays small on purpose.
#[derive(Debug, Serialize, Deserialize)]
pub struct GameSave {
    pub password: String,
}

/// Per-character save data. A character is scoped to one game (this game's
/// save directory), not portable across different games — see
/// DECISIONS.md's identity-model entry.
#[derive(Debug, Serialize, Deserialize)]
pub struct CharacterSave {
    pub password: String,
    pub level: Level,
    pub stats: Stats,
    pub points: UnspentStatPoints,
    pub known_skills: KnownSkills,
    pub skill_points: UnspentSkillPoints,
    pub inventory: Inventory,
    pub equipment: Equipment,
    pub runes: RuneInventory,
    pub currency: Currency,
}

/// Mirrors `content::ContentError`'s shape — this is the same kind of
/// file-I/O-plus-RON failure — with a `Serialize` variant added, since
/// unlike content templates, saves are written as well as read.
#[derive(Debug)]
pub enum SaveError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: Box<ron::error::SpannedError>,
    },
    Serialize {
        path: PathBuf,
        source: ron::Error,
    },
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::Io { path, source } => {
                write!(
                    f,
                    "failed to read/write save file {}: {source}",
                    path.display()
                )
            }
            SaveError::Parse { path, source } => {
                write!(f, "failed to parse save file {}: {source}", path.display())
            }
            SaveError::Serialize { path, source } => {
                write!(
                    f,
                    "failed to serialize save file {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SaveError::Io { source, .. } => Some(source),
            SaveError::Parse { source, .. } => Some(source),
            SaveError::Serialize { source, .. } => Some(source),
        }
    }
}

fn game_save_path(saves_dir: &Path, game_id: &str) -> PathBuf {
    saves_dir.join(game_id).join("game.ron")
}

fn character_save_path(saves_dir: &Path, game_id: &str, character_id: u128) -> PathBuf {
    saves_dir
        .join(game_id)
        .join("characters")
        .join(format!("{character_id}.ron"))
}

/// `Ok(None)` means the file doesn't exist yet — a brand-new game or
/// character, not an error. Anything else wrong with it is a real
/// `SaveError`, surfaced to the caller rather than silently defaulting.
fn load_ron<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, SaveError> {
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(path).map_err(|source| SaveError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    ron::from_str(&contents)
        .map(Some)
        .map_err(|source| SaveError::Parse {
            path: path.to_path_buf(),
            source: Box::new(source),
        })
}

fn save_ron<T: Serialize>(path: &Path, value: &T) -> Result<(), SaveError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SaveError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let text =
        ron::ser::to_string_pretty(value, ron::ser::PrettyConfig::default()).map_err(|source| {
            SaveError::Serialize {
                path: path.to_path_buf(),
                source,
            }
        })?;
    fs::write(path, text).map_err(|source| SaveError::Io {
        path: path.to_path_buf(),
        source,
    })
}

pub fn load_game_save(saves_dir: &Path, game_id: &str) -> Result<Option<GameSave>, SaveError> {
    load_ron(&game_save_path(saves_dir, game_id))
}

pub fn save_game_save(saves_dir: &Path, game_id: &str, save: &GameSave) -> Result<(), SaveError> {
    save_ron(&game_save_path(saves_dir, game_id), save)
}

pub fn load_character_save(
    saves_dir: &Path,
    game_id: &str,
    character_id: u128,
) -> Result<Option<CharacterSave>, SaveError> {
    load_ron(&character_save_path(saves_dir, game_id, character_id))
}

pub fn save_character_save(
    saves_dir: &Path,
    game_id: &str,
    character_id: u128,
    save: &CharacterSave,
) -> Result<(), SaveError> {
    save_ron(&character_save_path(saves_dir, game_id, character_id), save)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "vrekan_persistence_test_{}_{name}",
            std::process::id()
        ))
    }

    #[test]
    fn load_game_save_returns_none_when_file_is_missing() {
        let dir = temp_dir("missing_game");
        assert!(load_game_save(&dir, "some_game").unwrap().is_none());
    }

    #[test]
    fn game_save_round_trips_through_save_and_load() {
        let dir = temp_dir("game_round_trip");
        let save = GameSave {
            password: "hunter2".to_string(),
        };

        save_game_save(&dir, "some_game", &save).unwrap();
        let loaded = load_game_save(&dir, "some_game").unwrap().unwrap();

        assert_eq!(loaded.password, "hunter2");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn character_save_round_trips_through_save_and_load() {
        let dir = temp_dir("character_round_trip");
        let save = CharacterSave {
            password: "correct-horse".to_string(),
            level: Level { level: 3, xp: 42.0 },
            stats: Stats::default(),
            points: UnspentStatPoints(2),
            known_skills: KnownSkills::default(),
            skill_points: UnspentSkillPoints(1),
            inventory: Inventory::default(),
            equipment: Equipment::default(),
            runes: RuneInventory::default(),
            currency: Currency(50),
        };

        save_character_save(&dir, "some_game", 777, &save).unwrap();
        let loaded = load_character_save(&dir, "some_game", 777)
            .unwrap()
            .unwrap();

        assert_eq!(loaded.password, "correct-horse");
        assert_eq!(loaded.level, Level { level: 3, xp: 42.0 });
        assert_eq!(loaded.currency, Currency(50));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_reports_a_corrupt_file_as_a_parse_error_not_a_panic() {
        let dir = temp_dir("corrupt_game");
        fs::create_dir_all(dir.join("some_game")).unwrap();
        fs::write(dir.join("some_game").join("game.ron"), "not valid ron (").unwrap();

        let result = load_game_save(&dir, "some_game");

        assert!(matches!(result, Err(SaveError::Parse { .. })));
        fs::remove_dir_all(&dir).unwrap();
    }
}
