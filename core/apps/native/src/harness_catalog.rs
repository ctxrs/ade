use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use gpui::{Image, ImageFormat};
use image::imageops::colorops::{huerotate_in_place, invert};
use image::{DynamicImage, ImageFormat as PngFormat};

#[derive(Clone, Debug)]
pub(crate) struct HarnessCatalogEntry {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) invert_in_dark: bool,
    pub(crate) image: Arc<Image>,
    pub(crate) inverted_image: Arc<Image>,
}

const LOGO_AMP: &[u8] = include_bytes!("../assets/emdash-logos/ampcode.png");
const LOGO_ATLASSIAN: &[u8] = include_bytes!("../assets/emdash-logos/atlassian.png");
const LOGO_AUGMENT: &[u8] = include_bytes!("../assets/emdash-logos/augmentcode.png");
const LOGO_CHARM: &[u8] = include_bytes!("../assets/emdash-logos/charm.png");
const LOGO_CLAUDE: &[u8] = include_bytes!("../assets/emdash-logos/claude.png");
const LOGO_CLINE: &[u8] = include_bytes!("../assets/emdash-logos/cline.png");
const LOGO_CODEBUFF: &[u8] = include_bytes!("../assets/emdash-logos/codebuff.png");
const LOGO_CURSOR: &[u8] = include_bytes!("../assets/emdash-logos/cursorlogo.png");
const LOGO_DROID: &[u8] = include_bytes!("../assets/emdash-logos/factorydroid.png");
const LOGO_GEMINI: &[u8] = include_bytes!("../assets/emdash-logos/gemini.png");
const LOGO_COPILOT: &[u8] = include_bytes!("../assets/emdash-logos/ghcopilot.png");
const LOGO_GOOSE: &[u8] = include_bytes!("../assets/emdash-logos/goose.png");
const LOGO_KIMI: &[u8] = include_bytes!("../assets/emdash-logos/kimi.png");
const LOGO_KIRO: &[u8] = include_bytes!("../assets/emdash-logos/kiro.png");
const LOGO_MISTRAL: &[u8] = include_bytes!("../assets/emdash-logos/mistral.png");
const LOGO_CODEX: &[u8] = include_bytes!("../assets/emdash-logos/openai.png");
const LOGO_OPENCODE: &[u8] = include_bytes!("../assets/emdash-logos/opencode.png");
const LOGO_QWEN: &[u8] = include_bytes!("../assets/emdash-logos/qwen.png");

const LOGO_AIDER: &[u8] = include_bytes!("../assets/harness-logos/aider.png");
const LOGO_CAGENT: &[u8] = include_bytes!("../assets/harness-logos/cagent.png");
const LOGO_CODY: &[u8] = include_bytes!("../assets/harness-logos/cody.png");
const LOGO_CONTINUE: &[u8] = include_bytes!("../assets/harness-logos/continue.png");
const LOGO_JUNIE: &[u8] = include_bytes!("../assets/harness-logos/junie.png");
const LOGO_KILO: &[u8] = include_bytes!("../assets/harness-logos/kilo.png");
const LOGO_OPENHANDS: &[u8] = include_bytes!("../assets/harness-logos/openhands.png");
const LOGO_SWE_AGENT: &[u8] = include_bytes!("../assets/harness-logos/swe-agent.png");

pub(crate) fn harness_catalog() -> &'static [HarnessCatalogEntry] {
    static CATALOG: OnceLock<Vec<HarnessCatalogEntry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        vec![
            entry("claude", "Claude Code", LOGO_CLAUDE, false),
            entry("codex", "Codex", LOGO_CODEX, true),
            entry("qwen", "Qwen Code", LOGO_QWEN, false),
            entry("cursor", "Cursor", LOGO_CURSOR, true),
            entry("amp", "Amp", LOGO_AMP, false),
            entry("droid", "Droid", LOGO_DROID, true),
            entry("gemini", "Gemini", LOGO_GEMINI, false),
            entry("copilot", "Copilot", LOGO_COPILOT, true),
            entry("opencode", "OpenCode", LOGO_OPENCODE, true),
            entry("cline", "Cline", LOGO_CLINE, false),
            entry("mistral", "Mistral Vibe", LOGO_MISTRAL, false),
            entry("auggie", "Auggie", LOGO_AUGMENT, true),
            entry("goose", "Goose", LOGO_GOOSE, false),
            entry("kimi", "Kimi", LOGO_KIMI, false),
            entry("kiro", "Kiro", LOGO_KIRO, false),
            entry("codebuff", "Codebuff", LOGO_CODEBUFF, false),
            entry("charm", "Charm", LOGO_CHARM, false),
            entry("rovo", "Rovo Dev", LOGO_ATLASSIAN, false),
            entry("aider", "Aider", LOGO_AIDER, false),
            entry("continue", "Continue", LOGO_CONTINUE, false),
            entry("openhands", "OpenHands", LOGO_OPENHANDS, false),
            entry("swe-agent", "SWE-agent", LOGO_SWE_AGENT, false),
            entry("cagent", "cagent", LOGO_CAGENT, false),
            entry("kilo", "Kilo Code", LOGO_KILO, false),
            entry("cody", "Cody", LOGO_CODY, false),
            entry("junie", "JetBrains Junie", LOGO_JUNIE, false),
        ]
    })
}

pub(crate) fn harness_entry(id: &str) -> Option<&'static HarnessCatalogEntry> {
    harness_catalog().iter().find(|entry| entry.id == id)
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
