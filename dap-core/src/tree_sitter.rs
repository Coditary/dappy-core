//! Tree-sitter core integration. Language grammars are loaded from
//! `default_tree_sitter_dir()` (or `DAP_TREE_SITTER_DIR`) as shared libraries;
//! highlight queries live beside them under `<lang>/queries/`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use libloading::{Library, Symbol};
use tree_sitter::{Language, Parser, Tree};
use tree_sitter_highlight::{
    Highlight, HighlightConfiguration, HighlightEvent, Highlighter,
};

use crate::terminal_style::TerminalStyle;

type LanguageFn = unsafe extern "C" fn() -> *const ();

const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "boolean",
    "carriage-return",
    "comment",
    "constant",
    "constructor",
    "embedded",
    "emphasis",
    "emphasis.strong",
    "error",
    "escape",
    "function",
    "function.builtin",
    "function.macro",
    "function.method",
    "include",
    "keyword",
    "keyword.function",
    "keyword.macro",
    "keyword.operator",
    "keyword.return",
    "keyword.type",
    "module",
    "number",
    "operator",
    "property",
    "property.builtin",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string",
    "string.escape",
    "string.regex",
    "string.special",
    "tag",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

/// Default directory for tree-sitter grammar shared libraries and queries.
pub fn default_tree_sitter_dir() -> PathBuf {
    std::env::var("DAP_TREE_SITTER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|_| {
                    std::env::var("HOME").map(|home| PathBuf::from(home).join(".local/share"))
                })
                .unwrap_or_else(|_| PathBuf::from("/tmp"))
                .join("dap/tree-sitter")
        })
}

/// Rough resident memory for diagnostics (`/proc/self/status` VmRSS when available).
pub fn resident_memory_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("VmRSS:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse().ok())
        })
}

struct LoadedLanguage {
    _library: Option<Library>,
    language: Language,
    highlights_query: String,
    injections_query: String,
    locals_query: String,
}

struct LanguageRuntime {
    configuration: HighlightConfiguration,
}

struct TreeSitterState {
    languages: HashMap<String, LoadedLanguage>,
    runtimes: HashMap<String, LanguageRuntime>,
}

static STATE: OnceLock<Mutex<TreeSitterState>> = OnceLock::new();

fn state() -> &'static Mutex<TreeSitterState> {
    STATE.get_or_init(|| {
        Mutex::new(TreeSitterState {
            languages: HashMap::new(),
            runtimes: HashMap::new(),
        })
    })
}

/// Register a language from an external tree-sitter grammar directory.
pub fn register_language_from_dir(language: &str, dir: &Path) -> Result<()> {
    let loaded = load_language_from_dir(language, dir)?;
    let mut guard = state().lock().expect("tree-sitter state lock");
    guard.languages.insert(language.to_string(), loaded);
    guard.runtimes.remove(language);
    Ok(())
}

/// Register a statically linked tree-sitter language (tests / dev tooling).
pub fn register_language_static(
    language: &str,
    grammar: Language,
    highlights_query: &str,
    injections_query: &str,
    locals_query: &str,
) -> Result<()> {
    let mut guard = state().lock().expect("tree-sitter state lock");
    guard.languages.insert(
        language.to_string(),
        LoadedLanguage {
            _library: None,
            language: grammar,
            highlights_query: highlights_query.to_string(),
            injections_query: injections_query.to_string(),
            locals_query: locals_query.to_string(),
        },
    );
    guard.runtimes.remove(language);
    Ok(())
}

fn load_language_from_dir(language: &str, dir: &Path) -> Result<LoadedLanguage> {
    let parser_lib = find_parser_library(dir, language)?;
    let library = unsafe { Library::new(&parser_lib) }
        .with_context(|| format!("load tree-sitter grammar {}", parser_lib.display()))?;
    let symbol_name = format!("tree_sitter_{}", language.replace('-', "_"));
    let language_fn: Symbol<LanguageFn> = unsafe { library.get(symbol_name.as_bytes()) }
        .with_context(|| format!("symbol {symbol_name} in {}", parser_lib.display()))?;
    let raw = unsafe { language_fn() };
    let grammar = unsafe { Language::from_raw(raw as *const _) };

    let queries_dir = dir.join("queries");
    let highlights_query = read_query_file(&queries_dir, "highlights.scm")?;
    let injections_query = read_query_file(&queries_dir, "injections.scm").unwrap_or_default();
    let locals_query = read_query_file(&queries_dir, "locals.scm").unwrap_or_default();

    Ok(LoadedLanguage {
        _library: Some(library),
        language: grammar,
        highlights_query,
        injections_query,
        locals_query,
    })
}

fn find_parser_library(dir: &Path, language: &str) -> Result<PathBuf> {
    let candidates = [
        dir.join(format!("libtree-sitter-{language}.so")),
        dir.join(format!("tree-sitter-{language}.so")),
        dir.join("parser.so"),
        dir.join(format!("libtree-sitter-{language}.dylib")),
        dir.join(format!("tree-sitter-{language}.dylib")),
        dir.join("parser.dylib"),
    ];
    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    anyhow::bail!(
        "no tree-sitter parser library found in {} for language {}",
        dir.display(),
        language
    )
}

fn read_query_file(dir: &Path, name: &str) -> Result<String> {
    let path = dir.join(name);
    std::fs::read_to_string(&path)
        .with_context(|| format!("read tree-sitter query {}", path.display()))
}

fn ensure_runtime(language: &str) -> Result<()> {
    let mut guard = state().lock().expect("tree-sitter state lock");
    if guard.runtimes.contains_key(language) {
        return Ok(());
    }
    let loaded = guard
        .languages
        .get(language)
        .context("tree-sitter language not registered")?;
    let mut configuration = HighlightConfiguration::new(
        loaded.language.clone(),
        language,
        &loaded.highlights_query,
        &loaded.injections_query,
        &loaded.locals_query,
    )?;
    configuration.configure(HIGHLIGHT_NAMES);
    guard
        .runtimes
        .insert(language.to_string(), LanguageRuntime { configuration });
    Ok(())
}

fn language_dir(language: &str) -> PathBuf {
    default_tree_sitter_dir().join(language)
}

pub fn language_is_loaded(language: &str) -> bool {
    state()
        .lock()
        .expect("tree-sitter state lock")
        .languages
        .contains_key(language)
}

/// Hint for installing an external tree-sitter grammar.
pub fn language_setup_hint(language: &str, reason: &str) -> String {
    format!(
        "tree-sitter grammar for '{language}' unavailable ({reason}).\n  \
         Install under {root}/{language}/:\n    \
         libtree-sitter-{language}.so (or parser.so)\n    \
         queries/highlights.scm\n  \
         Or set DAP_TREE_SITTER_DIR to another root. Basic syntax highlighting will be used.",
        root = default_tree_sitter_dir().display(),
    )
}

/// Load a grammar from the default directory; returns a user-facing message on failure.
pub fn try_load_language(language: &str) -> Result<(), String> {
    if language_is_loaded(language) {
        return Ok(());
    }
    let dir = language_dir(language);
    if !dir.is_dir() {
        return Err(language_setup_hint(language, "directory not found"));
    }
    register_language_from_dir(language, &dir)
        .map_err(|err| language_setup_hint(language, &err.to_string()))?;
    ensure_runtime(language).map_err(|err| language_setup_hint(language, &err.to_string()))?;
    Ok(())
}

/// Languages to probe when the user enables syntax colors in the REPL.
pub fn warm_languages_for_colors() -> Vec<String> {
    ["python", "rust"]
        .iter()
        .filter_map(|language| try_load_language(language).err())
        .collect()
}

fn try_load_from_default_dir(language: &str) -> bool {
    try_load_language(language).is_ok()
}

/// Highlight a source snippet when a grammar is available; otherwise `None`.
pub fn highlight_snippet(language: &str, source: &str, style: &TerminalStyle) -> Option<String> {
    if !style.enabled {
        return None;
    }
    if source.is_empty() {
        return Some(String::new());
    }

    {
        let guard = state().lock().expect("tree-sitter state lock");
        if !guard.languages.contains_key(language) {
            drop(guard);
            if !try_load_from_default_dir(language) {
                return None;
            }
        }
    }

    ensure_runtime(language).ok()?;

    let guard = state().lock().expect("tree-sitter state lock");
    let runtime = guard.runtimes.get(language)?;
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(&runtime.configuration, source.as_bytes(), None, |_| None)
        .ok()?;

    Some(render_highlight_events(source, events, style))
}

/// Parse-only helper for memory measurements.
pub fn parse_snippet(language: &str, source: &str) -> Result<Tree> {
    let guard = state().lock().expect("tree-sitter state lock");
    let loaded = guard
        .languages
        .get(language)
        .context("tree-sitter language not registered")?;
    let mut parser = Parser::new();
    parser.set_language(&loaded.language)?;
    parser
        .parse(source, None)
        .context("parse source snippet")
}

fn render_highlight_events(
    source: &str,
    events: impl Iterator<Item = Result<HighlightEvent, tree_sitter_highlight::Error>>,
    style: &TerminalStyle,
) -> String {
    let mut out = String::new();
    let mut active: Option<Highlight> = None;
    for event in events {
        match event {
            Ok(HighlightEvent::Source { start, end }) => {
                let chunk = &source[start..end];
                if let Some(highlight) = active {
                    out.push_str(&style_for_highlight(highlight, chunk, style));
                } else {
                    out.push_str(chunk);
                }
            }
            Ok(HighlightEvent::HighlightStart(highlight)) => active = Some(highlight),
            Ok(HighlightEvent::HighlightEnd) => active = None,
            Err(_) => {}
        }
    }
    out
}

fn style_for_highlight(highlight: Highlight, text: &str, style: &TerminalStyle) -> String {
    let name = HIGHLIGHT_NAMES
        .get(highlight.0 as usize)
        .copied()
        .unwrap_or("variable");
    match name {
        n if n.starts_with("keyword") => style.blue(text),
        n if n.starts_with("string") => style.yellow(text),
        "comment" => style.dim(text),
        n if n.starts_with("function") => style.cyan(text),
        "number" | "boolean" | "constant" => style.cyan(text),
        "type" | "type.builtin" => style.green(text),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register_python_from_dev_dep() {
        let _ = register_language_static(
            "python",
            tree_sitter_python::LANGUAGE.into(),
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        );
    }

    #[test]
    fn highlights_python_keywords_with_tree_sitter() {
        register_python_from_dev_dep();
        let style = TerminalStyle { enabled: true };
        let highlighted = highlight_snippet(
            "python",
            "if True:\n    return 1\n",
            &style,
        )
        .expect("highlight");
        assert!(highlighted.contains("if"));
        assert!(highlighted.contains("return"));
    }

    #[test]
    fn parse_snippet_footprint_is_small() {
        register_python_from_dev_dep();
        let source = "if True:\n    return 1\n".repeat(100);
        let before = resident_memory_kib();
        let tree = parse_snippet("python", &source).expect("parse");
        let _bytes = tree.root_node().end_byte();
        let during = resident_memory_kib();
        drop(tree);
        if let (Some(before), Some(during)) = (before, during) {
            let delta = during.saturating_sub(before);
            assert!(
                delta < 8192,
                "expected <8 MiB RSS delta for ~300 lines, got {delta} KiB"
            );
        }
    }
}
