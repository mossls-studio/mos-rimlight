// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 MOSSLS

//! mos_rimlight v1.0.0:
//! Classic 2D Offset Rim Light effect for EffectCraft (API 2 external helper).
//!
//! Pipeline (matches classic AE 2D lighting workflow):
//! 1. Base layer exposure adjustment (linear-light EV exposure, offset, gamma).
//! 2. Optional under-glow gradient overlay (B&W Ramp or 4-Color Gradient with selectable
//!    blend modes), applied strictly beneath the lit rim & glow layer.
//! 3. Offset alpha-inverted matte with layer-boundary tiling (Mirror / Clamp / Repeat / None)
//!    so characters/objects touching the frame edge never leak false rim edges.
//! 4. Rim edge exposure brightening + optional color tint.
//! 5. Dedicated Glow switch with fast ROI-cropped downsampled dual-layer Gaussian bloom &
//!    automatic canvas padding.

use crate::blur;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

// Flat parameter indices (67 f64 values total)
pub const I_LIGHT_ANGLE: usize = 0;
pub const I_EDGE_DISTANCE: usize = 1;
pub const I_RIM_EXPOSURE: usize = 2;
pub const I_BASE_EXPOSURE: usize = 3;

pub const I_TILE_MODE: usize = 4;
pub const I_OFFSET_MODE: usize = 5;
pub const I_OFFSET_X: usize = 6;
pub const I_OFFSET_Y: usize = 7;
pub const I_MASK_SCALE: usize = 8;
pub const I_MASK_FEATHER: usize = 9;
pub const I_MASK_CHOKE: usize = 10;

pub const I_BASE_OFFSET: usize = 11;
pub const I_BASE_GAMMA: usize = 12;

pub const I_RIM_GAMMA: usize = 13;
pub const I_RIM_TINT_MIX: usize = 14;
pub const I_RIM_COLOR_R: usize = 15;
pub const I_RIM_COLOR_G: usize = 16;
pub const I_RIM_COLOR_B: usize = 17;
#[allow(dead_code)]
pub const I_RIM_COLOR_A: usize = 18;
pub const I_RIM_OPACITY: usize = 19;

pub const I_OVERLAY_ENABLE: usize = 20;
pub const I_OVERLAY_TYPE: usize = 21;
pub const I_OVERLAY_BLEND: usize = 22;
pub const I_OVERLAY_OPACITY: usize = 23;

pub const I_BW_START_X: usize = 24;
pub const I_BW_START_Y: usize = 25;
pub const I_BW_END_X: usize = 26;
pub const I_BW_END_Y: usize = 27;
pub const I_BW_SHAPE: usize = 28;
pub const I_BW_INVERT: usize = 29;

pub const I_C4_P1_X: usize = 30;
pub const I_C4_P1_Y: usize = 31;
pub const I_C4_COL1_R: usize = 32;
pub const I_C4_COL1_G: usize = 33;
pub const I_C4_COL1_B: usize = 34;
#[allow(dead_code)]
pub const I_C4_COL1_A: usize = 35;

pub const I_C4_P2_X: usize = 36;
pub const I_C4_P2_Y: usize = 37;
pub const I_C4_COL2_R: usize = 38;
pub const I_C4_COL2_G: usize = 39;
pub const I_C4_COL2_B: usize = 40;
#[allow(dead_code)]
pub const I_C4_COL2_A: usize = 41;

pub const I_C4_P3_X: usize = 42;
pub const I_C4_P3_Y: usize = 43;
pub const I_C4_COL3_R: usize = 44;
pub const I_C4_COL3_G: usize = 45;
pub const I_C4_COL3_B: usize = 46;
#[allow(dead_code)]
pub const I_C4_COL3_A: usize = 47;

pub const I_C4_P4_X: usize = 48;
pub const I_C4_P4_Y: usize = 49;
pub const I_C4_COL4_R: usize = 50;
pub const I_C4_COL4_G: usize = 51;
pub const I_C4_COL4_B: usize = 52;
#[allow(dead_code)]
pub const I_C4_COL4_A: usize = 53;
pub const I_C4_BLEND: usize = 54;

pub const I_GLOW_ENABLE: usize = 55;
pub const I_GLOW_THRESHOLD: usize = 56;
pub const I_GLOW_RADIUS: usize = 57;
pub const I_GLOW_INTENSITY: usize = 58;
pub const I_GLOW_OUTSIDE: usize = 59;
pub const I_GLOW_TINT_MIX: usize = 60;
pub const I_GLOW_COLOR_R: usize = 61;
pub const I_GLOW_COLOR_G: usize = 62;
pub const I_GLOW_COLOR_B: usize = 63;
#[allow(dead_code)]
pub const I_GLOW_COLOR_A: usize = 64;

pub const I_OUTPUT_MODE: usize = 65;
pub const I_MIX: usize = 66;

pub fn manifest() -> serde_json::Value {
    serde_json::json!({
        "api": 2,
        "id": "mos.rimlight",
        "name": "mos_打光",
        "category": "MOS 视效",
        "version": "1.0.0",
        "author": "MOSSLS",
        "description": "传统 2D 偏移边缘打光插件：通过镜像拼贴偏移遮罩提取受光边缘（杜绝画面边界穿帮），支持原图层曝光压暗、发光层之下叠加黑白渐变或四色渐变、边缘高光曝光提亮，以及高速独立发光开关。",
        "params": [
            // Top-level main controls
            {"id": "light_angle", "name": "光照角度", "type": "angle", "default": 120},
            {"id": "edge_distance", "name": "边缘厚度 px", "type": "slider", "default": 18, "min": 0, "max": 200, "sliderMax": 80, "decimals": 1},
            {"id": "rim_exposure", "name": "边缘曝光 (EV)", "type": "slider", "default": 2.5, "min": -6, "max": 10, "sliderMin": -2, "sliderMax": 6, "decimals": 2},
            {"id": "base_exposure", "name": "原图层曝光 (EV)", "type": "slider", "default": -1.5, "min": -8, "max": 4, "sliderMin": -5, "sliderMax": 2, "decimals": 2},

            // Matte & Tiling
            {"id": "tile_mode", "name": "拼贴模式", "type": "popup", "options": ["镜像边缘 (Mirror)", "边缘延展 (Clamp)", "平铺重复 (Repeat)", "关闭拼贴 (None)"], "default": 0, "group": "遮罩与拼贴"},
            {"id": "offset_mode", "name": "偏移方式", "type": "popup", "options": ["角度 + 距离", "手动 X / Y 偏移"], "default": 0, "group": "遮罩与拼贴"},
            {"id": "offset_x", "name": "遮罩 X 偏移 px", "type": "slider", "default": 8, "min": -200, "max": 200, "sliderMin": -60, "sliderMax": 60, "decimals": 1, "group": "遮罩与拼贴/手动偏移"},
            {"id": "offset_y", "name": "遮罩 Y 偏移 px", "type": "slider", "default": -16, "min": -200, "max": 200, "sliderMin": -60, "sliderMax": 60, "decimals": 1, "group": "遮罩与拼贴/手动偏移"},
            {"id": "mask_scale", "name": "遮罩缩放 %", "type": "slider", "default": 100, "min": 20, "max": 300, "sliderMin": 80, "sliderMax": 120, "decimals": 1, "group": "遮罩与拼贴"},
            {"id": "mask_feather", "name": "遮罩羽化 px", "type": "slider", "default": 0, "min": 0, "max": 40, "sliderMax": 15, "decimals": 1, "group": "遮罩与拼贴"},
            {"id": "mask_choke", "name": "遮罩收缩 %", "type": "slider", "default": 0, "min": -100, "max": 100, "decimals": 1, "group": "遮罩与拼贴"},

            // Base Exposure
            {"id": "base_offset", "name": "原图层偏移", "type": "slider", "default": 0, "min": -0.5, "max": 0.5, "decimals": 3, "group": "原图层曝光"},
            {"id": "base_gamma", "name": "原图层伽马", "type": "slider", "default": 1.0, "min": 0.1, "max": 4.0, "sliderMax": 2.5, "decimals": 2, "group": "原图层曝光"},

            // Rim Light Color
            {"id": "rim_gamma", "name": "边缘伽马", "type": "slider", "default": 1.0, "min": 0.1, "max": 4.0, "sliderMax": 2.5, "decimals": 2, "group": "边缘光色彩"},
            {"id": "rim_tint_mix", "name": "边缘染色 %", "type": "slider", "default": 0, "min": 0, "max": 100, "decimals": 1, "group": "边缘光色彩"},
            {"id": "rim_color", "name": "边缘颜色", "type": "color", "default": [1.0, 0.95, 0.85, 1.0], "group": "边缘光色彩"},
            {"id": "rim_opacity", "name": "边缘不透明度 %", "type": "slider", "default": 100, "min": 0, "max": 100, "decimals": 1, "group": "边缘光色彩"},

            // Under-Glow Gradient Overlay (strictly beneath rim & glow layer)
            {"id": "overlay_enable", "name": "启用渐变叠加", "type": "checkbox", "default": false, "group": "中层渐变叠加"},
            {"id": "overlay_type", "name": "渐变类型", "type": "popup", "options": ["黑白渐变", "四色渐变"], "default": 0, "group": "中层渐变叠加"},
            {"id": "overlay_blend", "name": "混合模式", "type": "popup", "options": ["叠加 (Overlay)", "柔光 (Soft Light)", "正片叠底 (Multiply)", "滤色 (Screen)", "正常 (Normal)"], "default": 0, "group": "中层渐变叠加"},
            {"id": "overlay_opacity", "name": "不透明度 %", "type": "slider", "default": 60, "min": 0, "max": 100, "decimals": 1, "group": "中层渐变叠加"},

            {"id": "bw_start", "name": "起点位置 (亮)", "type": "point", "default": [0.5, 0.15], "group": "中层渐变叠加/黑白渐变"},
            {"id": "bw_end", "name": "终点位置 (暗)", "type": "point", "default": [0.5, 0.85], "group": "中层渐变叠加/黑白渐变"},
            {"id": "bw_shape", "name": "渐变形状", "type": "popup", "options": ["线性渐变", "径向渐变"], "default": 0, "group": "中层渐变叠加/黑白渐变"},
            {"id": "bw_invert", "name": "反转黑白", "type": "checkbox", "default": false, "group": "中层渐变叠加/黑白渐变"},

            {"id": "c4_p1", "name": "点 1 位置", "type": "point", "default": [0.15, 0.15], "group": "中层渐变叠加/四色渐变"},
            {"id": "c4_col1", "name": "颜色 1", "type": "color", "default": [1.0, 0.85, 0.65, 1.0], "group": "中层渐变叠加/四色渐变"},
            {"id": "c4_p2", "name": "点 2 位置", "type": "point", "default": [0.85, 0.15], "group": "中层渐变叠加/四色渐变"},
            {"id": "c4_col2", "name": "颜色 2", "type": "color", "default": [0.35, 0.72, 1.0, 1.0], "group": "中层渐变叠加/四色渐变"},
            {"id": "c4_p3", "name": "点 3 位置", "type": "point", "default": [0.15, 0.85], "group": "中层渐变叠加/四色渐变"},
            {"id": "c4_col3", "name": "颜色 3", "type": "color", "default": [0.75, 0.35, 0.95, 1.0], "group": "中层渐变叠加/四色渐变"},
            {"id": "c4_p4", "name": "点 4 位置", "type": "point", "default": [0.85, 0.85], "group": "中层渐变叠加/四色渐变"},
            {"id": "c4_col4", "name": "颜色 4", "type": "color", "default": [0.12, 0.16, 0.32, 1.0], "group": "中层渐变叠加/四色渐变"},
            {"id": "c4_blend", "name": "融合平滑度 %", "type": "slider", "default": 100, "min": 10, "max": 300, "decimals": 1, "group": "中层渐变叠加/四色渐变"},

            // Glow
            {"id": "glow_enable", "name": "发光开关", "type": "checkbox", "default": true, "group": "发光"},
            {"id": "glow_threshold", "name": "发光阈值 %", "type": "slider", "default": 60, "min": 0, "max": 100, "decimals": 1, "group": "发光/发光参数"},
            {"id": "glow_radius", "name": "发光半径 px", "type": "slider", "default": 21, "min": 0, "max": 250, "sliderMax": 100, "decimals": 1, "group": "发光/发光参数"},
            {"id": "glow_intensity", "name": "发光强度 %", "type": "slider", "default": 100, "min": 0, "max": 400, "sliderMax": 200, "decimals": 1, "group": "发光/发光参数"},
            {"id": "glow_outside", "name": "溢出到透明区", "type": "checkbox", "default": true, "group": "发光/发光参数"},
            {"id": "glow_tint_mix", "name": "光晕染色 %", "type": "slider", "default": 0, "min": 0, "max": 100, "decimals": 1, "group": "发光/发光颜色"},
            {"id": "glow_color", "name": "发光颜色", "type": "color", "default": [0.75, 0.90, 1.0, 1.0], "group": "发光/发光颜色"},

            // Output
            {"id": "output_mode", "name": "输出模式", "type": "popup", "options": ["合成结果", "仅边缘光与发光", "预览偏移遮罩"], "default": 0, "group": "输出"},
            {"id": "mix", "name": "总体混合 %", "type": "slider", "default": 100, "min": 0, "max": 100, "decimals": 1, "group": "输出"}
        ]
    })
}

#[inline]
fn get_f(flat: &[f64], idx: usize, default: f32) -> f32 {
    flat.get(idx)
        .copied()
        .filter(|v| v.is_finite())
        .map(|v| v as f32)
        .unwrap_or(default)
}

#[inline]
fn get_b(flat: &[f64], idx: usize, default: bool) -> bool {
    flat.get(idx)
        .copied()
        .filter(|v| v.is_finite())
        .map(|v| v >= 0.5)
        .unwrap_or(default)
}

#[inline]
fn get_i(flat: &[f64], idx: usize, default: i32) -> i32 {
    flat.get(idx)
        .copied()
        .filter(|v| v.is_finite())
        .map(|v| v.round() as i32)
        .unwrap_or(default)
}

pub fn padding(flat: &[f64], scale: f64) -> u32 {
    let glow_enable = get_b(flat, I_GLOW_ENABLE, true);
    let glow_outside = get_b(flat, I_GLOW_OUTSIDE, true);
    let output_mode = get_i(flat, I_OUTPUT_MODE, 0);
    let mix = get_f(flat, I_MIX, 100.0);
    if !glow_enable || !glow_outside || output_mode == 2 || mix <= 0.01 {
        return 0;
    }
    let radius = get_f(flat, I_GLOW_RADIUS, 21.0).max(0.0);
    let glow_int = get_f(flat, I_GLOW_INTENSITY, 100.0).max(0.0);
    let rim_op = get_f(flat, I_RIM_OPACITY, 100.0).max(0.0);
    if radius <= 0.05 || glow_int <= 0.05 || rim_op <= 0.05 {
        return 0;
    }
    let s = if scale.is_finite() && scale > 0.0 {
        scale as f32
    } else {
        1.0
    };
    (radius * s * 1.5).ceil().clamp(0.0, 256.0) as u32
}

#[inline]
fn srgb_to_linear(c: f32) -> f32 {
    let x = c.max(0.0);
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

#[inline]
fn linear_to_srgb(c: f32) -> f32 {
    let x = c.max(0.0);
    if x <= 0.0031308 {
        x * 12.92
    } else if x <= 1.0 {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    } else {
        // Preserve HDR values above 1.0 smoothly
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

#[inline]
fn apply_exposure_channel(srgb: f32, gain: f32, offset: f32, inv_gamma: f32) -> f32 {
    let lin = (srgb_to_linear(srgb) * gain + offset).max(0.0);
    let g = if (inv_gamma - 1.0).abs() < 1e-4 {
        lin
    } else {
        lin.powf(inv_gamma)
    };
    linear_to_srgb(g)
}

#[inline]
fn blend_channel(mode: i32, base: f32, top: f32) -> f32 {
    let b = base.clamp(0.0, 1.0);
    let s = top.clamp(0.0, 1.0);
    match mode {
        // 0: Overlay
        0 => {
            if b < 0.5 {
                2.0 * b * s
            } else {
                1.0 - 2.0 * (1.0 - b) * (1.0 - s)
            }
        }
        // 1: Soft Light (Pegtop / W3C)
        1 => (1.0 - 2.0 * s) * b * b + 2.0 * s * b,
        // 2: Multiply
        2 => b * s,
        // 3: Screen
        3 => b + s - b * s,
        // 4: Normal
        _ => s,
    }
}

#[inline]
fn soft_threshold(luma: f32, threshold: f32) -> f32 {
    if threshold <= 0.0001 {
        return 1.0;
    }
    let knee = (threshold * 0.35).max(0.06);
    let soft = (luma - threshold + knee).clamp(0.0, 2.0 * knee);
    let contrib = (soft * soft) / (4.0 * knee + 1e-6);
    let excess = (luma - threshold).max(contrib);
    if luma > 1e-6 {
        (excess / luma).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Mirror-fold coordinate `v` into `[0, len - 1]` (matches AE Motion Tile Mirror Edges).
#[inline]
fn tile_coord(v: f32, len: f32, tile_mode: i32) -> Option<f32> {
    if len <= 1.0 {
        return Some(0.0);
    }
    let max_v = len - 1.0;
    match tile_mode {
        // 0: Mirror Edges
        0 => {
            let period = 2.0 * max_v;
            let mut m = v.rem_euclid(period);
            if m > max_v {
                m = period - m;
            }
            Some(m.clamp(0.0, max_v))
        }
        // 1: Clamp Edges
        1 => Some(v.clamp(0.0, max_v)),
        // 2: Repeat Tile
        2 => Some(v.rem_euclid(len).clamp(0.0, max_v)),
        // 3: None (transparent outside layer bounds)
        _ => {
            if v < 0.0 || v > max_v {
                None
            } else {
                Some(v)
            }
        }
    }
}

#[inline]
fn sample_alpha_bilinear(
    alpha: &[f32],
    w: usize,
    h: usize,
    lx0: f32,
    ly0: f32,
    lw: f32,
    lh: f32,
    sx: f32,
    sy: f32,
    tile_mode: i32,
) -> f32 {
    let Some(tx) = tile_coord(sx - lx0, lw, tile_mode) else {
        return 0.0;
    };
    let Some(ty) = tile_coord(sy - ly0, lh, tile_mode) else {
        return 0.0;
    };
    let bx = (lx0 + tx).clamp(0.0, (w - 1) as f32);
    let by = (ly0 + ty).clamp(0.0, (h - 1) as f32);
    let x0 = bx.floor() as usize;
    let y0 = by.floor() as usize;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let wx = bx - x0 as f32;
    let wy = by - y0 as f32;

    let a00 = alpha[y0 * w + x0];
    let a10 = alpha[y0 * w + x1];
    let a01 = alpha[y1 * w + x0];
    let a11 = alpha[y1 * w + x1];

    let top = a00 * (1.0 - wx) + a10 * wx;
    let bot = a01 * (1.0 - wx) + a11 * wx;
    top * (1.0 - wy) + bot * wy
}

pub fn render(
    px: &mut [f32],
    w: usize,
    h: usize,
    origin: [f64; 2],
    layer_size: [f64; 2],
    scale: f64,
    flat: &[f64],
) -> Result<(), String> {
    if w == 0 || h == 0 || px.len() < w * h * 4 {
        return Ok(());
    }

    let mix = (get_f(flat, I_MIX, 100.0) / 100.0).clamp(0.0, 1.0);
    let output_mode = get_i(flat, I_OUTPUT_MODE, 0).clamp(0, 2);
    if mix <= 0.0001 && output_mode == 0 {
        return Ok(());
    }

    let s = if scale.is_finite() && scale > 0.0 {
        scale as f32
    } else {
        1.0
    };

    // Exact source layer bounds inside the (possibly padded) buffer for accurate Mirror Tiling.
    let lx0 = (origin[0] as f32).clamp(0.0, (w - 1) as f32);
    let ly0 = (origin[1] as f32).clamp(0.0, (h - 1) as f32);
    let lw = if layer_size[0].is_finite() && layer_size[0] > 0.5 {
        ((layer_size[0] as f32) * s).clamp(1.0, (w as f32) - lx0)
    } else {
        (w as f32) - lx0
    };
    let lh = if layer_size[1].is_finite() && layer_size[1] > 0.5 {
        ((layer_size[1] as f32) * s).clamp(1.0, (h as f32) - ly0)
    } else {
        (h as f32) - ly0
    };

    let light_angle = get_f(flat, I_LIGHT_ANGLE, 120.0);
    let edge_dist = get_f(flat, I_EDGE_DISTANCE, 18.0).max(0.0) * s;
    let rim_ev = get_f(flat, I_RIM_EXPOSURE, 2.5);
    let base_ev = get_f(flat, I_BASE_EXPOSURE, -1.5);

    let tile_mode = get_i(flat, I_TILE_MODE, 0).clamp(0, 3);
    let offset_mode = get_i(flat, I_OFFSET_MODE, 0).clamp(0, 1);
    let manual_ox = get_f(flat, I_OFFSET_X, 8.0) * s;
    let manual_oy = get_f(flat, I_OFFSET_Y, -16.0) * s;
    let mask_scale = (get_f(flat, I_MASK_SCALE, 100.0) / 100.0).clamp(0.1, 5.0);
    let inv_mask_scale = 1.0 / mask_scale;
    let layer_cx = lx0 + lw * 0.5;
    let layer_cy = ly0 + lh * 0.5;
    let mask_feather = get_f(flat, I_MASK_FEATHER, 0.0).max(0.0) * s;
    let mask_choke = (get_f(flat, I_MASK_CHOKE, 0.0) / 100.0).clamp(-1.0, 1.0);

    // Matte offset (ox, oy): shifting the inverted matte away from light exposes the lit edge.
    let (matte_ox, matte_oy) = if offset_mode == 1 {
        (manual_ox, manual_oy)
    } else {
        let rad = light_angle.to_radians();
        (-rad.cos() * edge_dist, -rad.sin() * edge_dist)
    };

    let base_offset = get_f(flat, I_BASE_OFFSET, 0.0);
    let base_gamma = get_f(flat, I_BASE_GAMMA, 1.0).clamp(0.1, 4.0);
    let base_gain = 2.0f32.powf(base_ev);
    let base_inv_gamma = 1.0 / base_gamma;

    let rim_gamma = get_f(flat, I_RIM_GAMMA, 1.0).clamp(0.1, 4.0);
    let rim_gain = 2.0f32.powf(rim_ev);
    let rim_inv_gamma = 1.0 / rim_gamma;
    let rim_tint_mix = (get_f(flat, I_RIM_TINT_MIX, 0.0) / 100.0).clamp(0.0, 1.0);
    let rim_col_r = get_f(flat, I_RIM_COLOR_R, 1.0).max(0.0);
    let rim_col_g = get_f(flat, I_RIM_COLOR_G, 0.95).max(0.0);
    let rim_col_b = get_f(flat, I_RIM_COLOR_B, 0.85).max(0.0);
    let rim_opacity = (get_f(flat, I_RIM_OPACITY, 100.0) / 100.0).clamp(0.0, 1.0);

    // Under-glow gradient overlay (B&W or 4-Color)
    let overlay_enable = get_b(flat, I_OVERLAY_ENABLE, false);
    let overlay_type = get_i(flat, I_OVERLAY_TYPE, 0).clamp(0, 1);
    let overlay_blend = get_i(flat, I_OVERLAY_BLEND, 0).clamp(0, 4);
    let overlay_opacity = (get_f(flat, I_OVERLAY_OPACITY, 60.0) / 100.0).clamp(0.0, 1.0);

    let bw_sx = get_f(flat, I_BW_START_X, lx0 + lw * 0.5);
    let bw_sy = get_f(flat, I_BW_START_Y, ly0 + lh * 0.15);
    let bw_ex = get_f(flat, I_BW_END_X, lx0 + lw * 0.5);
    let bw_ey = get_f(flat, I_BW_END_Y, ly0 + lh * 0.85);
    let bw_shape = get_i(flat, I_BW_SHAPE, 0).clamp(0, 1);
    let bw_invert = get_b(flat, I_BW_INVERT, false);

    let c4_pts = [
        (get_f(flat, I_C4_P1_X, lx0 + lw * 0.15), get_f(flat, I_C4_P1_Y, ly0 + lh * 0.15)),
        (get_f(flat, I_C4_P2_X, lx0 + lw * 0.85), get_f(flat, I_C4_P2_Y, ly0 + lh * 0.15)),
        (get_f(flat, I_C4_P3_X, lx0 + lw * 0.15), get_f(flat, I_C4_P3_Y, ly0 + lh * 0.85)),
        (get_f(flat, I_C4_P4_X, lx0 + lw * 0.85), get_f(flat, I_C4_P4_Y, ly0 + lh * 0.85)),
    ];
    let c4_cols = [
        [get_f(flat, I_C4_COL1_R, 1.0), get_f(flat, I_C4_COL1_G, 0.85), get_f(flat, I_C4_COL1_B, 0.65)],
        [get_f(flat, I_C4_COL2_R, 0.35), get_f(flat, I_C4_COL2_G, 0.72), get_f(flat, I_C4_COL2_B, 1.0)],
        [get_f(flat, I_C4_COL3_R, 0.75), get_f(flat, I_C4_COL3_G, 0.35), get_f(flat, I_C4_COL3_B, 0.95)],
        [get_f(flat, I_C4_COL4_R, 0.12), get_f(flat, I_C4_COL4_G, 0.16), get_f(flat, I_C4_COL4_B, 0.32)],
    ];
    let c4_blend = (get_f(flat, I_C4_BLEND, 100.0) / 100.0).clamp(0.1, 3.0);
    let c4_power = 2.0 / c4_blend;
    let inv_diag2 = 1.0 / (lw * lw + lh * lh).max(1.0);

    // Glow
    let glow_enable = get_b(flat, I_GLOW_ENABLE, true);
    let glow_threshold = (get_f(flat, I_GLOW_THRESHOLD, 60.0) / 100.0).clamp(0.0, 1.0);
    let glow_radius = get_f(flat, I_GLOW_RADIUS, 21.0).max(0.0) * s;
    let glow_intensity = (get_f(flat, I_GLOW_INTENSITY, 100.0) / 100.0).max(0.0);
    let glow_outside = get_b(flat, I_GLOW_OUTSIDE, true);
    let glow_tint_mix = (get_f(flat, I_GLOW_TINT_MIX, 0.0) / 100.0).clamp(0.0, 1.0);
    let glow_col_r = get_f(flat, I_GLOW_COLOR_R, 0.75).max(0.0);
    let glow_col_g = get_f(flat, I_GLOW_COLOR_G, 0.90).max(0.0);
    let glow_col_b = get_f(flat, I_GLOW_COLOR_B, 1.0).max(0.0);

    let n = w * h;

    // 1. Extract source alpha plane (optionally feathered for smooth offset matte)
    let mut src_alpha = vec![0.0f32; n];
    for i in 0..n {
        src_alpha[i] = px[i * 4 + 3].clamp(0.0, 1.0);
    }
    let matte_alpha_src = if mask_feather > 0.25 {
        blur::gauss_plane_fast(&src_alpha, w, h, mask_feather * 0.5)
    } else {
        src_alpha.clone()
    };

    let threads = thread::available_parallelism()
        .map(|t| t.get())
        .unwrap_or(1)
        .min(h.max(1))
        .max(1);
    let chunk_rows = h.div_ceil(threads);

    // 2. Build rim RGBA buffer (exposed rim pixels) & glow source RGB buffer with ROI tracking
    let need_glow = glow_enable
        && output_mode != 2
        && glow_radius > 0.1
        && glow_intensity > 0.001
        && rim_opacity > 0.001;

    let mut rim_buf = vec![0.0f32; n * 4]; // premultiplied rim light RGBA
    let mut glow_src = if need_glow {
        vec![0.0f32; n * 3]
    } else {
        Vec::new()
    };

    let roi_min_x = AtomicUsize::new(w);
    let roi_min_y = AtomicUsize::new(h);
    let roi_max_x = AtomicUsize::new(0);
    let roi_max_y = AtomicUsize::new(0);

    let choke_shift = mask_choke * 0.45;
    {
        let px_ref: &[f32] = px;
        let sa_ref: &[f32] = &src_alpha;
        let ma_ref: &[f32] = &matte_alpha_src;
        let roi_min_x_ref = &roi_min_x;
        let roi_min_y_ref = &roi_min_y;
        let roi_max_x_ref = &roi_max_x;
        let roi_max_y_ref = &roi_max_y;

        thread::scope(|scope| {
            if need_glow {
                for (ci, (r_chunk, g_chunk)) in rim_buf
                    .chunks_mut(chunk_rows * w * 4)
                    .zip(glow_src.chunks_mut(chunk_rows * w * 3))
                    .enumerate()
                {
                    let y_start = ci * chunk_rows;
                    scope.spawn(move || {
                        let rows = r_chunk.len() / (w * 4);
                        let mut l_min_x = w;
                        let mut l_min_y = h;
                        let mut l_max_x = 0usize;
                        let mut l_max_y = 0usize;
                        let mut any_glow = false;

                        for ry in 0..rows {
                            let y = y_start + ry;
                            for x in 0..w {
                                let idx = y * w + x;
                                let a = sa_ref[idx];
                                if a <= 0.0005 {
                                    continue;
                                }
                                let sx = layer_cx + (x as f32 - matte_ox - layer_cx) * inv_mask_scale;
                                let sy = layer_cy + (y as f32 - matte_oy - layer_cy) * inv_mask_scale;
                                let shifted_a = sample_alpha_bilinear(
                                    ma_ref, w, h, lx0, ly0, lw, lh, sx, sy, tile_mode,
                                );
                                let mut rim_m = (a * (1.0 - shifted_a)).clamp(0.0, 1.0);
                                if choke_shift.abs() > 1e-4 {
                                    rim_m = ((rim_m - choke_shift) / (1.0 - choke_shift.abs()).max(0.05))
                                        .clamp(0.0, 1.0);
                                }
                                if rim_m <= 0.0005 {
                                    continue;
                                }

                                let inv_a = 1.0 / a;
                                let sr = (px_ref[idx * 4] * inv_a).max(0.0);
                                let sg = (px_ref[idx * 4 + 1] * inv_a).max(0.0);
                                let sb = (px_ref[idx * 4 + 2] * inv_a).max(0.0);

                                let mut rr = apply_exposure_channel(sr, rim_gain, 0.0, rim_inv_gamma);
                                let mut rg = apply_exposure_channel(sg, rim_gain, 0.0, rim_inv_gamma);
                                let mut rb = apply_exposure_channel(sb, rim_gain, 0.0, rim_inv_gamma);

                                if rim_tint_mix > 0.0001 {
                                    let luma = (0.2126 * rr + 0.7152 * rg + 0.0722 * rb).max(0.15);
                                    rr = rr * (1.0 - rim_tint_mix) + rim_col_r * luma * rim_tint_mix;
                                    rg = rg * (1.0 - rim_tint_mix) + rim_col_g * luma * rim_tint_mix;
                                    rb = rb * (1.0 - rim_tint_mix) + rim_col_b * luma * rim_tint_mix;
                                }

                                let eff_a = (rim_m * rim_opacity).clamp(0.0, 1.0);
                                let pr = rr * eff_a;
                                let pg = rg * eff_a;
                                let pb = rb * eff_a;

                                let ro = (ry * w + x) * 4;
                                r_chunk[ro] = pr;
                                r_chunk[ro + 1] = pg;
                                r_chunk[ro + 2] = pb;
                                r_chunk[ro + 3] = eff_a;

                                let luma = 0.2126 * pr + 0.7152 * pg + 0.0722 * pb;
                                let th = soft_threshold(luma, glow_threshold);
                                if th > 0.0001 {
                                    let go = (ry * w + x) * 3;
                                    let gr = pr * (1.0 - glow_tint_mix) + glow_col_r * luma * glow_tint_mix;
                                    let gg = pg * (1.0 - glow_tint_mix) + glow_col_g * luma * glow_tint_mix;
                                    let gb = pb * (1.0 - glow_tint_mix) + glow_col_b * luma * glow_tint_mix;
                                    g_chunk[go] = gr * th;
                                    g_chunk[go + 1] = gg * th;
                                    g_chunk[go + 2] = gb * th;
                                    if !any_glow {
                                        any_glow = true;
                                        l_min_x = x;
                                        l_max_x = x + 1;
                                        l_min_y = y;
                                        l_max_y = y + 1;
                                    } else {
                                        if x < l_min_x { l_min_x = x; }
                                        if x + 1 > l_max_x { l_max_x = x + 1; }
                                        if y < l_min_y { l_min_y = y; }
                                        if y + 1 > l_max_y { l_max_y = y + 1; }
                                    }
                                }
                            }
                        }

                        if any_glow {
                            roi_min_x_ref.fetch_min(l_min_x, Ordering::Relaxed);
                            roi_min_y_ref.fetch_min(l_min_y, Ordering::Relaxed);
                            roi_max_x_ref.fetch_max(l_max_x, Ordering::Relaxed);
                            roi_max_y_ref.fetch_max(l_max_y, Ordering::Relaxed);
                        }
                    });
                }
            } else {
                for (ci, r_chunk) in rim_buf.chunks_mut(chunk_rows * w * 4).enumerate() {
                    let y_start = ci * chunk_rows;
                    scope.spawn(move || {
                        let rows = r_chunk.len() / (w * 4);
                        for ry in 0..rows {
                            let y = y_start + ry;
                            for x in 0..w {
                                let idx = y * w + x;
                                let a = sa_ref[idx];
                                if a <= 0.0005 {
                                    continue;
                                }
                                let sx = layer_cx + (x as f32 - matte_ox - layer_cx) * inv_mask_scale;
                                let sy = layer_cy + (y as f32 - matte_oy - layer_cy) * inv_mask_scale;
                                let shifted_a = sample_alpha_bilinear(
                                    ma_ref, w, h, lx0, ly0, lw, lh, sx, sy, tile_mode,
                                );
                                let mut rim_m = (a * (1.0 - shifted_a)).clamp(0.0, 1.0);
                                if choke_shift.abs() > 1e-4 {
                                    rim_m = ((rim_m - choke_shift) / (1.0 - choke_shift.abs()).max(0.05))
                                        .clamp(0.0, 1.0);
                                }
                                if rim_m <= 0.0005 {
                                    continue;
                                }
                                let inv_a = 1.0 / a;
                                let sr = (px_ref[idx * 4] * inv_a).max(0.0);
                                let sg = (px_ref[idx * 4 + 1] * inv_a).max(0.0);
                                let sb = (px_ref[idx * 4 + 2] * inv_a).max(0.0);

                                let mut rr = apply_exposure_channel(sr, rim_gain, 0.0, rim_inv_gamma);
                                let mut rg = apply_exposure_channel(sg, rim_gain, 0.0, rim_inv_gamma);
                                let mut rb = apply_exposure_channel(sb, rim_gain, 0.0, rim_inv_gamma);

                                if rim_tint_mix > 0.0001 {
                                    let luma = (0.2126 * rr + 0.7152 * rg + 0.0722 * rb).max(0.15);
                                    rr = rr * (1.0 - rim_tint_mix) + rim_col_r * luma * rim_tint_mix;
                                    rg = rg * (1.0 - rim_tint_mix) + rim_col_g * luma * rim_tint_mix;
                                    rb = rb * (1.0 - rim_tint_mix) + rim_col_b * luma * rim_tint_mix;
                                }

                                let eff_a = (rim_m * rim_opacity).clamp(0.0, 1.0);
                                let ro = (ry * w + x) * 4;
                                r_chunk[ro] = rr * eff_a;
                                r_chunk[ro + 1] = rg * eff_a;
                                r_chunk[ro + 2] = rb * eff_a;
                                r_chunk[ro + 3] = eff_a;
                            }
                        }
                    });
                }
            }
        });
    }

    // 3. Fast ROI-cropped dual-layer optical bloom
    let glow_blurred: Option<Vec<f32>> = if need_glow {
        let bx0 = roi_min_x.load(Ordering::Relaxed);
        let by0 = roi_min_y.load(Ordering::Relaxed);
        let bx1 = roi_max_x.load(Ordering::Relaxed);
        let by1 = roi_max_y.load(Ordering::Relaxed);
        if bx1 > bx0 && by1 > by0 {
            let expand = (glow_radius * 1.65).ceil() as usize;
            let x0 = bx0.saturating_sub(expand);
            let y0 = by0.saturating_sub(expand);
            let x1 = (bx1 + expand).min(w);
            let y1 = (by1 + expand).min(h);
            Some(blur::bloom_rgb_roi(
                &glow_src,
                w,
                h,
                (x0, y0, x1, y1),
                glow_radius,
                glow_intensity,
            ))
        } else {
            None
        }
    } else {
        None
    };

    // Precompute B&W ramp vector
    let bw_dx = bw_ex - bw_sx;
    let bw_dy = bw_ey - bw_sy;
    let bw_len2 = (bw_dx * bw_dx + bw_dy * bw_dy).max(1.0);
    let bw_len = bw_len2.sqrt();

    // 4. Final compositing in strict layer order:
    //    Bottom: Base layer with base exposure
    //    Middle (beneath rim & glow): B&W Gradient or 4-Color Gradient overlay
    //    Top: Exposed rim edge + optical bloom glow
    let glow_ref: Option<&[f32]> = glow_blurred.as_deref();
    thread::scope(|scope| {
        for (ci, out_chunk) in px.chunks_mut(chunk_rows * w * 4).enumerate() {
            let y_start = ci * chunk_rows;
            let rim_ref: &[f32] = &rim_buf;
            scope.spawn(move || {
                let rows = out_chunk.len() / (w * 4);
                for ry in 0..rows {
                    let y = y_start + ry;
                    let fy = y as f32 + 0.5;
                    for x in 0..w {
                        let idx = y * w + x;
                        let lo = (ry * w + x) * 4;
                        let orig_r = out_chunk[lo];
                        let orig_g = out_chunk[lo + 1];
                        let orig_b = out_chunk[lo + 2];
                        let orig_a = out_chunk[lo + 3].clamp(0.0, 1.0);

                        let rm_r = rim_ref[idx * 4];
                        let rm_g = rim_ref[idx * 4 + 1];
                        let rm_b = rim_ref[idx * 4 + 2];
                        let rm_a = rim_ref[idx * 4 + 3];

                        let (gl_r, gl_g, gl_b) = match glow_ref {
                            Some(gb) => {
                                let gate = if glow_outside { 1.0 } else { orig_a };
                                (gb[idx * 3] * gate, gb[idx * 3 + 1] * gate, gb[idx * 3 + 2] * gate)
                            }
                            None => (0.0, 0.0, 0.0),
                        };

                        let (mut out_r, mut out_g, mut out_b, mut out_a) = match output_mode {
                            // 2: Preview offset matte
                            2 => (rm_a, rm_a, rm_a, rm_a),
                            // 1: Rim & glow only
                            1 => {
                                let r = rm_r + gl_r;
                                let g = rm_g + gl_g;
                                let b = rm_b + gl_b;
                                let gl_a = (gl_r.max(gl_g).max(gl_b)).clamp(0.0, 1.0);
                                let a = (rm_a + gl_a * (1.0 - rm_a)).clamp(0.0, 1.0);
                                (r, g, b, a)
                            }
                            // 0: Full composite
                            _ => {
                                // Step A: Base layer exposure
                                let (mut br, mut bg, mut bb) = if orig_a > 0.0005 {
                                    let inv_a = 1.0 / orig_a;
                                    let sr = (orig_r * inv_a).max(0.0);
                                    let sg = (orig_g * inv_a).max(0.0);
                                    let sb = (orig_b * inv_a).max(0.0);
                                    let mut er = apply_exposure_channel(sr, base_gain, base_offset, base_inv_gamma);
                                    let mut eg = apply_exposure_channel(sg, base_gain, base_offset, base_inv_gamma);
                                    let mut eb = apply_exposure_channel(sb, base_gain, base_offset, base_inv_gamma);

                                    // Step B: Under-glow gradient overlay (B&W or 4-Color)
                                    if overlay_enable && overlay_opacity > 0.0005 {
                                        let fx = x as f32 + 0.5;
                                        let (gr, gg, gb) = if overlay_type == 0 {
                                            // B&W Gradient Ramp
                                            let px_dx = fx - bw_sx;
                                            let px_dy = fy - bw_sy;
                                            let t = if bw_shape == 1 {
                                                ((px_dx * px_dx + px_dy * px_dy).sqrt() / bw_len).clamp(0.0, 1.0)
                                            } else {
                                                ((px_dx * bw_dx + px_dy * bw_dy) / bw_len2).clamp(0.0, 1.0)
                                            };
                                            let v = if bw_invert { t } else { 1.0 - t };
                                            (v, v, v)
                                        } else {
                                            // 4-Color Gradient (IDW interpolation)
                                            let mut w_sum = 0.0f32;
                                            let (mut cr, mut cg, mut cb) = (0.0f32, 0.0f32, 0.0f32);
                                            for k in 0..4 {
                                                let ddx = fx - c4_pts[k].0;
                                                let ddy = fy - c4_pts[k].1;
                                                let d2 = (ddx * ddx + ddy * ddy) * inv_diag2;
                                                let wk = 1.0 / (d2.powf(c4_power * 0.5) + 1e-4);
                                                w_sum += wk;
                                                cr += c4_cols[k][0] * wk;
                                                cg += c4_cols[k][1] * wk;
                                                cb += c4_cols[k][2] * wk;
                                            }
                                            let inv_w = 1.0 / w_sum.max(1e-6);
                                            (cr * inv_w, cg * inv_w, cb * inv_w)
                                        };
                                        let mr = blend_channel(overlay_blend, er, gr);
                                        let mg = blend_channel(overlay_blend, eg, gg);
                                        let mb = blend_channel(overlay_blend, eb, gb);
                                        er = er + (mr - er) * overlay_opacity;
                                        eg = eg + (mg - eg) * overlay_opacity;
                                        eb = eb + (mb - eb) * overlay_opacity;
                                    }

                                    (er * orig_a, eg * orig_a, eb * orig_a)
                                } else {
                                    (0.0, 0.0, 0.0)
                                };

                                // Step C: Composite exposed rim edge ON TOP of base + gradient (source-over)
                                let inv_rm = 1.0 - rm_a;
                                br = rm_r + br * inv_rm;
                                bg = rm_g + bg * inv_rm;
                                bb = rm_b + bb * inv_rm;

                                // Step D: Add optical bloom glow on top
                                let r = br + gl_r;
                                let g = bg + gl_g;
                                let b = bb + gl_b;
                                let gl_a = (gl_r.max(gl_g).max(gl_b)).clamp(0.0, 1.0);
                                let a = if glow_outside {
                                    (orig_a + gl_a * (1.0 - orig_a)).clamp(0.0, 1.0)
                                } else {
                                    orig_a
                                };
                                (r, g, b, a)
                            }
                        };

                        if mix < 0.9999 {
                            out_r = orig_r + (out_r - orig_r) * mix;
                            out_g = orig_g + (out_g - orig_g) * mix;
                            out_b = orig_b + (out_b - orig_b) * mix;
                            out_a = orig_a + (out_a - orig_a) * mix;
                        }

                        out_chunk[lo] = out_r.max(0.0);
                        out_chunk[lo + 1] = out_g.max(0.0);
                        out_chunk[lo + 2] = out_b.max(0.0);
                        out_chunk[lo + 3] = out_a.clamp(0.0, 1.0);
                    }
                }
            });
        }
    });

    Ok(())
}
