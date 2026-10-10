//! The brand palette, light or dark to follow the system appearance.

use gpui_kit::base::{Theme, ThemeAppearance};
use gpui_kit::{App, Global, Hsla, Window, WindowAppearance, rgb};

const INK: u32 = 0x000b_0d10;
const PAPER: u32 = 0x00e8_eaed;
const DIM_ON_INK: u32 = 0x009a_a0a6;
const DIM_ON_PAPER: u32 = 0x005f_6368;
const ACCENT: u32 = 0x003d_dc97;
/// Readable on both backgrounds: contrast 4.46 on ink and 3.62 on paper.
const ERROR: u32 = 0x00dc_3e42;
/// The selection is drawn under the glyphs, so it must let them show.
const SELECTION_ALPHA: f32 = 0.35;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub background: Hsla,
    pub text: Hsla,
    pub dim: Hsla,
    pub error: Hsla,
}

impl Global for Palette {}

impl Palette {
    fn for_appearance(appearance: WindowAppearance) -> Self {
        let (background, text, dim) = match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => (PAPER, INK, DIM_ON_PAPER),
            WindowAppearance::Dark | WindowAppearance::VibrantDark => (INK, PAPER, DIM_ON_INK),
        };
        Self {
            background: rgb(background).into(),
            text: rgb(text).into(),
            dim: rgb(dim).into(),
            error: rgb(ERROR).into(),
        }
    }
}

/// Applies the palette for `appearance` to this app's views and to the
/// prompt input, which takes its colors from the base theme.
pub fn apply(appearance: WindowAppearance, cx: &mut App) {
    let palette = Palette::for_appearance(appearance);
    let accent: Hsla = rgb(ACCENT).into();
    let theme = Theme::global_mut(cx);
    theme.appearance = match appearance {
        WindowAppearance::Light | WindowAppearance::VibrantLight => ThemeAppearance::Light,
        WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeAppearance::Dark,
    };
    let colors = &mut theme.tokens.colors;
    colors.background = palette.background;
    colors.surface = palette.background;
    colors.foreground = palette.text;
    colors.surface_foreground = palette.text;
    colors.muted_foreground = palette.dim;
    colors.accent = accent;
    colors.selection = accent.alpha(SELECTION_ALPHA);
    cx.set_global(palette);
    cx.refresh_windows();
}

/// Follows `window`'s appearance from now on.
pub fn follow(window: &Window) {
    window
        .observe_window_appearance(|window, cx| apply(window.appearance(), cx))
        .detach();
}
