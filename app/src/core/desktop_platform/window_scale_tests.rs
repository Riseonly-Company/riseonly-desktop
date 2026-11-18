use super::window_scale::{ENLARGE_KEYSTROKES, RESET_KEYSTROKES, SHRINK_KEYSTROKES, window_scale_key_bindings};

#[test]
fn every_zoom_key_parses_and_plus_minus_zero_are_all_there() {
    let bindings = window_scale_key_bindings();
    assert_eq!(bindings.len(), ENLARGE_KEYSTROKES.len() + SHRINK_KEYSTROKES.len() + RESET_KEYSTROKES.len());
    let keys: Vec<String> = bindings.iter().flat_map(|binding| binding.keystrokes().iter().map(|keystroke| keystroke.key().to_owned())).collect();
    for key in ["=", "+", "-", "0"] {
        assert!(keys.iter().any(|known| known == key), "{key} missing in {keys:?}");
    }
}
