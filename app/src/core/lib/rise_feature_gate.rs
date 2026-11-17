/// product sections switched off without removing their code
/// an off section has no entry point anywhere and turning the flag on restores all of them
pub enum RiseFeatureGate {}

impl RiseFeatureGate {
    pub const VACANCIES: bool = false;
}
