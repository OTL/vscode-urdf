use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

/// How an opening tag terminated (return value of [`scan_to_tag_close`]).
pub(crate) enum TagCloseResult {
    /// Saw `>` — element is open. `end` is the byte offset just past `>`.
    Open { end: usize },
    /// Saw `/>` — element is self-closing. `end` is the byte offset just past `>`.
    SelfClosing { end: usize },
    /// Reached end-of-input without seeing either (e.g. the tag is still being typed).
    Unterminated,
}

/// Walk `bytes` from `start` looking for the end of an XML opening tag,
/// honouring quoted attribute values so a `>` or `/` inside `attr="..."`
/// doesn't fool the scanner. Caller is responsible for positioning `start`
/// *after* the tag name (i.e. at the start of the attribute zone).
///
/// **Single source of truth** for the quote-aware tag-end scan.
///
/// **Consumers:**
///   - `scan_tag_balance()` in this file → push to stack on `Open`, skip on `SelfClosing`
///   - `find_opening_tag_end()` in this file → unwrap `end`, fall back to `bytes.len()` on `Unterminated`
///   - `inside_gazebo_block()` in `server/src/features.rs` → map `Open` → true, `SelfClosing` → false
pub(crate) fn scan_to_tag_close(bytes: &[u8], start: usize) -> TagCloseResult {
    let mut i = start;
    let mut in_quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match in_quote {
            Some(q) if b == q => in_quote = None,
            Some(_) => {}
            None => {
                if b == b'"' || b == b'\'' {
                    in_quote = Some(b);
                } else if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'>' {
                    return TagCloseResult::SelfClosing { end: i + 2 };
                } else if b == b'>' {
                    return TagCloseResult::Open { end: i + 1 };
                }
            }
        }
        i += 1;
    }
    TagCloseResult::Unterminated
}

/// Identifier set on every `Diagnostic.source` this server emits.
/// Editors use this to filter / group problems by which server produced them.
///
/// **Consumers (read from this constant):**
///   - `parse()` → XML parse-error diagnostic in this file
///   - `diag_at()` → tag-balance fallback diagnostics in this file
///   - `make_diag()` in `server/src/diagnostics.rs`
///
/// (see `server/src/document.rs` for source — this is it.)
pub(crate) const DIAGNOSTIC_SOURCE: &str = "urdf-lsp";

#[derive(Debug, Clone)]
pub struct Document {
    pub links: Vec<NamedItem>,
    pub joints: Vec<Joint>,
    pub materials: Vec<NamedItem>,
    pub xacro_properties: Vec<XacroProperty>,
    /// `reference` attribute values from `<gazebo reference="...">` elements.
    pub gazebo_refs: Vec<NameRef>,
    /// `<xacro:macro name="X">` definitions found anywhere in the tree.
    pub xacro_macros: Vec<NamedItem>,
    /// Call sites — every `<xacro:X .../>` whose local name isn't a built-in
    /// xacro element (macro / property / include / if / unless / arg / …).
    pub xacro_macro_calls: Vec<NameRef>,
    /// True when the root element declares xmlns:xacro — indicates a xacro fragment
    /// where some links/joints may be defined in included files.
    pub is_xacro: bool,
    /// False when XML parsing failed — semantic checks (undefined refs, etc.)
    /// must skip work, otherwise empty symbol tables flag everything as undefined.
    pub parse_ok: bool,
}

/// Element names under the `xacro:` namespace that are part of the xacro
/// language itself, not user-defined macro invocations. A `<xacro:foo>` whose
/// local name isn't in this list is treated as a call to a macro named `foo`.
pub(crate) const XACRO_BUILTINS: &[&str] = &[
    "macro", "property", "include", "if", "unless", "arg",
    "insert_block", "element", "attribute", "call",
];

#[derive(Debug, Clone)]
pub struct XacroProperty {
    pub name: String,
    pub value: String,
    pub range: Range,
}

#[derive(Debug, Clone)]
pub struct NamedItem {
    pub name: String,
    pub range: Range,
}

#[derive(Debug, Clone)]
pub struct Joint {
    pub name: String,
    pub range: Range,
    pub joint_type: Option<String>,
    pub parent: Option<NameRef>,
    pub child: Option<NameRef>,
    pub mimic: Option<NameRef>,
}

#[derive(Debug, Clone)]
pub struct NameRef {
    pub name: String,
    pub range: Range,
}

pub fn parse(text: &str) -> (Document, Vec<Diagnostic>) {
    let mut doc = Document {
        links: Vec::new(),
        joints: Vec::new(),
        materials: Vec::new(),
        xacro_properties: Vec::new(),
        gazebo_refs: Vec::new(),
        xacro_macros: Vec::new(),
        xacro_macro_calls: Vec::new(),
        is_xacro: false,
        parse_ok: true,
    };

    let xml = match roxmltree::Document::parse(text) {
        Ok(d) => d,
        Err(e) => {
            // Tag-balance scanner: when XML parsing fails, try to identify
            // mismatched/unclosed tags ourselves so the diagnostic points at
            // the actual misspelled opening tag, not the closing tag where
            // roxmltree first noticed the inconsistency.
            doc.parse_ok = false;
            let balance_diags = scan_tag_balance(text);
            if !balance_diags.is_empty() {
                return (doc, balance_diags);
            }
            let msg = e.to_string();
            let (line, col) = parse_xml_error_pos(&msg);
            let pos = Position::new(line, col);
            let diag = Diagnostic {
                range: Range::new(pos, Position::new(line, col + 1)),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some(DIAGNOSTIC_SOURCE.into()),
                message: format!("XML parse error: {e}"),
                ..Diagnostic::default()
            };
            return (doc, vec![diag]);
        }
    };

    let root = xml.root_element();

    // roxmltree exposes `xmlns:*` declarations through `namespaces()`, not
    // `attributes()`, so the old attribute-based detection was always false.
    doc.is_xacro = root.namespaces().any(|n| {
        n.name() == Some("xacro") || n.uri().contains("xacro")
    });

    for node in root.children() {
        if !node.is_element() {
            continue;
        }

        let tag = node.tag_name().name();

        match tag {
            "link" => {
                if let Some(name) = node.attribute("name") {
                    let range = attr_value_range(text, &node, "name");
                    doc.links.push(NamedItem {
                        name: name.to_string(),
                        range,
                    });
                }
            }
            "joint" => {
                if let Some(name) = node.attribute("name") {
                    let range = attr_value_range(text, &node, "name");
                    let joint_type = node.attribute("type").map(|s| s.to_string());

                    let mut parent: Option<NameRef> = None;
                    let mut child: Option<NameRef> = None;
                    let mut mimic: Option<NameRef> = None;

                    for child_node in node.children() {
                        if !child_node.is_element() {
                            continue;
                        }
                        match child_node.tag_name().name() {
                            "parent" => {
                                if let Some(link_name) = child_node.attribute("link") {
                                    let r = attr_value_range(text, &child_node, "link");
                                    parent = Some(NameRef {
                                        name: link_name.to_string(),
                                        range: r,
                                    });
                                }
                            }
                            "child" => {
                                if let Some(link_name) = child_node.attribute("link") {
                                    let r = attr_value_range(text, &child_node, "link");
                                    child = Some(NameRef {
                                        name: link_name.to_string(),
                                        range: r,
                                    });
                                }
                            }
                            "mimic" => {
                                if let Some(joint_name) = child_node.attribute("joint") {
                                    let r = attr_value_range(text, &child_node, "joint");
                                    mimic = Some(NameRef {
                                        name: joint_name.to_string(),
                                        range: r,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }

                    doc.joints.push(Joint {
                        name: name.to_string(),
                        range,
                        joint_type,
                        parent,
                        child,
                        mimic,
                    });
                }
            }
            "material" => {
                // Only top-level material definitions (those with a name attribute at robot level)
                if let Some(name) = node.attribute("name") {
                    let range = attr_value_range(text, &node, "name");
                    doc.materials.push(NamedItem {
                        name: name.to_string(),
                        range,
                    });
                }
            }
            "gazebo" => {
                if let Some(reference) = node.attribute("reference") {
                    let range = attr_value_range(text, &node, "reference");
                    doc.gazebo_refs.push(NameRef {
                        name: reference.to_string(),
                        range,
                    });
                }
            }
            _ => {
                let full_name = node.tag_name();
                let is_xacro_property = (full_name.name() == "property"
                    && full_name.namespace() == Some("http://www.ros.org/wiki/xacro"))
                    || tag == "xacro:property";
                if is_xacro_property {
                    if let Some(name) = node.attribute("name") {
                        let range = attr_value_range(text, &node, "name");
                        let value = node.attribute("value").unwrap_or("").to_string();
                        doc.xacro_properties.push(XacroProperty {
                            name: name.to_string(),
                            value,
                            range,
                        });
                    }
                }
            }
        }
    }

    // Macros (defs + calls) can be nested anywhere — macros call macros, calls
    // appear inside <link>/<joint>/<inertial>/etc. Walk the whole tree.
    collect_xacro_macros(root, text, &mut doc);

    (doc, vec![])
}

fn collect_xacro_macros(node: roxmltree::Node, text: &str, doc: &mut Document) {
    if !node.is_element() {
        return;
    }
    let local = xacro_local_name(&node);
    if let Some(local) = local {
        match local {
            "macro" => {
                if let Some(name) = node.attribute("name") {
                    let range = attr_value_range(text, &node, "name");
                    doc.xacro_macros.push(NamedItem {
                        name: name.to_string(),
                        range,
                    });
                }
            }
            // Calls are anything under `xacro:` that isn't a built-in.
            other if !XACRO_BUILTINS.contains(&other) => {
                let range = elem_name_range(text, &node);
                doc.xacro_macro_calls.push(NameRef {
                    name: other.to_string(),
                    range,
                });
            }
            _ => {}
        }
    }
    for child in node.children() {
        collect_xacro_macros(child, text, doc);
    }
}

/// Return the local name iff `node` is in the xacro namespace (either via
/// `xmlns:xacro=...` or via the `xacro:` prefix syntax).
fn xacro_local_name<'a>(node: &roxmltree::Node<'a, 'a>) -> Option<&'a str> {
    let tag = node.tag_name();
    if tag.namespace().map_or(false, |n| n.contains("xacro")) {
        return Some(tag.name());
    }
    // Fall back: roxmltree without a declared namespace keeps the prefix.
    // Strictly speaking a well-formed xacro file declares xmlns:xacro, but
    // be lenient.
    let raw = tag.name();
    raw.strip_prefix("xacro:")
}

fn elem_name_range(text: &str, node: &roxmltree::Node) -> Range {
    // Skip the leading '<' and any namespace prefix so the range points at the
    // local name the user types as the macro identifier.
    let start = node.range().start + 1;
    let name = node.tag_name().name();
    let bytes = text.as_bytes();
    // Walk past the prefix if present (`xacro:foo` → start at `foo`).
    let mut name_start = start;
    while name_start < bytes.len() && bytes[name_start] != b':'
        && (bytes[name_start] as char).is_ascii_graphic() && bytes[name_start] != b' '
        && bytes[name_start] != b'>' && bytes[name_start] != b'/'
    {
        name_start += 1;
    }
    if name_start < bytes.len() && bytes[name_start] == b':' {
        name_start += 1;
    } else {
        name_start = start;
    }
    byte_range_to_lsp(text, name_start..name_start + name.len())
}

/// Walk the document tracking opening/closing tag balance. Returns a single
/// diagnostic positioned on the actual misspelled (or unclosed) opening tag,
/// rather than at the closing tag where the inconsistency was detected.
/// Used as a fallback when roxmltree::Document::parse fails.
fn scan_tag_balance(text: &str) -> Vec<Diagnostic> {
    let bytes = text.as_bytes();
    // Stack entries: (tag name, byte start of name, byte end of name)
    let mut stack: Vec<(&str, usize, usize)> = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        i += 1;
        if i >= bytes.len() {
            break;
        }

        // <?xml … ?>  or  <?target …?>
        if bytes[i] == b'?' {
            i += 1;
            while i + 1 < bytes.len() && !(bytes[i] == b'?' && bytes[i + 1] == b'>') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }

        if bytes[i] == b'!' {
            // Comment <!-- … -->
            if i + 2 < bytes.len() && bytes[i + 1] == b'-' && bytes[i + 2] == b'-' {
                i += 3;
                while i + 2 < bytes.len()
                    && !(bytes[i] == b'-' && bytes[i + 1] == b'-' && bytes[i + 2] == b'>')
                {
                    i += 1;
                }
                i = (i + 3).min(bytes.len());
                continue;
            }
            // CDATA <![CDATA[ … ]]>
            if i + 7 < bytes.len() && &bytes[i + 1..i + 8] == b"[CDATA[" {
                i += 8;
                while i + 2 < bytes.len()
                    && !(bytes[i] == b']' && bytes[i + 1] == b']' && bytes[i + 2] == b'>')
                {
                    i += 1;
                }
                i = (i + 3).min(bytes.len());
                continue;
            }
            // DOCTYPE or any other declaration — skip to '>'
            while i < bytes.len() && bytes[i] != b'>' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
            continue;
        }

        // Closing tag </name>
        if bytes[i] == b'/' {
            i += 1;
            let name_start = i;
            while i < bytes.len() && is_name_char(bytes[i]) {
                i += 1;
            }
            let name_end = i;
            let name = &text[name_start..name_end];
            while i < bytes.len() && bytes[i] != b'>' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }

            match stack.pop() {
                Some((open_name, _, _)) if open_name == name => {}
                Some((open_name, _open_ns, _open_ne)) => {
                    return vec![diag_at(
                        text,
                        name_start..name_end,
                        format!(
                            "Unexpected closing tag </{name}>: the open element is <{open_name}>"
                        ),
                    )];
                }
                None => {
                    return vec![diag_at(
                        text,
                        name_start..name_end,
                        format!("Closing tag </{name}> has no matching opening tag"),
                    )];
                }
            }
            continue;
        }

        // Opening tag <name …>  or  <name … />
        let name_start = i;
        while i < bytes.len() && is_name_char(bytes[i]) {
            i += 1;
        }
        let name_end = i;
        let name = &text[name_start..name_end];
        if name.is_empty() {
            continue;
        }

        // Walk to '>' or '/>' via the canonical scanner.
        let self_closing = match scan_to_tag_close(bytes, i) {
            TagCloseResult::Open { end } => { i = end; false }
            TagCloseResult::SelfClosing { end } => { i = end; true }
            TagCloseResult::Unterminated => { i = bytes.len(); false }
        };

        if !self_closing {
            stack.push((name, name_start, name_end));
        }
    }

    // Anything left on the stack is an unclosed tag.
    if let Some(&(open_name, open_ns, open_ne)) = stack.last() {
        return vec![diag_at(
            text,
            open_ns..open_ne,
            format!("Tag <{open_name}> is never closed"),
        )];
    }

    Vec::new()
}

fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'-' | b'.')
}

fn diag_at(text: &str, range: std::ops::Range<usize>, msg: String) -> Diagnostic {
    Diagnostic {
        range: byte_range_to_lsp(text, range),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some(DIAGNOSTIC_SOURCE.into()),
        message: msg,
        ..Diagnostic::default()
    }
}

/// Extract a (row, col) position from a roxmltree error message.
/// Handles both `ROW:COL message` (older format) and
/// `message at ROW:COL` (current 0.20 format). Returns 0-indexed (line, char);
/// falls back to (0, 0) when no position can be parsed.
fn parse_xml_error_pos(msg: &str) -> (u32, u32) {
    if let Some(at) = msg.rfind(" at ") {
        if let Some(pos) = parse_row_col(msg[at + 4..].trim()) {
            return pos;
        }
    }
    parse_row_col(msg).unwrap_or((0, 0))
}

fn parse_row_col(s: &str) -> Option<(u32, u32)> {
    let mut parts = s.splitn(2, ':');
    let row: u32 = parts.next()?.trim().parse().ok()?;
    let rest = parts.next()?.trim_start();
    let col_digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let col: u32 = col_digits.parse().ok()?;
    Some((row.saturating_sub(1), col.saturating_sub(1)))
}

/// Get the LSP Range for the value of a named attribute on the given node.
/// Searches for the attribute value inside the element's source span.
/// Falls back to the node's own range if the value can't be located.
pub(crate) fn attr_value_range(text: &str, node: &roxmltree::Node, attr_name: &str) -> Range {
    let value = match node.attribute(attr_name) {
        Some(v) => v,
        None => {
            let span = node.range();
            return byte_range_to_lsp(text, span);
        }
    };

    // The element's byte range in the source.
    let elem_range = node.range();
    let elem_src = &text[elem_range.clone()];

    // Restrict the search to the opening-tag header so a child element that
    // happens to share the same `name="value"` pair doesn't get its span
    // returned instead. The header ends at the first unquoted `>` or `/>`.
    let header_end = find_opening_tag_end(elem_src);
    let header = &elem_src[..header_end];

    // Look for attr_name="value" or attr_name='value' inside the header only.
    let needle_dq = format!("{}=\"{}\"", attr_name, value);
    let needle_sq = format!("{}='{}'", attr_name, value);

    let offset = header
        .find(needle_dq.as_str())
        .map(|pos| pos + attr_name.len() + 2) // skip name="
        .or_else(|| {
            header
                .find(needle_sq.as_str())
                .map(|pos| pos + attr_name.len() + 2) // skip name='
        });

    if let Some(rel_start) = offset {
        let start = elem_range.start + rel_start;
        let end = start + value.len();
        byte_range_to_lsp(text, start..end)
    } else {
        // Fallback: use the element's span
        byte_range_to_lsp(text, elem_range)
    }
}

/// Return the byte offset just past the first unquoted `>` or `/>` in `elem_src`
/// (i.e. the end of the opening-tag header). Falls back to `elem_src.len()` if
/// neither is found. Derived from [`scan_to_tag_close`] — the canonical
/// quote-aware tag-end scanner.
fn find_opening_tag_end(elem_src: &str) -> usize {
    match scan_to_tag_close(elem_src.as_bytes(), 0) {
        TagCloseResult::Open { end } | TagCloseResult::SelfClosing { end } => end,
        TagCloseResult::Unterminated => elem_src.len(),
    }
}

/// Round `offset` down to the nearest UTF-8 char boundary (clamped to `text.len()`).
/// Used to guard against panics on slicing operations driven by externally-supplied
/// offsets (e.g. positions from the LSP client).
pub(crate) fn floor_char_boundary(text: &str, offset: usize) -> usize {
    let mut i = offset.min(text.len());
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Convert a UTF-8 byte offset to an LSP `Position` (line + byte-offset within the line).
/// Assumes UTF-8 position encoding was negotiated in `initialize`; for clients that
/// stayed on UTF-16, the `character` value will be off for non-ASCII content but
/// will never panic.
pub(crate) fn byte_offset_to_position(text: &str, offset: usize) -> Position {
    let safe = floor_char_boundary(text, offset);
    let before = &text[..safe];
    let line = before.bytes().filter(|&b| b == b'\n').count() as u32;
    let last_newline = before.rfind('\n').map(|p| p + 1).unwrap_or(0);
    let character = (safe - last_newline) as u32;
    Position { line, character }
}

/// Convert an LSP `Position` to a UTF-8 byte offset.
/// Handles both `\n` and `\r\n` line endings (the `character` field never crosses a `\n`).
/// Floors to a char boundary so the returned offset is always safe to slice on.
pub(crate) fn position_to_byte_offset(text: &str, pos: Position) -> usize {
    let bytes = text.as_bytes();
    let mut idx = 0usize;
    let mut cur_line = 0u32;
    while cur_line < pos.line && idx < bytes.len() {
        if bytes[idx] == b'\n' {
            cur_line += 1;
        }
        idx += 1;
    }
    let mut col = 0u32;
    while idx < bytes.len() && bytes[idx] != b'\n' && col < pos.character {
        idx += 1;
        col += 1;
    }
    floor_char_boundary(text, idx)
}

pub(crate) fn byte_range_to_lsp(text: &str, range: std::ops::Range<usize>) -> Range {
    Range {
        start: byte_offset_to_position(text, range.start),
        end: byte_offset_to_position(text, range.end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_offset_to_position_clamps_past_end() {
        let text = "ab\ncd";
        let pos = byte_offset_to_position(text, 9999);
        assert_eq!(pos, Position { line: 1, character: 2 });
    }

    #[test]
    fn byte_offset_to_position_floors_to_char_boundary() {
        // "héllo" — "é" is two bytes (0xC3 0xA9). Offset 2 lands mid-codepoint.
        let text = "héllo";
        // Asking for offset 2 (mid-é) must not panic and must floor to 1.
        let pos = byte_offset_to_position(text, 2);
        assert_eq!(pos, Position { line: 0, character: 1 });
    }

    #[test]
    fn position_to_byte_offset_handles_crlf() {
        let text = "a\r\nb\r\nc";
        // line 2, char 0 → byte offset of 'c'
        let off = position_to_byte_offset(text, Position { line: 2, character: 0 });
        assert_eq!(&text[off..], "c");
    }

    #[test]
    fn position_to_byte_offset_clamps_at_line_end() {
        let text = "ab\ncd";
        // Asking for character past EOL must clamp at \n, not cross into the next line.
        let off = position_to_byte_offset(text, Position { line: 0, character: 99 });
        assert_eq!(off, 2); // position of the \n
    }

    #[test]
    fn position_to_byte_offset_floors_char_boundary_on_nonascii() {
        // Even if a buggy client claims character=1 of "é" (which would be mid-codepoint
        // in UTF-8), we must not panic and must return a safe byte offset.
        let text = "é";
        let off = position_to_byte_offset(text, Position { line: 0, character: 1 });
        assert!(text.is_char_boundary(off));
    }

    #[test]
    fn scan_to_tag_close_open_tag() {
        let src = b"<gazebo reference=\"x\">rest";
        let TagCloseResult::Open { end } = scan_to_tag_close(src, b"<gazebo".len()) else {
            panic!("expected Open");
        };
        assert_eq!(&src[..end], b"<gazebo reference=\"x\">");
    }

    #[test]
    fn scan_to_tag_close_self_closing() {
        let src = b"<gazebo reference=\"x\"/>rest";
        let TagCloseResult::SelfClosing { end } = scan_to_tag_close(src, b"<gazebo".len()) else {
            panic!("expected SelfClosing");
        };
        assert_eq!(&src[..end], b"<gazebo reference=\"x\"/>");
    }

    #[test]
    fn scan_to_tag_close_quoted_gt_inside_attr_does_not_terminate() {
        // `>` inside a quoted attribute value must not be treated as the tag end.
        let src = b"<gazebo reference=\"a>b\">rest";
        let TagCloseResult::Open { end } = scan_to_tag_close(src, b"<gazebo".len()) else {
            panic!("expected Open");
        };
        assert_eq!(&src[..end], b"<gazebo reference=\"a>b\">");
    }

    #[test]
    fn scan_to_tag_close_quoted_slash_gt_does_not_self_close() {
        // `/>` inside a quoted attribute value must not be treated as self-closing.
        let src = b"<gazebo reference=\"a/>b\">rest";
        let TagCloseResult::Open { end } = scan_to_tag_close(src, b"<gazebo".len()) else {
            panic!("expected Open");
        };
        assert_eq!(&src[..end], b"<gazebo reference=\"a/>b\">");
    }

    #[test]
    fn scan_to_tag_close_unterminated_when_no_close() {
        let src = b"<gazebo reference=\"x\"  ";
        assert!(matches!(
            scan_to_tag_close(src, b"<gazebo".len()),
            TagCloseResult::Unterminated,
        ));
    }
}
