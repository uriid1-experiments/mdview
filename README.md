# mdview

English | [Русский](README.ru.md)

A desktop Markdown viewer and editor built on GitHub's rendering stack.

| Layer | GitHub | mdview |
|-------|--------|--------|
| Parser | cmark-gfm / comrak | **comrak** (GFM: tables, task lists, footnotes, `[!NOTE]` alerts, `:emoji:`, math) |
| Sanitizer | html-pipeline | **ammonia** (raw HTML passes through, `<script>` and `on*` attributes are stripped) |
| Syntax highlighting | linguist → `pl-*` classes | **syntect** (+ bat's syntaxes via two-face) → the same `pl-*` classes |
| Styles | Primer | **github-markdown-css** 5.8.1, embedded in the binary, light/dark themes |
| Math / diagrams | KaTeX / Mermaid on the client | same, embedded in the binary (`assets/vendor`), work offline |
| Window | — | **wry + tao** (WebKitGTK), GTK dialogs, no Node and no Tauri CLI |

## Build and run

```sh
cargo build --release
./target/release/mdview README.md
```

System dependencies: `webkit2gtk-4.1`, `gtk3` (Arch: `pacman -S webkit2gtk-4.1`).

Or use the Makefile:

```sh
make              # release build
make run FILE=samples/demo.md
make check        # unit tests + smoke test of the release binary
make install      # install to ~/.local (PREFIX=/usr for system-wide)
make default-app  # make mdview the default app for Markdown files
make help         # list all targets
```

## Features

- Menu bar "File / View / Go / Help", hidden and shown with `Ctrl+Shift+M` (shown by default).
- Open (`Ctrl+O`), new (`Ctrl+N`), save (`Ctrl+S`), save as (`Ctrl+Shift+S`), recent files, close (`Ctrl+W`), quit (`Ctrl+Q`).
- Modes: preview, editor + preview, editor (`Ctrl+E` cycles through them). The preview re-renders as you type.
- Editor: `Tab`/`Shift+Tab` indent, `Enter` continues lists, `Ctrl+B`/`Ctrl+I`/`` Ctrl+` `` wrap the selection, `Ctrl+K` inserts a link, status bar.
- Theme: follow system / light / dark (`Ctrl+Shift+T`), zoom `Ctrl+=`/`Ctrl+-`/`Ctrl+0`.
- Relative links to `.md` files open in the same window with back/forward history (`Alt+←`/`Alt+→`, mouse side buttons). External links go to the browser, directories to the file manager.
- File watching: external edits are reloaded automatically; if the editor has unsaved changes, a banner lets you choose. Manual reload with `Ctrl+R` or `F5`.
- Drag and drop a file onto the window. Relative images and videos are served through the built-in `mdview://` protocol.
- Settings (theme, mode, menu, zoom, recent files, window size) are stored in `~/.config/mdview/config.json`.

## Utility flags

- `mdview --dump file.md` prints the rendered HTML fragment to stdout.
- `mdview --syntaxes` prints the list of highlighting languages.
- `MDVIEW_WINDOW_SIZE=1200x900` sets the window size.
- `MDVIEW_DEBUG_SCRIPT=test.js` runs a script in the page after every load (for automated tests under Xvfb).

## Layout

```
src/main.rs     window, mdview:// protocol, IPC commands, GTK dialogs, config, file watcher
src/render.rs   comrak → highlighting into pl-* classes → ammonia; theme CSS assembly
assets/         github-markdown.css, index.html (shell), app.css, app.js
assets/vendor/  KaTeX and Mermaid (versions in VERSIONS), embedded via src/vendor.rs
samples/        demo.md showcasing every feature
```
