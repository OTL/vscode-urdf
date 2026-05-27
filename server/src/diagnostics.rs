use std::borrow::Cow;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Range};
use crate::document::Document;
use crate::workspace::WorkspaceIndex;

/// Type-kind tag for a Gazebo property element, used by `validate_gazebo_text`.
#[derive(Copy, Clone)]
pub enum GazeboPropKind {
    Float,
    NonNegFloat,
    PositiveFloat,
    Bool,
    Int,
    AnyString,
    Lenient,
}

/// Canonical Gazebo property schema — **single source of truth**.
///
/// Each entry is `(element_name, value_type)`. Everything else in the codebase
/// that talks about Gazebo property elements derives from this table:
///
/// **Consumers (read from this table):**
///   - `known_gazebo_prop()` in this file → name → `GazeboPropKind` lookup
///   - `features::completion()` in `server/src/features.rs` → completion labels
///     inside a `<gazebo>` block
///
/// **Consumer that cannot share data structurally (manual sync required):**
///   - `gazebo-prop-tag` regex alternation in `syntaxes/urdf.tmLanguage.json`
///     — JSON file, must be kept in sync by hand. There is no test asserting
///     equivalence (yet); if you add or rename a property here, update the
///     grammar regex too.
pub const GAZEBO_PROPS: &[(&str, GazeboPropKind)] = &[
    ("mu1",                  GazeboPropKind::NonNegFloat),
    ("mu2",                  GazeboPropKind::NonNegFloat),
    ("mu",                   GazeboPropKind::NonNegFloat),
    ("kp",                   GazeboPropKind::PositiveFloat),
    ("kd",                   GazeboPropKind::PositiveFloat),
    ("maxVel",               GazeboPropKind::NonNegFloat),
    ("minDepth",             GazeboPropKind::NonNegFloat),
    ("maxContacts",          GazeboPropKind::Int),
    ("selfCollide",          GazeboPropKind::Bool),
    ("turnGravityOff",       GazeboPropKind::Bool),
    ("gravity",              GazeboPropKind::Bool),
    ("implicitSpringDamper", GazeboPropKind::Bool),
    ("dampingFactor",        GazeboPropKind::Float),
    ("laserRetro",           GazeboPropKind::Float),
    ("material",             GazeboPropKind::AnyString),
    ("stopCfm",              GazeboPropKind::NonNegFloat),
    ("stopErp",              GazeboPropKind::NonNegFloat),
    ("fudgeFactor",          GazeboPropKind::Float),
    ("sensor",               GazeboPropKind::Lenient),
    ("plugin",               GazeboPropKind::Lenient),
];

/// One URDF element together with its attribute schema. Shared by both
/// [`GEOMETRY_PRIMITIVES`] and [`URDF_ELEMENTS`]; do not construct these elsewhere.
pub struct UrdfElement {
    pub name: &'static str,
    /// Attributes that **must** be present (drives "missing required attribute" diagnostics).
    pub required_attrs: &'static [&'static str],
    /// All attributes that are recognised on this element (drives "unknown attribute" diagnostics).
    /// Must be a superset of `required_attrs`.
    pub known_attrs: &'static [&'static str],
}

/// Canonical URDF geometry primitive schema — **single source of truth**.
///
/// **Consumers (read from this table):**
///   - `required_urdf_attrs()` in this file → looks up `required_attrs` per primitive
///   - `known_urdf_attrs()` in this file → looks up `known_attrs` per primitive
///
/// **Consumers that cannot share data structurally (manual sync required):**
///   - `geometry-tag` regex in `syntaxes/urdf.tmLanguage.json`
///   - `geometryFull` and `linkFull` snippet choice arrays in `snippets/snippets.json`
///
/// (see `server/src/diagnostics.rs` for source — this is it.)
///
/// Kept as a separate table from [`URDF_ELEMENTS`] because the grammar maps
/// geometry primitives to their own scope (`geometry-tag` → `support.type.urdf`)
/// — keeping the categories visibly distinct here mirrors that.
pub const GEOMETRY_PRIMITIVES: &[UrdfElement] = &[
    UrdfElement { name: "box",      required_attrs: &["size"],             known_attrs: &["size"] },
    UrdfElement { name: "cylinder", required_attrs: &["radius", "length"], known_attrs: &["radius", "length"] },
    UrdfElement { name: "sphere",   required_attrs: &["radius"],           known_attrs: &["radius"] },
    UrdfElement { name: "mesh",     required_attrs: &["filename"],         known_attrs: &["filename", "scale"] },
];

/// Canonical URDF element schema (everything except the geometry primitives,
/// which live in [`GEOMETRY_PRIMITIVES`]) — **single source of truth**.
///
/// **Consumers (read from this table):**
///   - `required_urdf_attrs()` in this file → looks up `required_attrs` per element
///   - `known_urdf_attrs()` in this file → looks up `known_attrs` per element;
///     a `None` lookup is what triggers the "Unknown URDF element" diagnostic
///
/// **Consumers that cannot share data structurally (manual sync required):**
///   - Element-name alternations in `syntaxes/urdf.tmLanguage.json` (`container-tag`,
///     `structure-tag`, `material-tag`, `inertial-tag`, etc.) — each grammar rule
///     covers a category subset, not the whole list, so there's no 1:1 mapping
///   - Snippet bodies in `snippets/snippets.json` that hard-code element names
///
/// (see `server/src/diagnostics.rs` for source — this is it.)
pub const URDF_ELEMENTS: &[UrdfElement] = &[
    UrdfElement { name: "robot",             required_attrs: &[],            known_attrs: &["name"] }, // name optional in xacro fragments
    UrdfElement { name: "link",              required_attrs: &["name"],      known_attrs: &["name"] },
    UrdfElement { name: "joint",             required_attrs: &["name", "type"], known_attrs: &["name", "type"] },
    UrdfElement { name: "visual",            required_attrs: &[],            known_attrs: &["name"] },
    UrdfElement { name: "collision",         required_attrs: &[],            known_attrs: &["name"] },
    UrdfElement { name: "inertial",          required_attrs: &[],            known_attrs: &[] },
    UrdfElement { name: "origin",            required_attrs: &[],            known_attrs: &["xyz", "rpy"] },
    UrdfElement { name: "geometry",          required_attrs: &[],            known_attrs: &[] },
    UrdfElement { name: "material",          required_attrs: &[],            known_attrs: &["name"] },
    UrdfElement { name: "color",             required_attrs: &["rgba"],      known_attrs: &["rgba"] },
    UrdfElement { name: "texture",           required_attrs: &[],            known_attrs: &["filename"] },
    UrdfElement { name: "mass",              required_attrs: &["value"],     known_attrs: &["value"] },
    UrdfElement { name: "inertia",           required_attrs: &[],            known_attrs: &["ixx", "ixy", "ixz", "iyy", "iyz", "izz"] },
    UrdfElement { name: "parent",            required_attrs: &["link"],      known_attrs: &["link"] },
    UrdfElement { name: "child",             required_attrs: &["link"],      known_attrs: &["link"] },
    UrdfElement { name: "axis",              required_attrs: &[],            known_attrs: &["xyz"] },
    UrdfElement { name: "limit",             required_attrs: &[],            known_attrs: &["lower", "upper", "effort", "velocity"] },
    UrdfElement { name: "dynamics",          required_attrs: &[],            known_attrs: &["damping", "friction"] },
    UrdfElement { name: "safety_controller", required_attrs: &[],            known_attrs: &["soft_lower_limit", "soft_upper_limit", "k_position", "k_velocity"] },
    UrdfElement { name: "calibration",       required_attrs: &[],            known_attrs: &["rising", "falling"] },
    UrdfElement { name: "mimic",             required_attrs: &["joint"],     known_attrs: &["joint", "multiplier", "offset"] },
    UrdfElement { name: "transmission",      required_attrs: &[],            known_attrs: &["name"] },
    UrdfElement { name: "gazebo",            required_attrs: &[],            known_attrs: &["reference"] },
];

/// Canonical URDF joint type values — **single source of truth**.
///
/// **Consumers (read from this table):**
///   - `validate_attr_value()` in this file → arm for `("joint", "type")`
///     emits an error if `type=` is not one of these strings.
///
/// **Consumers that cannot share data structurally (manual sync required):**
///   - `joint-type-value` regex in `syntaxes/urdf.tmLanguage.json`
///   - `joint` and `jointFull` snippet choice arrays in `snippets/snippets.json`
///
/// (see `server/src/diagnostics.rs` for source — this is it.)
pub const JOINT_TYPES: &[&str] = &[
    "revolute", "continuous", "prismatic", "fixed", "floating", "planar",
];

/// Per-joint-type human-readable docs surfaced in completion items —
/// `(short_detail, markdown_documentation)`. Order matches [`JOINT_TYPES`].
///
/// **Consumer:** `features::completion()` attaches `detail` + `documentation`
/// to each joint-type item so users see what they're picking. The text is
/// distilled from the URDF spec (https://wiki.ros.org/urdf/XML/joint).
pub const JOINT_TYPE_DOCS: &[(&str, &str, &str)] = &[
    (
        "revolute",
        "rotates around an axis, with limits",
        "**revolute** — A hinge that rotates around `<axis>` between `<limit lower>` and `<limit upper>`.\n\nRequires `<limit>` with `lower`, `upper`, `effort`, `velocity`.",
    ),
    (
        "continuous",
        "rotates around an axis, no limits",
        "**continuous** — A hinge that rotates around `<axis>` with no angular bounds (e.g. a wheel).\n\nNo `<limit>` required; `effort`/`velocity` may still be set.",
    ),
    (
        "prismatic",
        "slides along an axis, with limits",
        "**prismatic** — A linear (sliding) joint along `<axis>` between `<limit lower>` and `<limit upper>`.\n\nRequires `<limit>` with `lower`, `upper`, `effort`, `velocity`.",
    ),
    (
        "fixed",
        "rigid attachment, no motion",
        "**fixed** — Not really a joint: all 6 DoF are locked. Use to rigidly weld two links together.\n\nNo `<axis>` or `<limit>` needed.",
    ),
    (
        "floating",
        "free 6-DoF motion",
        "**floating** — Allows motion in all 6 degrees of freedom (3 translations + 3 rotations).\n\nRarely used; most physics engines treat this as a free body.",
    ),
    (
        "planar",
        "2-DoF motion in a plane",
        "**planar** — Allows motion in the plane perpendicular to `<axis>`. 2 translational DoF, no rotation about the axis.",
    ),
];

/// Run semantic checks on a parsed document.
///
/// `ws` is the cross-workspace entity index (populated from all open and scanned
/// files). A reference not found in this file but present in `ws` is silently
/// suppressed — the entity lives in an included/companion file. Pass `None` for
/// fully isolated (no-workspace) analysis.
pub fn check(
    doc: &Document,
    text: &str,
    ws: Option<&WorkspaceIndex>,
) -> Vec<Diagnostic> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    let link_names: std::collections::HashSet<&str> =
        doc.links.iter().map(|l| l.name.as_str()).collect();
    let joint_names: std::collections::HashSet<&str> =
        doc.joints.iter().map(|j| j.name.as_str()).collect();

    // In xacro files, links/joints may come from included files — use Warning
    // for refs not found anywhere in the workspace.
    let undef_severity = if doc.is_xacro {
        DiagnosticSeverity::WARNING
    } else {
        DiagnosticSeverity::ERROR
    };

    // 1 & 2. Undefined link references in joints (parent and child)
    for joint in &doc.joints {
        if let Some(parent_ref) = &joint.parent {
            if !link_names.contains(parent_ref.name.as_str())
                && !ws.is_some_and(|w| w.has_link(&parent_ref.name))
            {
                diags.push(make_diag(
                    parent_ref.range,
                    undef_severity,
                    format!("Undefined link '{}'", parent_ref.name),
                ));
            }
        }
        if let Some(child_ref) = &joint.child {
            if !link_names.contains(child_ref.name.as_str())
                && !ws.is_some_and(|w| w.has_link(&child_ref.name))
            {
                diags.push(make_diag(
                    child_ref.range,
                    undef_severity,
                    format!("Undefined link '{}'", child_ref.name),
                ));
            }
        }

        // 3. Undefined joint mimic reference
        if let Some(mimic_ref) = &joint.mimic {
            if !joint_names.contains(mimic_ref.name.as_str())
                && !ws.is_some_and(|w| w.has_joint(&mimic_ref.name))
            {
                diags.push(make_diag(
                    mimic_ref.range,
                    undef_severity,
                    format!("Undefined joint '{}' in mimic", mimic_ref.name),
                ));
            }
        }

        // Self-referential joint
        if let (Some(p), Some(c)) = (&joint.parent, &joint.child) {
            if p.name == c.name {
                diags.push(make_diag(
                    joint.range,
                    DiagnosticSeverity::ERROR,
                    format!("Joint '{}' has the same link as parent and child", joint.name),
                ));
            }
        }

        // Half-joints — URDF requires both <parent> and <child>. Without them
        // the kinematic-tree analysis drops the edge and the endpoints look
        // isolated. Flag the joint instead.
        if joint.parent.is_none() {
            diags.push(make_diag(
                joint.range,
                DiagnosticSeverity::ERROR,
                format!("Joint '{}' is missing required <parent> element", joint.name),
            ));
        }
        if joint.child.is_none() {
            diags.push(make_diag(
                joint.range,
                DiagnosticSeverity::ERROR,
                format!("Joint '{}' is missing required <child> element", joint.name),
            ));
        }
    }

    // 4. Duplicate link names
    {
        let mut seen = std::collections::HashSet::new();
        for item in &doc.links {
            if !seen.insert(item.name.as_str()) {
                diags.push(make_diag(
                    item.range,
                    DiagnosticSeverity::ERROR,
                    format!("Duplicate link name '{}'", item.name),
                ));
            }
        }
    }

    // 5. Duplicate joint names
    {
        let mut seen = std::collections::HashSet::new();
        for item in &doc.joints {
            if !seen.insert(item.name.as_str()) {
                diags.push(make_diag(
                    item.range,
                    DiagnosticSeverity::ERROR,
                    format!("Duplicate joint name '{}'", item.name),
                ));
            }
        }
    }

    // 6. Kinematic tree: multiple roots, disconnected links, cycles.
    // `ws` lets us suppress "isolated"/"disconnected" diagnostics for links
    // wired up by a joint living in another file. Runs whenever there are
    // joints (cycles are possible with one link and one self-loop joint).
    if !doc.links.is_empty() && !doc.joints.is_empty() {
        kinematic_tree_check(&doc, &mut diags, undef_severity, ws);
    }

    // 7. Xacro macro calls must resolve to a `<xacro:macro name="X">`
    // somewhere in the workspace.
    {
        let macro_names: std::collections::HashSet<&str> =
            doc.xacro_macros.iter().map(|m| m.name.as_str()).collect();
        for mref in &doc.xacro_macro_calls {
            if !macro_names.contains(mref.name.as_str())
                && !ws.is_some_and(|w| w.has_macro(&mref.name))
            {
                diags.push(make_diag(
                    mref.range,
                    undef_severity,
                    format!("Undefined xacro macro '{}'", mref.name),
                ));
            }
        }
    }

    // 8. Gazebo reference must point to a known link or joint
    for gref in &doc.gazebo_refs {
        let in_file = link_names.contains(gref.name.as_str()) || joint_names.contains(gref.name.as_str());
        let in_ws   = ws.is_some_and(|w| w.has_link(&gref.name) || w.has_joint(&gref.name));
        if !in_file && !in_ws {
            diags.push(make_diag(
                gref.range,
                undef_severity,
                format!("Undefined link or joint '{}' in gazebo reference", gref.name),
            ));
        }
    }

    // 7. Undefined xacro property references: scan text for ${varname}
    {
        let prop_names: std::collections::HashSet<&str> =
            doc.xacro_properties.iter().map(|p| p.name.as_str()).collect();

        let bytes = text.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == b'$' && bytes[i + 1] == b'{' {
                let start = i;
                let inner_start = i + 2;
                // Scan for closing '}', aborting at attribute/element boundaries so
                // an unclosed ${... doesn't silently swallow the rest of the file.
                let mut j = inner_start;
                let mut close: Option<usize> = None;
                while j < bytes.len() {
                    match bytes[j] {
                        b'}' => { close = Some(j); break; }
                        b'"' | b'\'' | b'<' | b'\n' => break,
                        _ => j += 1,
                    }
                }
                if let Some(inner_end) = close {
                    let varname = &text[inner_start..inner_end];
                    let is_complex = varname.contains(|c: char| {
                        matches!(c, ' ' | '+' | '-' | '*' | '/' | '(' | ')' | '.')
                    });
                    if !varname.is_empty() && !is_complex {
                        // Simple single-identifier: ${varname}
                        if !prop_names.contains(varname)
                            && !ws.is_some_and(|w| w.has_prop(varname))
                        {
                            let range = byte_range_to_lsp(text, start..inner_end + 1);
                            diags.push(make_diag(
                                range,
                                undef_severity,
                                format!("Undefined xacro property '{}'", varname),
                            ));
                        }
                    } else if is_complex {
                        // Complex expression: extract every identifier and check each one.
                        let expr = varname.as_bytes();
                        let mut k = 0;
                        while k < expr.len() {
                            if expr[k].is_ascii_alphabetic() || expr[k] == b'_' {
                                let id_start = k;
                                while k < expr.len()
                                    && (expr[k].is_ascii_alphanumeric() || expr[k] == b'_')
                                {
                                    k += 1;
                                }
                                let id = &varname[id_start..k];
                                if matches!(
                                    id,
                                    "pi" | "sin" | "cos" | "tan" | "abs"
                                        | "sqrt" | "radians" | "degrees"
                                ) {
                                    continue;
                                }
                                if !prop_names.contains(id)
                                    && !ws.is_some_and(|w| w.has_prop(id))
                                {
                                    let byte_start = inner_start + id_start;
                                    let byte_end = inner_start + k;
                                    let range = byte_range_to_lsp(text, byte_start..byte_end);
                                    diags.push(make_diag(
                                        range,
                                        undef_severity,
                                        format!("Undefined xacro property '{}'", id),
                                    ));
                                }
                            } else {
                                k += 1;
                            }
                        }
                    }
                    i = inner_end + 1;
                } else {
                    let range = byte_range_to_lsp(text, start..j);
                    diags.push(make_diag(
                        range,
                        DiagnosticSeverity::ERROR,
                        "Unclosed xacro expression: missing '}'".to_string(),
                    ));
                    i = j;
                }
                continue;
            }
            i += 1;
        }
    }

    diags
}

fn kinematic_tree_check(
    doc: &Document,
    diags: &mut Vec<Diagnostic>,
    severity: DiagnosticSeverity,
    ws: Option<&WorkspaceIndex>,
) {
    // Build parent→children adjacency and the set of all child link names.
    let mut adj: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    let mut child_set: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for joint in &doc.joints {
        if let (Some(p), Some(c)) = (&joint.parent, &joint.child) {
            adj.entry(p.name.as_str()).or_default().push(c.name.as_str());
            child_set.insert(c.name.as_str());
        }
    }

    // Isolated links: no parent joint AND no child joints in *this file*.
    // Suppress when the workspace shows the link is wired up elsewhere.
    for link in doc.links.iter()
        .filter(|l| !child_set.contains(l.name.as_str()) && !adj.contains_key(l.name.as_str()))
        .filter(|l| !ws.is_some_and(|w| w.link_touched_by_joint(&l.name)))
    {
        diags.push(make_diag(
            link.range,
            severity,
            format!("Link '{}' has no joints — not connected to the kinematic tree", link.name),
        ));
    }

    // Root links: no parent joint, but DO have children (i.e., appear as joint parents).
    let roots: Vec<&str> = doc.links.iter()
        .filter(|l| !child_set.contains(l.name.as_str()) && adj.contains_key(l.name.as_str()))
        .map(|l| l.name.as_str())
        .collect();

    // Multiple roots — flag every root so the user can pick which one to delete/fix
    // rather than chasing a document-order-dependent diagnostic.
    if roots.len() > 1 {
        for link in doc.links.iter()
            .filter(|l| !child_set.contains(l.name.as_str()) && adj.contains_key(l.name.as_str()))
        {
            diags.push(make_diag(
                link.range,
                severity,
                format!(
                    "Link '{}' is a root (no parent joint); kinematic tree must have exactly one root",
                    link.name
                ),
            ));
        }
    }

    // DFS in two passes so pure cycles (no incoming-edge-free node) still get
    // detected. Pass 1 seeds from roots — finds cycles reachable from a root.
    // Pass 2 seeds from any link the first pass didn't visit — that's only
    // possible if it sits inside a pure cycle, since every non-cycle node is
    // reachable from at least one root.
    let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut on_path: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut stack: Vec<(&str, usize)> = Vec::new();
    let mut reported_cycle_joints: std::collections::HashSet<usize> = std::collections::HashSet::new();

    let pass1_seeds: Vec<&str> = doc.links.iter()
        .filter(|l| !child_set.contains(l.name.as_str()))
        .map(|l| l.name.as_str())
        .collect();
    let pass2_seeds: Vec<&str> = doc.links.iter()
        .map(|l| l.name.as_str())
        .collect();

    for seeds in [pass1_seeds, pass2_seeds] {
        for &seed in &seeds {
            if visited.contains(seed) {
                continue;
            }
            stack.push((seed, 0));
            on_path.insert(seed);

            while !stack.is_empty() {
                let last = stack.len() - 1;
                let (node, idx) = stack[last];
                let kids: &[&str] = adj.get(node).map(|v| v.as_slice()).unwrap_or(&[]);

                if idx < kids.len() {
                    stack[last].1 += 1;
                    let kid = kids[idx];

                    if on_path.contains(kid) {
                        // Back edge — report the joint that closes the cycle, once.
                        if let Some((j_idx, joint)) = doc.joints.iter().enumerate().find(|(_, j)| {
                            j.parent.as_ref().map_or(false, |p| p.name == node)
                                && j.child.as_ref().map_or(false, |c| c.name == kid)
                        }) {
                            if reported_cycle_joints.insert(j_idx) {
                                diags.push(make_diag(
                                    joint.range,
                                    DiagnosticSeverity::ERROR,
                                    format!("Joint '{}' creates a cycle in the kinematic tree", joint.name),
                                ));
                            }
                        }
                    } else if !visited.contains(kid) {
                        on_path.insert(kid);
                        stack.push((kid, 0));
                    }
                } else {
                    visited.insert(node);
                    on_path.remove(node);
                    stack.pop();
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Schema validation: unknown element names and unknown attributes
// ---------------------------------------------------------------------------

pub fn check_schema(
    xml: &roxmltree::Document,
    text: &str,
    is_xacro: bool,
    ws: Option<&WorkspaceIndex>,
) -> Vec<Diagnostic> {
    let mut props: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for node in xml.root_element().children() {
        if node.is_element() {
            let tag = node.tag_name();
            if tag.name() == "property" && tag.namespace().map_or(false, |n| n.contains("xacro")) {
                if let (Some(name), Some(value)) = (node.attribute("name"), node.attribute("value")) {
                    props.insert(name, value);
                }
            }
        }
    }

    let soft_severity = if is_xacro {
        DiagnosticSeverity::WARNING
    } else {
        DiagnosticSeverity::ERROR
    };

    let mut diags = vec![];
    let mut defined_materials: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut material_refs: Vec<(roxmltree::Node, String)> = Vec::new();
    collect_materials(xml.root_element(), &mut defined_materials, &mut material_refs);
    for (node, name) in material_refs {
        if !defined_materials.contains(name.as_str())
            && !ws.is_some_and(|w| w.has_material(&name))
        {
            let range = attr_value_range_for(text, &node, "name");
            diags.push(make_diag(
                range,
                soft_severity,
                format!("Undefined material '{name}'"),
            ));
        }
    }

    walk_schema(xml.root_element(), text, &mut diags, false, &props, soft_severity);
    diags
}

/// Walk the tree gathering material definitions (any `<material name="X">`
/// containing a `<color>` or `<texture>` child) and reference-only uses
/// (`<material name="X"/>` with no such child). Mirrors urdf_parser's two-pass
/// resolution: a reference inside `<visual>` resolves against any inline or
/// top-level definition anywhere in the document, regardless of order.
fn collect_materials<'a>(
    node: roxmltree::Node<'a, 'a>,
    defined: &mut std::collections::HashSet<&'a str>,
    refs: &mut Vec<(roxmltree::Node<'a, 'a>, String)>,
) {
    if !node.is_element() {
        return;
    }
    if node.tag_name().name() == "material" && !is_xacro_element(&node) {
        if let Some(name) = node.attribute("name") {
            let has_def_child = node.children().any(|c| {
                c.is_element()
                    && matches!(c.tag_name().name(), "color" | "texture")
                    && !is_xacro_element(&c)
            });
            if has_def_child {
                defined.insert(name);
            } else {
                refs.push((node, name.to_string()));
            }
        }
    }
    for child in node.children() {
        collect_materials(child, defined, refs);
    }
}

fn attr_value_range_for(text: &str, node: &roxmltree::Node, attr_name: &str) -> Range {
    crate::document::attr_value_range(text, node, attr_name)
}

fn walk_schema(
    node: roxmltree::Node,
    text: &str,
    diags: &mut Vec<Diagnostic>,
    skip: bool,
    props: &std::collections::HashMap<&str, &str>,
    soft_severity: DiagnosticSeverity,
) {
    if node.is_text() {
        if !skip {
            let content = node.text().unwrap_or("").trim();
            if !content.is_empty() {
                let range = byte_range_to_lsp(text, node.range());
                diags.push(make_diag(
                    range,
                    DiagnosticSeverity::WARNING,
                    "Unexpected text content in URDF element".to_string(),
                ));
            }
        }
        return;
    }
    if !node.is_element() {
        return;
    }

    let tag = node.tag_name().name();
    let is_xacro = is_xacro_element(&node);
    let child_skip = skip || is_xacro || matches!(tag, "plugin" | "sensor" | "transmission");

    if !skip && !is_xacro {
        match known_urdf_attrs(tag) {
            Some(valid_attrs) => {
                for attr in node.attributes() {
                    let aname = attr.name();
                    if aname == "xmlns" || aname.starts_with("xmlns:") {
                        continue;
                    }
                    if !valid_attrs.contains(&aname) {
                        let range = attr_name_range(text, &node, aname);
                        diags.push(make_diag(
                            range,
                            DiagnosticSeverity::WARNING,
                            format!("Unknown attribute '{aname}' on element <{tag}>"),
                        ));
                    }
                }

                for req in required_urdf_attrs(tag) {
                    if node.attribute(*req).is_none() {
                        let range = elem_name_range(text, &node);
                        diags.push(make_diag(
                            range,
                            DiagnosticSeverity::ERROR,
                            format!("Element <{tag}> is missing required attribute '{req}'"),
                        ));
                    }
                }

                for attr in node.attributes() {
                    let aname = attr.name();
                    if aname == "xmlns" || aname.starts_with("xmlns:") {
                        continue;
                    }
                    if let Some(msg) = validate_attr_value(tag, aname, attr.value(), props) {
                        let range = attr_name_range(text, &node, aname);
                        diags.push(make_diag(range, DiagnosticSeverity::ERROR, msg));
                    }
                }

                // Revolute and prismatic joints require a <limit> child;
                // urdf_parser refuses to load the model otherwise.
                if tag == "joint" {
                    if let Some(t) = node.attribute("type") {
                        if let Some(effective) = resolve_effective(t, props) {
                            let kind = effective.trim();
                            if matches!(kind, "revolute" | "prismatic") {
                                let has_limit = node.children().any(|c| {
                                    c.is_element()
                                        && c.tag_name().name() == "limit"
                                        && !is_xacro_element(&c)
                                });
                                if !has_limit {
                                    let range = elem_name_range(text, &node);
                                    diags.push(make_diag(
                                        range,
                                        soft_severity,
                                        format!(
                                            "Joint of type '{kind}' is missing required <limit> element"
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            None => {
                let range = elem_name_range(text, &node);
                diags.push(make_diag(
                    range,
                    DiagnosticSeverity::WARNING,
                    format!("Unknown URDF element <{tag}>"),
                ));
            }
        }
    }

    if tag == "gazebo" && !skip && !is_xacro {
        for child in node.children() {
            walk_gazebo_child(child, text, diags, props);
        }
    } else {
        for child in node.children() {
            walk_schema(child, text, diags, child_skip, props, soft_severity);
        }
    }
}

/// Look up the value-type kind for a Gazebo property element name.
/// Derived from [`GAZEBO_PROPS`] — that table is the single source of truth.
fn known_gazebo_prop(tag: &str) -> Option<GazeboPropKind> {
    GAZEBO_PROPS.iter().find(|(name, _)| *name == tag).map(|(_, kind)| *kind)
}

fn walk_gazebo_child(
    node: roxmltree::Node,
    text: &str,
    diags: &mut Vec<Diagnostic>,
    props: &std::collections::HashMap<&str, &str>,
) {
    if !node.is_element() || is_xacro_element(&node) {
        return;
    }
    let tag = node.tag_name().name();
    match known_gazebo_prop(tag) {
        None => {
            let range = elem_name_range(text, &node);
            diags.push(make_diag(
                range,
                DiagnosticSeverity::WARNING,
                format!("Unknown Gazebo property <{tag}>"),
            ));
        }
        Some(GazeboPropKind::Lenient) => {}
        Some(kind) => {
            let mut text_content = String::new();
            let mut text_range: Option<std::ops::Range<usize>> = None;
            for child in node.children() {
                if child.is_text() {
                    let t = child.text().unwrap_or("");
                    if !t.trim().is_empty() {
                        text_content.push_str(t);
                        text_range = Some(child.range());
                    }
                }
            }
            let content = text_content.trim();
            if !content.is_empty() {
                if let Some(tr) = text_range {
                    if let Some(msg) = validate_gazebo_text(tag, content, kind, props) {
                        diags.push(make_diag(byte_range_to_lsp(text, tr), DiagnosticSeverity::ERROR, msg));
                    }
                }
            }
        }
    }
}

fn validate_gazebo_text(
    tag: &str,
    content: &str,
    kind: GazeboPropKind,
    props: &std::collections::HashMap<&str, &str>,
) -> Option<String> {
    let effective = resolve_effective(content, props)?;
    match kind {
        GazeboPropKind::Float         => expect_float(&effective, tag),
        GazeboPropKind::NonNegFloat   => expect_non_neg_float(&effective, tag),
        GazeboPropKind::PositiveFloat => expect_positive_float(&effective, tag),
        GazeboPropKind::Bool          => expect_bool(&effective, tag),
        GazeboPropKind::Int           => {
            if effective.trim().parse::<i64>().is_err() {
                Some(format!("'{tag}' must be an integer, got '{}'", effective.trim()))
            } else { None }
        }
        GazeboPropKind::AnyString | GazeboPropKind::Lenient => None,
    }
}

/// Look up the required attributes for a URDF element. Derived from
/// [`GEOMETRY_PRIMITIVES`] and [`URDF_ELEMENTS`] — those tables are the
/// single source of truth. Unknown elements return `&[]`.
fn required_urdf_attrs(element: &str) -> &'static [&'static str] {
    if let Some(e) = GEOMETRY_PRIMITIVES.iter().find(|e| e.name == element) {
        return e.required_attrs;
    }
    URDF_ELEMENTS.iter()
        .find(|e| e.name == element)
        .map(|e| e.required_attrs)
        .unwrap_or(&[])
}

fn validate_attr_value(
    element: &str,
    attr: &str,
    value: &str,
    props: &std::collections::HashMap<&str, &str>,
) -> Option<String> {
    let effective = resolve_effective(value, props)?;
    match (element, attr) {
        // Joint type must be one of the canonical values — see JOINT_TYPES above for source.
        ("joint", "type") => {
            let t = effective.trim();
            if JOINT_TYPES.iter().any(|v| *v == t) {
                None
            } else {
                Some(format!(
                    "'type' must be one of {}, got '{t}'",
                    JOINT_TYPES.join(", "),
                ))
            }
        }
        (_, "xyz") | (_, "rpy") => expect_n_floats(&effective, 3, attr),
        ("box", "size") => expect_n_floats(&effective, 3, attr)
            .or_else(|| {
                let ok = effective.split_whitespace()
                    .filter_map(|s| s.parse::<f64>().ok())
                    .all(|f| f > 0.0);
                if !ok { Some("'size' values must be positive".to_string()) } else { None }
            }),
        ("color", "rgba") => expect_n_floats(&effective, 4, attr)
            .or_else(|| {
                let vals: Vec<f64> = effective.split_whitespace()
                    .filter_map(|s| s.parse().ok())
                    .collect();
                if vals.len() == 4 && vals.iter().any(|&f| f < 0.0 || f > 1.0) {
                    Some("'rgba' values must be between 0 and 1".into())
                } else { None }
            }),
        (_, "radius") | ("cylinder", "length") => expect_positive_float(&effective, attr),
        (_, "lower") | (_, "upper") | (_, "effort") | (_, "velocity")
        | (_, "damping") | (_, "friction") | (_, "value")
        | (_, "ixx") | (_, "ixy") | (_, "ixz") | (_, "iyy") | (_, "iyz") | (_, "izz")
        | (_, "multiplier") | (_, "offset") => expect_float(&effective, attr),
        _ => None,
    }
}

/// Resolves xacro `${varname}` to the property value for simple single-identifier
/// substitutions. Returns `None` to signal "skip validation" when a `${` pattern
/// is present but unresolvable. Non-xacro values pass through as `Borrowed`.
fn resolve_effective<'a>(
    value: &'a str,
    props: &std::collections::HashMap<&str, &str>,
) -> Option<Cow<'a, str>> {
    if value.contains("${") {
        resolve_simple_xacro(value, props).map(Cow::Owned)
    } else {
        Some(Cow::Borrowed(value))
    }
}

fn resolve_simple_xacro(
    value: &str,
    props: &std::collections::HashMap<&str, &str>,
) -> Option<String> {
    let v = value.trim();
    if !v.starts_with("${") || !v.ends_with('}') {
        return None;
    }
    let inner = &v[2..v.len() - 1];
    if inner.contains(|c: char| matches!(c, '+' | '-' | '*' | '/' | '(' | ')' | ' ' | '\t')) {
        return None;
    }
    let resolved = props.get(inner)?;
    if resolved.contains("${") {
        return None;
    }
    Some(resolved.to_string())
}

fn is_xacro_element(node: &roxmltree::Node) -> bool {
    node.tag_name().namespace().map_or(false, |n| n.contains("xacro"))
        || node.tag_name().name().starts_with("xacro:")
}

fn expect_n_floats(value: &str, n: usize, attr: &str) -> Option<String> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() != n {
        return Some(format!("'{attr}' expects {n} numbers, got {}", parts.len()));
    }
    let bad: Vec<&str> = parts.iter().copied()
        .filter(|s| s.parse::<f64>().is_err())
        .collect();
    if !bad.is_empty() {
        return Some(format!("'{attr}' contains non-numeric value: '{}'", bad[0]));
    }
    None
}

fn expect_float(value: &str, attr: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.parse::<f64>().is_err() {
        Some(format!("'{attr}' must be a number, got '{trimmed}'"))
    } else { None }
}

fn expect_positive_float(value: &str, attr: &str) -> Option<String> {
    let trimmed = value.trim();
    match trimmed.parse::<f64>() {
        Ok(f) if f > 0.0 => None,
        Ok(_) => Some(format!("'{attr}' must be positive")),
        Err(_) => Some(format!("'{attr}' must be a number, got '{trimmed}'")),
    }
}

fn expect_non_neg_float(value: &str, attr: &str) -> Option<String> {
    let trimmed = value.trim();
    match trimmed.parse::<f64>() {
        Ok(f) if f >= 0.0 => None,
        Ok(_) => Some(format!("'{attr}' must be non-negative")),
        Err(_) => Some(format!("'{attr}' must be a number, got '{trimmed}'")),
    }
}

fn expect_bool(value: &str, attr: &str) -> Option<String> {
    match value.trim() {
        "true" | "false" | "1" | "0" => None,
        v => Some(format!("'{attr}' must be 'true' or 'false', got '{v}'")),
    }
}

/// Look up the recognised attributes for a URDF element. Derived from
/// [`GEOMETRY_PRIMITIVES`] and [`URDF_ELEMENTS`] — those tables are the
/// single source of truth. `None` indicates the element itself isn't a
/// known URDF element (which triggers the "Unknown URDF element" diagnostic).
fn known_urdf_attrs(element: &str) -> Option<&'static [&'static str]> {
    if let Some(e) = GEOMETRY_PRIMITIVES.iter().find(|e| e.name == element) {
        return Some(e.known_attrs);
    }
    URDF_ELEMENTS.iter()
        .find(|e| e.name == element)
        .map(|e| e.known_attrs)
}

fn elem_name_range(text: &str, node: &roxmltree::Node) -> Range {
    let start = node.range().start + 1; // skip '<'
    let name = node.tag_name().name();
    byte_range_to_lsp(text, start..start + name.len())
}

fn attr_name_range(text: &str, node: &roxmltree::Node, attr_name: &str) -> Range {
    let elem_range = node.range();
    let elem_src = &text[elem_range.clone()];
    let mut search = 0;
    while search < elem_src.len() {
        let Some(rel) = elem_src[search..].find(attr_name) else {
            break;
        };
        let abs = search + rel;
        let prev_ok = abs == 0 || elem_src.as_bytes()[abs - 1].is_ascii_whitespace();
        let after = abs + attr_name.len();
        let next_ok = after < elem_src.len()
            && matches!(elem_src.as_bytes()[after], b'=' | b' ' | b'\t' | b'\n' | b'\r');
        if prev_ok && next_ok {
            let start = elem_range.start + abs;
            return byte_range_to_lsp(text, start..start + attr_name.len());
        }
        search = abs + 1;
    }
    elem_name_range(text, node)
}

fn make_diag(range: Range, severity: DiagnosticSeverity, message: String) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(severity),
        source: Some(crate::document::DIAGNOSTIC_SOURCE.to_string()),
        message,
        ..Diagnostic::default()
    }
}

fn byte_range_to_lsp(text: &str, range: std::ops::Range<usize>) -> Range {
    crate::document::byte_range_to_lsp(text, range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document;

    fn diag_messages(text: &str) -> Vec<String> {
        let (doc, mut d) = document::parse(text);
        d.extend(check(&doc, text, None));
        d.iter().map(|d| d.message.clone()).collect()
    }

    #[test]
    fn tree_isolated_link() {
        // A link with no joints at all should be reported as "no joints"
        let msgs = diag_messages(r#"<?xml version="1.0"?><robot name="r">
          <link name="base_link"/>
          <link name="child"/>
          <link name="orphan"/>
          <joint name="j1" type="fixed">
            <parent link="base_link"/><child link="child"/>
          </joint>
        </robot>"#);
        assert!(msgs.iter().any(|m| m.contains("orphan") && m.contains("no joints")),
            "expected isolated-link diagnostic, got: {:?}", msgs);
    }

    #[test]
    fn tree_multiple_roots() {
        let msgs = diag_messages(r#"<?xml version="1.0"?><robot name="r">
          <link name="root1"/>
          <link name="root2"/>
          <link name="child"/>
          <joint name="j1" type="fixed">
            <parent link="root1"/><child link="child"/>
          </joint>
        </robot>"#);
        assert!(msgs.iter().any(|m| m.contains("root2") && m.contains("root")),
            "expected multiple-roots diagnostic, got: {:?}", msgs);
    }

    #[test]
    fn tree_cycle() {
        let msgs = diag_messages(r#"<?xml version="1.0"?><robot name="r">
          <link name="base_link"/>
          <link name="link_a"/>
          <link name="link_b"/>
          <joint name="j1" type="fixed"><parent link="base_link"/><child link="link_a"/></joint>
          <joint name="j2" type="fixed"><parent link="link_a"/><child link="link_b"/></joint>
          <joint name="j3" type="fixed"><parent link="link_b"/><child link="link_a"/></joint>
        </robot>"#);
        assert!(msgs.iter().any(|m| m.contains("cycle")),
            "expected cycle diagnostic, got: {:?}", msgs);
    }

    #[test]
    fn tag_mismatch_points_to_close_tag() {
        // Open tag has a typo (<mateaaaaaaa>), close tag is </material>.
        // Diagnostic lands on the close tag (line 3, 0-indexed) naming both sides,
        // so the user can see what was open and what was written.
        let text = "<robot name=\"r\">\n  <mateaaaaaaa name=\"x\">\n    <color rgba=\"1 1 1 1\"/>\n  </material>\n</robot>\n";
        let (_, diags) = document::parse(text);
        assert_eq!(diags.len(), 1, "expected exactly one diagnostic, got {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>());
        assert!(diags[0].message.contains("mateaaaaaaa") && diags[0].message.contains("material"),
            "expected mismatch message naming both tags, got: {}", diags[0].message);
        assert_eq!(diags[0].range.start.line, 3, "expected diagnostic on the closing tag line, got: {:?}",
            diags[0].range);
    }

    #[test]
    fn tag_unclosed_mismatch_is_flagged() {
        let text = "<robot>\n  <link name=\"foo\">\n</robot>\n";
        let (_, diags) = document::parse(text);
        assert!(diags.iter().any(|d| d.message.contains("never closed") || d.message.contains("Unexpected closing tag")),
            "expected unclosed/mismatch diagnostic, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    #[test]
    fn xml_parse_error_position_extracted_from_tail() {
        // roxmltree 0.20 emits messages like "expected '=' not 'e' at 26:19".
        // Position must be parsed from the tail, not from the start of the message.
        let text = "<robot name=\"r\">\n  <link name=\"a\"/>\n  <material nam e=\"x\"/>\n</robot>\n";
        let (_, diags) = document::parse(text);
        assert_eq!(diags.len(), 1, "expected one parse-error diagnostic");
        let d = &diags[0];
        assert!(d.message.contains("XML parse error"),
            "expected XML parse error, got: {}", d.message);
        assert_eq!(d.range.start.line, 2,
            "expected diagnostic on line 2 (the <material> line), got: {:?}", d.range);
    }

    #[test]
    fn document_colors_finds_rgba_attributes() {
        let text = r#"<robot name="r">
  <material name="orange"><color rgba="1 0.3 0.1 1"/></material>
  <material name="blue"><color rgba="0.2 0.2 1 0.5"/></material>
</robot>"#;
        let colors = crate::features::document_colors(text);
        assert_eq!(colors.len(), 2, "expected 2 rgba swatches");
        assert!((colors[0].color.red - 1.0).abs() < 1e-6);
        assert!((colors[1].color.alpha - 0.5).abs() < 1e-6);
    }

    #[test]
    fn folding_ranges_cover_multiline_elements() {
        let text = "<robot name=\"r\">\n  <link name=\"a\">\n    <visual>\n      <geometry>\n        <box size=\"1 1 1\"/>\n      </geometry>\n    </visual>\n  </link>\n  <joint name=\"j\" type=\"fixed\"/>\n</robot>\n";
        let ranges = crate::features::folding_ranges(text);
        // Expected foldable elements (multi-line): robot (0..9), link (1..7), visual (2..6), geometry (3..5)
        let starts: Vec<u32> = ranges.iter().map(|r| r.start_line).collect();
        assert!(starts.contains(&0), "robot should fold from line 0, got: {:?}", ranges);
        assert!(starts.contains(&1), "link should fold from line 1, got: {:?}", ranges);
        assert!(starts.contains(&2), "visual should fold from line 2, got: {:?}", ranges);
        assert!(starts.contains(&3), "geometry should fold from line 3, got: {:?}", ranges);
        // The single-line self-closing <joint> at line 8 should NOT produce a fold
        assert!(!starts.contains(&8), "self-closing joint should not fold");
    }

    #[test]
    fn xml_parse_failure_does_not_cascade_undefined_props() {
        // Malformed XML elsewhere in the file must not turn every ${...}
        // reference into a false "Undefined xacro property" — properties
        // are defined at the top but parsing fails on the bad line, so
        // doc.xacro_properties is empty. We rely on parse_ok to skip check().
        let text = r#"<robot xmlns:xacro="http://www.ros.org/wiki/xacro" name="r">
  <xacro:property name="wheel_radius" value="0.033"/>
  <link name="x"><visual><geometry>
    <cylinder radius=aa"ssdada${wheel_radius}s" length="${wheel_thickness}"/>
  </geometry></visual></link>
</robot>"#;
        let (doc, mut diags) = document::parse(text);
        if doc.parse_ok {
            diags.extend(check(&doc, text, None));
        }
        let undef: Vec<_> = diags.iter().filter(|d| d.message.contains("Undefined xacro property")).collect();
        assert!(undef.is_empty(),
            "no Undefined-xacro-property diagnostics should fire when XML parse fails, got: {:?}",
            undef.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    #[test]
    fn unclosed_xacro_expression_is_flagged() {
        let text = r#"<robot xmlns:xacro="http://www.ros.org/wiki/xacro" name="r">
  <xacro:property name="wheel_thickness" value="0.026"/>
  <link name="x"><visual><geometry>
    <cylinder radius="${asaso" length="${wheel_thickness}"/>
  </geometry></visual></link>
</robot>"#;
        let (doc, mut d) = document::parse(text);
        d.extend(check(&doc, text, None));
        assert!(d.iter().any(|m| m.message.contains("Unclosed xacro expression")),
            "expected unclosed-expression diagnostic, got: {:?}",
            d.iter().map(|x| &x.message).collect::<Vec<_>>());
        // wheel_thickness should NOT be flagged as undefined
        assert!(!d.iter().any(|m| m.message.contains("wheel_thickness") && m.message.contains("Undefined")),
            "wheel_thickness should not be flagged");
    }

    #[test]
    fn completion_inside_dollar_brace() {
        use tower_lsp::lsp_types::Position;
        let text = "<robot xmlns:xacro=\"http://www.ros.org/wiki/xacro\" name=\"r\">\n  <xacro:property name=\"wheel_radius\" value=\"0.033\"/>\n  <xacro:property name=\"wheel_thickness\" value=\"0.026\"/>\n  <link name=\"x\"><visual><geometry>\n    <cylinder radius=\"${w}\" length=\"0\"/>\n  </geometry></visual></link>\n</robot>";
        let (doc, _) = document::parse(text);
        // Line 4 (0-indexed): "    <cylinder radius=\"${w}\" length=\"0\"/>"
        // Cursor right after the 'w' (before the closing '}')
        let line = "    <cylinder radius=\"${w}\" length=\"0\"/>";
        let col_after_w = line.find("${w").unwrap() + 3; // position right after 'w'
        let pos = Position::new(4, col_after_w as u32);
        let items = crate::features::completion(&doc, pos, text);
        let labels: Vec<&str> = items.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"wheel_radius"), "expected wheel_radius in completions, got: {:?}", labels);
        assert!(labels.contains(&"wheel_thickness"), "expected wheel_thickness in completions, got: {:?}", labels);
    }

    #[test]
    fn undefined_var_inside_complex_expression_is_flagged() {
        let text = r#"<robot xmlns:xacro="http://www.ros.org/wiki/xacro" name="r">
  <xacro:property name="chassis_length" value="0.35"/>
  <xacro:property name="chassis_mass"   value="1"/>
  <link name="x">
    <inertial>
      <inertia ixx="${(1/12)*chassis_mass*(chassis_width*chassis_length)}" ixy="0" ixz="0"
               iyy="0" iyz="0" izz="0"/>
    </inertial>
  </link>
</robot>"#;
        let (doc, mut d) = document::parse(text);
        d.extend(check(&doc, text, None));
        assert!(
            d.iter().any(|m| m.message.contains("chassis_width") && m.message.contains("Undefined")),
            "expected chassis_width to be flagged as undefined, got: {:?}",
            d.iter().map(|x| &x.message).collect::<Vec<_>>()
        );
        assert!(
            !d.iter().any(|m| m.message.contains("chassis_length") && m.message.contains("Undefined")),
            "chassis_length is defined and must not be flagged"
        );
        assert!(
            !d.iter().any(|m| m.message.contains("chassis_mass") && m.message.contains("Undefined")),
            "chassis_mass is defined and must not be flagged"
        );
    }

    #[test]
    fn builtins_in_complex_expression_not_flagged() {
        let text = r#"<robot xmlns:xacro="http://www.ros.org/wiki/xacro" name="r">
  <xacro:property name="angle" value="1.57"/>
  <link name="x">
    <inertial>
      <inertia ixx="${sin(angle) + pi}" ixy="0" ixz="0" iyy="0" iyz="0" izz="0"/>
    </inertial>
  </link>
</robot>"#;
        let (doc, mut d) = document::parse(text);
        d.extend(check(&doc, text, None));
        assert!(
            !d.iter().any(|m| m.message.contains("Undefined")),
            "sin/pi are builtins and must not be flagged, got: {:?}",
            d.iter().map(|x| &x.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn inlay_hints_on_math_expressions() {
        use tower_lsp::lsp_types::{Position, Range};
        let text = r#"<?xml version="1.0"?>
<robot xmlns:xacro="http://www.ros.org/wiki/xacro" name="r">
  <xacro:property name="chassis_length" value="0.335"/>
  <xacro:property name="chassis_width" value="0.265"/>
  <link name="base_link"/>
  <link name="chassis">
    <visual>
      <origin xyz="${chassis_length/2} 0 ${chassis_width/2}"/>
    </visual>
  </link>
</robot>"#;
        let (doc, _) = document::parse(text);
        let full = Range::new(Position::new(0, 0), Position::new(100, 0));
        let hints = crate::features::inlay_hints(&doc, text, full);
        // Expect two hints: 0.335/2 = 0.1675 and 0.265/2 = 0.1325
        assert_eq!(hints.len(), 2, "expected 2 hints, got {:?}", hints.iter().map(|h| &h.label).collect::<Vec<_>>());
        let labels: Vec<String> = hints.iter().map(|h| match &h.label {
            tower_lsp::lsp_types::InlayHintLabel::String(s) => s.clone(),
            _ => String::new(),
        }).collect();
        assert!(labels.iter().any(|l| l.contains("0.1675")), "expected 0.1675, got: {:?}", labels);
        assert!(labels.iter().any(|l| l.contains("0.1325")), "expected 0.1325, got: {:?}", labels);
    }

    #[test]
    fn urdf_element_unknown_attribute_is_flagged() {
        // `<link foo="bar">` — foo is not in URDF_ELEMENTS' entry for `link`.
        let text = r#"<?xml version="1.0"?><robot name="r">
          <link name="a" foo="bar"/>
        </robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, false, None);
        assert!(
            diags.iter().any(|d| d.message.contains("Unknown attribute 'foo'") && d.message.contains("<link>")),
            "expected unknown-attribute diagnostic on <link>, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn urdf_element_unknown_element_is_flagged() {
        // `<weeble>` isn't in URDF_ELEMENTS or GEOMETRY_PRIMITIVES → known_urdf_attrs
        // returns None → "Unknown URDF element" diagnostic.
        let text = r#"<?xml version="1.0"?><robot name="r">
          <weeble/>
        </robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, false, None);
        assert!(
            diags.iter().any(|d| d.message.contains("Unknown URDF element") && d.message.contains("<weeble>")),
            "expected unknown-element diagnostic on <weeble>, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn urdf_element_missing_required_attr_via_table() {
        // `<mimic>` without `joint` — required_attrs comes from URDF_ELEMENTS entry.
        let text = r#"<?xml version="1.0"?><robot name="r">
          <joint name="j" type="revolute">
            <parent link="a"/><child link="b"/>
            <mimic/>
          </joint>
        </robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, false, None);
        assert!(
            diags.iter().any(|d| d.message.contains("<mimic>") && d.message.contains("'joint'")),
            "expected missing-required-attr diagnostic on <mimic>, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn geometry_primitive_missing_required_attr() {
        // <box> without size is invalid — confirms GEOMETRY_PRIMITIVES.required_attrs
        // is wired through required_urdf_attrs().
        let text = r#"<?xml version="1.0"?><robot name="r">
          <link name="a"><visual><geometry><box/></geometry></visual></link>
        </robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, false, None);
        assert!(
            diags.iter().any(|d| d.message.contains("<box>") && d.message.contains("'size'")),
            "expected missing-size diagnostic, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn geometry_primitive_unknown_attr() {
        // <sphere> with `length` is invalid — confirms GEOMETRY_PRIMITIVES.known_attrs
        // is wired through known_urdf_attrs().
        let text = r#"<?xml version="1.0"?><robot name="r">
          <link name="a"><visual><geometry><sphere radius="0.1" length="0.2"/></geometry></visual></link>
        </robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, false, None);
        assert!(
            diags.iter().any(|d| d.message.contains("Unknown attribute 'length'") && d.message.contains("<sphere>")),
            "expected unknown-attribute diagnostic on sphere, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn joint_type_typo_is_flagged() {
        // `rotational` is not a URDF joint type — must be flagged against JOINT_TYPES.
        let text = r#"<?xml version="1.0"?><robot name="r">
          <link name="a"/>
          <link name="b"/>
          <joint name="j" type="rotational">
            <parent link="a"/><child link="b"/>
          </joint>
        </robot>"#;
        let (_, mut diags) = document::parse(text);
        let xml = roxmltree::Document::parse(text).unwrap();
        diags.extend(check_schema(&xml, text, false, None));
        assert!(
            diags.iter().any(|d| d.message.contains("'type' must be one of") && d.message.contains("rotational")),
            "expected joint-type validator to flag 'rotational', got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn joint_type_valid_value_passes() {
        let text = r#"<?xml version="1.0"?><robot name="r">
          <link name="a"/>
          <link name="b"/>
          <joint name="j" type="revolute">
            <parent link="a"/><child link="b"/>
          </joint>
        </robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, false, None);
        assert!(
            !diags.iter().any(|d| d.message.contains("'type' must be one of")),
            "valid joint type should not be flagged, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn revolute_joint_missing_limit_is_flagged() {
        // urdf_parser refuses to load a revolute joint without <limit>;
        // the LSP should catch this before runtime.
        let text = r#"<?xml version="1.0"?><robot name="r">
          <link name="a"/>
          <link name="b"/>
          <joint name="j" type="revolute">
            <parent link="a"/><child link="b"/>
            <axis xyz="0 0 1"/>
          </joint>
        </robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, false, None);
        assert!(
            diags.iter().any(|d| d.message.contains("revolute") && d.message.contains("<limit>")),
            "expected missing-limit diagnostic on revolute joint, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn prismatic_joint_missing_limit_is_flagged() {
        let text = r#"<?xml version="1.0"?><robot name="r">
          <link name="a"/>
          <link name="b"/>
          <joint name="j" type="prismatic">
            <parent link="a"/><child link="b"/>
            <axis xyz="0 0 1"/>
          </joint>
        </robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, false, None);
        assert!(
            diags.iter().any(|d| d.message.contains("prismatic") && d.message.contains("<limit>")),
            "expected missing-limit diagnostic on prismatic joint, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn revolute_joint_with_limit_passes() {
        let text = r#"<?xml version="1.0"?><robot name="r">
          <link name="a"/>
          <link name="b"/>
          <joint name="j" type="revolute">
            <parent link="a"/><child link="b"/>
            <axis xyz="0 0 1"/>
            <limit lower="-1" upper="1" effort="10" velocity="1"/>
          </joint>
        </robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, false, None);
        assert!(
            !diags.iter().any(|d| d.message.contains("<limit>")),
            "revolute with <limit> must not be flagged, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn continuous_joint_does_not_require_limit() {
        // Only revolute and prismatic need a limit; continuous/fixed/floating do not.
        let text = r#"<?xml version="1.0"?><robot name="r">
          <link name="a"/>
          <link name="b"/>
          <joint name="j" type="continuous">
            <parent link="a"/><child link="b"/>
            <axis xyz="0 0 1"/>
          </joint>
        </robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, false, None);
        assert!(
            !diags.iter().any(|d| d.message.contains("<limit>")),
            "continuous joint should not require <limit>, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn undefined_material_reference_is_flagged() {
        // <material name="blue"/> inside <visual> without a top-level or
        // inline definition of "blue" should be flagged — urdf_parser warns
        // "material 'blue' undefined" at runtime.
        let text = r#"<?xml version="1.0"?><robot name="r">
          <link name="chassis">
            <visual>
              <geometry><box size="1 1 1"/></geometry>
              <material name="blue"/>
            </visual>
          </link>
        </robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, false, None);
        assert!(
            diags.iter().any(|d| d.message.contains("Undefined material 'blue'")),
            "expected undefined-material diagnostic, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn material_reference_resolved_by_top_level_definition() {
        let text = r#"<?xml version="1.0"?><robot name="r">
          <material name="blue"><color rgba="0 0 1 1"/></material>
          <link name="chassis">
            <visual>
              <geometry><box size="1 1 1"/></geometry>
              <material name="blue"/>
            </visual>
          </link>
        </robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, false, None);
        assert!(
            !diags.iter().any(|d| d.message.contains("Undefined material")),
            "top-level definition should satisfy the reference, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn material_reference_resolved_by_inline_definition_elsewhere() {
        // urdf_parser treats any inline `<material name="X"><color/></material>`
        // as a global definition by name; a bare ref in another link should
        // resolve to it regardless of document order.
        let text = r#"<?xml version="1.0"?><robot name="r">
          <link name="a">
            <visual>
              <geometry><box size="1 1 1"/></geometry>
              <material name="blue"/>
            </visual>
          </link>
          <link name="b">
            <visual>
              <geometry><box size="1 1 1"/></geometry>
              <material name="blue"><color rgba="0 0 1 1"/></material>
            </visual>
          </link>
        </robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, false, None);
        assert!(
            !diags.iter().any(|d| d.message.contains("Undefined material")),
            "inline definition elsewhere should satisfy the reference, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn material_reference_resolved_by_workspace_index() {
        // The current file only references "blue"; the definition lives in
        // another file that has been indexed in the workspace. The reference
        // must NOT be flagged.
        use crate::workspace::{FileSummary, WorkspaceIndex};
        use tower_lsp::lsp_types::{Position, Range, Url};

        let mut ws = WorkspaceIndex::default();
        ws.upsert(
            Url::parse("file:///materials.urdf.xacro").unwrap(),
            FileSummary {
                materials: vec![(
                    "blue".into(),
                    Range::new(Position::new(0, 0), Position::new(0, 1)),
                )],
                ..Default::default()
            },
        );

        let text = r#"<?xml version="1.0"?>
<robot xmlns:xacro="http://www.ros.org/wiki/xacro">
  <link name="chassis">
    <visual>
      <geometry><box size="1 1 1"/></geometry>
      <material name="blue"/>
    </visual>
  </link>
</robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, true, Some(&ws));
        assert!(
            !diags.iter().any(|d| d.message.contains("Undefined material")),
            "workspace-indexed material should satisfy the reference, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn xacro_softens_material_and_limit_diagnostics() {
        // In xacro fragments, definitions may come from xacro:include or macros;
        // missing material / missing limit should be Warning, not Error.
        let text = r#"<?xml version="1.0"?>
<robot xmlns:xacro="http://www.ros.org/wiki/xacro" name="r">
  <link name="a"/>
  <link name="b"/>
  <link name="c">
    <visual>
      <geometry><box size="1 1 1"/></geometry>
      <material name="blue"/>
    </visual>
  </link>
  <joint name="j" type="revolute">
    <parent link="a"/><child link="b"/>
    <axis xyz="0 0 1"/>
  </joint>
</robot>"#;
        let xml = roxmltree::Document::parse(text).unwrap();
        let diags = check_schema(&xml, text, true, None);
        let material_diag = diags.iter().find(|d| d.message.contains("Undefined material"));
        let limit_diag = diags.iter().find(|d| d.message.contains("<limit>"));
        assert!(material_diag.is_some(), "expected material diagnostic in xacro mode");
        assert!(limit_diag.is_some(), "expected limit diagnostic in xacro mode");
        assert_eq!(material_diag.unwrap().severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(limit_diag.unwrap().severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn xacro_macro_call_with_definition_passes() {
        // Macro defined in the same file → no diagnostic.
        let text = r#"<?xml version="1.0"?>
<robot xmlns:xacro="http://www.ros.org/wiki/xacro" name="r">
  <xacro:macro name="wheel" params="prefix">
    <link name="${prefix}_wheel"/>
  </xacro:macro>
  <xacro:wheel prefix="left"/>
</robot>"#;
        let (doc, mut diags) = document::parse(text);
        diags.extend(check(&doc, text, None));
        assert!(
            !diags.iter().any(|d| d.message.contains("Undefined xacro macro")),
            "defined macro must not be flagged, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn undefined_xacro_macro_call_is_flagged() {
        let text = r#"<?xml version="1.0"?>
<robot xmlns:xacro="http://www.ros.org/wiki/xacro" name="r">
  <xacro:fenagle prefix="left"/>
</robot>"#;
        let (doc, mut diags) = document::parse(text);
        diags.extend(check(&doc, text, None));
        assert!(
            diags.iter().any(|d| d.message.contains("Undefined xacro macro 'fenagle'")),
            "expected undefined-macro diagnostic, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn xacro_macro_call_resolved_by_workspace_index() {
        use crate::workspace::{FileSummary, WorkspaceIndex};
        use tower_lsp::lsp_types::{Position, Range, Url};

        let mut ws = WorkspaceIndex::default();
        ws.upsert(
            Url::parse("file:///macros.urdf.xacro").unwrap(),
            FileSummary {
                macros: vec![(
                    "wheel".into(),
                    Range::new(Position::new(0, 0), Position::new(0, 1)),
                )],
                ..Default::default()
            },
        );
        let text = r#"<?xml version="1.0"?>
<robot xmlns:xacro="http://www.ros.org/wiki/xacro" name="r">
  <xacro:wheel prefix="left"/>
</robot>"#;
        let (doc, mut diags) = document::parse(text);
        diags.extend(check(&doc, text, Some(&ws)));
        assert!(
            !diags.iter().any(|d| d.message.contains("Undefined xacro macro")),
            "workspace-indexed macro should satisfy the call, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn xacro_builtins_are_not_treated_as_macro_calls() {
        // <xacro:if>, <xacro:include>, etc. are part of the xacro language,
        // not macro invocations — they must never produce "Undefined macro".
        let text = r#"<?xml version="1.0"?>
<robot xmlns:xacro="http://www.ros.org/wiki/xacro" name="r">
  <xacro:include filename="other.urdf.xacro"/>
  <xacro:property name="foo" value="1"/>
  <xacro:if value="true">
    <link name="cond"/>
  </xacro:if>
  <xacro:unless value="false">
    <link name="cond2"/>
  </xacro:unless>
</robot>"#;
        let (doc, mut diags) = document::parse(text);
        diags.extend(check(&doc, text, None));
        let undef: Vec<_> = diags.iter().filter(|d| d.message.contains("Undefined xacro macro")).collect();
        assert!(undef.is_empty(),
            "built-in xacro tags must not be flagged as macro calls, got: {:?}",
            undef.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    #[test]
    fn isolated_link_suppressed_by_cross_file_joint() {
        // File-local view: link "lonely" has no joints in this file → would
        // be flagged as isolated. But the workspace says some joint somewhere
        // connects "lonely" — suppress.
        use crate::workspace::{FileSummary, WorkspaceIndex};
        use tower_lsp::lsp_types::{Position, Range, Url};

        let mut ws = WorkspaceIndex::default();
        ws.upsert(
            Url::parse("file:///elsewhere.urdf.xacro").unwrap(),
            FileSummary {
                linked_by_joint: vec![(
                    "lonely".into(),
                    Range::new(Position::new(0, 0), Position::new(0, 1)),
                )],
                ..Default::default()
            },
        );
        let text = r#"<?xml version="1.0"?>
<robot xmlns:xacro="http://www.ros.org/wiki/xacro" name="r">
  <link name="lonely"/>
  <link name="other"/>
  <joint name="j" type="fixed">
    <parent link="other"/><child link="extra"/>
  </joint>
  <link name="extra"/>
</robot>"#;
        let (doc, mut diags) = document::parse(text);
        diags.extend(check(&doc, text, Some(&ws)));
        let lonely_diags: Vec<_> = diags.iter()
            .filter(|d| d.message.contains("lonely") && (d.message.contains("no joints") || d.message.contains("not connected")))
            .collect();
        assert!(lonely_diags.is_empty(),
            "cross-file joint should suppress isolated-link diagnostic, got: {:?}",
            lonely_diags.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    #[test]
    fn tree_valid_chain() {
        let msgs = diag_messages(r#"<?xml version="1.0"?><robot name="r">
          <link name="base_link"/>
          <link name="link_a"/>
          <link name="link_b"/>
          <joint name="j1" type="fixed"><parent link="base_link"/><child link="link_a"/></joint>
          <joint name="j2" type="fixed"><parent link="link_a"/><child link="link_b"/></joint>
        </robot>"#);
        let tree_diags: Vec<_> = msgs.iter()
            .filter(|m| m.contains("root") || m.contains("connected") || m.contains("cycle"))
            .collect();
        assert!(tree_diags.is_empty(), "expected no tree diagnostics, got: {:?}", tree_diags);
    }
}
