use std::borrow::Cow;

use tree_sitter_tags::{TagsContext, TagsConfiguration};

use helix_core::{syntax::RopeProvider, Selection};
use helix_lsp::{lsp, OffsetEncoding, util::lsp_range_to_range};
use helix_view::{Document, DocumentId};

use crate::{ui, job};

use super::{overlaid, compositor, push_jump, align_view, Align, PromptEvent};

pub fn ts_symbol_picker_command(
    cx: &mut compositor::Context,
    _args: &[Cow<str>],
    ev: PromptEvent,
) -> anyhow::Result<()> {
    if ev != PromptEvent::Validate {
        return Ok(());
    }
    ts_symbol_picker(cx);
    Ok(())
}

pub fn ts_symbol_picker(cx: &mut compositor::Context) {
    let current_url = doc!(cx.editor).url().unwrap();
    let mut symbols = Vec::new();
    for doc in cx.editor.documents.values() {
        tags_for_doc(doc, &mut symbols);
    }

    let callback = async move {
        Ok(job::Callback::EditorCompositor(Box::new(
            move |editor, compositor| {
                let picker = ui::FilePicker::new(
                    symbols,
                    None,
                    move |cx, symbol, action| {
                        let (view, doc) = current!(cx.editor);
                        push_jump(view, doc);

                        let location = &symbol.location;
                        if current_url != location.uri {
                            let uri = &location.uri;
                            let path = match uri.to_file_path() {
                                Ok(path) => path,
                                Err(_) => {
                                    let err = format!("unable to convert URI to filepath: {}", uri);
                                    cx.editor.set_error(err);
                                    return;
                                }
                            };
                            if let Err(err) = cx.editor.open(&path, action) {
                                let err = format!("failed to open document: {}: {}", uri, err);
                                log::error!("{}", err);
                                cx.editor.set_error(err);
                                return;
                            }
                        }

                        let line = Some((
                            location.range.start.line as usize,
                            location.range.end.line as usize,
                        ));
                        let (view, doc) = current!(cx.editor);
                        if let Some(range) = lsp_range_to_range(doc.text(), location.range, OffsetEncoding::Utf8) {
                            doc.set_selection(view.id, Selection::single(range.head, range.anchor));
                            align_view(doc, view, Align::Center);
                        }
                    },
                    move |editor, symbol| Some({
                        let location = &symbol.location;
                        let path = location.uri.to_file_path().unwrap();
                        let line = Some((
                            location.range.start.line as usize,
                            location.range.end.line as usize,
                        ));
                        (path.into(), line)
                    }),
                );
                compositor.push(Box::new(overlaid(picker)));
            }
        )))
    };
    cx.jobs.callback(callback);
}

fn tags_for_doc(doc: &Document, symbols: &mut Vec<lsp::SymbolInformation>) {
    let syntax = match doc.syntax() {
        Some(syntax) => syntax,
        None => return,
    };

    let tree = syntax.tree();
    let language = match doc.language_config() {
        Some(language) => language,
        None => return,
    };
    if !language.is_highlight_initialized() {
        return;
    }
    let highlight_config = match language.highlight_config(&[]) {
        Some(config) => config,
        None => return,
    };
    let grammar = &highlight_config.language;
    let tags_query = language.tags_query().expect("tags_query");
    let tags_query = language.tags_query().expect("tags_query");
    let provider = RopeProvider(doc.text().slice(..));

    let config = TagsConfiguration::new(
        grammar.clone(),
        tags_query,
        "",
    ).unwrap();

    let url = doc.url().unwrap();

    let mut context = TagsContext::new();
    for tag in context.generate_tags_from_tree(&config, tree, provider, None) {
        let tag = match tag {
            Ok(tag) if tag.is_definition => tag,
            _ => continue,
        };
        let range = lsp::Range {
            start: lsp::Position::new(tag.span.start.row as u32, tag.span.start.column as u32),
            end: lsp::Position::new(tag.span.end.row as u32, tag.span.end.column as u32),
        };
        symbols.push(lsp::SymbolInformation {
            name: tag.name.clone(),
            kind: lsp::SymbolKind::CLASS,
            tags: None,
            deprecated: None,
            location: lsp::Location::new(url.clone(), range),
            container_name: None,
        });
    }
}
