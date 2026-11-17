use std::collections::HashMap;

use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftCatalogItemDTO, RiseGiftInstanceDTO, RiseGiftWornDTO,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GiftsCatalogState {
    pub items: Vec<RiseGiftCatalogItemDTO>,
    pub loaded: bool,
    pub loading: bool,
    pub failed: bool,
}

impl GiftsCatalogState {
    pub fn empty() -> Self {
        Self::default()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GiftsAppearanceState {
    pub instances: HashMap<String, RiseGiftInstanceDTO>,
    pub locale: String,
}

impl GiftsAppearanceState {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn instance(&self, worn: Option<&RiseGiftWornDTO>) -> Option<&RiseGiftInstanceDTO> {
        worn.and_then(|worn| self.instances.get(&worn.instance_id))
    }

    pub fn worn_name(&self, worn: Option<&RiseGiftWornDTO>) -> Option<String> {
        let worn = worn?;
        Some(
            self.instances
                .get(&worn.instance_id)
                .map(|instance| instance.name.clone())
                .unwrap_or_default(),
        )
    }
}
