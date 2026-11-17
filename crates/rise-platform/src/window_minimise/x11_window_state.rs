pub const ICONIC_STATE: u32 = 3;

/// what the branch read from the window and the root for one answer
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct X11WindowState {
    pub hidden_atom: u32,
    pub manager_supports_hidden: bool,
    pub net_wm_state: Vec<u32>,
    pub wm_state: Option<u32>,
}

impl X11WindowState {
    /// true while the window is minimised
    pub fn minimised(&self) -> bool {
        if self.manager_supports_hidden {
            return self.net_wm_state.contains(&self.hidden_atom);
        }
        self.net_wm_state.contains(&self.hidden_atom) || self.wm_state == Some(ICONIC_STATE)
    }
}

/// whether a property change on the watched window can change the answer
pub fn is_state_property(changed: u32, net_wm_state_atom: u32, wm_state_atom: u32) -> bool {
    changed == net_wm_state_atom || changed == wm_state_atom
}
