//! The two type families. Both ship with macOS, so the app bundles no font.
//! GPUI looks a family up by its Core Text family name and picks the face
//! nearest the asked weight and style.

use gpui_kit::App;
use gpui_kit::base::Theme;

/// Prose, echo lines, and the request block's words.
pub const SANS: &str = "Avenir Next";
/// Commands, paths, tool lines, labels, the prompt, and the status bar.
/// The mockup sets some of these spans at medium weight. Menlo has no
/// medium face, so the app asks for none.
pub const MONO: &str = "Menlo";

/// Makes inline code inside the scrollback's text views use [`MONO`].
pub fn install(cx: &mut App) {
    Theme::global_mut(cx).tokens.typography.mono = MONO.into();
}
