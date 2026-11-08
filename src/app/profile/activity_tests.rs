use gpui::Styled;

use super::{ACTIVITY_PROMPTS, ProfilePalette, modal::activity_empty};

#[test]
fn activity_empty_state_has_no_card_surface() {
    for prompt in &ACTIVITY_PROMPTS {
        let mut empty = activity_empty("SampleUser", prompt, ProfilePalette::of(None));
        assert!(empty.style().background.is_none());
    }
}
