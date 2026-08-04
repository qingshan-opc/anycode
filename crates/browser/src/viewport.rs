//! Browser layout viewport — prefer the host display's CSS-pixel size + DPR.

use crate::types::ViewportSpec;
use std::sync::OnceLock;

const MIN_W: u32 = 800;
const MIN_H: u32 = 600;
const MAX_W: u32 = 3840;
const MAX_H: u32 = 2160;
const FALLBACK_W: u32 = 1920;
const FALLBACK_H: u32 = 1080;
/// Keep screencast bitmaps under this edge so 2× Retina stays interactive.
const MAX_BITMAP_EDGE: f64 = 3840.0;

/// Clamp CSS layout size to a sane range.
pub fn clamp_viewport(width: u32, height: u32) -> (u32, u32) {
    (width.clamp(MIN_W, MAX_W), height.clamp(MIN_H, MAX_H))
}

/// Clamp DPR: at least 1, at most 2 (enough for Retina sharpness).
pub fn clamp_device_scale_factor(dpr: f64) -> f64 {
    if !dpr.is_finite() || dpr < 1.0 {
        1.0
    } else {
        dpr.min(2.0)
    }
}

/// Cap DPR so width×dpr / height×dpr stay within [`MAX_BITMAP_EDGE`].
pub fn effective_device_scale_factor(width: u32, height: u32, dpr: f64) -> f64 {
    let dpr = clamp_device_scale_factor(dpr);
    let max_w = MAX_BITMAP_EDGE / f64::from(width.max(1));
    let max_h = MAX_BITMAP_EDGE / f64::from(height.max(1));
    dpr.min(max_w).min(max_h).max(1.0)
}

/// Resolve CSS size only (legacy helpers).
pub fn resolve_viewport(requested: Option<(u32, u32)>) -> (u32, u32) {
    let spec = resolve_viewport_spec(
        requested.map(|(w, h)| ViewportSpec::new(w, h, system_device_scale_factor())),
    );
    (spec.width, spec.height)
}

pub fn resolve_viewport_spec(requested: Option<ViewportSpec>) -> ViewportSpec {
    let base = if let Some(req) = requested {
        let (w, h) = clamp_viewport(req.width, req.height);
        let dpr = if req.device_scale_factor > 0.0 {
            req.device_scale_factor
        } else {
            system_device_scale_factor()
        };
        ViewportSpec::new(w, h, dpr)
    } else {
        let (w, h) = system_viewport();
        ViewportSpec::new(w, h, system_device_scale_factor())
    };
    ViewportSpec::new(
        base.width,
        base.height,
        effective_device_scale_factor(base.width, base.height, base.device_scale_factor),
    )
}

/// Primary display size in CSS pixels (points), cached for the process.
pub fn system_viewport() -> (u32, u32) {
    static CACHED: OnceLock<(u32, u32)> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let (w, h) = detect_system_viewport();
        clamp_viewport(w, h)
    })
}

/// Host device pixel ratio (Retina ≈ 2), cached.
pub fn system_device_scale_factor() -> f64 {
    static CACHED: OnceLock<f64> = OnceLock::new();
    *CACHED.get_or_init(|| clamp_device_scale_factor(detect_system_device_scale_factor()))
}

fn detect_system_viewport() -> (u32, u32) {
    #[cfg(target_os = "macos")]
    {
        if let Some(v) = macos_desktop_bounds() {
            return v;
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(v) = linux_xrandr_primary() {
            return v;
        }
    }
    (FALLBACK_W, FALLBACK_H)
}

fn detect_system_device_scale_factor() -> f64 {
    #[cfg(target_os = "macos")]
    {
        if let Some(dpr) = macos_retina_scale() {
            return dpr;
        }
        // Built-in Liquid Retina MacBooks are almost always ≥2×.
        return 2.0;
    }
    #[cfg(not(target_os = "macos"))]
    {
        1.0
    }
}

/// Finder desktop bounds are in points (logical CSS pixels), e.g. `0, 0, 1710, 1112`.
#[cfg(target_os = "macos")]
fn macos_desktop_bounds() -> Option<(u32, u32)> {
    let output = std::process::Command::new("osascript")
        .args([
            "-e",
            "tell application \"Finder\" to get bounds of window of desktop",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_bounds_csv(&text)
}

#[cfg(target_os = "macos")]
fn parse_bounds_csv(text: &str) -> Option<(u32, u32)> {
    let nums: Vec<u32> = text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter_map(|p| p.trim().parse().ok())
        .collect();
    if nums.len() >= 4 {
        let w = nums[2].saturating_sub(nums[0]);
        let h = nums[3].saturating_sub(nums[1]);
        if w >= MIN_W && h >= MIN_H {
            return Some((w, h));
        }
    }
    None
}

/// Parse `Resolution: 2560 x 1664 Retina` vs desktop logical bounds → DPR.
#[cfg(target_os = "macos")]
fn macos_retina_scale() -> Option<f64> {
    let (logical_w, logical_h) = macos_desktop_bounds()?;
    let output = std::process::Command::new("system_profiler")
        .args(["SPDisplaysDataType"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (phys_w, phys_h) = parse_retina_resolution(&text)?;
    let sx = f64::from(phys_w) / f64::from(logical_w.max(1));
    let sy = f64::from(phys_h) / f64::from(logical_h.max(1));
    let dpr = ((sx + sy) / 2.0).round() / 1.0; // keep fractional (e.g. 1.5)
                                               // Prefer rounded common scales: 1, 1.5, 2
    let snapped = if (dpr - 2.0).abs() < 0.2 {
        2.0
    } else if (dpr - 1.5).abs() < 0.2 {
        1.5
    } else if (dpr - 1.0).abs() < 0.2 {
        1.0
    } else {
        dpr.clamp(1.0, 2.0)
    };
    Some(snapped)
}

#[cfg(target_os = "macos")]
fn parse_retina_resolution(text: &str) -> Option<(u32, u32)> {
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("Resolution:") {
            continue;
        }
        // "Resolution: 2560 x 1664 Retina" or "Resolution: 1920 x 1080 (1080p)"
        let rest = line.strip_prefix("Resolution:")?.trim();
        let mut parts = rest.split_whitespace();
        let w: u32 = parts.next()?.parse().ok()?;
        if parts.next()? != "x" {
            continue;
        }
        let h: u32 = parts.next()?.parse().ok()?;
        if w >= MIN_W && h >= MIN_H {
            return Some((w, h));
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn linux_xrandr_primary() -> Option<(u32, u32)> {
    let output = std::process::Command::new("xrandr")
        .arg("--current")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if !line.contains(" connected") {
            continue;
        }
        let rest = line.split(" connected").nth(1)?;
        for token in rest.split_whitespace() {
            if let Some((w, h)) = token.split_once('x') {
                let w = w.parse().ok()?;
                let h = h.split('+').next()?.parse().ok()?;
                if w >= MIN_W && h >= MIN_H {
                    return Some((w, h));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_enforces_bounds() {
        assert_eq!(clamp_viewport(100, 100), (MIN_W, MIN_H));
        assert_eq!(clamp_viewport(9999, 9999), (MAX_W, MAX_H));
        assert_eq!(clamp_viewport(1710, 1112), (1710, 1112));
    }

    #[test]
    fn effective_scale_caps_bitmap() {
        let dpr = effective_device_scale_factor(3000, 2000, 2.0);
        assert!(dpr < 2.0);
        assert!(dpr >= 1.0);
        assert!((effective_device_scale_factor(1280, 720, 2.0) - 2.0).abs() < f64::EPSILON);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_finder_bounds() {
        assert_eq!(parse_bounds_csv("0, 0, 1710, 1112\n"), Some((1710, 1112)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_retina_line() {
        let sample = "          Resolution: 2560 x 1664 Retina\n";
        assert_eq!(parse_retina_resolution(sample), Some((2560, 1664)));
    }
}
