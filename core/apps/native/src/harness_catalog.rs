<<<<<<< HEAD
use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use gpui::{Image, ImageFormat};
use image::{DynamicImage, ImageFormat as PngFormat};
use image::imageops::colorops::{huerotate_in_place, invert};

pub(crate) struct HarnessCatalogEntry {
    pub(crate) id: &'static str,
    pub(crate) invert_in_dark: bool,
    pub(crate) image: Arc<Image>,
    pub(crate) inverted_image: Arc<Image>,
}

pub(crate) fn harness_catalog() -> &'static [HarnessCatalogEntry] {
    static CATALOG: OnceLock<Vec<HarnessCatalogEntry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        vec![
            entry("claude", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/claude.png")), false),
            entry("codex", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/openai.png")), true),
            entry("qwen", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/qwen.png")), false),
            entry("cursor", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/cursorlogo.png")), true),
            entry("amp", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/ampcode.png")), false),
            entry("droid", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/factorydroid.png")), true),
            entry("gemini", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/gemini.png")), false),
            entry("copilot", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/ghcopilot.png")), true),
            entry("opencode", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/opencode.png")), true),
            entry("cline", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/cline.png")), false),
            entry("mistral", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/mistral.png")), false),
            entry("auggie", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/augmentcode.png")), true),
            entry("goose", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/goose.png")), false),
            entry("kimi", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/kimi.png")), false),
            entry("kiro", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/kiro.png")), false),
            entry("codebuff", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/codebuff.png")), false),
            entry("charm", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/charm.png")), false),
            entry("rovo", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/emdash-logos/atlassian.png")), false),
            entry("aider", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/aider.png")), false),
            entry("continue", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/continue.png")), false),
            entry("openhands", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/openhands.png")), false),
            entry("swe-agent", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/swe-agent.png")), false),
            entry("cagent", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/cagent.png")), false),
            entry("kilo", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/kilo.png")), false),
            entry("cody", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/cody.png")), false),
            entry("junie", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/assets/harness-logos/junie.png")), false),
        ]
    })
}

pub(crate) fn harness_entry(id: &str) -> Option<&'static HarnessCatalogEntry> {
    harness_catalog().iter().find(|entry| entry.id == id)
}

fn entry(
    id: &'static str,
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
=======
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{Image, ImageFormat};

#[derive(Clone, Debug)]
pub(crate) struct HarnessCatalogEntry {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) logo_bytes: Option<&'static [u8]>,
    pub(crate) invert_in_dark: bool,
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

pub(crate) const HARNESS_CATALOG: &[HarnessCatalogEntry] = &[
    HarnessCatalogEntry {
        id: "claude",
        label: "Claude Code",
        logo_bytes: Some(LOGO_CLAUDE),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "codex",
        label: "Codex",
        logo_bytes: Some(LOGO_CODEX),
        invert_in_dark: true,
    },
    HarnessCatalogEntry {
        id: "qwen",
        label: "Qwen Code",
        logo_bytes: Some(LOGO_QWEN),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "cursor",
        label: "Cursor",
        logo_bytes: Some(LOGO_CURSOR),
        invert_in_dark: true,
    },
    HarnessCatalogEntry {
        id: "amp",
        label: "Amp",
        logo_bytes: Some(LOGO_AMP),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "droid",
        label: "Droid",
        logo_bytes: Some(LOGO_DROID),
        invert_in_dark: true,
    },
    HarnessCatalogEntry {
        id: "gemini",
        label: "Gemini",
        logo_bytes: Some(LOGO_GEMINI),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "copilot",
        label: "Copilot",
        logo_bytes: Some(LOGO_COPILOT),
        invert_in_dark: true,
    },
    HarnessCatalogEntry {
        id: "opencode",
        label: "OpenCode",
        logo_bytes: Some(LOGO_OPENCODE),
        invert_in_dark: true,
    },
    HarnessCatalogEntry {
        id: "cline",
        label: "Cline",
        logo_bytes: Some(LOGO_CLINE),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "mistral",
        label: "Mistral Vibe",
        logo_bytes: Some(LOGO_MISTRAL),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "auggie",
        label: "Auggie",
        logo_bytes: Some(LOGO_AUGMENT),
        invert_in_dark: true,
    },
    HarnessCatalogEntry {
        id: "goose",
        label: "Goose",
        logo_bytes: Some(LOGO_GOOSE),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "kimi",
        label: "Kimi",
        logo_bytes: Some(LOGO_KIMI),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "kiro",
        label: "Kiro",
        logo_bytes: Some(LOGO_KIRO),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "codebuff",
        label: "Codebuff",
        logo_bytes: Some(LOGO_CODEBUFF),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "charm",
        label: "Charm",
        logo_bytes: Some(LOGO_CHARM),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "rovo",
        label: "Rovo Dev",
        logo_bytes: Some(LOGO_ATLASSIAN),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "aider",
        label: "Aider",
        logo_bytes: Some(LOGO_AIDER),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "continue",
        label: "Continue",
        logo_bytes: Some(LOGO_CONTINUE),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "openhands",
        label: "OpenHands",
        logo_bytes: Some(LOGO_OPENHANDS),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "swe-agent",
        label: "SWE-agent",
        logo_bytes: Some(LOGO_SWE_AGENT),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "cagent",
        label: "cagent",
        logo_bytes: Some(LOGO_CAGENT),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "kilo",
        label: "Kilo Code",
        logo_bytes: Some(LOGO_KILO),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "cody",
        label: "Cody",
        logo_bytes: Some(LOGO_CODY),
        invert_in_dark: false,
    },
    HarnessCatalogEntry {
        id: "junie",
        label: "JetBrains Junie",
        logo_bytes: Some(LOGO_JUNIE),
        invert_in_dark: false,
    },
];

pub(crate) fn harness_entry(id: &str) -> Option<&'static HarnessCatalogEntry> {
    HARNESS_CATALOG.iter().find(|entry| entry.id == id)
}

pub(crate) fn harness_label(id: &str) -> String {
    harness_entry(id)
        .map(|entry| entry.label.to_string())
        .unwrap_or_else(|| id.to_string())
}

pub(crate) fn build_harness_logo_map() -> HashMap<String, Arc<Image>> {
    let mut map = HashMap::new();
    for entry in HARNESS_CATALOG {
        let Some(bytes) = entry.logo_bytes else { continue; };
        let image = Image::from_bytes(ImageFormat::Png, bytes.to_vec());
        map.insert(entry.id.to_string(), Arc::new(image));
    }
    map
>>>>>>> 44e8674 (Implement native composer parity)
}
