use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use flate2::read::GzDecoder;
use regex::Regex;
use reqwest::header;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use tokio::time::sleep;
use url::Url;

#[derive(Parser, Debug)]
#[command(name = "ctx-docs-mirror")]
#[command(about = "Deterministic docs mirror helper", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Plan {
        #[arg(long)]
        config: PathBuf,
    },
    Mirror {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = true)]
        clean: bool,
    },
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Strategy {
    Auto,
    Llms,
    Repo,
    Sitemap,
    EditLink,
    Html,
}

#[derive(Debug, Deserialize)]
struct DocsMirrorConfig {
    source: String,
    title: Option<String>,
    docs_url: Option<String>,
    repo_url: Option<String>,
    revision: Option<String>,
    repo_subpath: Option<String>,
    llms_url: Option<String>,
    sitemap_url: Option<String>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    strip_prefix: Option<String>,
    max_pages: Option<usize>,
    min_pages: Option<usize>,
    strategy: Option<Strategy>,
    throttle_ms: Option<u64>,
}

#[derive(Debug, Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum MirrorMethod {
    Llms,
    Repo,
    SitemapMd,
    EditLink,
    Html,
}

#[derive(Debug, Serialize)]
struct MirrorManifest {
    version: u32,
    generated_at: String,
    method: MirrorMethod,
    source: String,
    title: Option<String>,
    docs_url: Option<String>,
    repo_url: Option<String>,
    revision: Option<String>,
    pages: Vec<ManifestPage>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ManifestPage {
    path: String,
    url: Option<String>,
}

struct MirrorPlan {
    method: MirrorMethod,
    docs_url: Option<String>,
    repo_url: Option<String>,
    revision: Option<String>,
    pages: Vec<PageRef>,
    warnings: Vec<String>,
    repo_root: Option<PathBuf>,
    // Keeps cloned repo tempdir alive for mirror output.
    repo_temp: Option<TempDir>,
}

#[derive(Debug)]
struct PageRef {
    path: PathBuf,
    url: Option<String>,
}

struct RepoPlan {
    temp_dir: TempDir,
    docs_root: PathBuf,
    pages: Vec<PageRef>,
}

#[derive(Clone)]
struct RepoHint {
    repo_url: String,
    subpath: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Plan { config } => {
            let cfg = load_config(&config)?;
            let client = http_client()?;
            let plan = resolve_plan(&client, &cfg).await?;
            let manifest = manifest_from_plan(&cfg, &plan);
            let txt = serde_json::to_string_pretty(&manifest)?;
            println!("{txt}");
        }
        Commands::Mirror { config, out, clean } => {
            let cfg = load_config(&config)?;
            let client = http_client()?;
            let plan = resolve_plan(&client, &cfg).await?;
            let out_dir = out
                .or_else(|| std::env::var_os("CTX_DOCS_OUTPUT_DIR").map(PathBuf::from))
                .ok_or_else(|| anyhow!("output directory required (--out or CTX_DOCS_OUTPUT_DIR)"))?;
            if clean {
                clean_output_dir(&out_dir)?;
            }
            fs::create_dir_all(&out_dir)?;
            execute_plan(&client, &cfg, &plan, &out_dir).await?;
            let manifest = manifest_from_plan(&cfg, &plan);
            let manifest_path = out_dir.join("manifest.json");
            fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)
                .with_context(|| format!("writing {}", manifest_path.display()))?;
        }
    }
    Ok(())
}

fn load_config(path: &Path) -> Result<DocsMirrorConfig> {
    let txt = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let cfg: DocsMirrorConfig = toml::from_str(&txt).context("parsing config")?;
    Ok(cfg)
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("ctx-docs-mirror/0.1")
        .build()?)
}

fn clean_output_dir(out: &Path) -> Result<()> {
    if !out.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(out)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

async fn resolve_plan(client: &reqwest::Client, cfg: &DocsMirrorConfig) -> Result<MirrorPlan> {
    let strategy = cfg.strategy.unwrap_or(Strategy::Auto);
    let mut warnings = Vec::new();

    let docs_url = resolve_docs_url(cfg);
    let repo_url = resolve_repo_url(cfg);
    let repo_hint = docs_url
        .as_deref()
        .and_then(infer_repo_hint_from_docs_url);

    let min_pages = cfg.min_pages.unwrap_or(10);
    let mut candidate: Option<(MirrorMethod, Vec<PageRef>)> = None;

    if matches!(strategy, Strategy::Auto | Strategy::Llms) {
        if let Some(urls) = try_llms_urls(client, cfg, docs_url.as_deref()).await? {
            let page_refs = urls
                .into_iter()
                .map(|url| {
                    let path = derive_output_path(cfg, docs_url.as_deref(), &url)?;
                    Ok(PageRef { path, url: Some(url) })
                })
                .collect::<Result<Vec<_>>>()?;
            if matches!(strategy, Strategy::Llms) {
                return Ok(MirrorPlan {
                    method: MirrorMethod::Llms,
                    docs_url: docs_url.clone(),
                    repo_url: None,
                    revision: cfg.revision.clone(),
                    pages: page_refs,
                    warnings,
                    repo_root: None,
                    repo_temp: None,
                });
            }
            candidate = Some((MirrorMethod::Llms, page_refs));
        }
        if matches!(strategy, Strategy::Llms) {
            return Err(anyhow!("llms.txt strategy failed"));
        }
    }

    if matches!(strategy, Strategy::Auto | Strategy::Repo) {
        if let Some(repo_url) = repo_url.clone() {
            let repo_plan = mirror_repo_plan(cfg, &repo_url, None)?;
            return Ok(MirrorPlan {
                method: MirrorMethod::Repo,
                docs_url: docs_url.clone(),
                repo_url: Some(repo_url),
                revision: cfg.revision.clone(),
                pages: repo_plan.pages,
                warnings,
                repo_root: Some(repo_plan.docs_root),
                repo_temp: Some(repo_plan.temp_dir),
            });
        } else if let Some(hint) = repo_hint.clone() {
            let repo_plan = mirror_repo_plan(cfg, &hint.repo_url, hint.subpath.as_deref())?;
            return Ok(MirrorPlan {
                method: MirrorMethod::Repo,
                docs_url: docs_url.clone(),
                repo_url: Some(hint.repo_url),
                revision: cfg.revision.clone(),
                pages: repo_plan.pages,
                warnings,
                repo_root: Some(repo_plan.docs_root),
                repo_temp: Some(repo_plan.temp_dir),
            });
        }
        if matches!(strategy, Strategy::Repo) {
            return Err(anyhow!("repo strategy requires repo_url or git source"));
        }
    }

    let (pages_from_sitemap, sitemap_warnings) = if matches!(strategy, Strategy::Auto | Strategy::Sitemap | Strategy::EditLink | Strategy::Html)
    {
        if let Some(base_url) = docs_url.as_deref() {
            match try_sitemap_pages(client, cfg, base_url).await {
                Ok((pages, warns)) => (Some(pages), warns),
                Err(err) => {
                    warnings.push(format!("sitemap lookup failed: {err}"));
                    (None, vec![])
                }
            }
        } else {
            (None, vec!["docs_url missing".to_string()])
        }
    } else {
        (None, vec![])
    };

    warnings.extend(sitemap_warnings);

    if matches!(strategy, Strategy::Auto | Strategy::Sitemap) {
        if let Some(pages) = pages_from_sitemap.clone() {
            if let Some(md_urls) = try_sitemap_md_urls(client, cfg, &pages).await? {
                let page_refs = md_urls
                    .into_iter()
                    .map(|url| {
                        let path = derive_output_path(cfg, docs_url.as_deref(), &url)?;
                        Ok(PageRef { path, url: Some(url) })
                    })
                    .collect::<Result<Vec<_>>>()?;
                if matches!(strategy, Strategy::Sitemap) {
                    return Ok(MirrorPlan {
                        method: MirrorMethod::SitemapMd,
                        docs_url: docs_url.clone(),
                        repo_url: None,
                        revision: cfg.revision.clone(),
                        pages: page_refs,
                        warnings,
                        repo_root: None,
                        repo_temp: None,
                    });
                }
                candidate = Some((MirrorMethod::SitemapMd, page_refs));
            }
        }
        if matches!(strategy, Strategy::Sitemap) {
            return Err(anyhow!("sitemap strategy failed to find markdown endpoints"));
        }
    }

    if matches!(strategy, Strategy::Auto | Strategy::EditLink) {
        if let Some(pages) = pages_from_sitemap.clone() {
            if let Some(raw_urls) = try_edit_link_urls(client, cfg, &pages).await? {
                let page_refs = raw_urls
                    .into_iter()
                    .map(|url| {
                        let path = derive_output_path(cfg, docs_url.as_deref(), &url)?;
                        Ok(PageRef { path, url: Some(url) })
                    })
                    .collect::<Result<Vec<_>>>()?;
                if matches!(strategy, Strategy::EditLink) {
                    return Ok(MirrorPlan {
                        method: MirrorMethod::EditLink,
                        docs_url: docs_url.clone(),
                        repo_url: None,
                        revision: cfg.revision.clone(),
                        pages: page_refs,
                        warnings,
                        repo_root: None,
                        repo_temp: None,
                    });
                }
                let replace = candidate
                    .as_ref()
                    .map(|(_, pages)| page_refs.len() > pages.len())
                    .unwrap_or(true);
                if replace {
                    candidate = Some((MirrorMethod::EditLink, page_refs));
                }
            }
        }
        if matches!(strategy, Strategy::EditLink) {
            return Err(anyhow!("edit_link strategy failed"));
        }
    }

    if matches!(strategy, Strategy::Auto | Strategy::Html) {
        if matches!(strategy, Strategy::Html) {
            let base_url = docs_url
                .clone()
                .ok_or_else(|| anyhow!("html fallback requires docs_url"))?;
            let mut pages = crawl_html_pages(client, cfg, &base_url).await?;
            if pages.len() <= min_pages {
                match crawl_html_pages_rendered(client, cfg, &base_url).await {
                    Ok(Some(rendered)) => {
                        if rendered.len() > pages.len() {
                            pages = rendered;
                            warnings.push("html crawl used rendered links".to_string());
                        }
                    }
                    Ok(None) => {
                        warnings.push("playwright not available for rendered crawl".to_string());
                    }
                    Err(err) => {
                        warnings.push(format!("rendered crawl failed: {err}"));
                    }
                }
            }
            if pages.is_empty() {
                warnings.push("html crawl found no pages; using docs_url only".to_string());
                pages.push(base_url.clone());
            }
            let page_refs = pages
                .into_iter()
                .map(|url| {
                    let path = derive_output_path(cfg, docs_url.as_deref(), &url)?;
                    Ok(PageRef { path, url: Some(url) })
                })
                .collect::<Result<Vec<_>>>()?;
            return Ok(MirrorPlan {
                method: MirrorMethod::Html,
                docs_url,
                repo_url: None,
                revision: cfg.revision.clone(),
                pages: page_refs,
                warnings,
                repo_root: None,
                repo_temp: None,
            });
        }

        let candidate_count = candidate.as_ref().map(|(_, pages)| pages.len()).unwrap_or(0);
        let html_threshold = match candidate.as_ref().map(|(method, _)| *method) {
            Some(MirrorMethod::EditLink) => min_pages.max(20),
            _ => min_pages,
        };
        let should_consider_html = candidate.is_none() || candidate_count <= html_threshold;
        if should_consider_html {
            let mut pages = pages_from_sitemap.clone().unwrap_or_default();
            if pages.len() <= min_pages {
                let base_url = docs_url
                    .clone()
                    .ok_or_else(|| anyhow!("html fallback requires docs_url"))?;
                let crawled = crawl_html_pages(client, cfg, &base_url).await?;
                if crawled.len() > pages.len() {
                    pages = crawled;
                }
                if pages.len() <= min_pages {
                    match crawl_html_pages_rendered(client, cfg, &base_url).await {
                        Ok(Some(rendered)) => {
                            if rendered.len() > pages.len() {
                                pages = rendered;
                                warnings.push("html crawl used rendered links".to_string());
                            }
                        }
                        Ok(None) => {
                            warnings.push("playwright not available for rendered crawl".to_string());
                        }
                        Err(err) => {
                            warnings.push(format!("rendered crawl failed: {err}"));
                        }
                    }
                }
            }
            if pages.is_empty() {
                if let Some(base_url) = docs_url.clone() {
                    warnings.push("html crawl found no pages; using docs_url only".to_string());
                    pages.push(base_url);
                }
            }
            let page_refs = pages
                .into_iter()
                .map(|url| {
                    let path = derive_output_path(cfg, docs_url.as_deref(), &url)?;
                    Ok(PageRef { path, url: Some(url) })
                })
                .collect::<Result<Vec<_>>>()?;
            if page_refs.len() > candidate_count {
                candidate = Some((MirrorMethod::Html, page_refs));
            }
        }
    }

    if let Some((method, pages)) = candidate {
        return Ok(MirrorPlan {
            method,
            docs_url,
            repo_url: None,
            revision: cfg.revision.clone(),
            pages,
            warnings,
            repo_root: None,
            repo_temp: None,
        });
    }

    Err(anyhow!("no mirror strategy succeeded"))
}

fn resolve_docs_url(cfg: &DocsMirrorConfig) -> Option<String> {
    if let Some(url) = cfg.docs_url.as_ref() {
        return Some(url.clone());
    }
    if cfg.source.starts_with("http://") || cfg.source.starts_with("https://") {
        Some(cfg.source.clone())
    } else {
        None
    }
}

fn resolve_repo_url(cfg: &DocsMirrorConfig) -> Option<String> {
    if let Some(url) = cfg.repo_url.as_ref() {
        return Some(url.clone());
    }
    if looks_like_git_url(&cfg.source) {
        return Some(cfg.source.clone());
    }
    None
}

fn looks_like_git_url(value: &str) -> bool {
    value.ends_with(".git") || value.contains("git@") || value.starts_with("ssh://")
}

fn infer_repo_hint_from_docs_url(docs_url: &str) -> Option<RepoHint> {
    let url = Url::parse(docs_url).ok()?;
    let host = url.host_str()?;
    let mut segments: Vec<&str> = url
        .path()
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    if host == "raw.githubusercontent.com" {
        if segments.len() < 4 {
            return None;
        }
        let org = segments[0];
        let repo = segments[1];
        segments.drain(0..3);
        let subpath = derive_repo_subpath(&segments);
        return Some(RepoHint {
            repo_url: format!("https://github.com/{org}/{repo}.git"),
            subpath,
        });
    }
    if host == "github.com" {
        if segments.len() < 5 {
            return None;
        }
        let org = segments[0];
        let repo = segments[1];
        let kind = segments[2];
        if kind != "blob" && kind != "raw" {
            return None;
        }
        segments.drain(0..4);
        let subpath = derive_repo_subpath(&segments);
        return Some(RepoHint {
            repo_url: format!("https://github.com/{org}/{repo}.git"),
            subpath,
        });
    }
    None
}

fn derive_repo_subpath(segments: &[&str]) -> Option<String> {
    if segments.is_empty() {
        return None;
    }
    let last = segments.last().copied().unwrap_or_default();
    let has_extension = Path::new(last).extension().is_some();
    let end = if has_extension {
        segments.len().saturating_sub(1)
    } else {
        segments.len()
    };
    if end == 0 {
        return None;
    }
    let joined = segments[..end].join("/");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

const RETRY_MAX_ATTEMPTS: usize = 3;
const RETRY_BASE_MS: u64 = 500;
const RETRY_MAX_MS: u64 = 5_000;

async fn send_with_retries(client: &reqwest::Client, url: &str) -> Result<reqwest::Response> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let resp = client.get(url).send().await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        if !should_retry(status) || attempt >= RETRY_MAX_ATTEMPTS {
            return Err(anyhow!("HTTP status {} for url ({})", status, url));
        }
        let delay = retry_delay(&resp, attempt);
        sleep(delay).await;
    }
}

fn should_retry(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
        || status.is_server_error()
}

fn retry_delay(resp: &reqwest::Response, attempt: usize) -> Duration {
    if let Some(value) = resp.headers().get(header::RETRY_AFTER) {
        if let Ok(text) = value.to_str() {
            if let Ok(seconds) = text.trim().parse::<u64>() {
                return Duration::from_secs(seconds);
            }
        }
    }
    let shift = attempt.saturating_sub(1) as u32;
    let multiplier = 1_u64.checked_shl(shift).unwrap_or(u64::MAX);
    let backoff = RETRY_BASE_MS.saturating_mul(multiplier);
    Duration::from_millis(backoff.min(RETRY_MAX_MS))
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let resp = send_with_retries(client, url).await?;
    Ok(resp.text().await?)
}

async fn fetch_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let resp = send_with_retries(client, url).await?;
    Ok(resp.bytes().await?.to_vec())
}

async fn fetch_sitemap_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let bytes = fetch_bytes(client, url).await?;
    if url.ends_with(".gz") {
        let mut decoder = GzDecoder::new(bytes.as_slice());
        let mut out = String::new();
        decoder
            .read_to_string(&mut out)
            .with_context(|| format!("decoding gzip sitemap {}", url))?;
        Ok(out)
    } else {
        String::from_utf8(bytes)
            .with_context(|| format!("decoding sitemap {}", url))
    }
}

async fn throttle(cfg: &DocsMirrorConfig) {
    if let Some(ms) = cfg.throttle_ms {
        sleep(Duration::from_millis(ms)).await;
    }
}

async fn try_llms_urls(
    client: &reqwest::Client,
    cfg: &DocsMirrorConfig,
    docs_url: Option<&str>,
) -> Result<Option<Vec<String>>> {
    let mut candidates = Vec::new();
    if let Some(url) = cfg.llms_url.as_deref() {
        candidates.push(url.to_string());
    } else if let Some(base) = docs_url {
        if let Ok(base_url) = Url::parse(base) {
            if let Ok(url) = base_url.join("llms.txt") {
                candidates.push(url.to_string());
            }
            if let Ok(origin) = base_url.join("/") {
                if let Ok(url) = origin.join("llms.txt") {
                    candidates.push(url.to_string());
                }
            }
        }
    }
    candidates.sort();
    candidates.dedup();

    for llms_url in candidates {
        let text = match fetch_text(client, &llms_url).await {
            Ok(txt) => txt,
            Err(_) => continue,
        };
        if looks_like_html(&text) {
            continue;
        }

        let url_re = Regex::new("https?://[^\\s)\\\"'>]+").unwrap();
        let mut urls: Vec<String> = url_re
            .find_iter(&text)
            .map(|m| m.as_str().to_string())
            .collect();
        urls.retain(|url| Url::parse(url).is_ok());
        urls.sort();
        urls.dedup();

        let urls = filter_urls(cfg, urls);
        let urls = apply_max(cfg, urls);
        if !urls.is_empty() {
            return Ok(Some(urls));
        }
    }

    Ok(None)
}

fn looks_like_html(text: &str) -> bool {
    let trimmed = text.trim_start().to_ascii_lowercase();
    trimmed.starts_with("<!doctype html") || trimmed.starts_with("<html")
}

fn filter_urls(cfg: &DocsMirrorConfig, urls: Vec<String>) -> Vec<String> {
    let includes = cfg.include.as_deref().unwrap_or(&[]);
    let excludes = cfg.exclude.as_deref().unwrap_or(&[]);
    if includes.is_empty() && excludes.is_empty() {
        return urls;
    }

    urls
        .into_iter()
        .filter(|url| {
            let path = Url::parse(url).ok().map(|u| u.path().to_string());
            let Some(path) = path else { return false; };
            let include_ok = if includes.is_empty() {
                true
            } else {
                includes.iter().any(|p| path.starts_with(p))
            };
            let exclude_ok = !excludes.iter().any(|p| path.starts_with(p));
            include_ok && exclude_ok
        })
        .collect()
}

fn apply_max(cfg: &DocsMirrorConfig, mut urls: Vec<String>) -> Vec<String> {
    if let Some(max) = cfg.max_pages {
        urls.truncate(max);
    }
    urls
}

async fn try_sitemap_pages(
    client: &reqwest::Client,
    cfg: &DocsMirrorConfig,
    base_url: &str,
) -> Result<(Vec<String>, Vec<String>)> {
    let mut warnings = Vec::new();
    let base = Url::parse(base_url).context("parsing docs_url")?;
    let origin = base
        .join("/")
        .context("building origin url")?;
    let mut sitemap_candidates = Vec::new();
    if let Some(url) = cfg.sitemap_url.as_deref() {
        sitemap_candidates.push(url.to_string());
    } else {
        if let Ok(url) = base.join("sitemap.xml") {
            sitemap_candidates.push(url.to_string());
        }
        if let Ok(url) = base.join("sitemap.xml.gz") {
            sitemap_candidates.push(url.to_string());
        }
        if let Ok(url) = base.join("sitemap-index.xml") {
            sitemap_candidates.push(url.to_string());
        }
        if let Ok(url) = origin.join("sitemap.xml") {
            sitemap_candidates.push(url.to_string());
        }
        if let Ok(url) = origin.join("sitemap.xml.gz") {
            sitemap_candidates.push(url.to_string());
        }
        if let Ok(url) = origin.join("sitemap-index.xml") {
            sitemap_candidates.push(url.to_string());
        }
    }

    let mut locs = Vec::new();
    for sitemap_url in sitemap_candidates {
        match fetch_sitemap_text(client, &sitemap_url).await {
            Ok(xml) => {
                let parsed = parse_sitemap_locs(&xml);
                if parsed.is_empty() {
                    warnings.push(format!("sitemap {} returned no locs", sitemap_url));
                    continue;
                }
                locs = parsed;
                break;
            }
            Err(err) => {
                warnings.push(format!("sitemap fetch failed for {}: {err}", sitemap_url));
            }
        }
    }

    if locs.is_empty() && cfg.sitemap_url.is_none() {
        let mut robots_sources = Vec::new();
        if let Ok(robots_url) = base.join("robots.txt") {
            robots_sources.push(robots_url);
        }
        if base != origin {
            if let Ok(robots_url) = origin.join("robots.txt") {
                robots_sources.push(robots_url);
            }
        }
        let mut found = false;
        for robots_url in robots_sources {
            match fetch_text(client, robots_url.as_str()).await {
                Ok(text) => {
                    let robots_sitemaps = parse_robots_sitemaps(&text);
                    if !robots_sitemaps.is_empty() {
                        locs = robots_sitemaps;
                        found = true;
                        break;
                    }
                }
                Err(err) => {
                    warnings.push(format!(
                        "robots.txt fetch failed for {}: {err}",
                        robots_url
                    ));
                }
            }
        }
        if !found {
            warnings.push("robots.txt had no sitemap entries".to_string());
        }
    }

    let mut pages = Vec::new();
    if locs.iter().all(|loc| is_sitemap_url(loc)) {
        for loc in locs {
            let xml = fetch_sitemap_text(client, &loc).await?;
            let inner = parse_sitemap_locs(&xml);
            pages.extend(inner);
            throttle(cfg).await;
        }
    } else {
        pages = locs;
    }

    let docs_prefix = path_scope_prefix(base.path());
    let pages = if !docs_prefix.is_empty() && docs_prefix != "/" {
        pages
            .into_iter()
            .filter(|url| {
                Url::parse(url)
                    .ok()
                    .map(|u| u.path().starts_with(&docs_prefix))
                    .unwrap_or(false)
            })
            .collect()
    } else {
        pages
    };
    let pages = filter_urls(cfg, pages);
    let pages = apply_max(cfg, pages);
    if pages.is_empty() {
        warnings.push("sitemap returned no pages after filtering".to_string());
    }
    Ok((pages, warnings))
}

fn parse_sitemap_locs(xml: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let re = Regex::new(r"<loc>([^<]+)</loc>").unwrap();
    for cap in re.captures_iter(xml) {
        urls.push(cap[1].trim().to_string());
    }
    urls
}

async fn crawl_html_pages(
    client: &reqwest::Client,
    cfg: &DocsMirrorConfig,
    docs_url: &str,
) -> Result<Vec<String>> {
    crawl_html_pages_with_seeds(client, cfg, docs_url, &[]).await
}

async fn crawl_html_pages_rendered(
    client: &reqwest::Client,
    cfg: &DocsMirrorConfig,
    docs_url: &str,
) -> Result<Option<Vec<String>>> {
    let Some(seeds) = try_playwright_links(docs_url).await? else {
        return Ok(None);
    };
    let pages = crawl_html_pages_with_seeds(client, cfg, docs_url, &seeds).await?;
    Ok(Some(pages))
}

async fn crawl_html_pages_with_seeds(
    client: &reqwest::Client,
    cfg: &DocsMirrorConfig,
    docs_url: &str,
    seeds: &[String],
) -> Result<Vec<String>> {
    let base = Url::parse(docs_url).context("parsing docs_url")?;
    let origin_scheme = base.scheme().to_string();
    let origin_host = base
        .host_str()
        .ok_or_else(|| anyhow!("docs_url missing host"))?
        .to_string();
    let origin_port = base.port_or_known_default();
    let mut prefix = path_scope_prefix(base.path());
    let mut allow_all = prefix.is_empty() || prefix == "/";
    let max_pages = cfg.max_pages.unwrap_or(200);

    let start = normalize_url(base.clone());
    let mut queue = VecDeque::new();
    let mut seen = HashSet::new();
    let mut results = Vec::new();
    let start_key = start.to_string();
    seen.insert(start_key);
    queue.push_back(start);

    let mut seed_urls = Vec::new();
    for link in seeds {
        let resolved = match base.join(link) {
            Ok(url) => url,
            Err(_) => continue,
        };
        if let Some(normalized) = normalize_candidate(
            resolved,
            &origin_scheme,
            &origin_host,
            origin_port,
            &prefix,
            allow_all,
        ) {
            let key = normalized.to_string();
            if seen.insert(key) {
                seed_urls.push(normalized);
            }
        }
    }
    seed_urls.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    for seed in seed_urls {
        if results.len() + queue.len() >= max_pages {
            break;
        }
        queue.push_back(seed);
    }

    while let Some(url) = queue.pop_front() {
        if results.len() >= max_pages {
            break;
        }
        let resp = match send_with_retries(client, url.as_str()).await {
            Ok(resp) => resp,
            Err(_) => {
                throttle(cfg).await;
                continue;
            }
        };
        let effective_url = resp.url().clone();
        let content_type = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string())
            .unwrap_or_default();
        let html = match resp.text().await {
            Ok(text) => text,
            Err(_) => {
                throttle(cfg).await;
                continue;
            }
        };
        let is_first = results.is_empty();
        results.push(url.to_string());
        if is_first {
            if let Some(host) = effective_url.host_str() {
                if host == origin_host
                    && effective_url.scheme() == origin_scheme
                    && effective_url.port_or_known_default() == origin_port
                {
                    prefix = path_scope_prefix(effective_url.path());
                    allow_all = prefix.is_empty() || prefix == "/";
                }
            }
        }
        let mut next_urls = Vec::new();
        let is_html = content_type.contains("text/html")
            || content_type.contains("application/xhtml")
            || looks_like_html(&html);
        let link_candidates = if is_html {
            extract_links(&html)
        } else {
            extract_markdown_links(&html)
        };
        for link in link_candidates {
            if link.is_empty()
                || link.starts_with('#')
                || link.starts_with("mailto:")
                || link.starts_with("javascript:")
                || link.starts_with("tel:")
            {
                continue;
            }
            let resolved = match effective_url.join(&link) {
                Ok(u) => u,
                Err(_) => continue,
            };
            if resolved.scheme() != "http" && resolved.scheme() != "https" {
                continue;
            }
            let Some(normalized) = normalize_candidate(
                resolved,
                &origin_scheme,
                &origin_host,
                origin_port,
                &prefix,
                allow_all,
            ) else {
                continue;
            };
            let key = normalized.to_string();
            if seen.contains(&key) {
                continue;
            }
            seen.insert(key);
            next_urls.push(normalized);
        }
        next_urls.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        for next in next_urls {
            if results.len() + queue.len() >= max_pages {
                break;
            }
            queue.push_back(next);
        }
        throttle(cfg).await;
    }

    Ok(results)
}

fn normalize_candidate(
    url: Url,
    origin_scheme: &str,
    origin_host: &str,
    origin_port: Option<u16>,
    prefix: &str,
    allow_all: bool,
) -> Option<Url> {
    if url.scheme() != origin_scheme {
        return None;
    }
    if url.host_str() != Some(origin_host) {
        return None;
    }
    if url.port_or_known_default() != origin_port {
        return None;
    }
    if !allow_all && !path_in_scope(url.path(), prefix) {
        return None;
    }
    if is_asset_path(url.path()) {
        return None;
    }
    Some(normalize_url(url))
}

fn parse_robots_sitemaps(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let re = Regex::new(r"(?i)^sitemap:\s*(\S+)").unwrap();
    for line in text.lines() {
        let line = line.trim();
        if let Some(cap) = re.captures(line) {
            urls.push(cap[1].trim().to_string());
        }
    }
    urls.sort();
    urls.dedup();
    urls
}

fn is_sitemap_url(url: &str) -> bool {
    let url = url.split('#').next().unwrap_or(url);
    url.ends_with(".xml") || url.ends_with(".xml.gz")
}

fn normalize_url(mut url: Url) -> Url {
    url.set_fragment(None);
    url.set_query(None);
    let trimmed = url.path().trim_end_matches('/').to_string();
    if trimmed.is_empty() {
        url.set_path("/");
    } else {
        url.set_path(&trimmed);
    }
    url
}

fn path_in_scope(path: &str, prefix: &str) -> bool {
    if prefix.is_empty() || prefix == "/" {
        return true;
    }
    if path == prefix {
        return true;
    }
    path.starts_with(prefix) && path[prefix.len()..].starts_with('/')
}

fn path_scope_prefix(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_string();
    }
    let ext = Path::new(trimmed)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !ext.is_empty() {
        if let Some((dir, _)) = trimmed.rsplit_once('/') {
            if dir.is_empty() {
                return "/".to_string();
            }
            return dir.to_string();
        }
    }
    trimmed.to_string()
}

fn is_asset_path(path: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext.is_empty() {
        return false;
    }
    matches!(
        ext.as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "svg"
            | "webp"
            | "woff"
            | "woff2"
            | "ttf"
            | "otf"
            | "eot"
            | "css"
            | "js"
            | "json"
            | "map"
            | "ico"
            | "webmanifest"
            | "xml"
            | "txt"
            | "pdf"
            | "zip"
            | "tar"
            | "gz"
            | "bz2"
            | "xz"
            | "7z"
            | "mp4"
            | "mp3"
            | "wav"
            | "avi"
            | "mov"
            | "mkv"
    )
}

fn extract_links(html: &str) -> Vec<String> {
    let href_re = Regex::new("href=[\"']([^\"']+)[\"']").unwrap();
    let mut links = Vec::new();
    for cap in href_re.captures_iter(html) {
        links.push(cap[1].to_string());
    }
    links
}

fn extract_markdown_links(text: &str) -> Vec<String> {
    let inline_re = Regex::new(r"\[[^\]]+\]\(([^)\s]+)").unwrap();
    let ref_re = Regex::new(r"(?m)^\s*\[[^\]]+\]:\s*(\S+)").unwrap();
    let auto_re = Regex::new(r"<(https?://[^>]+)>").unwrap();
    let mut links = Vec::new();
    for cap in inline_re.captures_iter(text) {
        links.push(cap[1].to_string());
    }
    for cap in ref_re.captures_iter(text) {
        links.push(cap[1].to_string());
    }
    for cap in auto_re.captures_iter(text) {
        links.push(cap[1].to_string());
    }
    links.sort();
    links.dedup();
    links
}

async fn try_playwright_links(docs_url: &str) -> Result<Option<Vec<String>>> {
    let Some(root) = find_playwright_root() else {
        return Ok(None);
    };
    let script = r#"
const { createRequire } = require('module');
const req = createRequire(process.cwd() + '/');
const { chromium } = req('playwright');
const url = process.argv[2];
(async () => {
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 45000 });
  await page.waitForTimeout(1000);
  const links = await page.$$eval('a[href]', els => els.map(el => el.getAttribute('href')).filter(Boolean));
  console.log(JSON.stringify(links));
  await browser.close();
})().catch(err => {
  console.error(String(err));
  process.exit(1);
});
"#;
    let mut tmp = tempfile::NamedTempFile::new()?;
    tmp.write_all(script.as_bytes())?;
    let output = Command::new("node")
        .arg(tmp.path())
        .arg(docs_url)
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr.trim();
        if msg.is_empty() {
            return Err(anyhow!("playwright crawl failed"));
        }
        return Err(anyhow!("playwright crawl failed: {}", msg));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut links: Vec<String> = serde_json::from_str(&stdout).unwrap_or_default();
    links.retain(|link| !link.trim().is_empty());
    links.sort();
    links.dedup();
    Ok(Some(links))
}

fn find_playwright_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join("apps/web/node_modules/playwright");
        if candidate.exists() {
            return Some(dir.join("apps/web"));
        }
        let candidate = dir.join("core/apps/web/node_modules/playwright");
        if candidate.exists() {
            return Some(dir.join("core/apps/web"));
        }
        let candidate = dir.join("node_modules/playwright");
        if candidate.exists() {
            return Some(dir.clone());
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

async fn try_sitemap_md_urls(
    client: &reqwest::Client,
    cfg: &DocsMirrorConfig,
    pages: &[String],
) -> Result<Option<Vec<String>>> {
    let mut md_urls = Vec::new();
    for page in pages {
        let page = page.split('#').next().unwrap_or(page).trim().to_string();
        let mut parsed = match Url::parse(&page) {
            Ok(url) => url,
            Err(_) => continue,
        };
        let path = parsed.path();
        if path.is_empty() || path == "/" {
            continue;
        }
        parsed.set_query(None);
        parsed.set_fragment(None);
        let mut url = parsed.to_string();
        if url.ends_with('/') {
            url.pop();
        }
        if !url.ends_with(".md") && !url.ends_with(".mdx") {
            url.push_str(".md");
        }
        if send_with_retries(client, &url).await.is_ok() {
            md_urls.push(url);
        }
        throttle(cfg).await;
    }
    md_urls.sort();
    md_urls.dedup();
    if md_urls.is_empty() {
        return Ok(None);
    }
    Ok(Some(md_urls))
}

async fn try_edit_link_urls(
    client: &reqwest::Client,
    cfg: &DocsMirrorConfig,
    pages: &[String],
) -> Result<Option<Vec<String>>> {
    let mut raw_urls = Vec::new();
    for page in pages {
        let html = match fetch_text(client, page).await {
            Ok(txt) => txt,
            Err(_) => continue,
        };
        if let Some(raw) = extract_raw_url_from_edit_link(&html) {
            raw_urls.push(raw);
        }
        throttle(cfg).await;
    }
    raw_urls.sort();
    raw_urls.dedup();
    if raw_urls.is_empty() {
        return Ok(None);
    }
    let raw_urls = apply_max(cfg, raw_urls);
    Ok(Some(raw_urls))
}

fn extract_raw_url_from_edit_link(html: &str) -> Option<String> {
    let href_re = Regex::new("href=\"([^\"]+)\"").unwrap();
    let mut links = Vec::new();
    for cap in href_re.captures_iter(html) {
        links.push(cap[1].to_string());
    }

    let candidates: Vec<String> = links
        .iter()
        .filter(|href| href.contains("github.com") || href.contains("gitlab.com"))
        .cloned()
        .collect();

    for href in candidates {
        if let Some(raw) = github_edit_to_raw(&href).or_else(|| gitlab_edit_to_raw(&href)) {
            return Some(raw);
        }
    }
    None
}

fn github_edit_to_raw(href: &str) -> Option<String> {
    let href = href.split('#').next().unwrap_or(href);
    let url = Url::parse(href).ok()?;
    if url.domain()? != "github.com" {
        return None;
    }
    let path = url.path().trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 5 {
        return None;
    }
    let org = parts[0];
    let repo = parts[1];
    let mode = parts[2];
    if mode != "edit" && mode != "blob" {
        return None;
    }
    let branch = parts[3];
    let file_path = parts[4..].join("/");
    let raw = format!("https://raw.githubusercontent.com/{org}/{repo}/{branch}/{file_path}");
    Some(raw)
}

fn gitlab_edit_to_raw(href: &str) -> Option<String> {
    let href = href.split('#').next().unwrap_or(href);
    let url = Url::parse(href).ok()?;
    if url.domain()? != "gitlab.com" {
        return None;
    }
    let path = url.path().trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 6 {
        return None;
    }
    let org = parts[0];
    let repo = parts[1];
    if parts[2] != "-" || parts[3] != "edit" {
        return None;
    }
    let branch = parts[4];
    let file_path = parts[5..].join("/");
    let raw = format!("https://gitlab.com/{org}/{repo}/-/raw/{branch}/{file_path}");
    Some(raw)
}

fn mirror_repo_plan(
    cfg: &DocsMirrorConfig,
    repo_url: &str,
    override_subpath: Option<&str>,
) -> Result<RepoPlan> {
    let temp_dir = tempfile::tempdir()?;
    let root = temp_dir.path().join("repo");
    let mut cmd = Command::new("git");
    cmd.arg("clone").arg("--depth").arg("1").arg("--no-tags");
    if let Some(rev) = cfg.revision.as_deref() {
        if !looks_like_sha(rev) {
            cmd.arg("--branch").arg(rev);
        }
    }
    cmd.arg(repo_url).arg(&root);
    let output = cmd.output().context("running git clone")?;
    if !output.status.success() {
        return Err(anyhow!(
            "git clone failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    if let Some(rev) = cfg.revision.as_deref() {
        if looks_like_sha(rev) {
            let fetch = Command::new("git")
                .arg("-C")
                .arg(&root)
                .arg("fetch")
                .arg("--depth")
                .arg("1")
                .arg("origin")
                .arg(rev)
                .output()
                .context("running git fetch")?;
            if !fetch.status.success() {
                return Err(anyhow!(
                    "git fetch failed: {}",
                    String::from_utf8_lossy(&fetch.stderr)
                ));
            }
            let checkout = Command::new("git")
                .arg("-C")
                .arg(&root)
                .arg("checkout")
                .arg(rev)
                .output()
                .context("running git checkout")?;
            if !checkout.status.success() {
                return Err(anyhow!(
                    "git checkout failed: {}",
                    String::from_utf8_lossy(&checkout.stderr)
                ));
            }
        }
    }

    let docs_root = if let Some(subpath) = override_subpath.or(cfg.repo_subpath.as_deref()) {
        root.join(subpath)
    } else {
        find_docs_root(&root).ok_or_else(|| anyhow!("no docs root found"))?
    };
    if !docs_root.exists() {
        return Err(anyhow!("docs root not found at {}", docs_root.display()));
    }

    let pages = collect_markdown_files(&docs_root)?;
    let page_refs = pages
        .into_iter()
        .map(|path| {
            let rel = path.strip_prefix(&docs_root).unwrap_or(&path).to_path_buf();
            PageRef { path: rel, url: None }
        })
        .collect();

    Ok(RepoPlan {
        temp_dir,
        docs_root,
        pages: page_refs,
    })
}

fn find_docs_root(repo: &Path) -> Option<PathBuf> {
    let candidates = [
        "docs",
        "website/docs",
        "doc",
        "documentation",
        "site/docs",
        "docs/content",
        "content/docs",
        "content",
    ];
    for cand in candidates {
        let path = repo.join(cand);
        if path.exists() && path.is_dir() {
            return Some(path);
        }
    }
    if let Ok(entries) = fs::read_dir(repo) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.eq_ignore_ascii_case("README.md")
                || name.eq_ignore_ascii_case("README.mdx")
            {
                return Some(repo.to_path_buf());
            }
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext == "md" || ext == "mdx" {
                return Some(repo.to_path_buf());
            }
        }
    }
    None
}

fn collect_markdown_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in walk_dir(root)? {
        let path = entry;
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        if ext == "md" || ext == "mdx" {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn walk_dir(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_dir(&path)?);
        } else {
            out.push(path);
        }
    }
    Ok(out)
}

fn looks_like_sha(value: &str) -> bool {
    let len = value.len();
    if !(7..=40).contains(&len) {
        return false;
    }
    value.chars().all(|c| c.is_ascii_hexdigit())
}

fn derive_output_path(cfg: &DocsMirrorConfig, docs_url: Option<&str>, url: &str) -> Result<PathBuf> {
    let parsed = Url::parse(url).context("parsing url")?;
    let mut path = parsed.path().to_string();
    if path.starts_with('/') {
        path = path.trim_start_matches('/').to_string();
    }

    if let Some(prefix) = cfg.strip_prefix.as_deref() {
        let prefix = prefix.trim_start_matches('/');
        if path.starts_with(prefix) {
            path = path[prefix.len()..].trim_start_matches('/').to_string();
        }
    } else if let Some(base) = docs_url {
        if let Ok(base_url) = Url::parse(base) {
            let base_path = base_url.path().trim_start_matches('/');
            if !base_path.is_empty() && path.starts_with(base_path) {
                path = path[base_path.len()..].trim_start_matches('/').to_string();
            }
        }
    }

    if path.is_empty() {
        path = "index".to_string();
    } else if path.ends_with('/') {
        path.push_str("index");
    }

    if path.ends_with(".html") {
        path = path.trim_end_matches(".html").to_string();
    }

    if !path.ends_with(".md") && !path.ends_with(".mdx") {
        path.push_str(".md");
    }

    Ok(PathBuf::from(path))
}

async fn execute_plan(
    client: &reqwest::Client,
    cfg: &DocsMirrorConfig,
    plan: &MirrorPlan,
    out_dir: &Path,
) -> Result<()> {
    match plan.method {
        MirrorMethod::Repo => mirror_repo_output(plan, out_dir),
        MirrorMethod::Llms | MirrorMethod::SitemapMd | MirrorMethod::EditLink => {
            mirror_url_output(client, cfg, plan, out_dir).await
        }
        MirrorMethod::Html => mirror_html_output(client, cfg, plan, out_dir).await,
    }
}

fn mirror_repo_output(plan: &MirrorPlan, out_dir: &Path) -> Result<()> {
    let _keep_temp = plan.repo_temp.as_ref();
    let Some(repo_root) = plan.repo_root.as_ref() else {
        return Err(anyhow!("repo_root missing"));
    };
    for page in &plan.pages {
        let src = repo_root.join(&page.path);
        let dest = out_dir.join(&page.path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&src, &dest).with_context(|| format!("copying {}", src.display()))?;
    }
    Ok(())
}

async fn mirror_url_output(
    client: &reqwest::Client,
    cfg: &DocsMirrorConfig,
    plan: &MirrorPlan,
    out_dir: &Path,
) -> Result<()> {
    for page in &plan.pages {
        let Some(url) = page.url.as_deref() else {
            continue;
        };
        let body = fetch_bytes(client, url).await?;
        let dest = out_dir.join(&page.path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&dest, &body).with_context(|| format!("writing {}", dest.display()))?;
        throttle(cfg).await;
    }
    Ok(())
}

async fn mirror_html_output(
    client: &reqwest::Client,
    cfg: &DocsMirrorConfig,
    plan: &MirrorPlan,
    out_dir: &Path,
) -> Result<()> {
    for page in &plan.pages {
        let Some(url) = page.url.as_deref() else {
            continue;
        };
        let html = fetch_text(client, url).await?;
        let md = html2md::parse_html(&html);
        let dest = out_dir.join(&page.path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&dest, md).with_context(|| format!("writing {}", dest.display()))?;
        throttle(cfg).await;
    }
    Ok(())
}

fn manifest_from_plan(cfg: &DocsMirrorConfig, plan: &MirrorPlan) -> MirrorManifest {
    let pages = plan
        .pages
        .iter()
        .map(|page| ManifestPage {
            path: page.path.to_string_lossy().to_string(),
            url: page.url.clone(),
        })
        .collect::<Vec<_>>();

    MirrorManifest {
        version: 1,
        generated_at: Utc::now().to_rfc3339(),
        method: plan.method,
        source: cfg.source.clone(),
        title: cfg.title.clone(),
        docs_url: plan.docs_url.clone(),
        repo_url: plan.repo_url.clone(),
        revision: plan.revision.clone(),
        pages,
        warnings: plan.warnings.clone(),
    }
}
