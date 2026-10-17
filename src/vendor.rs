//! Vendored client libraries (KaTeX, Mermaid), embedded into the binary and served
//! at `mdview://localhost/static/<path>`. Generated from assets/vendor/.

pub const STATIC_FILES: &[(&str, &str, &[u8])] = &[
    ("katex/fonts/KaTeX_AMS-Regular.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_AMS-Regular.woff2")),
    ("katex/fonts/KaTeX_Caligraphic-Bold.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Caligraphic-Bold.woff2")),
    ("katex/fonts/KaTeX_Caligraphic-Regular.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Caligraphic-Regular.woff2")),
    ("katex/fonts/KaTeX_Fraktur-Bold.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Fraktur-Bold.woff2")),
    ("katex/fonts/KaTeX_Fraktur-Regular.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Fraktur-Regular.woff2")),
    ("katex/fonts/KaTeX_Main-Bold.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Main-Bold.woff2")),
    ("katex/fonts/KaTeX_Main-BoldItalic.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Main-BoldItalic.woff2")),
    ("katex/fonts/KaTeX_Main-Italic.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Main-Italic.woff2")),
    ("katex/fonts/KaTeX_Main-Regular.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Main-Regular.woff2")),
    ("katex/fonts/KaTeX_Math-BoldItalic.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Math-BoldItalic.woff2")),
    ("katex/fonts/KaTeX_Math-Italic.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Math-Italic.woff2")),
    ("katex/fonts/KaTeX_SansSerif-Bold.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_SansSerif-Bold.woff2")),
    ("katex/fonts/KaTeX_SansSerif-Italic.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_SansSerif-Italic.woff2")),
    ("katex/fonts/KaTeX_SansSerif-Regular.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_SansSerif-Regular.woff2")),
    ("katex/fonts/KaTeX_Script-Regular.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Script-Regular.woff2")),
    ("katex/fonts/KaTeX_Size1-Regular.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Size1-Regular.woff2")),
    ("katex/fonts/KaTeX_Size2-Regular.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Size2-Regular.woff2")),
    ("katex/fonts/KaTeX_Size3-Regular.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Size3-Regular.woff2")),
    ("katex/fonts/KaTeX_Size4-Regular.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Size4-Regular.woff2")),
    ("katex/fonts/KaTeX_Typewriter-Regular.woff2", "font/woff2", include_bytes!("../assets/vendor/katex/fonts/KaTeX_Typewriter-Regular.woff2")),
    ("katex/katex.min.css", "text/css; charset=utf-8", include_bytes!("../assets/vendor/katex/katex.min.css")),
    ("katex/katex.min.js", "application/javascript; charset=utf-8", include_bytes!("../assets/vendor/katex/katex.min.js")),
    ("mermaid/mermaid.min.js", "application/javascript; charset=utf-8", include_bytes!("../assets/vendor/mermaid/mermaid.min.js")),
];

pub fn lookup(path: &str) -> Option<(&'static str, &'static [u8])> {
    STATIC_FILES
        .iter()
        .find(|(name, _, _)| *name == path)
        .map(|(_, mime, bytes)| (*mime, *bytes))
}
