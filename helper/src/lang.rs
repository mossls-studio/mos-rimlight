// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 MOSSLS

//! Language selection for the mos_rimlight external helper.

use serde_json::Value;
use std::path::PathBuf;
use std::sync::OnceLock;

pub const ZH_NAME: &str = "mos_打光";
pub const EN_NAME: &str = "mos_rimlight";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Langs {
    pub names: Lang,
    pub params: Lang,
}

impl Langs {
    const ZH: Langs = Langs {
        names: Lang::Zh,
        params: Lang::Zh,
    };
}

pub fn current() -> Langs {
    static LANGS: OnceLock<Langs> = OnceLock::new();
    *LANGS.get_or_init(detect)
}

fn detect() -> Langs {
    let mut langs = from_config().unwrap_or(Langs::ZH);

    if let Ok(v) = std::env::var("MOS_LANG") {
        if let Some(l) = from_code(&v) {
            langs = Langs { names: l, params: l };
        }
    }
    if let Ok(v) = std::env::var("MOS_NAMES_LANG") {
        if let Some(l) = from_code(&v) {
            langs.names = l;
        }
    }
    if let Ok(v) = std::env::var("MOS_PARAMS_LANG") {
        if let Some(l) = from_code(&v) {
            langs.params = l;
        }
    }
    langs
}

fn from_config() -> Option<Langs> {
    let text = std::fs::read_to_string(suite_config()?).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;

    if let Some(code) = json.get("effectsLanguage").and_then(|v| v.as_str()) {
        if let Some(l) = from_code(code) {
            return Some(Langs { names: l, params: l });
        }
    }

    let names = json
        .get("namesResolved")
        .and_then(|v| v.as_str())
        .and_then(from_code)
        .or_else(|| {
            json.get("namesLanguage")
                .and_then(|v| v.as_str())
                .and_then(from_code)
        })
        .unwrap_or(Lang::Zh);

    let params = json
        .get("paramsResolved")
        .and_then(|v| v.as_str())
        .and_then(from_code)
        .or_else(|| {
            json.get("paramsLanguage")
                .and_then(|v| v.as_str())
                .and_then(from_code)
        })
        .unwrap_or(Lang::Zh);

    Some(Langs { names, params })
}

fn from_code(code: &str) -> Option<Lang> {
    match code.trim().to_ascii_lowercase().as_str() {
        "en" | "en-us" | "en_us" | "english" => Some(Lang::En),
        "zh" | "zh-cn" | "zh_cn" | "zh-hans" | "chinese" => Some(Lang::Zh),
        _ => None,
    }
}

fn suite_config() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(appdata)
            .join("EffectCraft")
            .join("mos-suite.json"),
    )
}

pub fn code(lang: Lang) -> &'static str {
    match lang {
        Lang::Zh => "zh",
        Lang::En => "en",
    }
}

pub fn is_en(lang: Lang) -> bool {
    lang == Lang::En
}

pub fn effect_name() -> &'static str {
    if is_en(current().names) {
        EN_NAME
    } else {
        ZH_NAME
    }
}

fn i18n() -> &'static Value {
    static TABLE: OnceLock<Value> = OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str(include_str!("i18n.json")).expect("helper i18n.json is valid JSON")
    })
}

fn pick<'a>(node: &'a Value, lang: &str) -> Option<&'a str> {
    node.get(lang).and_then(|v| v.as_str())
}

pub fn localize_manifest(manifest: &mut Value) {
    let langs = current();
    let names = code(langs.names);
    let params = code(langs.params);
    let table = i18n();

    for key in ["category", "name", "description"] {
        if let Some(s) = table.get(key).and_then(|v| pick(v, names)) {
            manifest[key] = Value::String(s.to_owned());
        }
    }

    let groups = table.get("groups");
    let overrides = table.get("params");
    let Some(list) = manifest.get_mut("params").and_then(|v| v.as_array_mut()) else {
        return;
    };

    for param in list.iter_mut() {
        if let Some(group_path) = param
            .get("group")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
        {
            let localized = groups
                .and_then(|gm| gm.get(&group_path))
                .and_then(|v| pick(v, params))
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    group_path
                        .split('/')
                        .map(|seg| {
                            groups
                                .and_then(|gm| gm.get(seg))
                                .and_then(|v| pick(v, params))
                                .unwrap_or(seg)
                        })
                        .collect::<Vec<_>>()
                        .join("/")
                });
            param["group"] = Value::String(localized);
        }

        let Some(id) = param
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
        else {
            continue;
        };
        let Some(entry) = overrides.and_then(|p| p.get(&id)) else {
            continue;
        };
        if let Some(name) = entry.get("name").and_then(|v| pick(v, params)) {
            param["name"] = Value::String(name.to_owned());
        }
        if let Some(options) = entry
            .get("options")
            .and_then(|v| v.get(params))
            .and_then(|v| v.as_array())
        {
            param["options"] = Value::Array(options.clone());
        }
    }
}
