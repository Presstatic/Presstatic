// Presstatic page builder: editor nel pannello. Tiene la struttura della pagina (JSON), la manda al server per l'anteprima,
// gestisce impostazioni, dispositivi, annulla/ripeti, salvataggio e pubblicazione.
(() => {
  const $ = s => document.querySelector(s), data = JSON.parse($('#pb-data').textContent);
  let doc = data.doc, sel = null, device = '', hist = [], fut = [], dirty = false, timer;
  const frame = $('#pb-frame'), cats = data.cats, Q = '?pagina=' + data.qname;
  const uid = p => p + Math.random().toString(36).slice(2, 8);
  const CAT = ['category', 'Categoria', 'select', Object.assign({ '': 'Tutte le categorie' }, Object.fromEntries(cats.map(c => [c, c])))];
  const W = {
    heading: ['Titolo', 'base', 'H', { text: 'Nuovo titolo', tag: 'h2' }, [['text', 'Testo', 'text'], ['tag', 'Livello', 'select', { h1: 'H1 (uno solo per pagina)', h2: 'H2', h3: 'H3', h4: 'H4' }], ['link', 'Link', 'text'], ['size', 'Dimensione del testo (px)', 'number', 1, 'style']]],
    text: ['Testo', 'base', '¶', { text: 'Scrivi qui il testo. Una riga vuota separa i paragrafi.' }, [['text', 'Testo', 'textarea'], ['size', 'Dimensione del testo (px)', 'number', 1, 'style']]],
    image: ['Immagine', 'base', '▣', {}, [['src', 'Immagine', 'image'], ['alt', 'Testo alternativo', 'text'], ['caption', 'Didascalia', 'text'], ['link', 'Link', 'text'], ['width', 'Larghezza (%)', 'number', 1, 'style']]],
    button: ['Pulsante', 'base', '▭', { label: 'Scopri di più', url: '#' }, [['label', 'Testo', 'text'], ['url', 'Link', 'text'], ['newtab', 'Apri in una nuova scheda', 'toggle'], ['btn_bg', 'Colore del pulsante', 'color', 0, 'style'], ['btn_color', 'Colore del testo', 'color', 0, 'style']]],
    divider: ['Separatore', 'base', '—', {}, [['style', 'Stile', 'select', { solid: 'Continuo', dashed: 'Tratteggiato', dotted: 'Puntini' }], ['thick', 'Spessore (px)', 'number'], ['width', 'Larghezza (%)', 'number'], ['line', 'Colore', 'color']]],
    spacer: ['Spazio', 'base', '↕', { h: 40 }, [['h', 'Altezza (px)', 'number', 1]]],
    video: ['Video', 'base', '▶', {}, [['url', 'Indirizzo YouTube o Vimeo', 'text'], ['title', 'Titolo (accessibilità)', 'text']]],
    list: ['Elenco', 'base', '☰', { items: 'Primo punto\nSecondo punto\nTerzo punto' }, [['items', 'Voci (una per riga)', 'textarea'], ['marker', 'Segno', 'select', { dot: 'Punto', check: 'Spunta', number: 'Numeri' }]]],
    hero: ['Apertura', 'news', '★', { mode: 'featured' }, [['mode', 'Quale articolo', 'select', { featured: 'Il primo in evidenza (o il più recente)', latest: 'Il più recente' }], CAT, ['layout', 'Aspetto', 'select', { side: 'Foto e testo affiancati', overlay: 'Titolo sopra la foto' }]]],
    posts_grid: ['Griglia di articoli', 'news', '▦', { title: 'Notizie', count: 6, cols: 3 }, [['title', 'Titolo del blocco', 'text'], CAT, ['count', 'Quanti articoli', 'number'], ['skip', 'Salta i primi', 'number'], ['cols', 'Colonne', 'number', 1], ['show_desc', 'Mostra il sommario', 'toggle'], ['no_img', 'Senza foto', 'toggle']]],
    posts_list: ['Elenco di articoli', 'news', '≡', { title: 'Altre notizie', count: 5 }, [['title', 'Titolo del blocco', 'text'], CAT, ['count', 'Quanti articoli', 'number'], ['skip', 'Salta i primi', 'number'], ['show_time', "Mostra l'ora", 'toggle']]],
    latest: ['Ultime notizie', 'news', '◷', { title: 'Ultime notizie', count: 8 }, [['title', 'Titolo del blocco', 'text'], CAT, ['count', 'Quanti articoli', 'number']]],
    most_read: ['I più letti', 'news', '↗', { title: 'I più letti', count: 5 }, [['title', 'Titolo del blocco', 'text'], ['count', 'Quanti articoli', 'number']]],
    newsletter: ['Newsletter', 'news', '✉', {}, []],
    form: ['Modulo', 'news', '✎', { form: '' }, [['form', 'Quale modulo', 'select', Object.assign({ '': 'Scegli un modulo…' }, Object.fromEntries((data.forms || []).map(f => [String(f[0]), f[1]])))]]],
    ad: ['Pubblicità', 'news', '$', { slot: 'ad_top' }, [['slot', 'Codice', 'select', { ad_top: 'Spazio in alto', ad_bottom: 'Spazio in basso', ad_inarticle: 'Spazio nel testo' }]]],
    html: ['HTML', 'news', '</>', { code: '' }, [['code', 'Codice HTML', 'textarea']]],
    tabs: ['Schede', 'pro', '⊟', { items: 'Prima scheda | Il testo della prima scheda.\nSeconda scheda | Il testo della seconda.' }, [['title', 'Titolo del blocco', 'text'], ['items', 'Schede (una per riga: Titolo | testo; // va a capo)', 'textarea']]],
    accordion: ['Fisarmonica', 'pro', '▾', { items: 'Prima domanda | La risposta.\nSeconda domanda | Un\'altra risposta.', first_open: true }, [['title', 'Titolo del blocco', 'text'], ['items', 'Voci (una per riga: Domanda | risposta; // va a capo)', 'textarea'], ['first_open', 'La prima voce è aperta', 'toggle']]],
    carousel: ['Carosello di articoli', 'pro', '⇆', { title: 'Da non perdere', count: 8 }, [['title', 'Titolo del blocco', 'text'], ['category', 'Categoria', 'select', Object.assign({ '': 'Tutte le categorie' }, Object.fromEntries(cats.map(c => [c, c])))], ['count', 'Quanti articoli', 'number'], ['skip', 'Salta i primi', 'number'], ['show_desc', 'Mostra il sommario', 'toggle']]],
    gallery: ['Galleria', 'pro', '▤', { cols: 3 }, [['title', 'Titolo del blocco', 'text'], ['images', 'Foto (una per riga)', 'images'], ['cols', 'Colonne', 'number', 1]]],
    counter: ['Numero', 'pro', '#', { number: '120', label: 'articoli al mese' }, [['prefix', 'Prima del numero', 'text'], ['number', 'Numero', 'text'], ['suffix', 'Dopo il numero', 'text'], ['label', 'Descrizione', 'text'], ['size', 'Dimensione (px)', 'number', 1, 'style'], ['num_color', 'Colore del numero', 'color', 0, 'style']]],
    quote: ['Citazione', 'pro', '❝', { text: 'Una frase importante da mettere in evidenza.' }, [['text', 'Citazione', 'textarea'], ['author', 'Autore', 'text'], ['role', 'Ruolo', 'text'], ['photo', 'Foto', 'image']]],
    card: ['Scheda', 'pro', '▯', { heading: 'Titolo della scheda', label: 'Scopri di più', url: '#' }, [['src', 'Immagine', 'image'], ['alt', 'Testo alternativo', 'text'], ['heading', 'Titolo', 'text'], ['text', 'Testo', 'textarea'], ['label', 'Pulsante', 'text'], ['url', 'Link del pulsante', 'text']]],
    logo: ['Logo', 'site', '◎', { tagline: true }, [['show', 'Mostra', 'select', { logo: 'Il logo (se caricato nelle Impostazioni)', name: 'Il nome del sito' }], ['tagline', 'Mostra la descrizione del sito', 'toggle'], ['h', 'Altezza del logo (px)', 'number', 1, 'style']]],
    menu: ['Menu', 'site', '☰', { upper: true }, [['source', 'Voci', 'select', { site: 'Quelle del menu (Impostazioni)', cats: 'Le sezioni principali', custom: 'Scritte qui sotto' }], ['items', 'Voci (una per riga: Nome | indirizzo)', 'textarea'], ['upper', 'Maiuscolo', 'toggle'], ['no_burger', 'Sul telefono senza il pulsante «Menu»', 'toggle'], ['size', 'Dimensione del testo (px)', 'number', 1, 'style']]],
    search: ['Ricerca', 'site', '⌕', {}, [['label', 'Testo del pulsante', 'text']]],
    social: ['Social', 'site', '@', {}, [['title', 'Titolo', 'text'], ['style', 'Aspetto', 'select', { brand: 'Colori dei social', plain: 'Sobrio' }]]],
    categories: ['Sezioni', 'site', '⋮', { title: 'Sezioni' }, [['title', 'Titolo', 'text']]],
    copyright: ['Copyright', 'site', '©', {}, [['text', 'Testo aggiunto dopo il nome del sito', 'text'], ['no_legal', 'Senza le note legali delle Impostazioni', 'toggle']]],
  };
  const ADV = [['mt', 'Margine sopra (px)', 'number', 1, 'adv'], ['mb', 'Margine sotto (px)', 'number', 1, 'adv'], ['pad', 'Spazio interno (px)', 'number', 1, 'adv'], ['bg', 'Sfondo', 'color', 0, 'adv'], ['color', 'Colore del testo', 'color', 0, 'adv'], ['radius', 'Angoli arrotondati (px)', 'number', 0, 'adv'], ['align', 'Allineamento', 'select', { '': 'Come il tema', left: 'Sinistra', center: 'Centro', right: 'Destra' }, 'adv'], ['hide_d', 'Nascondi sul computer', 'toggle', 0, 'adv'], ['hide_t', 'Nascondi sul tablet', 'toggle', 0, 'adv'], ['hide_m', 'Nascondi sul telefono', 'toggle', 0, 'adv']];
  const SEC = [['layout', 'Colonne', 'layout'], ['full', 'Larghezza piena', 'toggle'], ['max', 'Larghezza massima del contenuto (px)', 'number'], ['gap', 'Spazio tra le colonne (px)', 'number'], ['pad_y', 'Spazio sopra e sotto (px)', 'number', 1, 'style'], ['bg_img', 'Immagine di sfondo', 'image', 0, 'style'], ['stack_t', 'Colonne una sotto l\'altra sul tablet', 'toggle', 0, 'adv']];
  const COL = [['width', 'Larghezza (%, vuoto = automatica)', 'number']];
  const LAYOUTS = [[100], [50, 50], [66, 34], [34, 66], [33, 33, 34], [25, 25, 25, 25]];
  // ---------- struttura ----------
  function find(id, d = doc) {
    for (const [si, s] of d.sections.entries()) {
      if (s.id === id) return { kind: 'section', node: s, list: d.sections, i: si };
      for (const [ci, c] of s.columns.entries()) {
        if (c.id === id) return { kind: 'col', node: c, list: s.columns, i: ci, sec: s };
        for (const [wi, w] of c.widgets.entries()) if (w.id === id) return { kind: 'widget', node: w, list: c.widgets, i: wi, col: c };
      }
    }
    return null;
  }
  const newWidget = t => ({ id: uid('w'), type: t, settings: JSON.parse(JSON.stringify(W[t][3])) });
  const newSection = (widths = [100]) => ({ id: uid('s'), settings: {}, columns: widths.map(x => ({ id: uid('c'), settings: { width: widths.length > 1 ? x : 0 }, widgets: [] })) });
  const clone = n => { const c = JSON.parse(JSON.stringify(n)); const re = o => { if (o.id) o.id = uid(o.id[0]); (o.columns || []).forEach(re); (o.widgets || []).forEach(re); }; re(c); return c; };
  function change(fn) { hist.push(JSON.stringify(doc)); if (hist.length > 60) hist.shift(); fut = []; fn(); dirty = true; status(); render(); }
  function render() {
    clearTimeout(timer);
    timer = setTimeout(async () => {
      const r = await fetch('/admin/builder/render' + Q, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(doc) });
      const j = await r.json(); frame.contentWindow.postMessage({ pb: 1, type: 'html', html: j.html, sel }, location.origin);
    }, 120);
  }
  // ---------- messaggi dall'anteprima ----------
  addEventListener('message', e => {
    if (e.origin !== location.origin || !e.data || !e.data.pb || e.source !== frame.contentWindow) return;
    const m = e.data;
    if (m.type === 'ready') render();
    if (m.type === 'select') { sel = m.id; panel(); }
    if (m.type === 'op') op(m.op, m.id);
    if (m.type === 'text') { const f = find(m.id), w = f && (f.node || f.list[f.i]); if (w) { change(() => { w.settings = w.settings || {}; w.settings[m.field] = m.value; sel = w.id; }); panel(); } }
    if (m.type === 'rerender') render();
    if (m.type === 'pick') { const f = find(m.id), w = f && (f.node || f.list[f.i]); if (w) askImage(w); }
    if (m.type === 'drop') change(() => {
      let w;
      if (m.src.move) { const f = find(m.src.move); if (!f) return; w = f.list.splice(f.i, 1)[0]; }
      else if (W[m.src.add]) w = newWidget(m.src.add); else return;
      const t = find(m.target.id); if (!t) return;
      if (t.kind === 'col') t.node.widgets.push(w); else t.list.splice(t.i + (m.target.pos === 'after' ? 1 : 0), 0, w);
      sel = w.id; panel(); if (!m.src.move) afterAdd(w);
    });
  });
  function op(o, id) {
    const f = find(id); if (!f && o !== 'addsec') return;
    if (o === 'savesec') return saveBlock(f.node);
    change(() => {
      if (o === 'del') { f.list.splice(f.i, 1); if (f.kind === 'col' && !f.list.length) f.list.push(newSection().columns[0]); sel = null; }
      if (o === 'dup') { const c = clone(f.node); f.list.splice(f.i + 1, 0, c); sel = c.id; }
      if (o === 'up' && f.i > 0) [f.list[f.i - 1], f.list[f.i]] = [f.list[f.i], f.list[f.i - 1]];
      if (o === 'down' && f.i < f.list.length - 1) [f.list[f.i + 1], f.list[f.i]] = [f.list[f.i], f.list[f.i + 1]];
      if (o === 'addsec') { const s = newSection(); doc.sections.splice(f ? f.i + 1 : doc.sections.length, 0, s); sel = s.id; }
      panel();
    });
  }
  // ---------- pannello delle impostazioni ----------
  let tab = 'content';
  function field([key, label, kind, opt, t], node) {
    const resp = opt === 1 && kind === 'number', k = resp ? key + device : key, v = node.settings[k] ?? '';
    const devName = { '': 'computer', _t: 'tablet', _m: 'telefono' }[device];
    const wrap = document.createElement('label'); wrap.className = 'pbf-field';
    const head = document.createElement('span'); head.textContent = label; wrap.append(head);
    if (resp) { const b = document.createElement('small'); b.className = 'pb-dev'; b.textContent = 'per ' + devName; head.append(b); }
    let input;
    const set = val => {
      const now = node.settings[k], empty = val === '' || val === false;
      if ((empty && now === undefined) || (!empty && now !== undefined && String(now) === String(val))) return; // nessun cambiamento: niente nella cronologia
      change(() => { if (empty) delete node.settings[k]; else node.settings[k] = val; });
    };
    if (kind === 'textarea') { input = document.createElement('textarea'); input.rows = key === 'code' ? 8 : 5; input.value = v; input.onchange = () => set(input.value); }
    else if (kind === 'select') { input = document.createElement('select'); for (const [a, b] of Object.entries(opt)) { const o = new Option(b, a); input.append(o); } input.value = v; input.onchange = () => set(input.value); }
    else if (kind === 'toggle') { wrap.className = 'pbf-field pbf-tog'; input = document.createElement('input'); input.type = 'checkbox'; input.checked = v === true || v === 'on'; input.onchange = () => set(input.checked); wrap.prepend(input); return wrap; }
    else if (kind === 'color') { const row = document.createElement('span'); row.className = 'pbf-color'; input = document.createElement('input'); input.type = 'color'; input.value = /^#[0-9a-f]{6}$/i.test(v) ? v : '#ffffff'; const clr = document.createElement('button'); clr.type = 'button'; clr.textContent = v ? 'Togli' : 'Nessuno'; clr.onclick = () => set(''); input.onchange = () => set(input.value); row.append(input, clr); wrap.append(row); return wrap; }
    else if (kind === 'images') { input = document.createElement('textarea'); input.rows = 5; input.value = v; input.placeholder = '/media/…'; input.onchange = () => set(input.value.trim()); const b = document.createElement('button'); b.type = 'button'; b.className = 'btn ghost small'; b.textContent = 'Aggiungi dalla libreria'; b.onclick = () => pick(url => set(((node.settings[k] || '') + '\n' + url).trim())); wrap.append(input, b); return wrap; }
    else if (kind === 'image') { const row = document.createElement('span'); row.className = 'pbf-color'; input = document.createElement('input'); input.value = v; input.placeholder = '/media/…'; input.onchange = () => set(input.value.trim()); const b = document.createElement('button'); b.type = 'button'; b.textContent = 'Libreria'; b.onclick = () => pick(url => set(url)); row.append(input, b); wrap.append(row); return wrap; }
    else if (kind === 'layout') {
      input = document.createElement('span'); input.className = 'pbf-layouts';
      for (const L of LAYOUTS) { const b = document.createElement('button'); b.type = 'button'; b.innerHTML = L.map(x => '<i style="flex:' + x + '"></i>').join(''); b.title = L.join(' / ') + '%'; b.onclick = () => change(() => { const old = node.columns; node.columns = L.map((x, i) => ({ id: (old[i] || {}).id || uid('c'), settings: { width: L.length > 1 ? x : 0 }, widgets: [] })); old.forEach((c, i) => node.columns[Math.min(i, L.length - 1)].widgets.push(...c.widgets)); }); input.append(b); }
      wrap.append(input); return wrap;
    }
    else { input = document.createElement('input'); input.type = kind === 'number' ? 'number' : 'text'; input.value = v; input.onchange = () => set(kind === 'number' && input.value !== '' ? Number(input.value) : input.value); }
    wrap.append(input); return wrap;
  }
  function panel() {
    const box = $('#pb-props'); box.textContent = '';
    const f = sel && find(sel);
    if (!f) { box.innerHTML = '<div class="pbf-hint"><b>Clicca un elemento</b> nell\'anteprima per modificarlo, oppure trascina un widget dalla colonna a sinistra dentro una colonna.</div>'; $('#pb-props-title').textContent = 'Impostazioni'; return; }
    const fields = f.kind === 'section' ? SEC.concat(ADV) : f.kind === 'col' ? COL.concat(ADV) : W[f.node.type][4].concat(ADV);
    $('#pb-props-title').textContent = f.kind === 'section' ? 'Sezione' : f.kind === 'col' ? 'Colonna' : W[f.node.type][0];
    const tabs = document.createElement('div'); tabs.className = 'pbf-tabs';
    for (const [id, label] of [['content', 'Contenuto'], ['style', 'Stile'], ['adv', 'Avanzate']]) { const b = document.createElement('button'); b.type = 'button'; b.textContent = label; b.className = tab === id ? 'on' : ''; b.onclick = () => { tab = id; panel(); }; tabs.append(b); }
    box.append(tabs);
    const list = fields.filter(x => (x[4] || 'content') === tab);
    if (!list.length) { const p = document.createElement('p'); p.className = 'muted'; p.textContent = 'Niente da impostare qui.'; box.append(p); }
    list.forEach(x => box.append(field(x, f.node)));
  }
  // ---------- i miei blocchi: sezioni salvate da riusare in qualsiasi pagina ----------
  let blocks = data.blocks || [];
  async function saveBlock(section) {
    const name = prompt('Nome del blocco (per ritrovarlo in «I miei blocchi»):', 'Sezione ' + (blocks.length + 1)); if (!name) return;
    const r = await (await fetch('/admin/builder/blocchi', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name, section }) })).json();
    if (r.error) return alert(r.error);
    blocks = r.blocks; drawBlocks(); $('#pb-status').textContent = 'Blocco «' + name + '» salvato: lo trovi a sinistra, in «I miei blocchi».';
  }
  function drawBlocks() {
    const box = $('#pb-blocks'); box.textContent = '';
    if (!blocks.length) { box.innerHTML = '<p class="muted pb-help" style="margin:0">Nessun blocco ancora: seleziona una sezione e premi ★ nella barra blu.</p>'; return; }
    for (const b of blocks) {
      const row = document.createElement('div'); row.className = 'pb-block';
      const add = document.createElement('button'); add.type = 'button'; add.textContent = b.name; add.title = 'Inserisci una copia in questa pagina';
      add.onclick = () => change(() => { const c = clone(b.data), f = sel && find(sel), at = f && f.kind === 'section' ? f.i + 1 : doc.sections.length; doc.sections.splice(at, 0, c); sel = c.id; panel(); });
      const del = document.createElement('button'); del.type = 'button'; del.className = 'del'; del.textContent = '✕'; del.title = 'Elimina il blocco salvato (le pagine che lo usano non cambiano)';
      del.onclick = async () => { if (!confirm('Eliminare il blocco «' + b.name + '»? Le pagine in cui l\'hai già inserito non cambiano.')) return; blocks = (await (await fetch('/admin/builder/blocchi/' + b.id + '/elimina', { method: 'POST' })).json()).blocks; drawBlocks(); };
      row.append(add, del); box.append(row);
    }
  }
  // ---------- libreria media (per immagini e sfondi) ----------
  async function pick(done) {
    const d = $('#pb-pick'), g = $('#pb-pick-grid'); g.textContent = 'Carico…'; d.showModal();
    const r = await (await fetch('/admin/media/list?page=0&q=')).json(); g.textContent = '';
    r.items.forEach(it => { const b = document.createElement('button'); b.type = 'button'; b.innerHTML = '<img src="' + it.thumb + '" alt="">'; b.onclick = () => { d.close(); done(it.url); }; g.append(b); });
    if (!r.items.length) g.textContent = 'La libreria è vuota: carica le immagini da Media.';
  }
  $('#pb-pick-close').onclick = () => $('#pb-pick').close();
  // Immagine: la libreria si apre dal segnaposto nell'anteprima e subito dopo aver aggiunto il widget.
  function askImage(w) { pick(url => change(() => { w.settings = w.settings || {}; w.settings.src = url; sel = w.id; }), panel); }
  function afterAdd(w) { if (w.type === 'image' && !(w.settings && w.settings.src)) setTimeout(() => askImage(w), 60); }
  // ---------- barra in alto ----------
  function status() { $('#pb-status').textContent = dirty ? 'Modifiche non salvate' : 'Tutto salvato'; $('#pb-undo').disabled = !hist.length; $('#pb-redo').disabled = !fut.length; }
  $('#pb-undo').onclick = () => { if (!hist.length) return; fut.push(JSON.stringify(doc)); doc = JSON.parse(hist.pop()); dirty = true; status(); render(); panel(); };
  $('#pb-redo').onclick = () => { if (!fut.length) return; hist.push(JSON.stringify(doc)); doc = JSON.parse(fut.pop()); dirty = true; status(); render(); panel(); };
  document.querySelectorAll('[data-dev]').forEach(b => b.onclick = () => {
    device = b.dataset.dev; document.querySelectorAll('[data-dev]').forEach(x => x.classList.toggle('on', x === b));
    fit(); panel();
  });
  // L'anteprima si disegna alla larghezza vera del dispositivo (computer 1280 px) e si rimpicciolisce per stare nello spazio:
  // così in vista computer si vede il sito da computer, non da telefono.
  function fit() {
    const stage = $('.pb-stage'), W = { '': 1280, _t: 820, _m: 390 }[device], sc = Math.min(1, (stage.clientWidth - 20) / W);
    Object.assign(frame.style, { width: W + 'px', marginLeft: (-W / 2) + 'px', transform: sc < 1 ? 'scale(' + sc + ')' : 'none', height: ((stage.clientHeight - 20) / sc) + 'px' });
  }
  addEventListener('resize', fit); fit();
  async function save(publish) {
    const r = await fetch('/admin/builder/salva' + Q, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ doc, publish }) });
    const j = await r.json(); if (j.error) return alert(j.error);
    dirty = false; status(); $('#pb-status').textContent = j.msg;
  }
  $('#pb-save').onclick = () => save(false);
  $('#pb-publish').onclick = () => save(true);
  $('#pb-addsec').onclick = () => op('addsec', doc.sections.length ? doc.sections[doc.sections.length - 1].id : null);
  $('#pb-tpl').onchange = async e => {
    const t = e.target.value; e.target.value = ''; if (!t || !confirm('Sostituire la pagina con il modello? Puoi sempre annullare con ↶.')) return;
    const j = await (await fetch('/admin/builder/modello/' + t + Q)).json(); change(() => { doc = j; sel = null; }); panel();
  };
  // Widget: si trascinano nell'anteprima, oppure un clic li aggiunge alla colonna scelta (o in fondo).
  const pal = $('#pb-palette');
  const ICON = {"hero": "<rect x=\"3\" y=\"4\" width=\"18\" height=\"10\" rx=\"2\"/><path d=\"M3 18h12M3 21h8\"/>", "posts_grid": "<rect x=\"3\" y=\"3\" width=\"7\" height=\"7\" rx=\"1.5\"/><rect x=\"14\" y=\"3\" width=\"7\" height=\"7\" rx=\"1.5\"/><rect x=\"3\" y=\"14\" width=\"7\" height=\"7\" rx=\"1.5\"/><rect x=\"14\" y=\"14\" width=\"7\" height=\"7\" rx=\"1.5\"/>", "posts_list": "<rect x=\"3\" y=\"4\" width=\"6\" height=\"5\" rx=\"1\"/><rect x=\"3\" y=\"15\" width=\"6\" height=\"5\" rx=\"1\"/><path d=\"M12 5h9M12 8h6M12 16h9M12 19h6\"/>", "latest": "<circle cx=\"12\" cy=\"12\" r=\"9\"/><path d=\"M12 7v5l3 2\"/>", "most_read": "<path d=\"M3 17l6-6 4 4 8-8\"/><path d=\"M15 7h6v6\"/>", "newsletter": "<rect x=\"3\" y=\"5\" width=\"18\" height=\"14\" rx=\"2\"/><path d=\"m3 7 9 6 9-6\"/>", "form": "<rect x=\"4\" y=\"3\" width=\"16\" height=\"18\" rx=\"2\"/><path d=\"M8 8h8M8 12h8M8 16h5\"/>", "ad": "<path d=\"M3 11v2a1 1 0 0 0 1 1h2l5 4V6L6 10H4a1 1 0 0 0-1 1z\"/><path d=\"M15 9a4 4 0 0 1 0 6M18 6a8 8 0 0 1 0 12\"/>", "html": "<path d=\"m8 8-4 4 4 4M16 8l4 4-4 4M13.5 5l-3 14\"/>", "heading": "<path d=\"M6 4v16M18 4v16M6 12h12\"/>", "text": "<path d=\"M4 6h16M4 10h16M4 14h16M4 18h10\"/>", "image": "<rect x=\"3\" y=\"4\" width=\"18\" height=\"16\" rx=\"2\"/><circle cx=\"9\" cy=\"10\" r=\"2\"/><path d=\"m21 17-5-5-9 8\"/>", "button": "<rect x=\"3\" y=\"8\" width=\"18\" height=\"8\" rx=\"4\"/><path d=\"M9 12h6\"/>", "divider": "<path d=\"M3 12h18\"/>", "spacer": "<path d=\"M12 4v16M8 7l4-3 4 3M8 17l4 3 4-3\"/>", "video": "<rect x=\"3\" y=\"5\" width=\"18\" height=\"14\" rx=\"2\"/><path d=\"m10 9 5 3-5 3z\"/>", "list": "<path d=\"M9 6h11M9 12h11M9 18h11\"/><circle cx=\"4.5\" cy=\"6\" r=\"1\"/><circle cx=\"4.5\" cy=\"12\" r=\"1\"/><circle cx=\"4.5\" cy=\"18\" r=\"1\"/>", "tabs": "<path d=\"M3 9h18v10H3zM3 9V6h6v3M9 6h6v3\"/>", "accordion": "<rect x=\"3\" y=\"4\" width=\"18\" height=\"5\" rx=\"1\"/><rect x=\"3\" y=\"12\" width=\"18\" height=\"8\" rx=\"1\"/>", "carousel": "<rect x=\"6\" y=\"5\" width=\"12\" height=\"14\" rx=\"2\"/><path d=\"M3 8v8M21 8v8\"/>", "gallery": "<rect x=\"3\" y=\"3\" width=\"8\" height=\"8\" rx=\"1.5\"/><rect x=\"13\" y=\"3\" width=\"8\" height=\"8\" rx=\"1.5\"/><rect x=\"3\" y=\"13\" width=\"18\" height=\"8\" rx=\"1.5\"/>", "counter": "<path d=\"M4 9h16M4 15h16M10 3 8 21M16 3l-2 18\"/>", "quote": "<path d=\"M7 7h4v4c0 3-2 5-4 6M15 7h4v4c0 3-2 5-4 6\"/>", "card": "<rect x=\"4\" y=\"3\" width=\"16\" height=\"18\" rx=\"2\"/><path d=\"M4 11h16M8 15h8M8 18h5\"/>", "logo": "<path d=\"m12 3 2.6 5.6 6.1.7-4.5 4.2 1.2 6L12 16.6l-5.4 2.9 1.2-6-4.5-4.2 6.1-.7z\"/>", "menu": "<path d=\"M4 6h16M4 12h16M4 18h16\"/>", "search": "<circle cx=\"11\" cy=\"11\" r=\"7\"/><path d=\"m20 20-3.5-3.5\"/>", "social": "<circle cx=\"18\" cy=\"5\" r=\"3\"/><circle cx=\"6\" cy=\"12\" r=\"3\"/><circle cx=\"18\" cy=\"19\" r=\"3\"/><path d=\"m8.6 13.5 6.8 4M15.4 6.5l-6.8 4\"/>", "categories": "<path d=\"M3 12V4h8l10 10-8 8z\"/><circle cx=\"7.5\" cy=\"7.5\" r=\"1.5\"/>", "copyright": "<circle cx=\"12\" cy=\"12\" r=\"9\"/><path d=\"M15 9.5a4 4 0 1 0 0 5\"/>"};
  const svg = t => ICON[t] ? '<svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + ICON[t] + '</svg>' : null;
  { const find = document.createElement('input'); find.type = 'search'; find.id = 'pb-find'; find.placeholder = 'Cerca un widget…'; find.setAttribute('aria-label', 'Cerca un widget'); pal.before(find);
    find.oninput = () => { const q = find.value.trim().toLowerCase(); pal.querySelectorAll('.pb-pal-grid').forEach(g => { let n = 0; g.querySelectorAll('button').forEach(b => { const ok = !q || b.textContent.toLowerCase().includes(q); b.hidden = !ok; n += ok; }); g.hidden = !n; g.previousElementSibling.hidden = !n; }); }; }
  for (const [group, title] of (data.name === 'header' || data.name === 'footer' ? [['site', 'Testata e piede'], ['base', 'Base'], ['pro', 'Avanzati'], ['news', 'Notizie']] : [['news', 'Notizie'], ['base', 'Base'], ['pro', 'Avanzati'], ['site', 'Testata e piede']])) {
    const h = document.createElement('h3'); h.textContent = title; pal.append(h);
    const grid = document.createElement('div'); grid.className = 'pb-pal-grid'; pal.append(grid);
    for (const [t, w] of Object.entries(W).filter(([, w]) => w[1] === group)) {
      const b = document.createElement('button'); b.type = 'button'; b.draggable = true; b.dataset.type = t;
      b.innerHTML = '<i>' + (svg(t) || w[2].replace('<', '&lt;')) + '</i><span>' + w[0] + '</span>';
      b.ondragstart = e => { e.dataTransfer.setData('text/plain', 'new:' + t); e.dataTransfer.effectAllowed = 'copy'; };
      b.onclick = () => change(() => {
        const f = sel && find(sel), w = newWidget(t);
        if (f && f.kind === 'col') f.node.widgets.push(w); else if (f && f.kind === 'widget') f.list.splice(f.i + 1, 0, w);
        else { if (!doc.sections.length) doc.sections.push(newSection()); const s = doc.sections[doc.sections.length - 1]; s.columns[s.columns.length - 1].widgets.push(w); }
        sel = w.id; panel(); afterAdd(w);
      });
      grid.append(b);
    }
  }
  { const h = document.createElement('h3'); h.textContent = 'I miei blocchi'; const box = document.createElement('div'); box.id = 'pb-blocks'; pal.append(h, box); drawBlocks(); }
  addEventListener('beforeunload', e => { if (dirty) e.preventDefault(); });
  addEventListener('keydown', e => { if ((e.ctrlKey || e.metaKey) && e.key === 'z') { e.preventDefault(); $(e.shiftKey ? '#pb-redo' : '#pb-undo').click(); } if ((e.ctrlKey || e.metaKey) && e.key === 's') { e.preventDefault(); save(false); } });
  status(); panel();
})();
