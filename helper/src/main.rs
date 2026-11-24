#![recursion_limit = "256"]

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 MOSSLS

//! effectcraft-helper-mos-rimlight: EffectCraft external effect helper providing
//! `mos_打光` (`mos.rimlight`).

mod blur;
mod lang;
mod rimlight;

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const MAGIC: &str = "@ec-helper ";
const PROTOCOL_VERSION: u32 = 1;
const HELPER_VERSION: &str = "1.0.0";
const EFFECT_ID: &str = "mos.rimlight";

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct ImageRef {
    offset: u64,
    width: u32,
    height: u32,
    #[serde(default)]
    origin: [f64; 2],
    #[serde(default = "one")]
    scale: f64,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
#[allow(dead_code)]
enum Req {
    Hello {
        version: u32,
        frames: String,
    },
    Describe,
    Load {
        path: String,
    },
    Padding {
        effect: String,
        instance: u64,
        time: f64,
        scale: f64,
        layer_size: [f64; 2],
        flat: Vec<f64>,
        #[serde(default)]
        texts: Vec<String>,
    },
    Render {
        effect: String,
        instance: u64,
        time: f64,
        frame: ImageRef,
        layer_size: [f64; 2],
        flat: Vec<f64>,
        #[serde(default)]
        texts: Vec<String>,
        #[serde(default)]
        frame_rate: f64,
        #[serde(default)]
        frame_range: Option<(f64, f64)>,
    },
    Image {
        image: Option<ImageRef>,
    },
    Params {
        #[serde(default)]
        values: Option<(Vec<f64>, Vec<String>)>,
    },
    Destroy {
        instance: u64,
    },
    Ping,
    Shutdown,
}

struct Host {
    out: Arc<Mutex<std::io::Stdout>>,
    frames: Option<File>,
    px_buf: Vec<f32>,
}

fn main() {
    log_line(&format!("start: pid {}", std::process::id()));
    let mut host = Host {
        out: Arc::new(Mutex::new(std::io::stdout())),
        frames: None,
        px_buf: Vec::new(),
    };

    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                log_line(&format!("stdin read error: {e}"));
                break;
            }
        }
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let req: Req = match serde_json::from_str(t) {
            Ok(r) => r,
            Err(e) => {
                log_line(&format!("bad request: {e} in {t}"));
                continue;
            }
        };

        match req {
            Req::Hello { version, frames } => {
                if version != PROTOCOL_VERSION {
                    host.send(json!({
                        "op": "error",
                        "message": format!("this helper speaks protocol {PROTOCOL_VERSION}, not {version}")
                    }));
                    break;
                }
                match OpenOptions::new().read(true).write(true).open(&frames) {
                    Ok(f) => {
                        host.frames = Some(f);
                        log_line(&format!("hello: frames={frames}"));
                        host.send(json!({
                            "op": "hello",
                            "version": PROTOCOL_VERSION,
                            "name": lang::effect_name(),
                            "helper_version": HELPER_VERSION,
                            "loads": []
                        }));
                    }
                    Err(e) => {
                        host.send(json!({
                            "op": "error",
                            "message": format!("can't open the frame exchange file: {e}")
                        }));
                        break;
                    }
                }
            }
            Req::Describe => {
                let mut manifest = rimlight::manifest();
                lang::localize_manifest(&mut manifest);
                host.send(json!({
                    "op": "effects",
                    "effects": [{"manifest": manifest, "padding": true}],
                    "errors": []
                }));
            }
            Req::Load { path } => {
                log_line(&format!("load (ignored): {path}"));
                host.send(json!({
                    "op": "effects",
                    "effects": [],
                    "errors": ["this helper does not load files"]
                }));
            }
            Req::Padding { flat, scale, .. } => {
                let px = rimlight::padding(&flat, scale);
                host.send(json!({"op": "padding", "pixels": px}));
            }
            Req::Render {
                effect,
                time,
                frame,
                layer_size,
                flat,
                ..
            } => {
                if effect != EFFECT_ID {
                    host.send(json!({
                        "op": "error",
                        "message": format!("unknown effect `{effect}`")
                    }));
                    continue;
                }
                match host.render_frame(&frame, time, layer_size, &flat) {
                    Ok(()) => host.send(json!({"op": "done"})),
                    Err(e) => {
                        log_line(&format!("render error: {e}"));
                        host.send(json!({"op": "error", "message": e}));
                    }
                }
            }
            Req::Image { .. } | Req::Params { .. } | Req::Destroy { .. } => {}
            Req::Ping => host.send(json!({"op": "pong"})),
            Req::Shutdown => break,
        }
    }
}

impl Host {
    fn send(&self, v: serde_json::Value) {
        if let Ok(mut out) = self.out.lock() {
            let _ = writeln!(out, "{MAGIC}{v}");
            let _ = out.flush();
        }
    }

    fn render_frame(
        &mut self,
        frame: &ImageRef,
        time: f64,
        layer_size: [f64; 2],
        flat: &[f64],
    ) -> Result<(), String> {
        let started = Instant::now();
        let w = frame.width as usize;
        let h = frame.height as usize;
        self.read_image_into_buf(frame)?;
        log_line(&format!(
            "render {}x{} origin={:?} scale={} layer_size={:?} time={:.3} flat_len={}",
            w,
            h,
            frame.origin,
            frame.scale,
            layer_size,
            time,
            flat.len()
        ));
        rimlight::render(
            &mut self.px_buf,
            w,
            h,
            frame.origin,
            layer_size,
            frame.scale,
            flat,
        )?;
        self.write_image_from_buf(frame)?;
        log_line(&format!(
            "render {w}x{h} done in {} ms",
            started.elapsed().as_millis()
        ));
        Ok(())
    }

    fn read_image_into_buf(&mut self, r: &ImageRef) -> Result<(), String> {
        let n = r.width as usize * r.height as usize * 4;
        self.px_buf.resize(n, 0.0);
        let file = self.frames.as_mut().ok_or("no frame exchange file")?;
        file.seek(SeekFrom::Start(r.offset))
            .map_err(|e| e.to_string())?;
        #[cfg(target_endian = "little")]
        {
            let byte_slice = unsafe {
                std::slice::from_raw_parts_mut(self.px_buf.as_mut_ptr() as *mut u8, n * 4)
            };
            file.read_exact(byte_slice).map_err(|e| e.to_string())?;
        }
        #[cfg(not(target_endian = "little"))]
        {
            let mut tmp = vec![0u8; n * 4];
            file.read_exact(&mut tmp).map_err(|e| e.to_string())?;
            for (dst, chunk) in self.px_buf.iter_mut().zip(tmp.chunks_exact(4)) {
                *dst = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            }
        }
        Ok(())
    }

    fn write_image_from_buf(&mut self, r: &ImageRef) -> Result<(), String> {
        let n = r.width as usize * r.height as usize * 4;
        let file = self.frames.as_mut().ok_or("no frame exchange file")?;
        file.seek(SeekFrom::Start(r.offset))
            .map_err(|e| e.to_string())?;
        #[cfg(target_endian = "little")]
        {
            let byte_slice = unsafe {
                std::slice::from_raw_parts(self.px_buf.as_ptr() as *const u8, n * 4)
            };
            file.write_all(byte_slice).map_err(|e| e.to_string())?;
        }
        #[cfg(not(target_endian = "little"))]
        {
            let mut tmp = vec![0u8; n * 4];
            for (dst, &v) in tmp.chunks_exact_mut(4).zip(self.px_buf[..n].iter()) {
                dst.copy_from_slice(&v.to_le_bytes());
            }
            file.write_all(&tmp).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn log_line(msg: &str) {
    if !debug_on() {
        return;
    }
    let path = std::env::temp_dir().join("effectcraft-mos-rimlight.log");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{msg}");
    }
}

fn debug_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::temp_dir().join("mos_rimlight_dump").exists())
}
