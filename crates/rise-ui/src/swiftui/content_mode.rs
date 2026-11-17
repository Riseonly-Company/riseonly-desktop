use gpui::ObjectFit;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ContentMode {
    Fit,
    #[default]
    Fill,
}

impl ContentMode {
    /// object fit of a gpui image
    pub fn object_fit(self) -> ObjectFit {
        match self {
            Self::Fit => ObjectFit::Contain,
            Self::Fill => ObjectFit::Cover,
        }
    }
}
