use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

mod document;
mod diagnostics;
mod features;
mod workspace;
mod xacro_eval;

#[cfg(test)]
mod stress;

use workspace::{FileSummary, WorkspaceIndex};

// ── Workspace file scanner ───────────────────────────────────────────────────

fn collect_urdf_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return; };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_urdf_files(&p, out);
        } else if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if name.ends_with(".urdf") || name.ends_with(".xacro") {
                out.push(p);
            }
        }
    }
}

/// Synchronously scan workspace roots, parse every URDF/xacro file found,
/// and return their entity summaries. Intended to run inside `spawn_blocking`.
fn scan_workspace_sync(roots: Vec<std::path::PathBuf>) -> Vec<(Url, FileSummary)> {
    let mut files = Vec::new();
    for root in &roots {
        collect_urdf_files(root, &mut files);
    }
    files
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let (doc, _) = document::parse(&text);
            let uri = Url::from_file_path(&path).ok()?;
            Some((uri, FileSummary::from_doc(&doc)))
        })
        .collect()
}

// ── LSP backend ──────────────────────────────────────────────────────────────

#[derive(Debug)]
struct Backend {
    client: Client,
    docs: Arc<Mutex<HashMap<Url, (document::Document, String)>>>,
    workspace_roots: Arc<Mutex<Vec<std::path::PathBuf>>>,
    workspace_index: Arc<tokio::sync::RwLock<WorkspaceIndex>>,
    /// True iff the client advertised UTF-8 in `initialize` and we negotiated
    /// it. When false, the LSP default of UTF-16 is in effect but the rest of
    /// the codebase treats `Position.character` as a UTF-8 byte offset — see
    /// the warning emitted from `initialized()`.
    utf8_negotiated: Arc<std::sync::atomic::AtomicBool>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Prefer UTF-8 position encoding so `Position.character` is a byte offset
        // matching our internal UTF-8 text storage. Fall back to UTF-16 (the LSP
        // default) if the client doesn't advertise UTF-8 support — but record
        // the fact so `initialized()` can warn the user that non-ASCII files
        // may resolve to wrong cursor positions (we don't convert UTF-16 → UTF-8).
        let position_encoding = params
            .capabilities
            .general
            .as_ref()
            .and_then(|g| g.position_encodings.as_ref())
            .and_then(|encs| encs.iter().find(|e| **e == PositionEncodingKind::UTF8).cloned());
        self.utf8_negotiated.store(
            position_encoding.is_some(),
            std::sync::atomic::Ordering::SeqCst,
        );

        // Save workspace roots for the initial file scan in `initialized()`.
        {
            let mut roots = self.workspace_roots.lock().await;
            if let Some(folders) = params.workspace_folders.as_deref() {
                for f in folders {
                    if let Ok(p) = f.uri.to_file_path() {
                        roots.push(p);
                    }
                }
            } else if let Some(uri) = params.root_uri.as_ref() {
                if let Ok(p) = uri.to_file_path() {
                    roots.push(p);
                }
            }
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                position_encoding,
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        "$".to_string(),
                        "{".to_string(),
                        "\"".to_string(),
                        "<".to_string(),
                    ]),
                    ..CompletionOptions::default()
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Left(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                color_provider: Some(ColorProviderCapability::Simple(true)),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: "urdf-lsp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "urdf-lsp initialized")
            .await;

        if !self.utf8_negotiated.load(std::sync::atomic::Ordering::SeqCst) {
            self.client.log_message(
                MessageType::WARNING,
                "urdf-lsp: client did not advertise UTF-8 position encoding; \
                 the server is using UTF-16 but treats Position.character as a \
                 UTF-8 byte offset internally. Files containing non-ASCII \
                 characters may produce wrong cursor positions for hover / \
                 goto-definition. ASCII-only files are unaffected.",
            ).await;
        }

        let roots = self.workspace_roots.lock().await.clone();
        if roots.is_empty() {
            return;
        }

        // Spawn a background task: scan workspace files, populate the index,
        // then re-publish diagnostics for any already-open files so cross-file
        // refs resolve immediately on startup.
        let ws_idx   = self.workspace_index.clone();
        let docs_map = self.docs.clone();
        let client   = self.client.clone();

        tokio::spawn(async move {
            let Ok(entries) = tokio::task::spawn_blocking(move || {
                scan_workspace_sync(roots)
            }).await else { return; };

            // Chunk the upsert so the write lock yields back to interleaved
            // `validate` calls (active editing) every N files — otherwise a
            // cold-start scan of a huge workspace stalls every keystroke until
            // the entire index is populated.
            const UPSERT_CHUNK: usize = 32;
            let mut iter = entries.into_iter();
            loop {
                let chunk: Vec<_> = iter.by_ref().take(UPSERT_CHUNK).collect();
                if chunk.is_empty() { break; }
                let mut idx = ws_idx.write().await;
                for (uri, summary) in chunk {
                    idx.upsert(uri, summary);
                }
            }

            // Re-run diagnostics for every file that was opened before the scan
            // finished, so cross-file false positives are cleared immediately.
            let open: Vec<(Url, String)> = {
                let map = docs_map.lock().await;
                map.iter().map(|(u, (_, t))| (u.clone(), t.clone())).collect()
            };

            for (uri, text) in open {
                let (doc, mut diags) = document::parse(&text);
                if doc.parse_ok {
                    let idx = ws_idx.read().await;
                    diags.extend(diagnostics::check(&doc, &text, Some(&idx)));
                    if let Ok(ref xml) = roxmltree::Document::parse(&text) {
                        diags.extend(diagnostics::check_schema(xml, &text, doc.is_xacro, Some(&idx)));
                    }
                }
                client.publish_diagnostics(uri, diags, None).await;
            }
        });
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.validate(params.text_document.uri, params.text_document.text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.validate(
            params.text_document.uri,
            params.content_changes.into_iter().last().map(|c| c.text).unwrap_or_default(),
        )
        .await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let pos = params.text_document_position_params.position;
        let uri = params.text_document_position_params.text_document.uri;

        // Phase 1: same-file hover; also capture the ref name for phase 2.
        let (result, ref_info) = {
            let map = self.docs.lock().await;
            match map.get(&uri) {
                None => (None, None),
                Some((doc, _)) => {
                    let h = features::hover(doc, pos);
                    let ri = if h.is_none() { features::entity_at(doc, pos) } else { None };
                    (h, ri)
                }
            }
        };
        if result.is_some() { return Ok(result); }

        // Phase 2: cross-file — look up the referenced entity in the workspace index.
        if let Some((name, kind)) = ref_info {
            let idx = self.workspace_index.read().await;
            let defs = match kind {
                features::ItemKind::Link  => idx.link_defs(&name),
                features::ItemKind::Joint => idx.joint_defs(&name),
            };
            if let Some(hover) = Self::cross_file_hover(&name, kind, defs) {
                return Ok(Some(hover));
            }
        }
        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let pos = params.text_document_position_params.position;
        let uri = params.text_document_position_params.text_document.uri.clone();

        // Phase 1: same-file goto-def; also capture ref name for phase 2.
        let (result, ref_info) = {
            let map = self.docs.lock().await;
            match map.get(&uri) {
                None => (None, None),
                Some((doc, _)) => {
                    let r = features::goto_definition(doc, pos).map(|range| {
                        GotoDefinitionResponse::Scalar(Location { uri: uri.clone(), range })
                    });
                    let ri = if r.is_none() { features::entity_at(doc, pos) } else { None };
                    (r, ri)
                }
            }
        };
        if result.is_some() { return Ok(result); }

        // Phase 2: cross-file — return every definition across the workspace.
        // Multiple files defining the same name → Array response so the editor
        // can present the user with a picker rather than guessing.
        if let Some((name, kind)) = ref_info {
            let idx = self.workspace_index.read().await;
            let defs = match kind {
                features::ItemKind::Link  => idx.link_defs(&name),
                features::ItemKind::Joint => idx.joint_defs(&name),
            };
            if !defs.is_empty() {
                let locs: Vec<Location> = defs.iter()
                    .map(|(u, r)| Location { uri: u.clone(), range: *r })
                    .collect();
                return Ok(Some(GotoDefinitionResponse::Array(locs)));
            }
        }
        Ok(None)
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> Result<Option<CompletionResponse>> {
        let pos = params.text_document_position.position;
        let uri = params.text_document_position.text_document.uri;
        let map = self.docs.lock().await;
        Ok(map.get(&uri).and_then(|(doc, text)| {
            let items = features::completion(doc, pos, text);
            if items.is_empty() { None } else { Some(CompletionResponse::Array(items)) }
        }))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let map = self.docs.lock().await;
        Ok(map.get(&uri).and_then(|(doc, _)| {
            let symbols = features::document_symbols(doc);
            if symbols.is_empty() { None } else { Some(DocumentSymbolResponse::Nested(symbols)) }
        }))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let pos = params.text_document_position.position;
        let uri = params.text_document_position.text_document.uri;
        let map = self.docs.lock().await;
        Ok(map.get(&uri).map(|(doc, _)| {
            features::references(doc, pos)
                .into_iter()
                .map(|range| Location { uri: uri.clone(), range })
                .collect()
        }))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let pos = params.text_document_position.position;
        let new_name = params.new_name.clone();
        let uri = params.text_document_position.text_document.uri.clone();
        let map = self.docs.lock().await;
        Ok(map.get(&uri).and_then(|(doc, _)| {
            let changes = features::rename(doc, pos, &new_name);
            if changes.is_empty() { return None; }
            let edits = changes.into_iter()
                .map(|(range, new_text)| TextEdit { range, new_text })
                .collect::<Vec<_>>();
            let mut map = std::collections::HashMap::new();
            map.insert(uri.clone(), edits);
            Some(WorkspaceEdit { changes: Some(map), ..WorkspaceEdit::default() })
        }))
    }

    async fn prepare_rename(&self, params: TextDocumentPositionParams) -> Result<Option<PrepareRenameResponse>> {
        let pos = params.position;
        let uri = params.text_document.uri;
        let map = self.docs.lock().await;
        Ok(map.get(&uri).and_then(|(doc, _)| {
            features::prepare_rename(doc, pos).map(|(range, placeholder)| {
                PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }
            })
        }))
    }

    async fn document_color(&self, params: DocumentColorParams) -> Result<Vec<ColorInformation>> {
        let uri = params.text_document.uri;
        let map = self.docs.lock().await;
        Ok(map.get(&uri).map(|(_, text)| features::document_colors(text)).unwrap_or_default())
    }

    async fn color_presentation(&self, params: ColorPresentationParams) -> Result<Vec<ColorPresentation>> {
        Ok(features::color_presentations(params.color))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let uri = params.text_document.uri;
        let map = self.docs.lock().await;
        Ok(map.get(&uri).map(|(_, text)| features::folding_ranges(text)))
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let diags = params.context.diagnostics;
        let map = self.docs.lock().await;
        Ok(map.get(&uri).map(|(doc, text)| {
            features::code_actions(doc, text, &diags, &uri)
        }))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        let range = params.range;
        let map = self.docs.lock().await;
        Ok(map.get(&uri).map(|(doc, text)| features::inlay_hints(doc, text, range)))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

impl Backend {
    /// Build a cross-file hover for an entity defined in one or more other
    /// workspace files. Returns `None` if `defs` is empty.
    ///
    /// Multi-definition case: shows the first file and a `(+N more)` suffix —
    /// the editor's goto-definition picker is how the user navigates between
    /// them, so we don't try to list them all in the hover popup.
    fn cross_file_hover(
        name: &str,
        kind: features::ItemKind,
        defs: &[(Url, Range)],
    ) -> Option<Hover> {
        let ((def_uri, _), rest) = defs.split_first()?;
        let file = file_name_for_display(def_uri);
        let kind_word = match kind {
            features::ItemKind::Link  => "Link",
            features::ItemKind::Joint => "Joint",
        };
        let suffix = if rest.is_empty() {
            String::new()
        } else {
            format!(" (+{} more)", rest.len())
        };
        let label = format!("**{kind_word}** `{name}` — defined in `{file}`{suffix}");
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: label,
            }),
            range: None,
        })
    }

    async fn validate(&self, uri: Url, text: String) {
        let (doc, mut diags) = document::parse(&text);

        // Build the summary BEFORE taking the write lock — `from_doc` clones
        // every name string, and we don't want that work blocking other writers
        // (or readers, since RwLock writers exclude readers).
        let summary = FileSummary::from_doc(&doc);
        {
            let mut idx = self.workspace_index.write().await;
            idx.upsert(uri.clone(), summary);
        }

        // Run diagnostics with the live workspace index held by read lock —
        // no clones, no snapshot allocation per keystroke.
        if doc.parse_ok {
            let idx = self.workspace_index.read().await;
            diags.extend(diagnostics::check(&doc, &text, Some(&idx)));
            if let Ok(ref xml) = roxmltree::Document::parse(&text) {
                diags.extend(diagnostics::check_schema(xml, &text, doc.is_xacro, Some(&idx)));
            }
        }

        {
            let mut map = self.docs.lock().await;
            map.insert(uri.clone(), (doc, text));
        }
        self.client.publish_diagnostics(uri, diags, None).await;
    }
}

/// Final path segment of a file:// URL for display in hover tooltips.
/// Falls back to the full URL as last resort — non-file URLs shouldn't reach
/// the workspace index, but if they do we'd rather show *something* identifying
/// than a generic "another file" placeholder.
fn file_name_for_display(uri: &Url) -> String {
    if let Ok(path) = uri.to_file_path() {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            return name.to_string();
        }
    }
    uri.path_segments()
        .and_then(|mut s| s.next_back().filter(|seg| !seg.is_empty()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| uri.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end test of the cross-file hover/goto-def composition WITHOUT
    /// constructing a `Backend` (which needs a tower-lsp `Client`). The handlers
    /// in `Backend::hover` / `Backend::goto_definition` are pure glue around
    /// these four steps — if each step works individually and they wire up
    /// here, the handlers are correct too.
    #[test]
    fn cross_file_hover_chain_resolves_to_other_file() {
        // File A defines base_link; file B references it via <parent link="base_link"/>.
        let text_a = "<robot name=\"a\">\n  <link name=\"base_link\"/>\n</robot>";
        let text_b = "<robot name=\"b\">\n  <joint name=\"j\" type=\"fixed\">\n    <parent link=\"base_link\"/>\n    <child link=\"x\"/>\n  </joint>\n  <link name=\"x\"/>\n</robot>";

        let (doc_a, _) = document::parse(text_a);
        let (doc_b, _) = document::parse(text_b);
        let uri_a = Url::parse("file:///a.urdf").unwrap();
        let uri_b = Url::parse("file:///b.urdf").unwrap();

        let mut idx = WorkspaceIndex::default();
        idx.upsert(uri_a.clone(), FileSummary::from_doc(&doc_a));
        idx.upsert(uri_b.clone(), FileSummary::from_doc(&doc_b));

        // Step 1: same-file hover misses for the cross-file ref.
        let pos = Position::new(2, 18); // mid-`base_link` in <parent link="base_link"/>
        assert!(features::hover(&doc_b, pos).is_none(), "same-file hover must miss for cross-file ref");

        // Step 2: entity_at surfaces the referenced name and kind.
        let (name, kind) = features::entity_at(&doc_b, pos)
            .expect("entity_at must return the ref name");
        assert_eq!(name, "base_link");
        assert_eq!(kind, features::ItemKind::Link);

        // Step 3: workspace index resolves the name to file A.
        let defs = idx.link_defs(&name);
        assert_eq!(defs.len(), 1, "exactly one definition expected, got {defs:?}");
        assert_eq!(defs[0].0, uri_a);

        // Step 4: cross_file_hover builds the right tooltip.
        let hover = Backend::cross_file_hover(&name, kind, defs).expect("hover should be Some");
        match hover.contents {
            HoverContents::Markup(m) => {
                assert!(m.value.contains("**Link**"), "got: {}", m.value);
                assert!(m.value.contains("base_link"), "got: {}", m.value);
                assert!(m.value.contains("a.urdf"), "filename missing: {}", m.value);
                assert!(!m.value.contains("more"), "should not show '+N more' for single def");
            }
            _ => panic!("expected Markup hover content"),
        }
    }

    #[test]
    fn cross_file_hover_shows_multi_def_suffix_when_collision() {
        // Two files both define `base_link` — hover should mention "+1 more".
        let text = "<robot name=\"r\">\n  <link name=\"base_link\"/>\n</robot>";
        let (doc, _) = document::parse(text);
        let uri_a = Url::parse("file:///a.urdf").unwrap();
        let uri_b = Url::parse("file:///b.urdf").unwrap();
        let mut idx = WorkspaceIndex::default();
        idx.upsert(uri_a, FileSummary::from_doc(&doc));
        idx.upsert(uri_b, FileSummary::from_doc(&doc));

        let defs = idx.link_defs("base_link");
        assert_eq!(defs.len(), 2);
        let hover = Backend::cross_file_hover("base_link", features::ItemKind::Link, defs)
            .expect("hover should be Some");
        match hover.contents {
            HoverContents::Markup(m) => {
                assert!(m.value.contains("(+1 more)"),
                    "expected multi-def suffix in: {}", m.value);
            }
            _ => panic!("expected Markup hover content"),
        }
    }

    #[test]
    fn cross_file_hover_returns_none_for_empty_defs() {
        // Workspace doesn't know about this name — hover must return None,
        // not panic on empty slice.
        let hover = Backend::cross_file_hover("missing", features::ItemKind::Joint, &[]);
        assert!(hover.is_none());
    }

    #[test]
    fn prop_defs_accessor_returns_xacro_property_locations() {
        // Symmetric coverage of `prop_defs` so cross-file xacro property
        // resolution (future feature) has a tested foundation.
        let text = "<robot xmlns:xacro=\"http://www.ros.org/wiki/xacro\" name=\"r\">\n  <xacro:property name=\"wheel_radius\" value=\"0.05\"/>\n</robot>";
        let (doc, _) = document::parse(text);
        let uri = Url::parse("file:///props.urdf").unwrap();
        let mut idx = WorkspaceIndex::default();
        idx.upsert(uri.clone(), FileSummary::from_doc(&doc));

        let defs = idx.prop_defs("wheel_radius");
        assert_eq!(defs.len(), 1, "expected one prop def, got {defs:?}");
        assert_eq!(defs[0].0, uri);
        assert!(idx.prop_defs("nonexistent").is_empty());
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        docs: Arc::new(Mutex::new(HashMap::new())),
        workspace_roots: Arc::new(Mutex::new(Vec::new())),
        workspace_index: Arc::new(tokio::sync::RwLock::new(WorkspaceIndex::default())),
        utf8_negotiated: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
