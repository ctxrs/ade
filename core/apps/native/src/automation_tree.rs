#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use gpui::{App, Bounds, Pixels, Window};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq)]
pub struct AutomationBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AutomationNode {
    pub id: String,
    pub role: String,
    pub name: Option<String>,
    pub bounds: AutomationBounds,
    pub visible: bool,
    pub enabled: bool,
    pub children: Vec<AutomationNode>,
}

#[derive(Clone, Copy, Debug)]
pub struct AutomationTarget {
    pub id: &'static str,
    pub role: &'static str,
    pub name: Option<&'static str>,
    pub parent: Option<&'static str>,
}

impl AutomationTarget {
    pub const fn new(
        id: &'static str,
        role: &'static str,
        name: Option<&'static str>,
        parent: Option<&'static str>,
    ) -> Self {
        Self {
            id,
            role,
            name,
            parent,
        }
    }
}

#[derive(Clone, Debug)]
struct AutomationNodeState {
    id: String,
    role: String,
    name: Option<String>,
    parent: Option<String>,
    bounds: AutomationBounds,
    visible: bool,
    enabled: bool,
}

impl AutomationNodeState {
    fn placeholder(id: &str) -> Self {
        Self {
            id: id.to_string(),
            role: "root".to_string(),
            name: None,
            parent: None,
            bounds: AutomationBounds::default(),
            visible: false,
            enabled: false,
        }
    }
}

pub struct AutomationRegistry {
    nodes: Mutex<HashMap<String, AutomationNodeState>>,
}

impl AutomationRegistry {
    pub fn new() -> Self {
        Self {
            nodes: Mutex::new(HashMap::new()),
        }
    }

    pub fn update_target(
        &self,
        target: AutomationTarget,
        bounds: Option<AutomationBounds>,
        visible: bool,
        enabled: bool,
    ) {
        self.update_target_with(
            target.id.to_string(),
            target.role.to_string(),
            target.name.map(|name| name.to_string()),
            target.parent.map(|parent| parent.to_string()),
            bounds,
            visible,
            enabled,
        );
    }

    pub fn update_target_dynamic(
        &self,
        id: String,
        role: String,
        name: Option<String>,
        parent: Option<String>,
        bounds: Option<AutomationBounds>,
        visible: bool,
        enabled: bool,
    ) {
        self.update_target_with(id, role, name, parent, bounds, visible, enabled);
    }

    pub fn clear_prefix(&self, prefix: &str) {
        let mut nodes = self.nodes.lock().expect("automation registry lock");
        nodes.retain(|id, _| !id.starts_with(prefix));
    }

    fn update_target_with(
        &self,
        id: String,
        role: String,
        name: Option<String>,
        parent: Option<String>,
        bounds: Option<AutomationBounds>,
        visible: bool,
        enabled: bool,
    ) {
        let mut nodes = self.nodes.lock().expect("automation registry lock");
        let entry = nodes.entry(id.clone()).or_insert_with(|| AutomationNodeState {
            id,
            role,
            name,
            parent,
            bounds: AutomationBounds::default(),
            visible: false,
            enabled: true,
        });
        if let Some(bounds) = bounds {
            entry.bounds = bounds;
        } else {
            entry.bounds = AutomationBounds::default();
        }
        entry.visible = visible;
        entry.enabled = enabled;
    }

    pub fn snapshot(&self) -> AutomationNode {
        let nodes = self.nodes.lock().expect("automation registry lock");
        let root_id = "app-shell";
        let mut children_map: HashMap<String, Vec<String>> = HashMap::new();

        for (id, node) in nodes.iter() {
            if id == root_id {
                continue;
            }
            let parent = node
                .parent
                .clone()
                .unwrap_or_else(|| root_id.to_string());
            children_map.entry(parent).or_default().push(id.clone());
        }

        build_tree(root_id, &nodes, &children_map)
    }

    pub fn get_by_id(&self, id: &str) -> Option<AutomationNode> {
        let tree = self.snapshot();
        find_by_id(&tree, id)
    }

    pub fn get_by_role(&self, role: &str, name: Option<&str>) -> Vec<AutomationNode> {
        let tree = self.snapshot();
        let mut matches = Vec::new();
        collect_by_role(&tree, role, name, &mut matches);
        matches
    }

    pub fn get_by_text(&self, text: &str) -> Vec<AutomationNode> {
        let tree = self.snapshot();
        let mut matches = Vec::new();
        collect_by_text(&tree, text, &mut matches);
        matches
    }
}

static AUTOMATION_REGISTRY: OnceLock<AutomationRegistry> = OnceLock::new();

pub fn registry() -> &'static AutomationRegistry {
    AUTOMATION_REGISTRY.get_or_init(AutomationRegistry::new)
}

pub fn snapshot() -> AutomationNode {
    registry().snapshot()
}

pub fn get_by_text(text: &str) -> Vec<AutomationNode> {
    registry().get_by_text(text)
}

pub fn track_children_bounds(
    id: &'static str,
    role: &'static str,
    name: Option<&'static str>,
    parent: Option<&'static str>,
) -> impl Fn(Vec<Bounds<Pixels>>, &mut Window, &mut App) + 'static {
    let target = AutomationTarget::new(id, role, name, parent);
    let registry = registry();
    move |children_bounds, _window, _cx| {
        let bounds = union_children_bounds(&children_bounds);
        let visible = bounds.is_some();
        registry.update_target(target, bounds, visible, true);
    }
}

pub fn register_hidden(
    id: &'static str,
    role: &'static str,
    name: Option<&'static str>,
    parent: Option<&'static str>,
) {
    registry().update_target(AutomationTarget::new(id, role, name, parent), None, false, true);
}

pub fn track_children_bounds_dynamic(
    id: String,
    role: String,
    name: Option<String>,
    parent: Option<String>,
) -> impl Fn(Vec<Bounds<Pixels>>, &mut Window, &mut App) + 'static {
    let registry = registry();
    move |children_bounds, _window, _cx| {
        let bounds = union_children_bounds(&children_bounds);
        let visible = bounds.is_some();
        registry.update_target_dynamic(
            id.clone(),
            role.clone(),
            name.clone(),
            parent.clone(),
            bounds,
            visible,
            true,
        );
    }
}

pub fn clear_prefix(prefix: &str) {
    registry().clear_prefix(prefix);
}

fn union_children_bounds(children_bounds: &[Bounds<Pixels>]) -> Option<AutomationBounds> {
    let first = children_bounds.first()?;
    let mut min_x = f32::from(first.origin.x);
    let mut min_y = f32::from(first.origin.y);
    let mut max_x = min_x + f32::from(first.size.width);
    let mut max_y = min_y + f32::from(first.size.height);

    for bounds in &children_bounds[1..] {
        let x = f32::from(bounds.origin.x);
        let y = f32::from(bounds.origin.y);
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + width);
        max_y = max_y.max(y + height);
    }

    Some(AutomationBounds {
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    })
}

fn build_tree(
    id: &str,
    nodes: &HashMap<String, AutomationNodeState>,
    children_map: &HashMap<String, Vec<String>>,
) -> AutomationNode {
    let state = nodes
        .get(id)
        .cloned()
        .unwrap_or_else(|| AutomationNodeState::placeholder(id));
    let mut children = Vec::new();
    if let Some(child_ids) = children_map.get(id) {
        for child_id in child_ids {
            children.push(build_tree(child_id, nodes, children_map));
        }
    }
    children.sort_by(|a, b| a.id.cmp(&b.id));
    AutomationNode {
        id: state.id,
        role: state.role,
        name: state.name,
        bounds: state.bounds,
        visible: state.visible,
        enabled: state.enabled,
        children,
    }
}

fn find_by_id(node: &AutomationNode, id: &str) -> Option<AutomationNode> {
    if node.id == id {
        return Some(node.clone());
    }
    for child in &node.children {
        if let Some(found) = find_by_id(child, id) {
            return Some(found);
        }
    }
    None
}

fn collect_by_role(
    node: &AutomationNode,
    role: &str,
    name: Option<&str>,
    matches: &mut Vec<AutomationNode>,
) {
    let role_match = node.role == role;
    let name_match = name
        .map(|name| node.name.as_deref() == Some(name))
        .unwrap_or(true);
    if role_match && name_match {
        matches.push(node.clone());
    }
    for child in &node.children {
        collect_by_role(child, role, name, matches);
    }
}

fn collect_by_text(
    node: &AutomationNode,
    text: &str,
    matches: &mut Vec<AutomationNode>,
) {
    if node.name.as_deref() == Some(text) {
        matches.push(node.clone());
    }
    for child in &node.children {
        collect_by_text(child, text, matches);
    }
}

#[cfg(test)]
mod tests {
    use super::{AutomationBounds, AutomationRegistry, AutomationTarget};

    #[test]
    fn builds_tree_and_queries() {
        let registry = AutomationRegistry::new();
        registry.update_target(
            AutomationTarget::new("app-shell", "application", Some("ctx"), None),
            Some(AutomationBounds {
                x: 0.0,
                y: 0.0,
                width: 1024.0,
                height: 768.0,
            }),
            true,
            true,
        );
        registry.update_target(
            AutomationTarget::new(
                "composer-input",
                "textbox",
                Some("Composer"),
                Some("app-shell"),
            ),
            Some(AutomationBounds {
                x: 12.0,
                y: 700.0,
                width: 400.0,
                height: 40.0,
            }),
            true,
            true,
        );
        registry.update_target(
            AutomationTarget::new("sessions-list", "list", Some("Sessions"), Some("app-shell")),
            None,
            false,
            true,
        );

        let tree = registry.snapshot();
        assert_eq!(tree.id, "app-shell");

        let composer = registry.get_by_id("composer-input").expect("composer node");
        assert_eq!(composer.role, "textbox");

        let sessions = registry.get_by_role("list", Some("Sessions"));
        assert_eq!(sessions.len(), 1);
        assert!(!sessions[0].visible);
    }
}
