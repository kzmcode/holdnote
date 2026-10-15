//! Coral Night theme — the single source of truth for UI colors (requirements §4.2).
//! Do not hard-code colors anywhere else.
//! Design rule: flat, solid colors only — no gradients, blur, translucency, shadows or glow.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn from_hex(hex: u32) -> Self {
        Rgb(((hex >> 16) & 0xff) as u8, ((hex >> 8) & 0xff) as u8, (hex & 0xff) as u8)
    }
    pub fn components(self) -> (f64, f64, f64) {
        (self.0 as f64 / 255.0, self.1 as f64 / 255.0, self.2 as f64 / 255.0)
    }
    /// Derived color: mix toward white (t>0) or black (t<0). Base tokens stay unchanged.
    pub fn adjust(self, t: f64) -> Self {
        let f = |c: u8| -> u8 {
            let c = c as f64;
            let v = if t >= 0.0 { c + (255.0 - c) * t } else { c * (1.0 + t) };
            v.round().clamp(0.0, 255.0) as u8
        };
        Rgb(f(self.0), f(self.1), f(self.2))
    }
    /// Solid blend of `self` over `base` (t = share of `self`). Used instead of alpha so the UI
    /// stays flat: no translucency, gradients, blur or glow.
    pub fn mix(self, base: Rgb, t: f64) -> Self {
        let f = |a: u8, b: u8| -> u8 { (a as f64 * t + b as f64 * (1.0 - t)).round().clamp(0.0, 255.0) as u8 };
        Rgb(f(self.0, base.0), f(self.1, base.1), f(self.2, base.2))
    }
    /// WCAG relative luminance.
    pub fn luminance(self) -> f64 {
        let lin = |c: u8| {
            let c = c as f64 / 255.0;
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(self.0) + 0.7152 * lin(self.1) + 0.0722 * lin(self.2)
    }
    pub fn contrast(self, other: Rgb) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }
}

// Base 8 colors (fixed design values).
pub const BACKGROUND: Rgb = Rgb::from_hex(0x121212);
pub const SURFACE: Rgb = Rgb::from_hex(0x1E1E1E);
pub const TEXT_PRIMARY: Rgb = Rgb::from_hex(0xF4F7FF);
pub const TEXT_SECONDARY: Rgb = Rgb::from_hex(0x9AA4B2);
pub const ACCENT: Rgb = Rgb::from_hex(0x3B82F6);
pub const RECORDING: Rgb = Rgb::from_hex(0xFF4D4D);
pub const SUCCESS: Rgb = Rgb::from_hex(0x22C55E);
pub const BORDER: Rgb = Rgb::from_hex(0x333333);
// Semantic additions.
pub const WARNING: Rgb = Rgb::from_hex(0xF59E0B);
pub const ERROR: Rgb = Rgb::from_hex(0xFF4D4D);

// Overlay geometry (pt).
pub const OVERLAY_WIDTH: f64 = 280.0;
pub const OVERLAY_HEIGHT: f64 = 68.0;
pub const OVERLAY_RADIUS: f64 = 16.0;
pub const OVERLAY_TOP_MARGIN: f64 = 12.0;
pub const LEVEL_BAR_COUNT: usize = 11;
/// Max UI refresh rate while recording.
pub const OVERLAY_FPS: u32 = 10;

/// Convert a token to an `NSColor` (sRGB). Main-thread not required.
pub fn ns_color(c: Rgb) -> objc2::rc::Retained<objc2_app_kit::NSColor> {
    let (r, g, b) = c.components();
    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0)
}

pub fn ns_color_alpha(c: Rgb, alpha: f64) -> objc2::rc::Retained<objc2_app_kit::NSColor> {
    let (r, g, b) = c.components();
    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_tokens_match_spec() {
        assert_eq!(BACKGROUND, Rgb(0x12, 0x12, 0x12));
        assert_eq!(SURFACE, Rgb(0x1E, 0x1E, 0x1E));
        assert_eq!(TEXT_PRIMARY, Rgb(0xF4, 0xF7, 0xFF));
        assert_eq!(TEXT_SECONDARY, Rgb(0x9A, 0xA4, 0xB2));
        assert_eq!(ACCENT, Rgb(0x3B, 0x82, 0xF6));
        assert_eq!(RECORDING, Rgb(0xFF, 0x4D, 0x4D));
        assert_eq!(SUCCESS, Rgb(0x22, 0xC5, 0x5E));
        assert_eq!(BORDER, Rgb(0x33, 0x33, 0x33));
    }

    #[test]
    fn contrast_on_surface_meets_wcag() {
        assert!(TEXT_PRIMARY.contrast(SURFACE) >= 4.5);
        assert!(TEXT_SECONDARY.contrast(SURFACE) >= 4.5);
        assert!(ACCENT.contrast(SURFACE) >= 3.0);
        assert!(RECORDING.contrast(SURFACE) >= 3.0);
        assert!(SUCCESS.contrast(SURFACE) >= 3.0);
    }
}
