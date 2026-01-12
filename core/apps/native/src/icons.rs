use std::borrow::Cow;

use gpui::{
    App, AssetSource, Component, RenderOnce, Result, Rgba, SharedString, Transformation, Window,
    prelude::*, px, svg,
};

// Lucide icons (MIT), rendered as monochrome SVG masks.
const SETTINGS_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915\"/><circle cx=\"12\" cy=\"12\" r=\"3\"/></svg>"#;
const REFRESH_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8\"/><path d=\"M21 3v5h-5\"/><path d=\"M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16\"/><path d=\"M8 16H3v5\"/></svg>"#;
const SEND_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z\"/><path d=\"m21.854 2.147-10.94 10.939\"/></svg>"#;
const ARROW_UP_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"m5 12 7-7 7 7\"/><path d=\"M12 19V5\"/></svg>"#;
const INTERRUPT_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"12\" cy=\"12\" r=\"10\"/><rect x=\"9\" y=\"9\" width=\"6\" height=\"6\" rx=\"1\"/></svg>"#;
const SQUARE_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect width=\"18\" height=\"18\" x=\"3\" y=\"3\" rx=\"2\"/></svg>"#;
const CHEVRON_DOWN_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"m6 9 6 6 6-6\"/></svg>"#;
const AT_SIGN_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"12\" cy=\"12\" r=\"4\"/><path d=\"M16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-4 8\"/></svg>"#;
const SLASH_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M22 2 2 22\"/></svg>"#;
const MIC_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M12 19v3\"/><path d=\"M19 10v2a7 7 0 0 1-14 0v-2\"/><rect x=\"9\" y=\"2\" width=\"6\" height=\"13\" rx=\"3\"/></svg>"#;
const ELLIPSIS_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"12\" cy=\"12\" r=\"1\"/><circle cx=\"19\" cy=\"12\" r=\"1\"/><circle cx=\"5\" cy=\"12\" r=\"1\"/></svg>"#;
const LAPTOP_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M18 5a2 2 0 0 1 2 2v8.526a2 2 0 0 0 .212.897l1.068 2.127a1 1 0 0 1-.9 1.45H3.62a1 1 0 0 1-.9-1.45l1.068-2.127A2 2 0 0 0 4 15.526V7a2 2 0 0 1 2-2z\"/><path d=\"M20.054 15.987H3.946\"/></svg>"#;
const CONTAINER_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M22 7.7c0-.6-.4-1.2-.8-1.5l-6.3-3.9a1.72 1.72 0 0 0-1.7 0l-10.3 6c-.5.2-.9.8-.9 1.4v6.6c0 .5.4 1.2.8 1.5l6.3 3.9a1.72 1.72 0 0 0 1.7 0l10.3-6c.5-.3.9-1 .9-1.5Z\"/><path d=\"M10 21.9V14L2.1 9.1\"/><path d=\"m10 14 11.9-6.9\"/><path d=\"M14 19.8v-8.1\"/><path d=\"M18 17.5V9.4\"/></svg>"#;
const INFO_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"12\" cy=\"12\" r=\"10\"/><path d=\"M12 16v-4\"/><path d=\"M12 8h.01\"/></svg>"#;
const CANCEL_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"12\" cy=\"12\" r=\"10\"/><path d=\"m15 9-6 6\"/><path d=\"m9 9 6 6\"/></svg>"#;
const ARTIFACT_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z\"/><path d=\"M14 2v5a1 1 0 0 0 1 1h5\"/></svg>"#;
const IMAGE_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect width=\"18\" height=\"18\" x=\"3\" y=\"3\" rx=\"2\" ry=\"2\"/><circle cx=\"9\" cy=\"9\" r=\"2\"/><path d=\"m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21\"/></svg>"#;
const FOLDER_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M3 7h5l2 3h11a1 1 0 0 1 1 1v9a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V8a1 1 0 0 1 1-1z\"/></svg>"#;
const DIFF_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><line x1=\"6\" x2=\"6\" y1=\"3\" y2=\"15\"/><circle cx=\"18\" cy=\"6\" r=\"3\"/><circle cx=\"6\" cy=\"18\" r=\"3\"/><path d=\"M18 9a9 9 0 0 1-9 9\"/></svg>"#;
const SESSIONS_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect width=\"20\" height=\"14\" x=\"2\" y=\"3\" rx=\"2\"/><line x1=\"8\" x2=\"16\" y1=\"21\" y2=\"21\"/><line x1=\"12\" x2=\"12\" y1=\"17\" y2=\"21\"/></svg>"#;
const TERMINAL_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M12 19h8\"/><path d=\"m4 17 6-6-6-6\"/></svg>"#;
const ARCHIVE_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect width=\"20\" height=\"5\" x=\"2\" y=\"3\" rx=\"1\" /><path d=\"M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8\" /><path d=\"M10 12h4\" /></svg>"#;
const CHEVRON_LEFT_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"m15 18-6-6 6-6\" /></svg>"#;
const CHEVRON_RIGHT_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"m9 18 6-6-6-6\" /></svg>"#;
const LAYERS_PLUS_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 .83.18 2 2 0 0 0 .83-.18l8.58-3.9a1 1 0 0 0 0-1.831z\" /><path d=\"M16 17h6\" /><path d=\"M19 14v6\" /><path d=\"M2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 .825.178\" /><path d=\"M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l2.116-.962\" /></svg>"#;
const SPINNER_ARC_SVG: &str = r#"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"12\" height=\"12\" viewBox=\"0 0 12 12\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><circle cx=\"6\" cy=\"6\" r=\"5\" stroke-dasharray=\"7.9 23.5\" transform=\"rotate(-90 6 6)\" /></svg>"#;

#[derive(Clone, Copy, Debug)]
pub(crate) enum IconName {
    Settings,
    Refresh,
    Send,
    ArrowUp,
    Interrupt,
    Square,
    ChevronDown,
    AtSign,
    Slash,
    Mic,
    Ellipsis,
    Laptop,
    Container,
    Info,
    Cancel,
    Artifact,
    Image,
    Folder,
    Diff,
    Sessions,
    Terminal,
    Archive,
    ChevronLeft,
    ChevronRight,
    LayersPlus,
    SpinnerArc,
}

impl IconName {
    pub(crate) const fn path(self) -> &'static str {
        match self {
            Self::Settings => "icons/settings.svg",
            Self::Refresh => "icons/refresh-cw.svg",
            Self::Send => "icons/send.svg",
            Self::ArrowUp => "icons/arrow-up.svg",
            Self::Interrupt => "icons/circle-stop.svg",
            Self::Square => "icons/square.svg",
            Self::ChevronDown => "icons/chevron-down.svg",
            Self::AtSign => "icons/at-sign.svg",
            Self::Slash => "icons/slash.svg",
            Self::Mic => "icons/mic.svg",
            Self::Ellipsis => "icons/ellipsis.svg",
            Self::Laptop => "icons/laptop.svg",
            Self::Container => "icons/container.svg",
            Self::Info => "icons/info.svg",
            Self::Cancel => "icons/circle-x.svg",
            Self::Artifact => "icons/file.svg",
            Self::Image => "icons/image.svg",
            Self::Folder => "icons/folder.svg",
            Self::Diff => "icons/git-branch.svg",
            Self::Sessions => "icons/monitor.svg",
            Self::Terminal => "icons/terminal.svg",
            Self::Archive => "icons/archive.svg",
            Self::ChevronLeft => "icons/chevron-left.svg",
            Self::ChevronRight => "icons/chevron-right.svg",
            Self::LayersPlus => "icons/layers-plus.svg",
            Self::SpinnerArc => "icons/spinner-arc.svg",
        }
    }

    const fn svg(self) -> &'static str {
        match self {
            Self::Settings => SETTINGS_SVG,
            Self::Refresh => REFRESH_SVG,
            Self::Send => SEND_SVG,
            Self::ArrowUp => ARROW_UP_SVG,
            Self::Interrupt => INTERRUPT_SVG,
            Self::Square => SQUARE_SVG,
            Self::ChevronDown => CHEVRON_DOWN_SVG,
            Self::AtSign => AT_SIGN_SVG,
            Self::Slash => SLASH_SVG,
            Self::Mic => MIC_SVG,
            Self::Ellipsis => ELLIPSIS_SVG,
            Self::Laptop => LAPTOP_SVG,
            Self::Container => CONTAINER_SVG,
            Self::Info => INFO_SVG,
            Self::Cancel => CANCEL_SVG,
            Self::Artifact => ARTIFACT_SVG,
            Self::Image => IMAGE_SVG,
            Self::Folder => FOLDER_SVG,
            Self::Diff => DIFF_SVG,
            Self::Sessions => SESSIONS_SVG,
            Self::Terminal => TERMINAL_SVG,
            Self::Archive => ARCHIVE_SVG,
            Self::ChevronLeft => CHEVRON_LEFT_SVG,
            Self::ChevronRight => CHEVRON_RIGHT_SVG,
            Self::LayersPlus => LAYERS_PLUS_SVG,
            Self::SpinnerArc => SPINNER_ARC_SVG,
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
            "icons/arrow-up.svg" => IconName::ArrowUp.svg(),
            "icons/circle-stop.svg" => IconName::Interrupt.svg(),
            "icons/square.svg" => IconName::Square.svg(),
            "icons/chevron-down.svg" => IconName::ChevronDown.svg(),
            "icons/at-sign.svg" => IconName::AtSign.svg(),
            "icons/slash.svg" => IconName::Slash.svg(),
            "icons/mic.svg" => IconName::Mic.svg(),
            "icons/ellipsis.svg" => IconName::Ellipsis.svg(),
            "icons/laptop.svg" => IconName::Laptop.svg(),
            "icons/container.svg" => IconName::Container.svg(),
            "icons/info.svg" => IconName::Info.svg(),
            "icons/circle-x.svg" => IconName::Cancel.svg(),
            "icons/file.svg" => IconName::Artifact.svg(),
            "icons/image.svg" => IconName::Image.svg(),
            "icons/folder.svg" => IconName::Folder.svg(),
            "icons/git-branch.svg" => IconName::Diff.svg(),
            "icons/monitor.svg" => IconName::Sessions.svg(),
            "icons/terminal.svg" => IconName::Terminal.svg(),
            "icons/archive.svg" => IconName::Archive.svg(),
            "icons/chevron-left.svg" => IconName::ChevronLeft.svg(),
            "icons/chevron-right.svg" => IconName::ChevronRight.svg(),
            "icons/layers-plus.svg" => IconName::LayersPlus.svg(),
            "icons/spinner-arc.svg" => IconName::SpinnerArc.svg(),
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
                SharedString::from("arrow-up.svg"),
                SharedString::from("circle-stop.svg"),
                SharedString::from("square.svg"),
                SharedString::from("chevron-down.svg"),
                SharedString::from("at-sign.svg"),
                SharedString::from("slash.svg"),
                SharedString::from("mic.svg"),
                SharedString::from("ellipsis.svg"),
                SharedString::from("laptop.svg"),
                SharedString::from("container.svg"),
                SharedString::from("info.svg"),
                SharedString::from("circle-x.svg"),
                SharedString::from("file.svg"),
                SharedString::from("image.svg"),
                SharedString::from("folder.svg"),
                SharedString::from("git-branch.svg"),
                SharedString::from("monitor.svg"),
                SharedString::from("terminal.svg"),
                SharedString::from("archive.svg"),
                SharedString::from("chevron-left.svg"),
                SharedString::from("chevron-right.svg"),
                SharedString::from("layers-plus.svg"),
                SharedString::from("spinner-arc.svg"),
            ])
        } else {
            Ok(Vec::new())
        }
    }
}

pub(crate) struct Icon {
    name: IconName,
    size: f32,
    color: Option<Rgba>,
    transformation: Option<Transformation>,
}

impl Icon {
    pub(crate) fn new(name: IconName, size: f32, color: Rgba) -> Self {
        Self {
            name,
            size,
            color: Some(color),
            transformation: None,
        }
    }

    pub(crate) fn current(name: IconName, size: f32) -> Self {
        Self {
            name,
            size,
            color: None,
            transformation: None,
        }
    }

    pub(crate) fn transform(mut self, transformation: Transformation) -> Self {
        self.transformation = Some(transformation);
        self
    }

}

impl RenderOnce for Icon {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let color = self
            .color
            .unwrap_or_else(|| window.text_style().color.into());
        let mut icon = svg()
            .path(self.name.path())
            .w(px(self.size))
            .h(px(self.size))
            .flex_none()
            .text_color(color);

        if let Some(transformation) = self.transformation {
            icon = icon.with_transformation(transformation);
        }

        icon
    }
}

impl IntoElement for Icon {
    type Element = Component<Icon>;

    fn into_element(self) -> Self::Element {
        Component::new(self)
    }
}
