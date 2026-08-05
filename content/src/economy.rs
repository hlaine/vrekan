use std::path::Path;

use game_core::VendorListing;
use serde::Deserialize;

use crate::item::load_all_ron;
use crate::ContentError;

/// RON-facing mirror of `game_core::VendorListing` — kept separate for the
/// same reason `LootEntryTemplate` mirrors `LootEntry`: content authors
/// refer to items by their template key (a plain `String`), not an
/// already-resolved engine type.
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct VendorListingTemplate {
    pub item_template_key: String,
    pub price: u32,
}

impl VendorListingTemplate {
    fn into_listing(self) -> VendorListing {
        VendorListing {
            item_template_key: self.item_template_key,
            price: self.price,
        }
    }
}

/// Data-driven shape for a vendor's buyable stock, keyed by the same
/// `template_key` as its `Interactable` placement (see
/// `game_core::VendorLibrary`'s doc comment) — a separate content
/// directory/resource from `InteractableTemplate`, since not every
/// `Interactable` is a vendor.
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct VendorTemplate {
    pub inventory: Vec<VendorListingTemplate>,
}

impl VendorTemplate {
    pub fn into_listings(self) -> Vec<VendorListing> {
        self.inventory
            .into_iter()
            .map(VendorListingTemplate::into_listing)
            .collect()
    }
}

pub fn parse_vendor_template(ron_str: &str) -> ron::error::SpannedResult<VendorTemplate> {
    ron::from_str(ron_str)
}

pub fn load_vendor_template(path: &Path) -> Result<VendorTemplate, ContentError> {
    let contents = std::fs::read_to_string(path).map_err(|source| ContentError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_vendor_template(&contents).map_err(|source| ContentError::Parse {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

/// Loads every `.ron` file directly inside `dir` as a `VendorTemplate`,
/// keyed by filename (without extension) — same load-all shape as
/// `load_all_item_templates`, reusing its shared helper.
pub fn load_all_vendor_templates(
    dir: &Path,
) -> Result<Vec<(String, VendorTemplate)>, ContentError> {
    load_all_ron(dir, load_vendor_template)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn real_vendor_templates_load_successfully() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/vendors");
        load_all_vendor_templates(&dir).unwrap();
    }

    #[test]
    fn parses_a_well_formed_vendor_template() {
        let template = parse_vendor_template(
            r#"(
                inventory: [
                    (item_template_key: "rusty_sword", price: 50),
                    (item_template_key: "leather_armor", price: 30),
                ],
            )"#,
        )
        .unwrap();

        assert_eq!(template.inventory.len(), 2);
        assert_eq!(template.inventory[0].item_template_key, "rusty_sword");
        assert_eq!(template.inventory[0].price, 50);
    }

    #[test]
    fn rejects_malformed_ron_syntax() {
        assert!(parse_vendor_template("(inventory: [").is_err());
    }

    #[test]
    fn into_listings_converts_every_entry() {
        let template = VendorTemplate {
            inventory: vec![VendorListingTemplate {
                item_template_key: "rusty_sword".to_string(),
                price: 50,
            }],
        };

        let listings = template.into_listings();

        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].item_template_key, "rusty_sword");
        assert_eq!(listings[0].price, 50);
    }

    #[test]
    fn load_all_vendor_templates_reads_every_ron_file_sorted_by_name() {
        let dir = std::env::temp_dir().join(format!(
            "vrekan_content_test_{}_{}",
            std::process::id(),
            "vendor_load_all_sorted"
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("b_merchant.ron"),
            "(inventory: [(item_template_key: \"leather_armor\", price: 30)])",
        )
        .unwrap();
        fs::write(
            dir.join("a_blacksmith.ron"),
            "(inventory: [(item_template_key: \"rusty_sword\", price: 50)])",
        )
        .unwrap();

        let templates = load_all_vendor_templates(&dir).unwrap();

        assert_eq!(templates.len(), 2);
        assert_eq!(templates[0].0, "a_blacksmith");
        assert_eq!(templates[1].0, "b_merchant");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_all_vendor_templates_fails_loudly_on_first_malformed_file() {
        let dir = std::env::temp_dir().join(format!(
            "vrekan_content_test_{}_{}",
            std::process::id(),
            "vendor_load_all_malformed"
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("broken.ron"), "(inventory: not_a_list)").unwrap();

        let result = load_all_vendor_templates(&dir);

        assert!(matches!(result, Err(ContentError::Parse { .. })));

        fs::remove_dir_all(&dir).unwrap();
    }
}
