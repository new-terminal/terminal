//! The Paper palette, light or dark to follow the system appearance.

use gpui_kit::base::{Theme, ThemeAppearance};
use gpui_kit::{App, Global, Hsla, Window, WindowAppearance, rgb};

const ACCENT: u32 = 0x003d_dc97;
/// Readable on both grounds: contrast 4.46 on the dark one and 4.11 on the
/// light one.
const ERROR: u32 = 0x00dc_3e42;
/// Readable on both grounds: contrast 4.60 on the dark one and 3.98 on the
/// light one.
const WARNING: u32 = 0x00a3_7200;
/// The selection is drawn under the glyphs, so it must let them show.
const SELECTION_ALPHA: f32 = 0.35;

/// The colors of one appearance. `block_*` colors belong to the request
/// block, which inverts the ground so it is the one solid shape on screen.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub ground: Hsla,
    pub ink: Hsla,
    pub muted: Hsla,
    /// The tint behind a command band.
    pub band: Hsla,
    pub block: Hsla,
    pub block_ink: Hsla,
    pub block_muted: Hsla,
    /// The tint behind the request text inside the block.
    pub block_band: Hsla,
    /// The border of the block's secondary buttons.
    pub block_line: Hsla,
    pub accent: Hsla,
    /// "needs you" inside the block, readable on the block color.
    pub attention: Hsla,
    pub success: Hsla,
    pub error: Hsla,
    pub warning: Hsla,
}

impl Global for Palette {}

/// One appearance's values as `0xRRGGBB`, in [`Palette`] field order.
struct Values {
    ground: u32,
    ink: u32,
    muted: u32,
    band: u32,
    block: u32,
    block_ink: u32,
    block_muted: u32,
    block_band: u32,
    block_line: u32,
    attention: u32,
    success: u32,
}

const LIGHT: Values = Values {
    ground: 0x00f7_f8fa,
    ink: 0x000b_0d10,
    muted: 0x0064_6b74,
    band: 0x00ec_eef1,
    block: 0x000b_0d10,
    block_ink: 0x00e8_eaed,
    block_muted: 0x009a_a0a6,
    block_band: 0x001c_2026,
    block_line: 0x003a_3f46,
    attention: ACCENT,
    success: 0x0014_744e,
};

const DARK: Values = Values {
    ground: 0x000b_0d10,
    ink: 0x00e8_eaed,
    muted: 0x009a_a0a6,
    band: 0x0017_1a1e,
    block: 0x00e8_eaed,
    block_ink: 0x000b_0d10,
    block_muted: 0x005f_6670,
    block_band: 0x00dc_dfe3,
    block_line: 0x00b4_b9bf,
    attention: 0x0014_744e,
    success: ACCENT,
};

impl Palette {
    fn for_appearance(appearance: WindowAppearance) -> Self {
        let values = match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => &LIGHT,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => &DARK,
        };
        Self {
            ground: rgb(values.ground).into(),
            ink: rgb(values.ink).into(),
            muted: rgb(values.muted).into(),
            band: rgb(values.band).into(),
            block: rgb(values.block).into(),
            block_ink: rgb(values.block_ink).into(),
            block_muted: rgb(values.block_muted).into(),
            block_band: rgb(values.block_band).into(),
            block_line: rgb(values.block_line).into(),
            accent: rgb(ACCENT).into(),
            attention: rgb(values.attention).into(),
            success: rgb(values.success).into(),
            error: rgb(ERROR).into(),
            warning: rgb(WARNING).into(),
        }
    }
}

/// Applies the palette for `appearance` to this app's views and to the
/// prompt input, which takes its colors from the base theme.
pub fn apply(appearance: WindowAppearance, cx: &mut App) {
    let palette = Palette::for_appearance(appearance);
    let theme = Theme::global_mut(cx);
    theme.appearance = match appearance {
        WindowAppearance::Light | WindowAppearance::VibrantLight => ThemeAppearance::Light,
        WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeAppearance::Dark,
    };
    let colors = &mut theme.tokens.colors;
    colors.background = palette.ground;
    colors.surface = palette.ground;
    colors.foreground = palette.ink;
    colors.surface_foreground = palette.ink;
    colors.muted_foreground = palette.muted;
    colors.accent = palette.accent;
    colors.selection = palette.accent.alpha(SELECTION_ALPHA);
    cx.set_global(palette);
    cx.refresh_windows();
}

/// Follows `window`'s appearance from now on.
pub fn follow(window: &Window) {
    window
        .observe_window_appearance(|window, cx| apply(window.appearance(), cx))
        .detach();
}
