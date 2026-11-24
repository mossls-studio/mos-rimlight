// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 MOSSLS

//! Fast separable box blur approximating a Gaussian kernel.
//!
//! Optimizations for real-time interactive EffectCraft previews:
//! - Downsamples RGB glow planes aggressively (2x–8x) before blurring, then bilinearly upsamples.
//! - Uses 2-pass box blur on downsampled glow planes (indistinguishable from 3-pass after bilinear
//!   upsampling, ~33% faster).
//! - Supports ROI-cropped blurring so empty transparent regions outside the sweep band are skipped.

use std::thread;

fn boxes_for_gauss(sigma: f32, n: usize) -> Vec<usize> {
    if sigma <= 0.0 {
        return vec![1; n];
    }
    let w_ideal = (12.0 * sigma * sigma / n as f32 + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma
        - (n as f32) * (wl * wl) as f32
        - 4.0 * (n as f32) * wl as f32
        - 3.0 * (n as f32))
        / (-4.0 * wl as f32 - 4.0);
    let m = m_ideal.round().max(0.0) as usize;
    (0..n)
        .map(|i| if i < m { wl as usize } else { wu as usize })
        .collect()
}

#[inline]
fn clamp_i(i: i64, n: usize) -> usize {
    if i < 0 {
        0
    } else if i as usize >= n {
        n - 1
    } else {
        i as usize
    }
}

fn row_box(s: &[f32], d: &mut [f32], w: usize, r: usize) {
    if w == 0 {
        return;
    }
    if r == 0 {
        d[..w].copy_from_slice(&s[..w]);
        return;
    }
    let k = 1.0 / (2 * r + 1) as f32;
    let mut sum = 0.0f32;
    for i in -(r as i64)..=(r as i64) {
        sum += s[clamp_i(i, w)];
    }
    let mut x = 0usize;
    let left_end = r.min(w);
    while x < left_end {
        d[x] = sum * k;
        sum += s[(x + 1 + r).min(w - 1)] - s[0];
        x += 1;
    }
    let interior_end = (w - 1).saturating_sub(r);
    while x < interior_end {
        d[x] = sum * k;
        sum += s[x + 1 + r] - s[x - r];
        x += 1;
    }
    while x < w {
        d[x] = sum * k;
        sum += s[w - 1] - s[(x.saturating_sub(r)).min(w - 1)];
        x += 1;
    }
}

fn threads_for(h: usize) -> usize {
    let n = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    n.min(h.max(1)).max(1)
}

fn box_rows(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize) {
    if w == 0 || h == 0 {
        return;
    }
    if r == 0 {
        dst[..w * h].copy_from_slice(&src[..w * h]);
        return;
    }
    let threads = threads_for(h);
    if threads <= 1 || h < 96 {
        for y in 0..h {
            row_box(
                &src[y * w..(y + 1) * w],
                &mut dst[y * w..(y + 1) * w],
                w,
                r,
            );
        }
        return;
    }
    let chunk = h.div_ceil(threads);
    thread::scope(|scope| {
        for (ci, out) in dst.chunks_mut(chunk * w).enumerate() {
            let start = ci * chunk;
            scope.spawn(move || {
                for (yi, row) in out.chunks_mut(w).enumerate() {
                    let y = start + yi;
                    row_box(&src[y * w..(y + 1) * w], row, w, r);
                }
            });
        }
    });
}

fn transpose(src: &[f32], dst: &mut [f32], w: usize, h: usize) {
    const BLOCK: usize = 32;
    let mut y0 = 0;
    while y0 < h {
        let y1 = (y0 + BLOCK).min(h);
        let mut x0 = 0;
        while x0 < w {
            let x1 = (x0 + BLOCK).min(w);
            for y in y0..y1 {
                let row = &src[y * w..y * w + w];
                for x in x0..x1 {
                    dst[x * h + y] = row[x];
                }
            }
            x0 = x1;
        }
        y0 = y1;
    }
}

fn blur_plane(src: &[f32], w: usize, h: usize, widths: &[usize]) -> Vec<f32> {
    let n = w * h;
    if n == 0 {
        return Vec::new();
    }
    let mut a = src.to_vec();
    let mut b = vec![0.0f32; n];

    for &width in widths {
        let r = width.saturating_sub(1) / 2;
        box_rows(&a, &mut b, w, h, r);
        std::mem::swap(&mut a, &mut b);
    }

    transpose(&a, &mut b, w, h);
    std::mem::swap(&mut a, &mut b);

    for &width in widths {
        let r = width.saturating_sub(1) / 2;
        box_rows(&a, &mut b, h, w, r);
        std::mem::swap(&mut a, &mut b);
    }

    transpose(&a, &mut b, h, w);
    b
}

/// Fast 2-pass single-channel blur (used for alpha edge rim detection).
pub fn gauss_plane_fast(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma <= 0.05 || w == 0 || h == 0 {
        return src.to_vec();
    }
    blur_plane(src, w, h, &boxes_for_gauss(sigma, 2))
}

/// Blur 3 planar RGB channels in parallel across 3 threads.
fn blur_rgb_planar(
    r: &[f32],
    g: &[f32],
    b: &[f32],
    w: usize,
    h: usize,
    sigma: f32,
    passes: usize,
) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    if sigma <= 0.05 || w == 0 || h == 0 {
        return (r.to_vec(), g.to_vec(), b.to_vec());
    }
    let widths = boxes_for_gauss(sigma, passes);
    thread::scope(|scope| {
        let hr = scope.spawn(|| blur_plane(r, w, h, &widths));
        let hg = scope.spawn(|| blur_plane(g, w, h, &widths));
        let hb = scope.spawn(|| blur_plane(b, w, h, &widths));
        (
            hr.join().unwrap_or_default(),
            hg.join().unwrap_or_default(),
            hb.join().unwrap_or_default(),
        )
    })
}

/// Downsample an interleaved RGB ROI `[x0..x1, y0..y1]` from a `full_w x full_h` image into
/// 3 separate planar channels `(r, g, b)` of size `dw x dh`.
fn downsample_roi_to_planar(
    src: &[f32],
    full_w: usize,
    x0: usize,
    y0: usize,
    rw: usize,
    rh: usize,
    k: usize,
) -> (Vec<f32>, Vec<f32>, Vec<f32>, usize, usize) {
    let dw = (rw / k).max(1);
    let dh = (rh / k).max(1);
    let mut pr = vec![0.0f32; dw * dh];
    let mut pg = vec![0.0f32; dw * dh];
    let mut pb = vec![0.0f32; dw * dh];

    let threads = threads_for(dh);
    let chunk = dh.div_ceil(threads.max(1));
    thread::scope(|scope| {
        for (ci, ((rr, rg), rb)) in pr
            .chunks_mut(chunk * dw)
            .zip(pg.chunks_mut(chunk * dw))
            .zip(pb.chunks_mut(chunk * dw))
            .enumerate()
        {
            let start = ci * chunk;
            scope.spawn(move || {
                let rows = rr.len() / dw;
                for ri in 0..rows {
                    let dy = start + ri;
                    let sy0 = (dy * k).min(rh - 1);
                    let sy1 = (sy0 + k).min(rh);
                    let y_cnt = (sy1 - sy0).max(1);
                    for dx in 0..dw {
                        let sx0 = (dx * k).min(rw - 1);
                        let sx1 = (sx0 + k).min(rw);
                        let x_cnt = (sx1 - sx0).max(1);
                        let (mut a0, mut a1, mut a2) = (0.0f32, 0.0f32, 0.0f32);
                        for sy in sy0..sy1 {
                            let mut i = ((y0 + sy) * full_w + (x0 + sx0)) * 3;
                            for _ in sx0..sx1 {
                                a0 += src[i];
                                a1 += src[i + 1];
                                a2 += src[i + 2];
                                i += 3;
                            }
                        }
                        let inv = 1.0 / (x_cnt * y_cnt) as f32;
                        let o = ri * dw + dx;
                        rr[o] = a0 * inv;
                        rg[o] = a1 * inv;
                        rb[o] = a2 * inv;
                    }
                }
            });
        }
    });
    (pr, pg, pb, dw, dh)
}

/// Bilinearly upsample 3 planar channels `(dw x dh)` and accumulate into `out` ROI `[x0..x0+rw, y0..y0+rh]`
/// with scalar `gain`.
#[allow(clippy::too_many_arguments)]
fn upsample_planar_add_roi(
    pr: &[f32],
    pg: &[f32],
    pb: &[f32],
    dw: usize,
    dh: usize,
    out: &mut [f32],
    full_w: usize,
    x0: usize,
    y0: usize,
    rw: usize,
    rh: usize,
    gain: f32,
) {
    let mut xs: Vec<(usize, usize, f32)> = Vec::with_capacity(rw);
    let sx_scale = dw as f32 / rw as f32;
    for x in 0..rw {
        let fx = ((x as f32 + 0.5) * sx_scale - 0.5).max(0.0);
        let ix0 = (fx.floor() as usize).min(dw - 1);
        xs.push((ix0, (ix0 + 1).min(dw - 1), fx - ix0 as f32));
    }
    let dy_scale = dh as f32 / rh as f32;

    // Work directly on the slice of rows `[y0 .. y0 + rh]` in `out`.
    let row_stride = full_w * 3;
    let roi_slice = &mut out[y0 * row_stride..(y0 + rh) * row_stride];
    let threads = threads_for(rh);
    let chunk = rh.div_ceil(threads.max(1));

    thread::scope(|scope| {
        for (ci, out_rows) in roi_slice.chunks_mut(chunk * row_stride).enumerate() {
            let start = ci * chunk;
            let xs = &xs;
            scope.spawn(move || {
                let rows = out_rows.len() / row_stride;
                for ri in 0..rows {
                    let y = start + ri;
                    let fy = ((y as f32 + 0.5) * dy_scale - 0.5).max(0.0);
                    let iy0 = (fy.floor() as usize).min(dh - 1);
                    let iy1 = (iy0 + 1).min(dh - 1);
                    let wy = fy - iy0 as f32;
                    let w0 = 1.0 - wy;
                    let r0 = iy0 * dw;
                    let r1 = iy1 * dw;
                    let dst_row = &mut out_rows[ri * row_stride..(ri + 1) * row_stride];
                    for x in 0..rw {
                        let (ix0, ix1, wx) = xs[x];
                        let wx0 = 1.0 - wx;
                        let p00 = r0 + ix0;
                        let p10 = r0 + ix1;
                        let p01 = r1 + ix0;
                        let p11 = r1 + ix1;

                        let vr = (pr[p00] * wx0 + pr[p10] * wx) * w0
                            + (pr[p01] * wx0 + pr[p11] * wx) * wy;
                        let vg = (pg[p00] * wx0 + pg[p10] * wx) * w0
                            + (pg[p01] * wx0 + pg[p11] * wx) * wy;
                        let vb = (pb[p00] * wx0 + pb[p10] * wx) * w0
                            + (pb[p01] * wx0 + pb[p11] * wx) * wy;

                        let o = (x0 + x) * 3;
                        dst_row[o] += vr * gain;
                        dst_row[o + 1] += vg * gain;
                        dst_row[o + 2] += vb * gain;
                    }
                }
            });
        }
    });
}

/// Fast dual-layer optical bloom on an interleaved RGB buffer (`w x h x 3`) restricted to
/// the active ROI `(x0, y0, x1, y1)`. Both core and wide layers run on downsampled planar
/// buffers in parallel across RGB channels.
#[allow(clippy::too_many_arguments)]
pub fn bloom_rgb_roi(
    src: &[f32],
    w: usize,
    h: usize,
    roi: (usize, usize, usize, usize),
    glow_radius: f32,
    glow_intensity: f32,
) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h * 3];
    let (x0, y0, x1, y1) = roi;
    if x1 <= x0 || y1 <= y0 || glow_radius <= 0.1 || glow_intensity <= 0.0001 {
        return out;
    }
    let rw = x1 - x0;
    let rh = y1 - y0;

    let sigma_wide = (glow_radius * 0.55).max(0.8);
    let sigma_core = (glow_radius * 0.18).max(0.5);

    // Core halo: always at least 2x downsampled when radius >= 8px
    let k_core = if sigma_core >= 1.5 {
        ((sigma_core / 2.2) as usize).clamp(2, 6).min(rw).min(rh)
    } else {
        1
    };
    let (cr, cg, cb, cw, ch) = downsample_roi_to_planar(src, w, x0, y0, rw, rh, k_core);
    let (cr, cg, cb) = blur_rgb_planar(&cr, &cg, &cb, cw, ch, sigma_core / k_core as f32, 2);
    upsample_planar_add_roi(
        &cr,
        &cg,
        &cb,
        cw,
        ch,
        &mut out,
        w,
        x0,
        y0,
        rw,
        rh,
        0.45 * glow_intensity,
    );

    // Wide halo: downsample aggressively (2x–10x) for smooth, zero-lag wide bloom
    let k_wide = ((sigma_wide / 2.5) as usize).clamp(2, 10).min(rw).min(rh);
    let (wr, wg, wb, ww, wh) = downsample_roi_to_planar(src, w, x0, y0, rw, rh, k_wide);
    let (wr, wg, wb) = blur_rgb_planar(&wr, &wg, &wb, ww, wh, sigma_wide / k_wide as f32, 2);
    upsample_planar_add_roi(
        &wr,
        &wg,
        &wb,
        ww,
        wh,
        &mut out,
        w,
        x0,
        y0,
        rw,
        rh,
        0.75 * glow_intensity,
    );

    out
}
