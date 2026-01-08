use std::borrow::Cow;

use gpui::{AssetSource, Result, Rgba, SharedString, Svg, prelude::*, px, svg};

// Lucide icons (MIT), rendered as monochrome SVG masks.
const SETTINGS_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915"/><circle cx="12" cy="12" r="3"/></svg>"#;
const REFRESH_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/></svg>"#;
const SEND_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z"/><path d="m21.854 2.147-10.94 10.939"/></svg>"#;
const INTERRUPT_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><rect x="9" y="9" width="6" height="6" rx="1"/></svg>"#;
const CANCEL_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="m15 9-6 6"/><path d="m9 9 6 6"/></svg>"#;
const ARTIFACT_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"/><path d="M14 2v5a1 1 0 0 0 1 1h5"/></svg>"#;
const IMAGE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/></svg>"#;
const DIFF_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="6" x2="6" y1="3" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/></svg>"#;
const SESSIONS_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="20" height="14" x="2" y="3" rx="2"/><line x1="8" x2="16" y1="21" y2="21"/><line x1="12" x2="12" y1="17" y2="21"/></svg>"#;
const TERMINAL_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19h8"/><path d="m4 17 6-6-6-6"/></svg>"#;

#[derive(Clone, Copy, Debug)]
pub(crate) enum IconName {
    Settings,
    Refresh,
    Send,
    Interrupt,
    Cancel,
    Artifact,
    Image,
    Diff,
    Sessions,
    Terminal,
}

impl IconName {
    const fn path(self) -> &'static str {
        match self {
            Self::Settings => "icons/settings.svg",
            Self::Refresh => "icons/refresh-cw.svg",
            Self::Send => "icons/send.svg",
            Self::Interrupt => "icons/circle-stop.svg",
            Self::Cancel => "icons/circle-x.svg",
            Self::Artifact => "icons/file.svg",
            Self::Image => "icons/image.svg",
            Self::Diff => "icons/git-branch.svg",
            Self::Sessions => "icons/monitor.svg",
            Self::Terminal => "icons/terminal.svg",
        }
    }

    const fn svg(self) -> &'static str {
        match self {
            Self::Settings => SETTINGS_SVG,
            Self::Refresh => REFRESH_SVG,
            Self::Send => SEND_SVG,
            Self::Interrupt => INTERRUPT_SVG,
            Self::Cancel => CANCEL_SVG,
            Self::Artifact => ARTIFACT_SVG,
            Self::Image => IMAGE_SVG,
            Self::Diff => DIFF_SVG,
            Self::Sessions => SESSIONS_SVG,
            Self::Terminal => TERMINAL_SVG,
        }
    }
}

pub(crate) struct IconAssets;

impl IconAssets {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl AssetSource for IconAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let svg = match path {
            "icons/settings.svg" => IconName::Settings.svg(),
            "icons/refresh-cw.svg" => IconName::Refresh.svg(),
            "icons/send.svg" => IconName::Send.svg(),
            "icons/circle-stop.svg" => IconName::Interrupt.svg(),
            "icons/circle-x.svg" => IconName::Cancel.svg(),
            "icons/file.svg" => IconName::Artifact.svg(),
            "icons/image.svg" => IconName::Image.svg(),
            "icons/git-branch.svg" => IconName::Diff.svg(),
            "icons/monitor.svg" => IconName::Sessions.svg(),
            "icons/terminal.svg" => IconName::Terminal.svg(),
            _ => return Ok(None),
        };

        Ok(Some(Cow::Borrowed(svg.as_bytes())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let path = path.trim_end_matches('/');
        if path == "icons" {
            Ok(vec![
                SharedString::from("settings.svg"),
                SharedString::from("refresh-cw.svg"),
                SharedString::from("send.svg"),
                SharedString::from("circle-stop.svg"),
                SharedString::from("circle-x.svg"),
                SharedString::from("file.svg"),
                SharedString::from("image.svg"),
                SharedString::from("git-branch.svg"),
                SharedString::from("monitor.svg"),
                SharedString::from("terminal.svg"),
            ])
        } else {
            Ok(Vec::new())
        }
    }
}

pub(crate) struct Icon {
    name: IconName,
    size: f32,
    color: Rgba,
}

impl Icon {
    pub(crate) fn new(name: IconName, size: f32, color: Rgba) -> Self {
        Self { name, size, color }
    }
}

impl IntoElement for Icon {
    type Element = Svg;

    fn into_element(self) -> Self::Element {
        svg()
            .path(self.name.path())
            .w(px(self.size))
            .h(px(self.size))
            .text_color(self.color)
            .flex_none()
    }
}
