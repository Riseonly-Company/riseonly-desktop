use std::sync::Arc;

use gpui::RenderImage;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;

#[derive(Clone)]
pub struct WorkExperienceEntry {
    pub id: Uuid,
    pub company: String,
    pub position: String,
    pub from_date: String,
    pub to_date: String,
    pub is_current: bool,
    pub job_description: String,
    pub company_logo_url: Option<String>,
    pub logo_file: Option<SaiWsOptimisticLocalFile>,
    pub logo_preview: Option<Arc<RenderImage>>,
    pub logo_uploading: bool,
    pub logo_progress: f64,
}

impl Default for WorkExperienceEntry {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            company: String::new(),
            position: String::new(),
            from_date: String::new(),
            to_date: String::new(),
            is_current: false,
            job_description: String::new(),
            company_logo_url: None,
            logo_file: None,
            logo_preview: None,
            logo_uploading: false,
            logo_progress: 0.0,
        }
    }
}
