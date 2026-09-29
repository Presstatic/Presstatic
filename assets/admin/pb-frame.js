// Presstatic page builder: script dell'anteprima (solo nel pannello, mai sul sito). Selezione, trascinamento, barra degli strumenti.
(() => {
  const P = parent, root = () => document.getElementById('pb-root');
  const post = m => P.postMessage(Object.assign({ pb: 1 }, m), location.origin);
  let sel = null, ind = document.createElement('div'); ind.className = 'pbf-ind'; document.body.append(ind);
  const bar = document.createElement('div'); bar.className = 'pbf-bar'; document.body.append(bar);
  const labels = { section: 'Sezione', col: 'Colonna', widget: 'Widget' };
  const NAMES = { heading: 'Titolo', text: 'Testo', image: 'Immagine', button: 'Pulsante', divider: 'Separatore', spacer: 'Spazio', video: 'Video', list: 'Elenco', hero: 'Apertura', posts_grid: 'Griglia di articoli', posts_list: 'Elenco di articoli', latest: 'Ultime notizie', most_read: 'I più letti', newsletter: 'Newsletter', ad: 'Pubblicità', html: 'HTML', logo: 'Logo', menu: 'Menu', search: 'Ricerca', social: 'Social', categories: 'Sezioni', copyright: 'Copyright', tabs: 'Schede', accordion: 'Fisarmonica', carousel: 'Carosello', gallery: 'Galleria', counter: 'Numero', quote: 'Citazione', card: 'Scheda' };
  function mark() {
    document.querySelectorAll('.pbf-sel').forEach(e => e.classList.remove('pbf-sel'));
    const el = sel && document.querySelector('[data-pb-id="' + sel + '"]');
    bar.style.display = el ? 'flex' : 'none';
    if (!el) return;
    el.classList.add('pbf-sel');
    const kind = el.dataset.pb, r = el.getBoundingClientRect();
    bar.innerHTML = '<b>' + (kind === 'widget' ? (NAMES[el.dataset.pbType] || labels.widget) : labels[kind]) + '</b>' +
      (kind !== 'col' ? '<button data-op="up" title="Sposta su">↑</button><button data-op="down" title="Sposta giù">↓</button><button data-op="dup" title="Duplica">⧉</button>' : '') +
      (kind === 'section' ? '<button data-op="addsec" title="Aggiungi una sezione sotto">＋</button><button data-op="savesec" title="Salva tra i miei blocchi, per riusarla">★</button>' : '') +
      '<button data-op="del" title="Elimina">✕</button>';
    bar.style.top = Math.max(0, r.top + scrollY - 30) + 'px'; bar.style.left = (r.left + scrollX) + 'px';
  }
  bar.addEventListener('click', e => { const b = e.target.closest('button'); if (b) post({ type: 'op', op: b.dataset.op, id: sel }); });
  // Scrivere nell'anteprima: doppio clic (o clic su un widget già scelto) su titoli, testi, pulsanti, citazioni, schede,
  // elenchi. Invio conferma (nei testi va a capo), Esc annulla, un clic fuori salva. Solo testo semplice, anche incollando.
  let editing = null;
  function startEdit(el) {
    const w = el.closest('[data-pb="widget"]'); if (!w || editing) return;
    editing = { el, w, before: el.innerText }; w.draggable = false; bar.style.display = 'none';
    el.setAttribute('contenteditable', 'true'); el.classList.add('pbf-editing'); el.focus();
    const r = document.createRange(); r.selectNodeContents(el); r.collapse(false); const x = getSelection(); x.removeAllRanges(); x.addRange(r);
  }
  function endEdit(save) {
    if (!editing) return; const { el, w, before } = editing; editing = null;
    el.removeAttribute('contenteditable'); el.classList.remove('pbf-editing'); w.draggable = true;
    const multi = el.dataset.pbMulti, raw = el.innerText.replace(/\u00a0/g, ' ');
    const v = multi === 'para' ? raw.replace(/\n{3,}/g, '\n\n').trim() : multi === 'lines' ? raw.split('\n').map(s => s.trim()).filter(Boolean).join('\n') : raw.replace(/\s*\n\s*/g, ' ').trim();
    if (!save || v === before.trim()) { post({ type: 'rerender' }); return; }
    post({ type: 'text', id: w.dataset.pbId, field: el.dataset.pbEdit, value: v });
  }
  document.addEventListener('dblclick', e => { const el = e.target.closest('[data-pb-edit]'); if (el) { e.preventDefault(); startEdit(el); } });
  document.addEventListener('keydown', e => { if (!editing) return; if (e.key === 'Escape') { e.preventDefault(); endEdit(false); } else if (e.key === 'Enter' && !editing.el.dataset.pbMulti) { e.preventDefault(); editing.el.blur(); } });
  document.addEventListener('focusout', e => { if (editing && e.target === editing.el) endEdit(true); });
  document.addEventListener('paste', e => { if (!editing) return; e.preventDefault(); document.execCommand('insertText', false, (e.clipboardData || window.clipboardData).getData('text/plain')); });
  document.addEventListener('click', e => {
    if (e.target.closest('.pbf-bar')) return;
    if (editing && editing.el.contains(e.target)) return; // si sta scrivendo: il clic sposta solo il cursore
    const ed = e.target.closest('[data-pb-edit]'), wd = ed && ed.closest('[data-pb="widget"]');
    if (ed && wd && wd.dataset.pbId === sel && !editing) { e.preventDefault(); startEdit(ed); return; }
    // Nell'editor i link non portano via dalla pagina; linguette delle schede e domande della fisarmonica invece si aprono, per vederne il testo.
    if (!e.target.closest('.pb-tabs label, .pb-tabs input, .pb-acc summary')) e.preventDefault(); // l'etichetta genera un secondo clic sul pulsante nascosto: va lasciato passare
    const el = e.target.closest('[data-pb]'); sel = el ? el.dataset.pbId : null; mark(); post({ type: 'select', id: sel });
    if (sel && e.target.closest('[data-pb-pick]')) post({ type: 'pick', id: sel }); // segnaposto dell'immagine: apre la libreria
  }, true);
  document.addEventListener('submit', e => e.preventDefault(), true);
  // Trascinamento: widget nuovi dal pannello a sinistra, o widget già nella pagina.
  let moving = null;
  const unselect = () => { const x = window.getSelection(); if (x && x.rangeCount) x.removeAllRanges(); }; // nessun testo evidenziato durante il trascinamento
  document.addEventListener('dragenter', unselect); document.addEventListener('dragover', unselect, { once: true });
  let over = null; const unover = () => { if (over) over.classList.remove('pbf-over'); over = null; };
  document.addEventListener('dragstart', e => { const w = e.target.closest('[data-pb="widget"]'); if (w) { w.classList.add('pbf-moving'); moving = w.dataset.pbId; e.dataTransfer.setData('text/plain', 'move:' + moving); e.dataTransfer.effectAllowed = 'move'; } });
  document.addEventListener('dragend', () => { moving = null; ind.style.display = 'none'; unover(); document.querySelectorAll('.pbf-moving').forEach(x => x.classList.remove('pbf-moving')); });
  function spot(e) {
    const w = e.target.closest('[data-pb="widget"]'), c = e.target.closest('[data-pb="col"]');
    if (w && w.dataset.pbId !== moving) { const r = w.getBoundingClientRect(); return { id: w.dataset.pbId, kind: 'widget', pos: e.clientY < r.top + r.height / 2 ? 'before' : 'after', r }; }
    if (c) return { id: c.dataset.pbId, kind: 'col', pos: 'inside', r: c.getBoundingClientRect() };
    return null;
  }
  document.addEventListener('dragover', e => {
    const t = spot(e); if (!t) { ind.style.display = 'none'; unover(); return; }
    const col = e.target.closest('[data-pb="col"]'); if (col !== over) { unover(); over = col; if (over) over.classList.add('pbf-over'); }
    e.preventDefault(); e.dataTransfer.dropEffect = moving ? 'move' : 'copy';
    const y = t.kind === 'col' ? t.r.bottom - 3 : (t.pos === 'before' ? t.r.top - 3 : t.r.bottom);
    Object.assign(ind.style, { display: 'block', top: (y + scrollY) + 'px', left: (t.r.left + scrollX) + 'px', width: t.r.width + 'px' });
  });
  document.addEventListener('drop', e => {
    const t = spot(e); ind.style.display = 'none'; unover(); if (!t) return;
    e.preventDefault();
    const d = e.dataTransfer.getData('text/plain');
    post({ type: 'drop', src: d.startsWith('move:') ? { move: d.slice(5) } : { add: d.replace(/^new:/, '') }, target: { id: t.id, kind: t.kind, pos: t.pos } });
  });
  addEventListener('message', e => {
    if (e.origin !== location.origin || !e.data || !e.data.pb) return;
    if (e.data.type === 'html') { const r = root(); if (r) r.outerHTML = e.data.html; sel = e.data.sel; mark(); }
    if (e.data.type === 'sel') { sel = e.data.sel; mark(); const el = sel && document.querySelector('[data-pb-id="' + sel + '"]'); if (el) el.scrollIntoView({ block: 'nearest', behavior: 'smooth' }); }
  });
  addEventListener('resize', mark); addEventListener('scroll', () => { if (sel) mark(); });
  const r0 = root(); if (r0 && r0.tagName === 'FOOTER') r0.scrollIntoView(); // piè di pagina: si parte da lì
  post({ type: 'ready' });
})();
