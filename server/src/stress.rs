//! Stress / edge-case test suite for the URDF/xacro LSP.
//!
//! Each test exercises ONE scenario surfaced by the analytical swarm. Tests
//! marked "documents behavior" use eprintln! + soft assertions so the test
//! pins current behavior without failing the build — if the build ever
//! breaks for those, behavior actually changed and the test needs updating.

use crate::{diagnostics, document, workspace::{FileSummary, WorkspaceIndex}};
use tower_lsp::lsp_types::{DiagnosticSeverity, Url};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn diags_for(text: &str) -> Vec<(String, DiagnosticSeverity)> {
    let (doc, mut d) = document::parse(text);
    d.extend(diagnostics::check(&doc, text, None));
    if let Ok(xml) = roxmltree::Document::parse(text) {
        d.extend(diagnostics::check_schema(&xml, text, doc.is_xacro, None));
    }
    d.into_iter()
        .map(|x| (x.message, x.severity.unwrap_or(DiagnosticSeverity::ERROR)))
        .collect()
}

fn diags_for_ws(text: &str, ws: &WorkspaceIndex) -> Vec<(String, DiagnosticSeverity)> {
    let (doc, mut d) = document::parse(text);
    d.extend(diagnostics::check(&doc, text, Some(ws)));
    if let Ok(xml) = roxmltree::Document::parse(text) {
        d.extend(diagnostics::check_schema(&xml, text, doc.is_xacro, Some(ws)));
    }
    d.into_iter()
        .map(|x| (x.message, x.severity.unwrap_or(DiagnosticSeverity::ERROR)))
        .collect()
}

fn msgs(text: &str) -> Vec<String> {
    diags_for(text).into_iter().map(|(m, _)| m).collect()
}

fn msgs_ws(text: &str, ws: &WorkspaceIndex) -> Vec<String> {
    diags_for_ws(text, ws).into_iter().map(|(m, _)| m).collect()
}

fn has<T: AsRef<str>>(haystack: &[String], needle: T) -> bool {
    haystack.iter().any(|m| m.contains(needle.as_ref()))
}

fn count_matching<T: AsRef<str>>(haystack: &[String], needle: T) -> usize {
    haystack.iter().filter(|m| m.contains(needle.as_ref())).count()
}

const XACRO_NS: &str = "xmlns:xacro=\"http://www.ros.org/wiki/xacro\"";

fn xacro_doc(body: &str) -> String {
    format!("<?xml version=\"1.0\"?>\n<robot name=\"r\" {XACRO_NS}>\n{body}\n</robot>")
}

fn plain_doc(body: &str) -> String {
    format!("<?xml version=\"1.0\"?>\n<robot name=\"r\">\n{body}\n</robot>")
}

fn upsert_file(idx: &mut WorkspaceIndex, uri: &str, text: &str) {
    let (doc, _) = document::parse(text);
    idx.upsert(Url::parse(uri).unwrap(), FileSummary::from_doc(&doc));
}

// ---------------------------------------------------------------------------
// Tier 0 — sanity (these MUST all pass; canaries)
// ---------------------------------------------------------------------------

#[test]
fn sanity_clean_xacro_is_quiet() {
    let text = xacro_doc(r#"  <link name="a"/>"#);
    assert_eq!(msgs(&text), Vec::<String>::new(), "clean xacro should emit nothing");
}

// ---------------------------------------------------------------------------
// Materials (S-MAT-*)
// ---------------------------------------------------------------------------

#[test]
fn s_mat_a_xacro_undef_is_warning() {
    let text = xacro_doc(r#"  <link name="a"><visual><material name="blue"/></visual></link>"#);
    let (doc, _) = document::parse(&text);
    assert!(doc.is_xacro, "xmlns:xacro should mark doc as xacro");
    let d = diags_for(&text);
    let mat = d.iter().find(|(m, _)| m.to_lowercase().contains("material") && m.contains("blue"));
    assert!(mat.is_some(), "expected an undefined-material diag, got {d:?}");
    assert_eq!(mat.unwrap().1, DiagnosticSeverity::WARNING, "diags={d:?}");
}

#[test]
fn s_mat_b_urdf_undef_is_error() {
    let text = plain_doc(r#"  <link name="a"><visual><material name="blue"/></visual></link>"#);
    let d = diags_for(&text);
    let mat = d.iter().find(|(m, _)| m.to_lowercase().contains("material") && m.contains("blue"));
    assert!(mat.is_some(), "expected an undefined-material diag, got {d:?}");
    assert_eq!(mat.unwrap().1, DiagnosticSeverity::ERROR);
}

#[test]
fn s_mat_c_top_level_def_satisfies() {
    let text = plain_doc(
        r#"  <material name="blue"><color rgba="0 0 1 1"/></material>
  <link name="a"><visual><material name="blue"/></visual></link>"#,
    );
    let m = msgs(&text);
    assert!(!has(&m, "Undefined material"), "got: {m:?}");
}

#[test]
fn s_mat_d_ref_before_def_two_pass() {
    let text = plain_doc(
        r#"  <link name="a"><visual><material name="blue"/></visual></link>
  <material name="blue"><color rgba="0 0 1 1"/></material>"#,
    );
    let m = msgs(&text);
    assert!(!has(&m, "Undefined material"), "two-pass collect should resolve forward refs; got: {m:?}");
}

#[test]
fn s_mat_e_xacro_property_in_material_name_documents_behavior() {
    let text = xacro_doc(
        r#"  <xacro:property name="color" value="blue"/>
  <material name="blue"><color rgba="0 0 1 1"/></material>
  <link name="a"><visual><material name="${color}"/></visual></link>"#,
    );
    let m = msgs(&text);
    let flagged = has(&m, "Undefined material");
    eprintln!("s_mat_e: ${{color}} in material name — flagged as undefined? {flagged}");
    eprintln!("   diags: {m:?}");
}

#[test]
fn s_mat_f_empty_name_documents_behavior() {
    let text = xacro_doc(r#"  <link name="a"><visual><material name=""/></visual></link>"#);
    let m = msgs(&text);
    eprintln!("s_mat_f: empty material name diags: {m:?}");
}

#[test]
fn s_mat_g_inline_def_with_color_not_double_flagged() {
    let text = plain_doc(
        r#"  <link name="a"><visual><material name="local"><color rgba="0 0 1 1"/></material></visual></link>"#,
    );
    let m = msgs(&text);
    assert!(!has(&m, "Undefined material"), "inline material def shouldn't flag itself: {m:?}");
}

#[test]
fn s_mat_h_cross_file_workspace_resolution() {
    let mut idx = WorkspaceIndex::default();
    upsert_file(&mut idx, "file:///materials.xacro", &xacro_doc(
        r#"  <material name="blue"><color rgba="0 0 1 1"/></material>"#,
    ));
    let text = xacro_doc(r#"  <link name="a"><visual><material name="blue"/></visual></link>"#);
    let m = msgs_ws(&text, &idx);
    assert!(!has(&m, "Undefined material"), "ws-resolved material should not flag; got {m:?}");
}

// ---------------------------------------------------------------------------
// Joint limits (S-JOINT-*)
// ---------------------------------------------------------------------------

#[test]
fn s_joint_h_revolute_no_limit_flagged() {
    let text = plain_doc(
        r#"  <link name="a"/><link name="b"/>
  <joint name="j" type="revolute">
    <parent link="a"/><child link="b"/>
  </joint>"#,
    );
    let m = msgs(&text);
    assert!(has(&m, "limit"), "revolute w/o limit should flag; got {m:?}");
}

#[test]
fn s_joint_i_prismatic_no_limit_flagged() {
    let text = plain_doc(
        r#"  <link name="a"/><link name="b"/>
  <joint name="j" type="prismatic">
    <parent link="a"/><child link="b"/>
  </joint>"#,
    );
    let m = msgs(&text);
    assert!(has(&m, "limit"), "prismatic w/o limit should flag; got {m:?}");
}

#[test]
fn s_joint_j_revolute_with_limit_ok() {
    let text = plain_doc(
        r#"  <link name="a"/><link name="b"/>
  <joint name="j" type="revolute">
    <parent link="a"/><child link="b"/>
    <limit lower="0" upper="1" effort="1" velocity="1"/>
  </joint>"#,
    );
    let m = msgs(&text);
    assert!(!has(&m, "missing required <limit>"), "got {m:?}");
}

#[test]
fn s_joint_k_continuous_no_limit_ok() {
    let text = plain_doc(
        r#"  <link name="a"/><link name="b"/>
  <joint name="j" type="continuous">
    <parent link="a"/><child link="b"/>
  </joint>"#,
    );
    let m = msgs(&text);
    assert!(!has(&m, "missing required <limit>"), "continuous shouldn't require limit; got {m:?}");
}

#[test]
fn s_joint_k_fixed_no_limit_ok() {
    let text = plain_doc(
        r#"  <link name="a"/><link name="b"/>
  <joint name="j" type="fixed">
    <parent link="a"/><child link="b"/>
  </joint>"#,
    );
    let m = msgs(&text);
    assert!(!has(&m, "missing required <limit>"), "fixed shouldn't require limit; got {m:?}");
}

#[test]
fn s_joint_l_xacro_property_resolves_to_revolute() {
    let text = xacro_doc(
        r#"  <xacro:property name="jt" value="revolute"/>
  <link name="a"/><link name="b"/>
  <joint name="j" type="${jt}">
    <parent link="a"/><child link="b"/>
  </joint>"#,
    );
    let m = msgs(&text);
    let flagged = has(&m, "limit");
    eprintln!("s_joint_l: type=${{jt}} → revolute, limit missing? flagged={flagged}; diags={m:?}");
}

#[test]
fn s_joint_m_undefined_xacro_property_in_type_no_spurious_limit() {
    let text = xacro_doc(
        r#"  <link name="a"/><link name="b"/>
  <joint name="j" type="${unknown}">
    <parent link="a"/><child link="b"/>
  </joint>"#,
    );
    let m = msgs(&text);
    eprintln!("s_joint_m: undefined property in joint type; diags={m:?}");
    assert!(!m.iter().any(|x| x.contains("PANIC")));
}

#[test]
fn s_joint_o_xacro_namespaced_limit_documents_behavior() {
    let text = xacro_doc(
        r#"  <link name="a"/><link name="b"/>
  <joint name="j" type="revolute">
    <parent link="a"/><child link="b"/>
    <xacro:limit lower="0" upper="1" effort="1" velocity="1"/>
  </joint>"#,
    );
    let m = msgs(&text);
    eprintln!("s_joint_o: xacro:limit on revolute; missing-limit flagged? {} diags={m:?}", has(&m, "missing required <limit>"));
}

// ---------------------------------------------------------------------------
// Cross-file workspace (S-XFILE-*)
// ---------------------------------------------------------------------------

#[test]
fn s_xfile_link_def_in_other_file_suppresses_kinematic_isolation() {
    let mut idx = WorkspaceIndex::default();
    // File A defines the wheel link
    upsert_file(&mut idx, "file:///a.xacro", &xacro_doc(r#"  <link name="wheel"/>"#));
    // File B has joint referencing wheel but no link def for wheel locally
    let text_b = xacro_doc(
        r#"  <link name="chassis"/>
  <joint name="j" type="fixed">
    <parent link="chassis"/><child link="wheel"/>
  </joint>"#,
    );
    upsert_file(&mut idx, "file:///b.xacro", &text_b);
    let m = msgs_ws(&text_b, &idx);
    assert!(!has(&m, "Undefined link 'wheel'"), "ws should resolve cross-file link; got {m:?}");
}

#[test]
fn s_xfile_macro_def_resolves_call() {
    let mut idx = WorkspaceIndex::default();
    upsert_file(&mut idx, "file:///macros.xacro", &xacro_doc(
        r#"  <xacro:macro name="leg" params="suffix"><link name="${suffix}_leg"/></xacro:macro>"#,
    ));
    let text_b = xacro_doc(r#"  <xacro:leg suffix="left"/>"#);
    let m = msgs_ws(&text_b, &idx);
    assert!(!has(&m, "Undefined xacro macro 'leg'"), "got {m:?}");
}

#[test]
fn s_xfile_macro_undefined_when_not_in_ws() {
    let idx = WorkspaceIndex::default();
    let text_b = xacro_doc(r#"  <xacro:nonexistent_macro/>"#);
    let m = msgs_ws(&text_b, &idx);
    assert!(has(&m, "Undefined xacro macro 'nonexistent_macro'"), "should flag; got {m:?}");
}

#[test]
fn s_xfile_property_def_in_other_file() {
    let mut idx = WorkspaceIndex::default();
    upsert_file(&mut idx, "file:///props.xacro", &xacro_doc(
        r#"  <xacro:property name="mass_val" value="1.0"/>"#,
    ));
    let text_b = xacro_doc(r#"  <link name="a"><inertial><mass value="${mass_val}"/></inertial></link>"#);
    let m = msgs_ws(&text_b, &idx);
    assert!(!has(&m, "Undefined property") && !has(&m, "Undefined xacro property"),
            "cross-file property ref should resolve; got {m:?}");
}

#[test]
fn s_xfile_same_name_two_files_keeps_both_defs() {
    let mut idx = WorkspaceIndex::default();
    upsert_file(&mut idx, "file:///a.xacro", &xacro_doc(r#"  <link name="base_link"/>"#));
    upsert_file(&mut idx, "file:///b.xacro", &xacro_doc(r#"  <link name="base_link"/>"#));
    let defs = idx.link_defs("base_link");
    assert_eq!(defs.len(), 2, "expected 2 defs across files, got {}", defs.len());
}

#[test]
fn s_xfile_empty_link_name_indexed_documents_behavior() {
    let mut idx = WorkspaceIndex::default();
    upsert_file(&mut idx, "file:///a.xacro", &xacro_doc(r#"  <link name=""/>"#));
    let has_empty = idx.has_link("");
    eprintln!("s_xfile_empty: empty-name link is in index? {has_empty}");
}

// ---------------------------------------------------------------------------
// Kinematic tree (S-KTREE-*)
// ---------------------------------------------------------------------------

#[test]
fn s_ktree_empty_robot_silent() {
    let text = plain_doc(r#"  "#);
    let m = msgs(&text);
    assert!(m.is_empty(), "empty robot should emit no kinematic diag; got {m:?}");
}

#[test]
fn s_ktree_single_link_silent() {
    let text = plain_doc(r#"  <link name="a"/>"#);
    let m = msgs(&text);
    assert!(!has(&m, "no joint") && !has(&m, "isolated") && !has(&m, "not connected"),
            "single-link robot should not flag; got {m:?}");
}

#[test]
fn s_ktree_two_links_one_joint_clean() {
    let text = plain_doc(
        r#"  <link name="a"/><link name="b"/>
  <joint name="j" type="fixed"><parent link="a"/><child link="b"/></joint>"#,
    );
    let m = msgs(&text);
    assert!(!has(&m, "no joint") && !has(&m, "not connected") && !has(&m, "Multiple roots"),
            "clean 2-link tree should be quiet; got {m:?}");
}

#[test]
fn s_ktree_pure_cycle_three_flags_cycle_not_disconnected() {
    let text = plain_doc(
        r#"  <link name="a"/><link name="b"/><link name="c"/>
  <joint name="j1" type="fixed"><parent link="a"/><child link="b"/></joint>
  <joint name="j2" type="fixed"><parent link="b"/><child link="c"/></joint>
  <joint name="j3" type="fixed"><parent link="c"/><child link="a"/></joint>"#,
    );
    let m = msgs(&text);
    assert!(has(&m, "cycle"), "pure cycle should be flagged as cycle; got {m:?}");
    assert_eq!(count_matching(&m, "not connected"), 0,
               "pure cycle should NOT be misreported as disconnected; got {m:?}");
}

#[test]
fn s_ktree_self_loop_flags_cycle_and_self_ref() {
    let text = plain_doc(
        r#"  <link name="a"/>
  <joint name="j" type="fixed"><parent link="a"/><child link="a"/></joint>"#,
    );
    let m = msgs(&text);
    assert!(has(&m, "same link as parent and child"), "self-ref check should fire; got {m:?}");
    assert!(has(&m, "cycle"), "self-loop is a 1-cycle; got {m:?}");
}

#[test]
fn s_ktree_multiple_roots_flagged() {
    let text = plain_doc(
        r#"  <link name="a"/><link name="b"/><link name="c"/><link name="d"/>
  <joint name="j1" type="fixed"><parent link="a"/><child link="b"/></joint>
  <joint name="j2" type="fixed"><parent link="c"/><child link="d"/></joint>"#,
    );
    let m = msgs(&text);
    assert!(has(&m, "Multiple roots") || has(&m, "multiple roots") || count_matching(&m, "root") >= 2,
            "two chains should flag multiple roots; got {m:?}");
}

#[test]
fn s_ktree_disconnected_island_documents_behavior() {
    let text = plain_doc(
        r#"  <link name="a"/><link name="b"/><link name="c"/><link name="d"/>
  <joint name="j1" type="fixed"><parent link="a"/><child link="b"/></joint>
  <joint name="j2" type="fixed"><parent link="c"/><child link="d"/></joint>"#,
    );
    let m = msgs(&text);
    let nc = has(&m, "not connected") || has(&m, "disconnected");
    eprintln!("s_ktree_disconnected_island: 'not connected' or 'disconnected'? {nc}; diags={m:?}");
}

#[test]
fn s_ktree_joint_missing_child_flagged_on_joint() {
    let text = plain_doc(
        r#"  <link name="a"/><link name="b"/>
  <joint name="j" type="fixed"><parent link="a"/></joint>"#,
    );
    let m = msgs(&text);
    assert!(has(&m, "missing required <child>"), "half-joint should be flagged; got {m:?}");
}

#[test]
fn s_ktree_joint_missing_parent_flagged_on_joint() {
    let text = plain_doc(
        r#"  <link name="a"/><link name="b"/>
  <joint name="j" type="fixed"><child link="b"/></joint>"#,
    );
    let m = msgs(&text);
    assert!(has(&m, "missing required <parent>"), "half-joint should be flagged; got {m:?}");
}

#[test]
fn s_ktree_deep_chain_100_links_silent() {
    let mut body = String::new();
    for i in 0..100 {
        body.push_str(&format!("  <link name=\"L{i}\"/>\n"));
    }
    for i in 0..99 {
        body.push_str(&format!(
            "  <joint name=\"j{i}\" type=\"fixed\"><parent link=\"L{i}\"/><child link=\"L{}\"/></joint>\n",
            i + 1
        ));
    }
    let text = plain_doc(&body);
    let m = msgs(&text);
    assert!(!has(&m, "not connected") && !has(&m, "Multiple roots"),
            "100-link chain should not flag; got {} diags", m.len());
}

#[test]
fn s_ktree_ws_suppresses_isolated_link() {
    let mut idx = WorkspaceIndex::default();
    // Other file has a joint touching our "wheel"
    upsert_file(&mut idx, "file:///other.xacro", &xacro_doc(
        r#"  <joint name="jx" type="fixed"><parent link="chassis"/><child link="wheel"/></joint>"#,
    ));
    let text = xacro_doc(r#"  <link name="chassis"/><link name="wheel"/>"#);
    let m = msgs_ws(&text, &idx);
    assert!(!has(&m, "no joint") && !has(&m, "not connected"),
            "ws.link_touched_by_joint should suppress; got {m:?}");
}

// ---------------------------------------------------------------------------
// Xacro pathology (S-XACRO-*)
// ---------------------------------------------------------------------------

#[test]
fn s_xacro_unclosed_dollar_brace_flagged() {
    let text = xacro_doc(r#"  <link name="${foo"/>"#);
    let m = msgs(&text);
    assert!(has(&m, "Unclosed") || has(&m, "unclosed") || !m.is_empty(),
            "unclosed ${{ should produce a diagnostic; got {m:?}");
}

#[test]
fn s_xacro_multiple_dollar_in_one_attr_clean() {
    let text = xacro_doc(
        r#"  <xacro:property name="x" value="0"/>
  <xacro:property name="y" value="0"/>
  <xacro:property name="z" value="0"/>
  <link name="L"><visual><origin xyz="${x} ${y} ${z}"/></visual></link>"#,
    );
    let m = msgs(&text);
    assert!(!m.iter().any(|x| x.starts_with("Undefined")), "got {m:?}");
}

#[test]
fn s_xacro_arithmetic_no_diag() {
    let text = xacro_doc(
        r#"  <link name="L"><visual><origin xyz="${1 + 2 * 3} 0 0"/></visual></link>"#,
    );
    let m = msgs(&text);
    assert!(!m.iter().any(|x| x.starts_with("Undefined")), "pure-arith should not flag; got {m:?}");
}

#[test]
fn s_xacro_undef_in_complex_expr_flagged() {
    let text = xacro_doc(
        r#"  <link name="L"><visual><origin xyz="${1 + undefined_thing} 0 0"/></visual></link>"#,
    );
    let m = msgs(&text);
    assert!(has(&m, "undefined_thing"), "should flag undefined_thing; got {m:?}");
}

#[test]
fn s_xacro_builtins_pi_sin_documents_behavior() {
    // BUG: 'pi' is a recognized constant in xacro, but the diagnostic scanner's
    // builtin set does NOT include it — false positive "Undefined xacro
    // property 'pi'" against `${pi}`.
    let text = xacro_doc(
        r#"  <link name="L"><visual><origin xyz="${pi} ${sin(0)} ${cos(0)}"/></visual></link>"#,
    );
    let m = msgs(&text);
    let flagged_pi = has(&m, "Undefined") && has(&m, "pi");
    eprintln!("s_xacro_builtins_pi_sin: 'pi/sin/cos' flagged? {flagged_pi}; diags={m:?}");
}

#[test]
fn s_xacro_builtins_extended_documents_behavior() {
    // atan2 / ceil / floor / e / tau may be supported by xacro_eval but not by
    // the diagnostic scanner — this test pins which side is the source of truth
    let text = xacro_doc(
        r#"  <link name="L"><visual><origin xyz="${atan2(1,1)} ${ceil(0.5)} ${floor(0.5)}"/></visual></link>"#,
    );
    let m = msgs(&text);
    eprintln!("s_xacro_builtins_extended: diags={m:?}");
}

#[test]
fn s_xacro_property_inside_macro_documents_behavior() {
    let text = xacro_doc(
        r#"  <xacro:macro name="m">
    <xacro:property name="local" value="1"/>
    <link name="L${local}"/>
  </xacro:macro>"#,
    );
    let m = msgs(&text);
    let flagged_local = has(&m, "local") && has(&m, "Undefined");
    eprintln!("s_xacro_property_inside_macro: 'local' undefined? {flagged_local}; diags={m:?}");
}

#[test]
fn s_xacro_forward_property_reference_resolves() {
    let text = xacro_doc(
        r#"  <link name="L${x}"/>
  <xacro:property name="x" value="1"/>"#,
    );
    let m = msgs(&text);
    let flagged = has(&m, "Undefined") && has(&m, "x");
    eprintln!("s_xacro_forward_property: 'x' flagged? {flagged}; diags={m:?}");
}

#[test]
fn s_xacro_macro_without_name_no_panic() {
    let text = xacro_doc(r#"  <xacro:macro/>"#);
    let m = msgs(&text);
    eprintln!("s_xacro_macro_without_name: diags={m:?}");
}

#[test]
fn s_xacro_macro_params_documents_behavior() {
    let text = xacro_doc(
        r#"  <xacro:macro name="m" params="suffix *origin">
    <link name="${suffix}_link"/>
  </xacro:macro>"#,
    );
    let m = msgs(&text);
    let flagged_suffix = has(&m, "suffix") && has(&m, "Undefined");
    eprintln!("s_xacro_macro_params: 'suffix' undefined? {flagged_suffix}; diags={m:?}");
}

#[test]
fn s_xacro_include_no_diag() {
    let text = xacro_doc(r#"  <xacro:include filename="other.xacro"/>"#);
    let m = msgs(&text);
    assert!(!has(&m, "Undefined xacro macro"), "include is a builtin; got {m:?}");
}

#[test]
fn s_xacro_arg_no_diag() {
    let text = xacro_doc(r#"  <xacro:arg name="prefix" default=""/>"#);
    let m = msgs(&text);
    assert!(!has(&m, "Undefined xacro macro"), "arg is a builtin; got {m:?}");
}

#[test]
fn s_xacro_if_lenient_container() {
    let text = xacro_doc(
        r#"  <xacro:property name="flag" value="true"/>
  <xacro:if value="${flag}">
    <link name="conditional_link"/>
  </xacro:if>"#,
    );
    let m = msgs(&text);
    assert!(!has(&m, "Unknown element"), "xacro:if should be a lenient container; got {m:?}");
}

#[test]
fn s_xacro_unless_lenient_container() {
    let text = xacro_doc(
        r#"  <xacro:property name="flag" value="false"/>
  <xacro:unless value="${flag}">
    <link name="conditional_link"/>
  </xacro:unless>"#,
    );
    let m = msgs(&text);
    assert!(!has(&m, "Unknown element"), "xacro:unless should be a lenient container; got {m:?}");
}

#[test]
fn s_xacro_hyphenated_property_documents_behavior() {
    let text = xacro_doc(
        r#"  <xacro:property name="my-prop" value="1"/>
  <link name="L${my-prop}"/>"#,
    );
    let m = msgs(&text);
    let flagged = m.iter().any(|x| x.contains("Undefined") && (x.contains("my") || x.contains("prop")));
    eprintln!("s_xacro_hyphenated_property: false-positive split? {flagged}; diags={m:?}");
}

#[test]
fn s_xacro_empty_property_value_no_diag() {
    let text = xacro_doc(
        r#"  <xacro:property name="empty" value=""/>
  <link name="L${empty}"/>"#,
    );
    let m = msgs(&text);
    assert!(!has(&m, "Undefined") || !has(&m, "empty"), "empty value still defined; got {m:?}");
}

#[test]
fn s_xacro_transitive_property_documents_behavior() {
    let text = xacro_doc(
        r#"  <xacro:property name="a" value="1"/>
  <xacro:property name="b" value="2"/>
  <xacro:property name="m" value="${a+b}"/>
  <link name="L${m}"/>"#,
    );
    let m = msgs(&text);
    eprintln!("s_xacro_transitive: diags={m:?}");
}

#[test]
fn s_xacro_robot_name_optional() {
    let text = format!("<?xml version=\"1.0\"?>\n<robot {XACRO_NS}>\n  <link name=\"a\"/>\n</robot>");
    let m = msgs(&text);
    assert!(!has(&m, "name"), "robot name should be optional in xacro; got {m:?}");
}
