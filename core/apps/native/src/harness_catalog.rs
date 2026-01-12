use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use gpui::{Image, ImageFormat};
use image::{DynamicImage, ImageFormat as PngFormat};
use image::imageops::colorops::{huerotate_in_place, invert};

pub(crate) struct HarnessCatalogEntry {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) invert_in_dark: bool,
    pub(crate) image: Arc<Image>,
    pub(crate) inverted_image: Arc<Image>,
}

pub(crate) fn harness_catalog() -> &'static [HarnessCatalogEntry] {
    static CATALOG: OnceLock<Vec<HarnessCatalogEntry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        vec![
            entry("claude", "Claude Code", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/claude.png")), false),
            entry("codex", "Codex", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/openai.png")), true),
            entry("qwen", "Qwen Code", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/qwen.png")), false),
            entry("cursor", "Cursor", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/cursorlogo.png")), true),
            entry("amp", "Amp", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/ampcode.png")), false),
            entry("droid", "Droid", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/factorydroid.png")), true),
            entry("gemini", "Gemini", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/gemini.png")), false),
            entry("copilot", "Copilot", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/ghcopilot.png")), true),
            entry("opencode", "OpenCode", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/opencode.png")), true),
            entry("cline", "Cline", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/cline.png")), false),
            entry("mistral", "Mistral Vibe", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/mistral.png")), false),
            entry("auggie", "Auggie", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/augmentcode.png")), true),
            entry("goose", "Goose", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/goose.png")), false),
            entry("kimi", "Kimi", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/kimi.png")), false),
            entry("kiro", "Kiro", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/kiro.png")), false),
            entry("codebuff", "Codebuff", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/codebuff.png")), false),
            entry("charm", "Charm", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/charm.png")), false),
            entry("rovo", "Rovo Dev", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/atlassian.png")), false),
            entry("aider", "Aider", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/aider.png")), false),
            entry("continue", "Continue", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/continue.png")), false),
            entry("openhands", "OpenHands", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/openhands.png")), false),
            entry("swe-agent", "SWE-agent", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/swe-agent.png")), false),
            entry("cagent", "cagent", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/cagent.png")), false),
            entry("kilo", "Kilo Code", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/kilo.png")), false),
            entry("cody", "Cody", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/cody.png")), false),
            entry("junie", "JetBrains Junie", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/junie.png")), false),
        ]
    })
}

pub(crate) fn harness_entry(id: &str) -> Option<&'static HarnessCatalogEntry> {
    harness_catalog().iter().find(|entry| entry.id == id)
}

pub(crate) fn build_harness_logo_map(is_dark: bool) -> HashMap<String, Arc<Image>> {
    let mut map = HashMap::new();
    for entry in harness_catalog() {
        let image = if is_dark && entry.invert_in_dark {
            entry.inverted_image.clone()
        } else {
            entry.image.clone()
        };
        map.insert(entry.id.to_string(), image);
    }
    map
}

fn entry(
    id: &'static str,
    label: &'static str,
    bytes: &'static [u8],
    invert_in_dark: bool,
) -> HarnessCatalogEntry {
    let image = image_from_bytes(bytes);
    let inverted_image = if invert_in_dark {
        invert_for_dark(bytes).unwrap_or_else(|| image.clone())
    } else {
        image.clone()
    };
    HarnessCatalogEntry {
        id,
        label,
        invert_in_dark,
        image,
        inverted_image,
    }
}

fn image_from_bytes(bytes: &'static [u8]) -> Arc<Image> {
    Arc::new(Image::from_bytes(ImageFormat::Png, bytes.to_vec()))
}

fn invert_for_dark(bytes: &[u8]) -> Option<Arc<Image>> {
    let mut image = image::load_from_memory(bytes).ok()?.into_rgba8();
    invert(&mut image);
    huerotate_in_place(&mut image, 180);
    adjust_saturation(&mut image, 1.1);
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut out), PngFormat::Png)
        .ok()?;
    Some(Arc::new(Image::from_bytes(ImageFormat::Png, out)))
}

fn adjust_saturation(image: &mut image::RgbaImage, factor: f32) {
    for pixel in image.pixels_mut() {
        let (r, g, b, a) = (
            pixel[0] as f32 / 255.0,
            pixel[1] as f32 / 255.0,
            pixel[2] as f32 / 255.0,
            pixel[3],
        );
        let (h, mut s, l) = rgb_to_hsl(r, g, b);
        s = (s * factor).clamp(0.0, 1.0);
        let (nr, ng, nb) = hsl_to_rgb(h, s, l);
        pixel[0] = (nr * 255.0).round().clamp(0.0, 255.0) as u8;
        pixel[1] = (ng * 255.0).round().clamp(0.0, 255.0) as u8;
        pixel[2] = (nb * 255.0).round().clamp(0.0, 255.0) as u8;
        pixel[3] = a;
    }
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g.max(b));
    let min = r.min(g.min(b));
    let l = (max + min) / 2.0;

    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }

    let delta = max - min;
    let s = if l > 0.5 {
        delta / (2.0 - max - min)
    } else {
        delta / (max + min)
    };

    let mut h = if (max - r).abs() < f32::EPSILON {
        (g - b) / delta + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    h /= 6.0;

    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s == 0.0 {
        return (l, l, l);
    }

    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;

    let r = hue_to_rgb(p, q, h + 1.0 / 3.0);
    let g = hue_to_rgb(p, q, h);
    let b = hue_to_rgb(p, q, h - 1.0 / 3.0);

    (r, g, b)
}

fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 1.0 / 2.0 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}
