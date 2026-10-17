(() => {
  'use strict';

  const INITIAL = window.__INITIAL__ || { settings: {}, about: {} };
  const KATEX = '/static/katex/';            // vendored, see assets/vendor
  const MERMAID = '/static/mermaid/mermaid.min.js';

  const $ = (s) => document.querySelector(s);
  const el = {
    menubar: $('#menubar'),
    main: $('#main'),
    editor: $('#editor'),
    previewPane: $('#preview-pane'),
    content: $('#content'),
    status: $('#editor-status'),
    banner: $('#banner'),
    modal: $('#modal'),
    modalText: $('#modal-text'),
    modalActions: $('#modal-actions'),
    toast: $('#toast'),
    docpath: $('#docpath'),
    recent: $('#recent-list'),
  };
  const ipc = (msg) => {
    try { window.ipc.postMessage(JSON.stringify(msg)); } catch (e) { console.error('ipc', e); }
  };
  const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const hasScheme = (h) => /^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(h);

  const EMPTY_HTML = `<div class="mdview-empty">
    <p><strong>mdview</strong></p>
    <p>Перетащите сюда файл, нажмите <kbd>Ctrl</kbd>+<kbd>O</kbd> чтобы открыть или <kbd>Ctrl</kbd>+<kbd>N</kbd> чтобы создать новый.</p>
    <p class="hint">Меню: <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>M</kbd> · Редактор: <kbd>Ctrl</kbd>+<kbd>E</kbd></p>
  </div>`;

  const settings = Object.assign({ theme: 'system', menu: true, mode: 'preview', zoom: 1 }, INITIAL.settings);
  let doc = { path: null, name: '', dir: '', display: '', source: '', html: '', recent: [] };
  let lastSaved = '';
  let dirty = false;
  let previewStale = false;
  let afterSave = null; // continuation after a successful save
  let savingContent = null;
  const nav = { items: [], idx: -1, pendingPush: false, prevIdx: -1 };

  // ---------------------------------------------------------------- settings
  function applySettings() {
    const root = document.documentElement;
    if (settings.theme === 'light' || settings.theme === 'dark') root.setAttribute('data-theme', settings.theme);
    else root.removeAttribute('data-theme');
    el.menubar.hidden = !settings.menu;
    el.main.className = 'mode-' + settings.mode;
    document.querySelectorAll('[data-radio]').forEach((b) => {
      const [group, value] = b.dataset.radio.split(':');
      b.classList.toggle('checked', settings[group] === value);
    });
    if (settings.mode !== 'editor' && previewStale) renderPreview();
    updateStatus();
  }
  function patch(p) {
    const themeChanged = p.theme && p.theme !== settings.theme;
    Object.assign(settings, p);
    applySettings();
    ipc({ cmd: 'settings', patch: p });
    if (p.mode && p.mode !== 'preview') el.editor.focus({ preventScroll: true });
    if (themeChanged && hasDoc()) renderPreview(); // mermaid picks its theme at render time
  }
  const effectiveDark = () => settings.theme === 'dark' || (settings.theme === 'system' && matchMedia('(prefers-color-scheme: dark)').matches);
  const hasDoc = () => !!doc.path || el.editor.value.length > 0;

  // ---------------------------------------------------------------- document
  function setDoc(d, opts = {}) {
    doc = d;
    renderRecent(d.recent || []);
    const caret = el.editor.selectionStart, editorScroll = el.editor.scrollTop;
    el.editor.value = d.source || '';
    lastSaved = el.editor.value;
    setDirty(false);
    previewStale = false;
    if (d.error) {
      el.content.innerHTML = `<div class="mdview-empty mdview-error"><p>${esc(d.error)}</p></div>`;
    } else if (!d.path) {
      el.content.innerHTML = EMPTY_HTML;
    } else {
      el.content.innerHTML = d.html;
      enhance();
    }
    setPathLabel(d);
    if (opts.keepScroll) {
      const pos = Math.min(caret, el.editor.value.length);
      el.editor.setSelectionRange(pos, pos);
      el.editor.scrollTop = editorScroll;
    } else {
      el.previewPane.scrollTop = 0;
      el.editor.setSelectionRange(0, 0);
      el.editor.scrollTop = 0;
    }
    hideBanner();
    if (nav.pendingPush && d.path) {
      nav.items.length = nav.idx + 1;
      nav.items.push(d.path);
      nav.idx = nav.items.length - 1;
    }
    nav.pendingPush = false;
    updateNav();
    updateStatus();
  }
  function setPathLabel(d) {
    if (!d.path) { el.docpath.textContent = ''; el.docpath.title = ''; return; }
    const dir = (d.display || d.path).replace(/\/[^/]*$/, '');
    el.docpath.textContent = `${d.name}  ·  ${dir}`;
    el.docpath.title = d.path;
  }
  async function fetchDoc(opts) {
    try {
      const r = await fetch('/doc', { cache: 'no-store' });
      setDoc(await r.json(), opts);
    } catch (e) {
      console.error(e);
      toast('Не удалось загрузить документ');
    }
  }

  // live preview
  let renderTimer = null;
  function schedulePreview() {
    if (settings.mode === 'editor') { previewStale = true; return; }
    clearTimeout(renderTimer);
    renderTimer = setTimeout(renderPreview, 150);
  }
  async function renderPreview() {
    previewStale = false;
    const src = el.editor.value;
    try {
      const r = await fetch('/render', { method: 'POST', body: src, cache: 'no-store' });
      const html = await r.text();
      if (src !== el.editor.value) return; // a newer render is coming
      const x = el.previewPane.scrollLeft, y = el.previewPane.scrollTop;
      if (!src.trim() && !doc.path) el.content.innerHTML = EMPTY_HTML;
      else { el.content.innerHTML = html; enhance(); }
      el.previewPane.scrollTo(x, y);
    } catch (e) {
      console.error('render', e);
    }
  }

  function setDirty(v) {
    if (dirty === v) return;
    dirty = v;
    ipc({ cmd: 'dirty', value: v });
    updateStatus();
  }

  // ---------------------------------------------------------------- save / open
  function save(then) {
    afterSave = then || null;
    savingContent = el.editor.value;
    ipc({ cmd: 'save', content: savingContent });
  }
  function saveAs(then) {
    afterSave = then || null;
    savingContent = el.editor.value;
    ipc({ cmd: 'saveAs', content: savingContent });
  }
  function onSaved(ev) {
    lastSaved = savingContent ?? el.editor.value;
    setDirty(el.editor.value !== lastSaved);
    Object.assign(doc, { path: ev.path, name: ev.name, dir: ev.dir, display: ev.display });
    setPathLabel(doc);
    if (ev.pathChanged) {
      if (nav.idx >= 0) nav.items[nav.idx] = ev.path;
      else { nav.items = [ev.path]; nav.idx = 0; }
      updateNav();
      renderPreview(); // relative images resolve against the new directory
    }
    toast('Сохранено: ' + ev.name);
    const f = afterSave; afterSave = null;
    if (f) f();
  }
  function confirmDiscard(then) {
    if (!dirty) { then(); return; }
    modal(`<p>Сохранить изменения в «${esc(doc.name || 'Без имени')}»?</p>`, [
      { label: 'Сохранить', primary: true, run: () => save(then) },
      { label: 'Не сохранять', danger: true, run: then },
      { label: 'Отмена' },
    ]);
  }
  function navigateTo(path) {
    confirmDiscard(() => {
      nav.prevIdx = nav.idx;
      nav.pendingPush = true;
      ipc({ cmd: 'openFile', path });
    });
  }
  function navHistory(delta) {
    const i = nav.idx + delta;
    if (i < 0 || i >= nav.items.length) return;
    confirmDiscard(() => {
      nav.prevIdx = nav.idx;
      nav.idx = i;
      nav.pendingPush = false;
      ipc({ cmd: 'openFile', path: nav.items[i] });
    });
  }
  function updateNav() {
    $('#nav-back').disabled = nav.idx <= 0;
    $('#nav-forward').disabled = nav.idx >= nav.items.length - 1;
    document.querySelectorAll('[data-needs-doc]').forEach((b) => { b.disabled = !doc.path; });
  }
  function renderRecent(items) {
    el.recent.innerHTML = '';
    if (!items.length) return;
    const label = document.createElement('div');
    label.className = 'label';
    label.textContent = 'Недавние';
    el.recent.appendChild(label);
    items.slice(0, 8).forEach((it) => {
      const b = document.createElement('button');
      b.className = 'recent';
      b.dataset.recent = it.path;
      b.textContent = it.display;
      b.title = it.path;
      el.recent.appendChild(b);
    });
  }

  // ---------------------------------------------------------------- commands
  const commands = {
    new: () => confirmDiscard(() => {
      ipc({ cmd: 'new' });
      if (settings.mode === 'preview') patch({ mode: 'split' });
    }),
    open: () => confirmDiscard(() => ipc({ cmd: 'open' })),
    save: () => save(),
    saveAs: () => saveAs(),
    reload: () => {
      if (!doc.path) return;
      if (dirty) {
        modal('<p>Перечитать файл с диска? Несохранённые правки будут потеряны.</p>', [
          { label: 'Перечитать', danger: true, run: commands.reloadFromDisk },
          { label: 'Отмена' },
        ]);
      } else commands.reloadFromDisk();
    },
    reloadFromDisk: () => { nav.pendingPush = false; fetchDoc({ keepScroll: true }); },
    dismissBanner: () => hideBanner(),
    reveal: () => ipc({ cmd: 'reveal' }),
    close: () => confirmDiscard(() => ipc({ cmd: 'close' })),
    quit: () => confirmDiscard(() => ipc({ cmd: 'quit' })),
    back: () => navHistory(-1),
    forward: () => navHistory(1),
    scrollTop: () => { el.previewPane.scrollTop = 0; el.editor.scrollTop = 0; },
    scrollBottom: () => { el.previewPane.scrollTop = el.previewPane.scrollHeight; el.editor.scrollTop = el.editor.scrollHeight; },
    zoomIn: () => ipc({ cmd: 'zoom', action: 'in' }),
    zoomOut: () => ipc({ cmd: 'zoom', action: 'out' }),
    zoomReset: () => ipc({ cmd: 'zoom', action: 'reset' }),
    toggleMenu: () => patch({ menu: !settings.menu }),
    cycleMode: () => patch({ mode: { preview: 'split', split: 'editor', editor: 'preview' }[settings.mode] || 'preview' }),
    cycleTheme: () => patch({ theme: { system: 'light', light: 'dark', dark: 'system' }[settings.theme] || 'system' }),
    shortcuts: showShortcuts,
    about: showAbout,
  };
  function run(name, e) {
    if (e) e.preventDefault();
    closeMenus();
    if (commands[name]) commands[name]();
  }

  // ---------------------------------------------------------------- events from Rust
  window.__mdview = {
    dispatch(ev) {
      switch (ev.type) {
        case 'doc': fetchDoc(); break;
        case 'saved': onSaved(ev); break;
        case 'saveCancelled': afterSave = null; break;
        case 'fileChanged': if (dirty) showBanner(); else { nav.pendingPush = false; fetchDoc({ keepScroll: true }); } break;
        case 'openRequest': navigateTo(ev.path); break;
        case 'openFailed': nav.idx = nav.prevIdx; nav.pendingPush = false; updateNav(); break;
        case 'closeRequested': confirmDiscard(() => ipc({ cmd: 'quit' })); break;
        case 'toast': toast(ev.message); break;
        case 'zoom': settings.zoom = ev.level; updateStatus(); break;
        default: console.warn('unknown event', ev);
      }
    },
  };

  // ---------------------------------------------------------------- links
  el.content.addEventListener('click', (e) => {
    const a = e.target.closest('a[href]');
    if (!a) return;
    const href = a.getAttribute('href');
    if (!href) return;
    e.preventDefault();
    if (href.startsWith('#')) { scrollToAnchor(href.slice(1)); return; }
    if (hasScheme(href)) {
      if (!href.startsWith('mdview:')) ipc({ cmd: 'external', url: href });
      return;
    }
    ipc({ cmd: 'resolve', href });
  });
  el.content.addEventListener('auxclick', (e) => { if (e.target.closest('a[href]')) e.preventDefault(); });
  function scrollToAnchor(id) {
    let target = null;
    try { target = document.getElementById(decodeURIComponent(id)); } catch (_) { /* bad escape */ }
    if (!target) target = document.getElementById(id);
    if (target) target.scrollIntoView();
  }
  window.addEventListener('mouseup', (e) => {
    if (e.button === 3) run('back', e);
    else if (e.button === 4) run('forward', e);
  });

  // ---------------------------------------------------------------- keyboard
  document.addEventListener('keydown', (e) => {
    const k = e.key.toLowerCase();
    const ctrl = e.ctrlKey || e.metaKey;
    if (e.key === 'Escape') {
      if (!el.modal.hidden) { e.preventDefault(); closeModal(); }
      closeMenus();
      return;
    }
    if (ctrl && e.shiftKey && k === 'm') return run('toggleMenu', e);
    if (ctrl && e.shiftKey && k === 's') return run('saveAs', e);
    if (ctrl && e.shiftKey && k === 't') return run('cycleTheme', e);
    if (ctrl && !e.shiftKey && !e.altKey) {
      switch (k) {
        case 'o': return run('open', e);
        case 's': return run('save', e);
        case 'n': return run('new', e);
        case 'q': return run('quit', e);
        case 'w': return run('close', e);
        case 'e': return run('cycleMode', e);
        case 'r': return run('reload', e);
        case '=': case '+': return run('zoomIn', e);
        case '-': return run('zoomOut', e);
        case '0': return run('zoomReset', e);
        case 'home': if (e.target !== el.editor) return run('scrollTop', e); break;
        case 'end': if (e.target !== el.editor) return run('scrollBottom', e); break;
      }
    }
    if (e.key === 'F5') return run('reload', e);
    if (e.altKey && e.key === 'ArrowLeft') return run('back', e);
    if (e.altKey && e.key === 'ArrowRight') return run('forward', e);
  });

  // ---------------------------------------------------------------- editor
  const ta = el.editor;
  ta.addEventListener('input', () => {
    setDirty(ta.value !== lastSaved);
    schedulePreview();
    updateStatus();
  });
  ['keyup', 'click', 'select'].forEach((ev) => ta.addEventListener(ev, updateStatus));
  ta.addEventListener('keydown', (e) => {
    const ctrl = e.ctrlKey || e.metaKey;
    if (e.key === 'Tab') { e.preventDefault(); if (e.shiftKey) outdent(); else indent(); return; }
    if (e.key === 'Enter' && !ctrl && !e.shiftKey && !e.altKey) { if (continueList()) e.preventDefault(); return; }
    if (ctrl && !e.shiftKey && !e.altKey) {
      if (e.key === 'b') { e.preventDefault(); wrapSelection('**'); }
      else if (e.key === 'i') { e.preventDefault(); wrapSelection('*'); }
      else if (e.key === '`') { e.preventDefault(); wrapSelection('`'); }
      else if (e.key === 'k') { e.preventDefault(); insertLink(); }
    }
  });
  let syncing = false;
  ta.addEventListener('scroll', () => {
    if (settings.mode !== 'split' || syncing) return;
    const max = ta.scrollHeight - ta.clientHeight;
    if (max <= 0) return;
    const pp = el.previewPane;
    syncing = true;
    pp.scrollTop = (ta.scrollTop / max) * (pp.scrollHeight - pp.clientHeight);
    requestAnimationFrame(() => { syncing = false; });
  });

  const insertText = (text) => document.execCommand('insertText', false, text);
  function wrapSelection(mark) {
    const s = ta.selectionStart, e = ta.selectionEnd;
    const sel = ta.value.slice(s, e);
    const inner = sel || 'текст';
    insertText(mark + inner + mark);
    ta.setSelectionRange(s + mark.length, s + mark.length + inner.length);
    ta.dispatchEvent(new Event('input'));
  }
  function insertLink() {
    const s = ta.selectionStart, e = ta.selectionEnd;
    const sel = ta.value.slice(s, e) || 'текст';
    const text = `[${sel}](url)`;
    insertText(text);
    const u = s + sel.length + 3;
    ta.setSelectionRange(u, u + 3);
    ta.dispatchEvent(new Event('input'));
  }
  function selectedLines() {
    const s = ta.selectionStart, e = ta.selectionEnd;
    const start = ta.value.lastIndexOf('\n', s - 1) + 1;
    const nl = ta.value.indexOf('\n', e);
    const end = nl === -1 ? ta.value.length : nl;
    return { start, end, text: ta.value.slice(start, end) };
  }
  function indent() {
    const s = ta.selectionStart, e = ta.selectionEnd;
    if (s === e || !ta.value.slice(s, e).includes('\n')) { insertText('  '); return; }
    const { start, end, text } = selectedLines();
    const out = text.split('\n').map((l) => '  ' + l).join('\n');
    ta.setSelectionRange(start, end);
    insertText(out);
    ta.setSelectionRange(start, start + out.length);
    ta.dispatchEvent(new Event('input'));
  }
  function outdent() {
    const { start, end, text } = selectedLines();
    const out = text.split('\n').map((l) => l.replace(/^(\t| {1,2})/, '')).join('\n');
    if (out === text) return;
    ta.setSelectionRange(start, end);
    insertText(out);
    ta.setSelectionRange(start, start + out.length);
    ta.dispatchEvent(new Event('input'));
  }
  function continueList() {
    const s = ta.selectionStart;
    if (s !== ta.selectionEnd) return false;
    const ls = ta.value.lastIndexOf('\n', s - 1) + 1;
    const line = ta.value.slice(ls, s);
    const m = line.match(/^(\s*)([-*+]|\d+[.)])(\s+)(\[[ xX]\]\s+)?/);
    if (!m) return false;
    if (!line.slice(m[0].length).trim()) {
      // empty item: end the list
      ta.setSelectionRange(ls, s);
      document.execCommand('delete');
      ta.dispatchEvent(new Event('input'));
      return true;
    }
    let marker = m[2];
    if (/^\d/.test(marker)) marker = (parseInt(marker, 10) + 1) + marker.slice(-1);
    insertText('\n' + m[1] + marker + m[3] + (m[4] ? '[ ] ' : ''));
    ta.dispatchEvent(new Event('input'));
    return true;
  }
  function updateStatus() {
    const v = ta.value;
    const pos = ta.selectionStart;
    const before = v.slice(0, pos);
    const line = before.split('\n').length;
    const col = pos - before.lastIndexOf('\n');
    const words = (v.match(/\S+/g) || []).length;
    const zoom = Math.abs(settings.zoom - 1) > 0.001 ? `  ·  ${Math.round(settings.zoom * 100)}%` : '';
    el.status.textContent = `${dirty ? '● ' : ''}Стр ${line}, Кол ${col}  ·  ${words} слов  ·  ${v.length} симв.${zoom}`;
  }

  // ---------------------------------------------------------------- menus
  const menus = Array.from(document.querySelectorAll('.menu'));
  let openMenu = null;
  function setOpen(m) {
    menus.forEach((x) => x.classList.toggle('open', x === m));
    openMenu = m;
  }
  function closeMenus() { setOpen(null); }
  menus.forEach((m) => {
    const t = m.querySelector('.menu-title');
    t.addEventListener('click', (e) => { e.stopPropagation(); setOpen(openMenu === m ? null : m); });
    t.addEventListener('mouseenter', () => { if (openMenu && openMenu !== m) setOpen(m); });
  });
  document.addEventListener('click', (e) => {
    const b = e.target.closest('[data-cmd],[data-radio],[data-recent]');
    if (!b || b.disabled) { if (!e.target.closest('.dropdown')) closeMenus(); return; }
    e.stopPropagation();
    closeMenus();
    if (b.dataset.cmd) run(b.dataset.cmd);
    else if (b.dataset.radio) { const [g, v] = b.dataset.radio.split(':'); patch({ [g]: v }); }
    else if (b.dataset.recent) navigateTo(b.dataset.recent);
  });

  // ---------------------------------------------------------------- modal / toast / banner
  function modal(html, actions) {
    el.modalText.innerHTML = html;
    el.modalActions.innerHTML = '';
    actions.forEach((a) => {
      const b = document.createElement('button');
      b.textContent = a.label;
      b.className = 'btn' + (a.primary ? ' btn-primary' : '') + (a.danger ? ' btn-danger' : '');
      b.onclick = () => { closeModal(); if (a.run) a.run(); };
      el.modalActions.appendChild(b);
    });
    el.modal.hidden = false;
    (el.modalActions.querySelector('.btn-primary') || el.modalActions.firstChild).focus();
  }
  function closeModal() { el.modal.hidden = true; }
  el.modal.addEventListener('click', (e) => { if (e.target === el.modal) closeModal(); });
  let toastTimer = null;
  function toast(msg) {
    el.toast.textContent = msg;
    el.toast.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { el.toast.hidden = true; }, 2500);
  }
  function showBanner() { el.banner.hidden = false; }
  function hideBanner() { el.banner.hidden = true; }

  function showShortcuts() {
    const rows = [
      ['Ctrl+O', 'Открыть файл'], ['Ctrl+N', 'Новый файл'], ['Ctrl+S', 'Сохранить'], ['Ctrl+Shift+S', 'Сохранить как'],
      ['F5 / Ctrl+R', 'Перечитать с диска'], ['Ctrl+W', 'Закрыть файл'], ['Ctrl+Q', 'Выход'],
      ['Ctrl+E', 'Режим: просмотр → редактор и просмотр → редактор'], ['Ctrl+Shift+T', 'Тема: системная → светлая → тёмная'],
      ['Ctrl+Shift+M', 'Показать/скрыть меню'], ['Ctrl+= / Ctrl+− / Ctrl+0', 'Масштаб'],
      ['Alt+← / Alt+→', 'Назад / вперёд по открытым файлам'],
      ['Tab / Shift+Tab', 'В редакторе: отступ / убрать отступ'], ['Enter', 'В редакторе: продолжить список'],
      ['Ctrl+B / Ctrl+I / Ctrl+`', 'В редакторе: жирный / курсив / код'], ['Ctrl+K', 'В редакторе: ссылка'],
    ];
    const table = '<h3>Горячие клавиши</h3><table class="keys">' +
      rows.map(([k, d]) => `<tr><td>${k.split(' / ').map((x) => `<kbd class="k">${esc(x)}</kbd>`).join(' / ')}</td><td>${esc(d)}</td></tr>`).join('') +
      '</table>';
    modal(table, [{ label: 'Закрыть', primary: true }]);
  }
  function showAbout() {
    const a = INITIAL.about || {};
    modal(`<h3>mdview ${esc(a.version || '')}</h3>
      <p>Просмотрщик и редактор Markdown на стеке GitHub: comrak (GFM) → ammonia (санитайзер) → github-markdown-css.
      Подсветка кода: syntect в классы GitHub. Формулы и диаграммы: KaTeX и Mermaid.</p>
      <p>WebKitGTK ${esc(a.webkit || '?')}</p>`, [{ label: 'Закрыть', primary: true }]);
  }

  // ---------------------------------------------------------------- math & diagrams
  function loadScript(src) {
    return new Promise((res, rej) => {
      const s = document.createElement('script');
      s.src = src; s.onload = res; s.onerror = rej;
      document.head.appendChild(s);
    });
  }
  function loadCss(href) {
    const l = document.createElement('link');
    l.rel = 'stylesheet'; l.href = href;
    document.head.appendChild(l);
  }
  let katexReady = null;
  async function renderMath() {
    const els = el.content.querySelectorAll('code[data-math-style], span[data-math-style]');
    if (!els.length) return;
    try {
      if (!katexReady) { loadCss(KATEX + 'katex.min.css'); katexReady = loadScript(KATEX + 'katex.min.js'); }
      await katexReady;
    } catch (_) { return; }
    els.forEach((node) => {
      const display = node.dataset.mathStyle === 'display';
      const tex = node.textContent;
      let target = node;
      if (node.tagName === 'CODE' && node.parentElement && node.parentElement.tagName === 'PRE') {
        target = document.createElement('div');
        node.parentElement.replaceWith(target);
      }
      try { katex.render(tex, target, { displayMode: display, throwOnError: false, output: 'html' }); } catch (_) { /* leave source */ }
    });
  }
  let mermaidReady = null;
  async function renderMermaid() {
    const blocks = Array.from(el.content.querySelectorAll('pre[lang="mermaid"], code.language-mermaid'));
    if (!blocks.length) return;
    try {
      if (!mermaidReady) mermaidReady = loadScript(MERMAID);
      await mermaidReady;
    } catch (_) { return; }
    mermaid.initialize({ startOnLoad: false, theme: effectiveDark() ? 'dark' : 'default' });
    const nodes = [];
    for (const b of blocks) {
      const pre = b.closest('pre') || b;
      const div = document.createElement('div');
      div.className = 'mermaid';
      div.textContent = pre.textContent;
      pre.replaceWith(div);
      nodes.push(div);
    }
    try { await mermaid.run({ nodes }); } catch (err) { console.error(err); }
  }
  function enhance() { renderMath(); renderMermaid(); }

  // ---------------------------------------------------------------- crisp scrolling
  // WebKitGTK's smooth/kinetic scrolling can leave the compositor layer at a fractional
  // offset, so text sits between device pixels and looks blurry. Once scrolling settles,
  // re-apply the position as a whole device pixel. Assigning an equal value is a no-op for
  // WebKit, so go through a neighbouring value first; echoes of our own nudge are ignored.
  const snapTimers = new WeakMap();
  const snapQuiet = new WeakMap();
  function snapScroll(t) {
    const q = window.devicePixelRatio || 1;
    const top = Math.round(t.scrollTop * q) / q;
    const left = Math.round(t.scrollLeft * q) / q;
    snapQuiet.set(t, performance.now() + 150);
    if (t.scrollHeight > t.clientHeight) {
      t.scrollTop = top > 0 ? top - 1 : top + 1;
      t.scrollTop = top;
    }
    if (t.scrollWidth > t.clientWidth) {
      t.scrollLeft = left > 0 ? left - 1 : left + 1;
      t.scrollLeft = left;
    }
  }
  document.addEventListener('scroll', (e) => {
    const t = e.target;
    if (!(t instanceof Element)) return;
    if ((snapQuiet.get(t) || 0) > performance.now()) return;
    clearTimeout(snapTimers.get(t));
    snapTimers.set(t, setTimeout(() => snapScroll(t), 120));
  }, true);

  // ---------------------------------------------------------------- boot
  applySettings();
  nav.pendingPush = true;
  fetchDoc();
})();
