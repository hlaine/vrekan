use std::path::Path;

use game_core::InteractableDefinition;
use serde::Deserialize;

use crate::enemy::EffectTemplate;
use crate::item::load_all_ron;
use crate::ContentError;

/// Data-driven shape for an interactable (NPC or world object) — see
/// `game_core::Interactable`'s doc comment for the overall design. `range`
/// travels with the replicated `Interactable` component itself (not looked
/// up from this template at interaction time), but lives here too since a
/// content author picks it per-template just like every other field.
/// `dialog`/`opens_panels` are read client-side only; `effect`, if any, is
/// resolved server-side into `game_core::InteractableDefinition` via
/// `into_definition`. `opens_panels` is a list, not a single value — one
/// NPC can offer more than one capability (e.g. a blacksmith is both
/// `"forging"` and `"vendor"`). `grants_rune` (M8.10) is a direct rune
/// grant (id of a rune template) bypassing the rune-casting panel — see
/// `game_core::InteractableDefinition`'s doc comment.
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct InteractableTemplate {
    pub range: f32,
    #[serde(default)]
    pub dialog: Option<String>,
    #[serde(default)]
    pub effect: Option<EffectTemplate>,
    #[serde(default)]
    pub opens_panels: Vec<String>,
    #[serde(default)]
    pub grants_rune: Option<String>,
}

impl InteractableTemplate {
    pub fn into_definition(self) -> InteractableDefinition {
        InteractableDefinition {
            effect: self.effect.map(EffectTemplate::into_definition),
            opens_panels: self.opens_panels,
            grants_rune: self.grants_rune,
        }
    }
}

pub fn parse_interactable_template(
    ron_str: &str,
) -> ron::error::SpannedResult<InteractableTemplate> {
    ron::from_str(ron_str)
}

pub fn load_interactable_template(path: &Path) -> Result<InteractableTemplate, ContentError> {
    let contents = std::fs::read_to_string(path).map_err(|source| ContentError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_interactable_template(&contents).map_err(|source| ContentError::Parse {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

/// Loads every `.ron` file directly inside `dir` as an `InteractableTemplate`,
/// keyed by filename (without extension) — same load-all shape as
/// `load_all_item_templates`, reusing its shared helper.
pub fn load_all_interactable_templates(
    dir: &Path,
) -> Result<Vec<(String, InteractableTemplate)>, ContentError> {
    load_all_ron(dir, load_interactable_template)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn real_interactable_templates_load_successfully() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/interactables");
        load_all_interactable_templates(&dir).unwrap();
    }

    #[test]
    fn parses_a_minimal_template_with_all_optional_fields_omitted() {
        let template = parse_interactable_template("(range: 60.0)").unwrap();

        assert_eq!(template.range, 60.0);
        assert_eq!(template.dialog, None);
        assert_eq!(template.effect, None);
        assert!(template.opens_panels.is_empty());
    }

    #[test]
    fn parses_a_full_template_with_dialog_and_opens_panels() {
        let template = parse_interactable_template(
            r#"(
                range: 60.0,
                dialog: Some("An ancient rune hums with power."),
                opens_panels: ["forging", "vendor"],
            )"#,
        )
        .unwrap();

        assert_eq!(
            template.dialog,
            Some("An ancient rune hums with power.".to_string())
        );
        assert_eq!(
            template.opens_panels,
            vec!["forging".to_string(), "vendor".to_string()]
        );
    }

    #[test]
    fn rejects_malformed_ron_syntax() {
        assert!(parse_interactable_template("(range: 60.0,").is_err());
    }

    #[test]
    fn into_definition_converts_the_effect_and_carries_opens_panels_through() {
        let template = InteractableTemplate {
            range: 60.0,
            dialog: None,
            effect: None,
            opens_panels: vec!["forging".to_string(), "vendor".to_string()],
            grants_rune: None,
        };

        let definition = template.into_definition();

        assert_eq!(definition.effect, None);
        assert_eq!(
            definition.opens_panels,
            vec!["forging".to_string(), "vendor".to_string()]
        );
    }

    #[test]
    fn load_all_interactable_templates_reads_every_ron_file_sorted_by_name() {
        let dir = std::env::temp_dir().join(format!(
            "vrekan_content_test_{}_{}",
            std::process::id(),
            "interactable_load_all_sorted"
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("b_blacksmith.ron"), "(range: 60.0)").unwrap();
        fs::write(dir.join("a_runestone.ron"), "(range: 40.0)").unwrap();

        let templates = load_all_interactable_templates(&dir).unwrap();

        assert_eq!(templates.len(), 2);
        assert_eq!(templates[0].0, "a_runestone");
        assert_eq!(templates[1].0, "b_blacksmith");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_all_interactable_templates_fails_loudly_on_first_malformed_file() {
        let dir = std::env::temp_dir().join(format!(
            "vrekan_content_test_{}_{}",
            std::process::id(),
            "interactable_load_all_malformed"
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("broken.ron"), "(range: not_a_number)").unwrap();

        let result = load_all_interactable_templates(&dir);

        assert!(matches!(result, Err(ContentError::Parse { .. })));

        fs::remove_dir_all(&dir).unwrap();
    }
}
