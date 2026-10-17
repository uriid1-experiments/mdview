//! Markdown → HTML pipeline modelled on GitHub's own stack:
//! comrak (GFM parser) → syntect tokens mapped to GitHub's `pl-*` highlight
//! classes → ammonia (sanitizer, the html-pipeline role).

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::path::Path;

use comrak::adapters::SyntaxHighlighterAdapter;
use comrak::options::Plugins;
use comrak::{markdown_to_html_with_plugins, Options};
use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use syntect::easy::ScopeRegionIterator;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

pub const GH_CSS: &str = include_str!("../assets/github-markdown.css");

pub const MD_EXTENSIONS: &[&str] = &["md", "markdown", "mdown", "mkd", "mkdn", "mdwn", "txt"];

/// TextMate scope prefix → GitHub "prettylights" class (as used by github.com and
/// styled by github-markdown-css). Checked from the innermost scope outwards; the
/// first matching rule wins, so more specific prefixes come first.
const SCOPE_CLASSES: &[(&str, &str)] = &[
    ("comment", "pl-c"),
    ("punctuation.definition.comment", "pl-c"),
    ("string.regexp", "pl-sr"),
    ("punctuation.definition.string", "pl-pds"),
    ("string", "pl-s"),
    ("constant.character.escape", "pl-cce"),
    ("constant", "pl-c1"),
    ("keyword", "pl-k"),
    ("storage", "pl-k"),
    ("entity.name.function", "pl-en"),
    ("entity.name.tag", "pl-ent"),
    ("entity.name.type", "pl-v"),
    ("entity.name.class", "pl-v"),
    ("entity.other.attribute-name", "pl-e"),
    ("entity.other.inherited-class", "pl-e"),
    ("entity.name", "pl-en"),
    ("support", "pl-c1"),
    ("variable.language", "pl-c1"),
    ("variable.function", "pl-en"),
    ("punctuation.definition.variable", "pl-smi"),
    ("variable", "pl-smi"),
    ("markup.heading", "pl-mh"),
    ("markup.bold", "pl-mb"),
    ("markup.italic", "pl-mi"),
    ("markup.inserted", "pl-mi1"),
    ("markup.deleted", "pl-md"),
    ("markup.changed", "pl-mc"),
    ("meta.diff.range", "pl-mdr"),
    ("meta.diff.header", "pl-c1"),
    ("invalid.illegal", "pl-ii"),
];

/// Highlights fenced code into GitHub's `pl-*` spans. Implements comrak's adapter
/// so `<pre lang="…">` keeps its attributes (comrak's bundled syntect adapter drops them).
struct GithubHighlighter {
    syntaxes: SyntaxSet,
    rules: Vec<(Scope, &'static str)>,
    shell: Scope,
    variable_function: Scope,
    json_key: Scope,
}

impl GithubHighlighter {
    fn new() -> Self {
        let scope = |s: &str| Scope::new(s).expect("valid scope");
        Self {
            syntaxes: two_face::syntax::extra_newlines(),
            rules: SCOPE_CLASSES
                .iter()
                .map(|(prefix, class)| (scope(prefix), *class))
                .collect(),
            shell: scope("source.shell"),
            variable_function: scope("variable.function"),
            json_key: scope("meta.mapping.key"),
        }
    }

    fn find_syntax(&self, lang: &str) -> Option<&SyntaxReference> {
        let lang = lang.trim().to_ascii_lowercase();
        let token = match lang.as_str() {
            "" | "text" | "txt" | "plain" | "plaintext" | "none" | "mermaid" | "math" => return None,
            "shell" | "console" | "shell-session" | "zsh" | "sh" => "bash",
            "jsonc" | "json5" => "json",
            "yml" => "yaml",
            "viml" | "vimscript" => "vim",
            "docker" => "dockerfile",
            "cfg" | "conf" | "unit" | "service" | "desktop" => "ini",
            "make" | "mk" => "makefile",
            "patch" => "diff",
            other => other,
        };
        self.syntaxes
            .find_syntax_by_token(token)
            .or_else(|| self.syntaxes.find_syntax_by_extension(token))
    }

    fn class_for(&self, stack: &ScopeStack) -> Option<&'static str> {
        let scopes = stack.as_slice();
        if scopes.iter().any(|s| self.json_key.is_prefix_of(*s)) {
            return Some("pl-ent");
        }
        let in_shell = scopes.first().is_some_and(|s| self.shell.is_prefix_of(*s));
        for scope in scopes.iter().rev() {
            // GitHub leaves ordinary shell command names unstyled (only builtins get pl-c1).
            if in_shell && self.variable_function.is_prefix_of(*scope) {
                return None;
            }
            for (prefix, class) in &self.rules {
                if prefix.is_prefix_of(*scope) {
                    return Some(class);
                }
            }
        }
        None
    }

    fn highlight(&self, code: &str, syntax: &SyntaxReference) -> Result<String, syntect::Error> {
        let mut state = ParseState::new(syntax);
        let mut stack = ScopeStack::new();
        let mut out = String::with_capacity(code.len() * 2);
        let mut current: Option<&'static str> = None;

        for line in LinesWithEndings::from(code) {
            let ops = state.parse_line(line, &self.syntaxes)?;
            for (text, op) in ScopeRegionIterator::new(&ops, line) {
                stack.apply(op)?;
                if text.is_empty() {
                    continue;
                }
                let class = self.class_for(&stack);
                if class != current {
                    if current.is_some() {
                        out.push_str("</span>");
                    }
                    if let Some(c) = class {
                        let _ = write!(out, "<span class=\"{c}\">");
                    }
                    current = class;
                }
                out.push_str(&escape_html(text));
            }
        }
        if current.is_some() {
            out.push_str("</span>");
        }
        Ok(out)
    }

    pub fn syntax_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .syntaxes
            .syntaxes()
            .iter()
            .filter(|s| !s.hidden)
            .map(|s| format!("{} ({})", s.name, s.file_extensions.join(", ")))
            .collect();
        names.sort();
        names
    }
}

impl SyntaxHighlighterAdapter for GithubHighlighter {
    fn write_highlighted(
        &self,
        output: &mut dyn fmt::Write,
        lang: Option<&str>,
        code: &str,
    ) -> fmt::Result {
        let syntax = lang.and_then(|l| self.find_syntax(l));
        match syntax.and_then(|s| self.highlight(code, s).ok()) {
            Some(html) => output.write_str(&html),
            None => output.write_str(&escape_html(code)),
        }
    }

    fn write_pre_tag(
        &self,
        output: &mut dyn fmt::Write,
        attributes: HashMap<&'static str, Cow<'_, str>>,
    ) -> fmt::Result {
        comrak::html::write_opening_tag(output, "pre", attributes)
    }

    fn write_code_tag(
        &self,
        output: &mut dyn fmt::Write,
        attributes: HashMap<&'static str, Cow<'_, str>>,
    ) -> fmt::Result {
        comrak::html::write_opening_tag(output, "code", attributes)
    }
}

pub struct Renderer {
    highlighter: GithubHighlighter,
}

impl Renderer {
    pub fn new() -> Self {
        Self {
            highlighter: GithubHighlighter::new(),
        }
    }

    pub fn syntax_names(&self) -> Vec<String> {
        self.highlighter.syntax_names()
    }

    /// Markdown → sanitized HTML fragment. `base_dir` resolves relative image paths.
    pub fn render_markdown(&self, md: &str, base_dir: &Path) -> String {
        let mut options = Options::default();
        {
            let ext = &mut options.extension;
            ext.strikethrough = true;
            ext.table = true;
            ext.autolink = true;
            ext.tasklist = true;
            ext.footnotes = true;
            ext.alerts = true; // > [!NOTE] blocks
            ext.math_dollars = true;
            ext.math_code = true;
            ext.shortcodes = true; // :emoji:
            ext.header_id_prefix = Some(String::new()); // <h1 id="…"><a class="anchor">
            ext.front_matter_delimiter = Some("---".into());
        }
        {
            let render = &mut options.render;
            render.r#unsafe = true; // raw HTML passes through to the sanitizer, like GitHub
            render.github_pre_lang = true;
            render.gfm_quirks = true;
            render.tasklist_classes = true;
        }

        let mut plugins = Plugins::default();
        plugins.render.codefence_syntax_highlighter = Some(&self.highlighter);

        let raw = markdown_to_html_with_plugins(md, &options, &plugins);
        sanitize(&raw, base_dir)
    }
}

/// Mirrors GitHub's html-pipeline SanitizationFilter: a whitelist of tags and
/// attributes, with media sources rewritten to our `mdview://` asset protocol.
fn sanitize(html: &str, base_dir: &Path) -> String {
    let base = base_dir.to_path_buf();
    let mut builder = ammonia::Builder::default();
    builder
        .add_tags(&["input", "video", "source", "picture"])
        .add_tag_attributes("input", &["checked", "disabled"])
        .set_tag_attribute_value("input", "type", "checkbox")
        .add_tag_attributes(
            "video",
            &["src", "controls", "width", "height", "poster", "muted", "loop"],
        )
        .add_tag_attributes("source", &["src", "srcset", "type", "media"])
        .add_tag_attributes("ol", &["type"])
        .add_tag_attributes("pre", &["lang"])
        .add_generic_attributes(&["class", "id", "dir", "align"])
        .add_generic_attribute_prefixes(&["data-", "aria-"])
        .add_url_schemes(&["mdview"])
        .link_rel(None)
        .attribute_filter(move |element, attribute, value| {
            let is_src = matches!(
                (element, attribute),
                ("img" | "video" | "source", "src") | ("video", "poster")
            );
            if is_src {
                if let Some(url) = asset_url(&base, value) {
                    return Some(Cow::Owned(url));
                }
            }
            Some(Cow::Borrowed(value))
        });
    builder.clean(html).to_string()
}

/// Turns a relative media path into `mdview://localhost/asset/<encoded absolute path>`.
fn asset_url(base: &Path, value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() || v.starts_with('#') || v.starts_with("//") || has_scheme(v) {
        return None;
    }
    let clean = v.split(['?', '#']).next().unwrap_or("");
    if clean.is_empty() {
        return None;
    }
    let decoded = percent_decode_str(clean).decode_utf8_lossy();
    let full = base.join(&*decoded);
    let full_str = full.to_string_lossy();
    let encoded = utf8_percent_encode(&full_str, NON_ALPHANUMERIC);
    Some(format!("mdview://localhost/asset/{encoded}"))
}

pub fn has_scheme(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    for c in chars {
        if c == ':' {
            return true;
        }
        if !(c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) {
            return false;
        }
    }
    false
}

pub fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| MD_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// github-markdown-css picks light/dark purely via `prefers-color-scheme`. Rebuild it so
/// the theme can also be forced with `<html data-theme="light|dark">`.
pub fn themed_github_css() -> String {
    let (rest, dark) = cut_media_block(GH_CSS, "@media (prefers-color-scheme: dark)");
    let (rest, light) = cut_media_block(&rest, "@media (prefers-color-scheme: light)");
    let dark = rule_body(&dark);
    let light = rule_body(&light);
    format!(
        "{rest}\n\
         @media (prefers-color-scheme: dark) {{\n:root:not([data-theme=\"light\"]) .markdown-body {{{dark}}}\n}}\n\
         @media (prefers-color-scheme: light) {{\n:root:not([data-theme=\"dark\"]) .markdown-body {{{light}}}\n}}\n\
         :root[data-theme=\"dark\"] .markdown-body {{{dark}}}\n\
         :root[data-theme=\"light\"] .markdown-body {{{light}}}\n"
    )
}

/// Removes the first `@media …{…}` block whose header starts with `prefix`.
/// Returns the remaining CSS and the block's inner text.
fn cut_media_block(css: &str, prefix: &str) -> (String, String) {
    let Some(start) = css.find(prefix) else {
        return (css.to_string(), String::new());
    };
    let Some(open_rel) = css[start..].find('{') else {
        return (css.to_string(), String::new());
    };
    let open = start + open_rel;
    let mut depth = 0usize;
    let mut end = None;
    for (i, c) in css[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(end) = end else {
        return (css.to_string(), String::new());
    };
    let inner = css[open + 1..end].to_string();
    let mut rest = String::with_capacity(css.len());
    rest.push_str(&css[..start]);
    rest.push_str(&css[end + 1..]);
    (rest, inner)
}

/// `selector { body }` → `body`
fn rule_body(rule: &str) -> String {
    match (rule.find('{'), rule.rfind('}')) {
        (Some(o), Some(c)) if c > o => rule[o + 1..c].to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(md: &str) -> String {
        Renderer::new().render_markdown(md, Path::new("/docs"))
    }

    #[test]
    fn strips_scripts_and_event_handlers() {
        let html = render("hi <script>alert(1)</script> <img src=\"x.png\" onerror=\"alert(2)\">");
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("onerror"), "{html}");
        assert!(!html.contains("alert(1)"), "{html}");
    }

    #[test]
    fn rewrites_relative_images_to_asset_protocol() {
        let html = render("![a](img/dot.png)");
        assert!(
            html.contains("src=\"mdview://localhost/asset/%2Fdocs%2Fimg%2Fdot%2Epng\""),
            "{html}"
        );
        let html = render("![a](https://example.com/y.png)");
        assert!(html.contains("src=\"https://example.com/y.png\""), "{html}");
    }

    #[test]
    fn highlights_code_with_github_classes() {
        let html = render("```bash\necho \"hi\" # note\n```");
        assert!(html.contains("<pre lang=\"bash\">"), "{html}");
        assert!(html.contains("<span class=\"pl-c1\">echo</span>"), "{html}");
        assert!(html.contains("<span class=\"pl-c\">"), "{html}");
        assert!(!html.contains("hl-"), "{html}");
    }

    #[test]
    fn mermaid_and_unknown_languages_stay_plain() {
        let html = render("```mermaid\ngraph LR\n  A --> B\n```");
        assert!(html.contains("<pre lang=\"mermaid\">"), "{html}");
        assert!(!html.contains("pl-"), "{html}");
        let html = render("```nosuchlang\nx = 1\n```");
        assert!(html.contains("x = 1"), "{html}");
    }

    #[test]
    fn gfm_extensions_render() {
        let html = render("> [!NOTE]\n> text\n\n- [x] done\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n$E=mc^2$ :rocket:");
        assert!(html.contains("markdown-alert-note"), "{html}");
        assert!(html.contains("task-list-item-checkbox"), "{html}");
        assert!(html.contains("<table>"), "{html}");
        assert!(html.contains("data-math-style=\"inline\""), "{html}");
        assert!(html.contains("🚀"), "{html}");
    }

    #[test]
    fn themed_css_supports_forced_theme() {
        let css = themed_github_css();
        assert!(css.contains(":root[data-theme=\"dark\"] .markdown-body {"));
        assert!(css.contains(":root[data-theme=\"light\"] .markdown-body {"));
        assert!(css.contains(":root:not([data-theme=\"light\"]) .markdown-body {"));
        assert!(css.contains("--fgColor-default"));
        assert!(!css.contains(".markdown-body, [data-theme=\"dark\"]"));
    }

    #[test]
    fn helpers() {
        assert!(has_scheme("https://example.com"));
        assert!(has_scheme("mailto:a@b"));
        assert!(!has_scheme("img/a.png"));
        assert!(!has_scheme("../x.md"));
        assert!(is_markdown(Path::new("README.MD")));
        assert!(!is_markdown(Path::new("photo.png")));
        assert_eq!(escape_html("<a & b>"), "&lt;a &amp; b&gt;");
    }
}
