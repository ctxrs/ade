use std::collections::HashMap;

use anyhow::{anyhow, Result};
use gpui::Rgba;

#[derive(Debug, Clone)]
pub struct ThemeTokens {
    pub bg: String,
    pub panel: String,
    pub panel_2: String,
    pub border: String,
    pub border_strong: String,
    pub text: String,
    pub muted: String,
    #[allow(dead_code)]
    pub shadow: String,
    pub accent: String,
    pub success: String,
    pub warning: String,
    pub error: String,
    #[allow(dead_code)]
    pub mono: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ThemeColors {
    pub bg: Rgba,
    pub panel: Rgba,
    pub panel_2: Rgba,
    pub border: Rgba,
    pub border_strong: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub accent: Rgba,
    pub success: Rgba,
    pub warning: Rgba,
    pub error: Rgba,
}

impl ThemeTokens {
    pub fn dark_from_web() -> Result<Self> {
        let css = include_str!("../../web/src/styles.css");
        Self::from_css(css)
    }

    pub fn dark_fallback() -> Self {
        Self {
            bg: "#1e1e1e".to_string(),
            panel: "#252526".to_string(),
            panel_2: "#2d2d2d".to_string(),
            border: "rgba(255, 255, 255, 0.08)".to_string(),
            border_strong: "rgba(255, 255, 255, 0.14)".to_string(),
            text: "#d4d4d4".to_string(),
            muted: "rgba(212, 212, 212, 0.65)".to_string(),
            shadow: "none".to_string(),
            accent: "#3794ff".to_string(),
            success: "#22c55e".to_string(),
            warning: "#fbbf24".to_string(),
            error: "#ef4444".to_string(),
            mono: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \"Liberation Mono\", \"Courier New\", monospace".to_string(),
        }
    }

    pub fn light() -> Self {
        Self {
            bg: "#ffffff".to_string(),
            panel: "#f8f8f8".to_string(),
            panel_2: "#f3f3f3".to_string(),
            border: "#e0e0e0".to_string(),
            border_strong: "#d4d4d4".to_string(),
            text: "#3b3b3b".to_string(),
            muted: "rgba(59, 59, 59, 0.65)".to_string(),
            shadow: "none".to_string(),
            accent: "#005fb8".to_string(),
            success: "#2da44e".to_string(),
            warning: "#b97700".to_string(),
            error: "#f85149".to_string(),
            mono: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \"Liberation Mono\", \"Courier New\", monospace".to_string(),
        }
    }

    pub fn from_css(css: &str) -> Result<Self> {
        let map = parse_css_variables(css);
        Ok(Self {
            bg: get_token(&map, "bg")?,
            panel: get_token(&map, "panel")?,
            panel_2: get_token(&map, "panel-2")?,
            border: get_token(&map, "border")?,
            border_strong: get_token(&map, "border-strong")?,
            text: get_token(&map, "text")?,
            muted: get_token(&map, "muted")?,
            shadow: get_token(&map, "shadow")?,
            accent: get_token(&map, "accent")?,
            success: get_token(&map, "success")?,
            warning: get_token(&map, "warning")?,
            error: get_token(&map, "error")?,
            mono: get_token(&map, "mono")?,
        })
    }
}

impl ThemeColors {
    pub fn from_tokens(tokens: &ThemeTokens) -> Result<Self> {
        Ok(Self {
            bg: parse_color(&tokens.bg)?,
            panel: parse_color(&tokens.panel)?,
            panel_2: parse_color(&tokens.panel_2)?,
            border: parse_color(&tokens.border)?,
            border_strong: parse_color(&tokens.border_strong)?,
            text: parse_color(&tokens.text)?,
            muted: parse_color(&tokens.muted)?,
            accent: parse_color(&tokens.accent)?,
            success: parse_color(&tokens.success)?,
            warning: parse_color(&tokens.warning)?,
            error: parse_color(&tokens.error)?,
        })
    }

    pub fn fallback_dark() -> Self {
        Self::from_tokens(&ThemeTokens::dark_fallback()).unwrap_or(Self::fallback_safe())
    }

    fn fallback_safe() -> Self {
        let opaque = parse_color("#1e1e1e").unwrap_or(Rgba::default());
        Self {
            bg: opaque,
            panel: opaque,
            panel_2: opaque,
            border: opaque,
            border_strong: opaque,
            text: opaque,
            muted: opaque,
            accent: opaque,
            success: opaque,
            warning: opaque,
            error: opaque,
        }
    }
}

fn get_token(map: &HashMap<String, String>, key: &str) -> Result<String> {
    map.get(key)
        .cloned()
        .ok_or_else(|| anyhow!("missing CSS token --{}", key))
}

fn parse_css_variables(css: &str) -> HashMap<String, String> {
    let cleaned = strip_comments(css);
    let mut in_root = false;
    let mut root_body = String::new();
    for line in cleaned.lines() {
        if !in_root {
            if let Some(idx) = line.find(":root") {
                in_root = true;
                if let Some(open) = line[idx..].find('{') {
                    root_body.push_str(&line[idx + open + 1..]);
                    root_body.push('\n');
                }
            }
            continue;
        }
        if let Some(idx) = line.find('}') {
            root_body.push_str(&line[..idx]);
            break;
        }
        root_body.push_str(line);
        root_body.push('\n');
    }

    let mut vars = HashMap::new();
    let mut chars = root_body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '-' && chars.peek() == Some(&'-') {
            chars.next();
            let mut name = String::new();
            while let Some(c) = chars.next() {
                if c == ':' {
                    break;
                }
                name.push(c);
            }
            let mut value = String::new();
            while let Some(c) = chars.next() {
                if c == ';' {
                    break;
                }
                value.push(c);
            }
            let key = name.trim().to_string();
            if !key.is_empty() {
                let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
                vars.insert(key, normalized);
            }
        }
    }
    vars
}

fn strip_comments(css: &str) -> String {
    let mut output = String::new();
    let mut chars = css.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(inner) = chars.next() {
                if inner == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            continue;
        }
        output.push(ch);
    }
    output
}

fn parse_color(value: &str) -> Result<Rgba> {
    let trimmed = value.trim();
    if trimmed.starts_with('#') {
        let hex = trimmed.trim_start_matches('#');
        return parse_hex_color(hex);
    }
    if trimmed.starts_with("rgba(") && trimmed.ends_with(')') {
        return parse_rgba_function(trimmed);
    }
    Err(anyhow!("unsupported color value: {}", value))
}

fn parse_hex_color(hex: &str) -> Result<Rgba> {
    let expanded = match hex.len() {
        3 => {
            let mut out = String::new();
            for ch in hex.chars() {
                out.push(ch);
                out.push(ch);
            }
            out
        }
        6 => hex.to_string(),
        _ => return Err(anyhow!("invalid hex color: #{}", hex)),
    };
    let value = u32::from_str_radix(&expanded, 16)
        .map_err(|_| anyhow!("invalid hex color: #{}", hex))?;
    Ok(gpui::rgb(value))
}

fn parse_rgba_function(value: &str) -> Result<Rgba> {
    let inner = value.trim_start_matches("rgba(").trim_end_matches(')');
    let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
    if parts.len() != 4 {
        return Err(anyhow!("invalid rgba color: {}", value));
    }
    let r = parse_color_channel(parts[0])?;
    let g = parse_color_channel(parts[1])?;
    let b = parse_color_channel(parts[2])?;
    let a = parse_alpha_channel(parts[3])?;
    let rgba = ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | a as u32;
    Ok(gpui::rgba(rgba))
}

fn parse_color_channel(value: &str) -> Result<u8> {
    if let Some(percent) = value.strip_suffix('%') {
        let pct: f32 = percent
            .trim()
            .parse()
            .map_err(|_| anyhow!("invalid color percent: {}", value))?;
        let scaled = (pct.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8;
        return Ok(scaled);
    }
    let parsed: f32 = value
        .parse()
        .map_err(|_| anyhow!("invalid color channel: {}", value))?;
    Ok(parsed.clamp(0.0, 255.0).round() as u8)
}

fn parse_alpha_channel(value: &str) -> Result<u8> {
    let parsed: f32 = value
        .parse()
        .map_err(|_| anyhow!("invalid alpha channel: {}", value))?;
    Ok((parsed.clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_css_variables() {
        let css = r#"
        :root {
          --bg: #111111;
          --panel: #222222;
          --panel-2: #333333;
          --border: rgba(255, 255, 255, 0.08);
          --border-strong: rgba(255, 255, 255, 0.14);
          --text: #dddddd;
          --muted: rgba(212, 212, 212, 0.65);
          --shadow: none;
          --accent: #3794ff;
          --success: #22c55e;
          --warning: #fbbf24;
          --error: #ef4444;
          --mono: ui-monospace, SFMono-Regular,
            Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
        }
        "#;
        let tokens = ThemeTokens::from_css(css).unwrap();
        assert_eq!(tokens.bg, "#111111");
        assert_eq!(tokens.panel_2, "#333333");
        assert!(tokens.mono.contains("SFMono-Regular"));
        assert_eq!(tokens.shadow, "none");
    }
}
