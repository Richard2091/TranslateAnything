// 沉浸模式注入脚本
//
// 运行在目标页面的隔离环境（isolated world）里：和页面自己的脚本互相看不见，只共享 DOM，
// 不会污染页面的全局变量，也不受页面内容安全策略限制。
//
// 工作方式：
//   遍历 DOM 文本节点和 placeholder/title 属性 → 命中页面内预加载/缓存当场替换；
//   未命中的保留阅读顺序与轻量上下文（组件角色、内联拆句同组号），通过 CDP binding（__tfSend）交给随译主程序，
//   主程序经统一过滤、本地译文库或翻译引擎后调用 __tf.receive() 回填。
(() => {
  'use strict';
  if (globalThis.__tf) globalThis.__tf.dispose(true);

  const CFG = /*__TF_CONFIG__*/ {};

  const SKIP_TAGS = new Set([
    'script', 'style', 'noscript', 'template', 'textarea', 'input', 'select',
    'code', 'pre', 'kbd', 'samp', 'var', 'svg', 'math', 'canvas',
  ]);
  const EDITABLE = new Set(['', 'true', 'plaintext-only']);
  const NO_TRANSLATE = [':not(html):not(body)[translate="no"]', ':not(html):not(body).notranslate'];
  // 内置跳过：代码编辑器、终端、常见代码高亮块与图标字体节点
  const BUILTIN_SKIP = [
    '.monaco-editor .view-lines',
    '.xterm-rows', '.xterm-accessibility',
    '.CodeMirror-code',
    '.ace_text-layer',
    '.hljs', '.prism', '.shiki', '[class*="code-block"]', '[class*="codeblock"]',
    '.codicon', '.material-icons', '.material-symbols-outlined', '.fa', '.fas', '.far',
  ];
  const CUSTOM_SKIP = [...BUILTIN_SKIP, ...(CFG.skipSelectors || [])].join(',');
  const SKIP = [
    ...SKIP_TAGS,
    '[contenteditable=""]', '[contenteditable="true"]', '[contenteditable="plaintext-only"]',
    ...NO_TRANSLATE,
    ...BUILTIN_SKIP,
    ...(CFG.skipSelectors || []),
  ].join(',');
  const ATTR_SKIP = [
    '[contenteditable=""]', '[contenteditable="true"]', '[contenteditable="plaintext-only"]',
    ...NO_TRANSLATE,
    ...(CFG.skipSelectors || []),
  ].join(',');
  const ATTRS = ['placeholder', 'title'];
  const BATCH = 200;
  const SLICE = 8;
  const LETTER = /\p{L}/gu;
  const MONO_FONT = /monospace|\b(?:consolas|courier|menlo|monaco|fira code|jetbrains mono|cascadia|source code)\b/i;
  const CODE_MARKERS = /[{}[\]();=>/\\]|::|:=|[a-z][A-Z]|_[a-zA-Z]/;

  const BLOCK_TAGS = new Set([
    'p', 'li', 'td', 'th', 'dd', 'dt', 'blockquote', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
    'figcaption', 'summary', 'div', 'section', 'article', 'header', 'footer', 'aside', 'main',
  ]);
  const INLINE_TAGS = new Set(['a', 'b', 'strong', 'em', 'i', 'u', 'span', 'small', 'mark', 'sub', 'sup']);

  const REC = Symbol('tf');
  const AREC = Symbol('tfa');

  let enabled = CFG.enabled !== false;
  const cache = new Map(Object.entries(CFG.preload || {})); // 原文 → 译文
  const verdict = new Map(); // 原文 → 是否需要送主程序
  const waiting = new Map(); // key → Set(等待回填的目标)
  const queue = new Map(); // key → { text, role, group, core }（保持插入顺序）
  const inflight = new Set(); // 已发送、等待回填的 key
  const monoCache = new WeakMap(); // Element → 是否为等宽字体
  const inlineGroups = new WeakMap(); // Block Element → groupId
  let nextGroupId = 1;
  let timer = 0;
  let disposed = false;
  const TAKEOVER = '__translate_filter_takeover__';

  /** 廉价预过滤：具体格式/语言/用户规则统一交由 Rust filter.rs 判断 */
  function worth(t) {
    if (t.length < 2 || t.length > 5000) return false;
    const letters = (t.match(LETTER) || []).length;
    return letters >= 2;
  }

  function worthCore(core) {
    let v = verdict.get(core);
    if (v === undefined) {
      if (verdict.size > 50000) verdict.clear();
      verdict.set(core, (v = worth(core)));
    }
    return v;
  }

  function isMonospaceCode(el, core) {
    if (!el) return false;
    let mono = monoCache.get(el);
    if (mono === undefined) {
      try {
        mono = MONO_FONT.test(getComputedStyle(el).fontFamily || '');
      } catch {
        mono = false;
      }
      monoCache.set(el, mono);
    }
    return mono && CODE_MARKERS.test(core);
  }

  function detectRole(el, attr, core) {
    if (attr === 'placeholder') return 'ph';
    if (attr === 'title') return 'tip';
    if (!el || core.split(/\s+/).length >= 8) return null;
    let cur = el;
    for (let depth = 0; depth < 4 && cur && cur !== document.body; depth++, cur = cur.parentElement) {
      const tag = cur.localName;
      const ariaRole = cur.getAttribute('role');
      if (/^h[1-6]$/.test(tag) || ariaRole === 'heading') return 'h';
      if (tag === 'button' || ariaRole === 'button') return 'btn';
      if (tag === 'menu' || ariaRole === 'menuitem' || ariaRole === 'menu') return 'menu';
      if (tag === 'nav' || ariaRole === 'navigation') return 'nav';
      if (ariaRole === 'tab') return 'tab';
      if ((tag === 'a' && cur.hasAttribute('href')) || ariaRole === 'link') return 'link';
      if (tag === 'option' || ariaRole === 'option') return 'opt';
      if (tag === 'label') return 'lbl';
      if (BLOCK_TAGS.has(tag)) break;
    }
    return null;
  }

  function detectInlineGroup(node) {
    const parent = node.parentElement;
    if (!parent) return null;
    let block = null;
    let cur = parent;
    for (let d = 0; d < 5 && cur && cur !== document.body; d++, cur = cur.parentElement) {
      if (BLOCK_TAGS.has(cur.localName)) {
        block = cur;
        break;
      }
    }
    if (!block) return null;
    const inInline = parent !== block && INLINE_TAGS.has(parent.localName);
    if (!inInline) {
      let hasInlineChild = false;
      for (const ch of block.children) {
        if (INLINE_TAGS.has(ch.localName)) {
          hasInlineChild = true;
          break;
        }
      }
      if (!hasInlineChild) return null;
    }
    // 检查该块内是否确实有至少 2 个非空文本片段
    let textCount = 0;
    const w = document.createTreeWalker(block, NodeFilter.SHOW_TEXT);
    for (let n; (n = w.nextNode()); ) {
      if (n.data.trim().length > 0 && ++textCount >= 2) break;
    }
    if (textCount < 2) return null;
    let gid = inlineGroups.get(block);
    if (!gid) {
      gid = nextGroupId++;
      inlineGroups.set(block, gid);
    }
    return gid;
  }

  function record(v, core) {
    const lead = v.length - v.trimStart().length;
    return { src: v, a: v.slice(0, lead), core, b: v.slice(lead + core.length), shown: null };
  }

  function skipEl(el) {
    if (SKIP_TAGS.has(el.localName)) return true;
    const ce = el.getAttribute('contenteditable');
    if (ce !== null && EDITABLE.has(ce)) return true;
    const root = el === document.documentElement || el === document.body;
    if (!root && (el.getAttribute('translate') === 'no' || el.classList.contains('notranslate'))) return true;
    return CUSTOM_SKIP !== '' && el.matches(CUSTOM_SKIP);
  }

  function skippedText(el) {
    return !el || !!el.closest(SKIP);
  }

  // ───────────── 段落等待提示 ─────────────

  const PROSE = 'p, li, blockquote';
  const NOT_PROSE = 'nav, aside, menu, table, button, [role="navigation"], [role="menu"], [role="menuitem"], [role="tab"], [role="button"]';
  const loaders = new Map();
  const nodeLoaders = new WeakMap();
  const loaderElements = new WeakSet();
  const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)');

  function clearLoader(entry) {
    clearTimeout(entry.timer);
    entry.animation?.cancel();
    entry.icon?.remove();
    for (const node of entry.nodes) nodeLoaders.delete(node);
    loaders.delete(entry.block);
  }

  function settled(node) {
    const entry = nodeLoaders.get(node);
    if (!entry) return;
    nodeLoaders.delete(node);
    entry.nodes.delete(node);
    if (!entry.nodes.size) clearLoader(entry);
  }

  function trackLoading(node) {
    if (nodeLoaders.has(node)) return;
    const el = node.parentElement;
    const block = el?.closest(PROSE);
    if (!block || el.closest(NOT_PROSE)) return;
    let entry = loaders.get(block);
    if (!entry) {
      entry = { block, nodes: new Set(), timer: 0, icon: null };
      loaders.set(block, entry);
      entry.timer = setTimeout(() => showLoader(entry), 200);
    }
    entry.nodes.add(node);
    nodeLoaders.set(node, entry);
  }

  function showLoader(entry) {
    const { block } = entry;
    if (!block.isConnected) return clearLoader(entry);
    const icon = document.createElement('span');
    loaderElements.add(icon);
    icon.setAttribute('aria-hidden', 'true');
    icon.setAttribute('translate', 'no');
    icon.style.cssText = 'position:absolute;display:inline-block;box-sizing:border-box;'
      + 'width:.75em;height:.75em;margin-inline-start:.35em;margin-block-start:.22em;'
      + 'border:2px solid transparent;border-top-color:currentColor;border-inline-start-color:currentColor;'
      + 'border-radius:50%;opacity:.45;pointer-events:none;user-select:none';
    entry.icon = icon;
    block.append(icon);
    if (!reduceMotion.matches) {
      entry.animation = icon.animate([{ transform: 'rotate(0deg)' }, { transform: 'rotate(360deg)' }],
        { duration: 800, iterations: Infinity });
    }
    entry.timer = setTimeout(() => clearLoader(entry), 45000);
  }

  // ───────────── 文字节点 ─────────────

  function handleText(node, checked) {
    const v = node.data;
    const old = node[REC];
    if (old && v === old.shown) return;
    const loader = nodeLoaders.get(node);
    if (loader && (v !== old?.src || !loader.block.contains(node))) settled(node);
    const core = v.trim();
    const el = node.parentElement;
    if (!worthCore(core) || (!checked && skippedText(el)) || isMonospaceCode(el, core)) {
      settled(node);
      if (old) node[REC] = undefined;
      return;
    }
    const r = record(v, core);
    node[REC] = r;
    const group = detectInlineGroup(node);
    const tr = group ? undefined : cache.get(core);
    if (tr !== undefined) {
      settled(node);
      applyText(node, r, tr);
    } else {
      want(core, node, null, detectRole(el, null, core), group);
    }
  }

  function applyText(node, r, tr) {
    if (node[REC] !== r || node.data !== r.src) return;
    r.shown = r.a + tr + r.b;
    if (r.shown !== r.src) node.data = r.shown;
  }

  // ───────────── 属性 ─────────────

  function handleAttr(el, name) {
    const v = el.getAttribute(name);
    const m = el[AREC];
    const old = m && m[name];
    if (old && v === old.shown) return;
    const core = v == null ? '' : v.trim();
    if (!worthCore(core) || el.closest(ATTR_SKIP)) {
      if (old) delete m[name];
      return;
    }
    const r = record(v, core);
    (m || (el[AREC] = {}))[name] = r;
    const tr = cache.get(core);
    if (tr !== undefined) applyAttr(el, name, r, tr);
    else want(core, el, name, detectRole(el, name, core), null);
  }

  function applyAttr(el, name, r, tr) {
    const m = el[AREC];
    if (!m || m[name] !== r || el.getAttribute(name) !== r.src) return;
    r.shown = r.a + tr + r.b;
    if (r.shown !== r.src) el.setAttribute(name, r.shown);
  }

  function eachRecord(fn) {
    const roots = [document.documentElement];
    while (roots.length) {
      const root = roots.pop();
      if (!root) continue;
      const w = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT);
      for (let n = w.currentNode; n; n = w.nextNode()) {
        if (n.nodeType === 3) {
          if (n[REC]) fn(n, null, n[REC]);
          continue;
        }
        const m = n[AREC];
        if (m) for (const name of Object.keys(m)) fn(n, name, m[name]);
        if (n.shadowRoot) roots.push(n.shadowRoot);
      }
    }
  }

  // ───────────── 与主程序通信 ─────────────

  function itemKey(core, group) {
    return group != null ? `${group}\u0000${core}` : core;
  }

  function want(core, target, attr, role, group) {
    const key = itemKey(core, group);
    let set = waiting.get(key);
    if (!set) waiting.set(key, (set = new Set()));
    set.add(attr ? { el: target, attr } : { node: target });
    if (!attr) trackLoading(target);
    if (!inflight.has(key)) {
      if (!queue.has(key)) {
        const entry = { text: core, key };
        if (role) entry.role = role;
        if (group != null) entry.group = group;
        queue.set(key, entry);
      }
      if (!timer) timer = setTimeout(flush, 30);
    }
  }

  function flush() {
    timer = 0;
    if (!queue.size || !enabled) return;
    const entries = [...queue.values()].slice(0, BATCH);
    for (const it of entries) {
      queue.delete(it.key);
      inflight.add(it.key);
    }
    const items = entries.map(({ text, role, group }) => {
      const o = { text };
      if (role) o.role = role;
      if (group != null) o.group = group;
      return o;
    });
    try {
      globalThis.__tfSend(JSON.stringify({ title: document.title || '', items }));
    } catch (e) {
      for (const it of entries) {
        inflight.delete(it.key);
        queue.set(it.key, it);
      }
      timer = setTimeout(flush, 500);
      return;
    }
    if (queue.size) timer = setTimeout(flush, 30);
  }

  function applyWaiting(key, tr, isKeep) {
    inflight.delete(key);
    const set = waiting.get(key);
    waiting.delete(key);
    if (!set || !enabled) return;
    for (const t of set) {
      if (t.node) {
        const r = t.node[REC];
        if (r && itemKey(r.core, null) === key || (r && key.endsWith(`\u0000${r.core}`))) {
          if (t.node.data === r.src) {
            settled(t.node);
            if (!isKeep) applyText(t.node, r, tr);
          }
        }
      } else {
        const r = t.el[AREC]?.[t.attr];
        if (r && r.core === key && !isKeep) applyAttr(t.el, t.attr, r, tr);
      }
    }
  }

  /** 主程序回填译文：{ map: { 原文: 译文 }, grouped?: { "gid\0原文": 译文 }, keep?: [原文] } */
  function receive(msg) {
    if (disposed) return;
    for (const src of msg.keep || []) {
      verdict.set(src, false);
      applyWaiting(src, src, true);
    }
    for (const [src, tr] of Object.entries(msg.map || {})) {
      cache.set(src, tr);
      applyWaiting(src, tr, false);
    }
    for (const [gkey, tr] of Object.entries(msg.grouped || {})) {
      applyWaiting(gkey, tr, false);
    }
  }

  /** 翻译失败：过一会儿重试仍在等待的条目 */
  function failed(msg) {
    const keys = msg.keys || msg.texts || [];
    for (const k of keys) {
      inflight.delete(k);
      for (const { node } of waiting.get(k) || []) {
        if (node?.[REC]) settled(node);
      }
    }
    setTimeout(() => {
      for (const k of keys) {
        if (waiting.has(k) && !inflight.has(k) && !queue.has(k)) {
          const sep = k.indexOf('\u0000');
          if (sep >= 0) {
            queue.set(k, { text: k.slice(sep + 1), group: Number(k.slice(0, sep)), key: k });
          } else {
            queue.set(k, { text: k, key: k });
          }
        }
      }
      if (queue.size && !timer) timer = setTimeout(flush, 30);
    }, 10000);
  }

  // ───────────── 遍历与监听（分片） ─────────────

  let work = [];
  let head = 0;
  let scheduled = false;
  const channel = new MessageChannel();
  channel.port1.onmessage = () => {
    scheduled = false;
    pump();
  };

  function later() {
    if (!scheduled) {
      scheduled = true;
      channel.port2.postMessage(0);
    }
  }

  function visitElement(el) {
    if (el.hasAttribute('placeholder')) handleAttr(el, 'placeholder');
    if (el.hasAttribute('title')) handleAttr(el, 'title');
    if (el.shadowRoot) {
      observe(el.shadowRoot);
      work.push({ root: el.shadowRoot });
    }
    return !skipEl(el);
  }

  const FILTER = {
    acceptNode(n) {
      if (n.nodeType === 3) return NodeFilter.FILTER_ACCEPT;
      return visitElement(n) ? NodeFilter.FILTER_SKIP : NodeFilter.FILTER_REJECT;
    },
  };

  function startRoot(n) {
    if (!n.isConnected) return null;
    if (n.nodeType === 3) {
      handleText(n, false);
      return null;
    }
    if (n.nodeType === 1) {
      if (n.parentElement && n.parentElement.closest(SKIP)) {
        for (const a of ATTRS) if (n.hasAttribute(a)) handleAttr(n, a);
        return null;
      }
      if (!visitElement(n)) return null;
    } else if (n.nodeType !== 11) {
      return null;
    }
    return document.createTreeWalker(n, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, FILTER);
  }

  function pump() {
    if (disposed || !enabled) return;
    const end = performance.now() + SLICE;
    let n = 0;
    while (head < work.length) {
      const it = work[head];
      if (it.root) {
        const w = startRoot(it.root);
        if (!w) {
          head++;
        } else {
          work[head] = { walker: w };
        }
      } else if (it.walker) {
        let node;
        while ((node = it.walker.nextNode())) {
          handleText(node, true);
          if ((++n & 63) === 0 && performance.now() > end) return later();
        }
        head++;
      } else if (it.text) {
        head++;
        if (it.text.isConnected) handleText(it.text, false);
      } else {
        head++;
        if (it.attr[0].isConnected) handleAttr(it.attr[0], it.attr[1]);
      }
      if ((++n & 63) === 0 && performance.now() > end) return later();
    }
    work = [];
    head = 0;
  }

  const mo = new MutationObserver((list) => {
    for (const m of list) {
      if (m.type === 'characterData') {
        const r = m.target[REC];
        if (!r || m.target.data !== r.shown) work.push({ text: m.target });
      } else if (m.type === 'attributes') {
        const r = m.target[AREC]?.[m.attributeName];
        if (!r || m.target.getAttribute(m.attributeName) !== r.shown) work.push({ attr: [m.target, m.attributeName] });
      } else {
        for (const n of m.addedNodes) {
          if (!loaderElements.has(n)) work.push({ root: n });
        }
        for (const n of m.removedNodes) {
          if (n.isConnected || loaderElements.has(n)) continue;
          if (n.nodeType === 3) settled(n);
          else if (n.nodeType === 1 && loaders.size) {
            const walker = document.createTreeWalker(n, NodeFilter.SHOW_TEXT);
            for (let text; loaders.size && (text = walker.nextNode());) settled(text);
          }
        }
      }
    }
    pump();
  });

  function observe(root) {
    mo.observe(root, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ATTRS });
  }

  function start() {
    const root = document.documentElement;
    if (!root) return;
    observe(root);
    work.push({ root: document.body || root });
    pump();
  }

  function stop(restore = true) {
    mo.disconnect();
    mo.takeRecords();
    for (const entry of loaders.values()) clearLoader(entry);
    work = [];
    head = 0;
    eachRecord((node, name, r) => {
      if (name === null) {
        if (restore && r.shown !== null && node.data === r.shown) node.data = r.src;
        node[REC] = undefined;
      } else {
        if (restore && r.shown !== null && node.getAttribute(name) === r.shown) node.setAttribute(name, r.src);
        delete node[AREC][name];
      }
    });
    waiting.clear();
    queue.clear();
    inflight.clear();
  }

  function setEnabled(on) {
    if (disposed) return;
    on = !!on;
    if (on === enabled) return;
    enabled = on;
    if (on) start();
    else stop();
  }

  /** 主程序推送修改：set 立即替换；del 重新翻译；keep 恢复原文并跳过；resetFilter 重置过滤缓存并重扫 */
  function patch(msg) {
    if (disposed) return;
    const set = msg.set || {};
    const del = new Set(msg.del || []);
    const keep = new Set(msg.keep || []);
    for (const [src, tr] of Object.entries(set)) {
      cache.set(src, tr);
      verdict.set(src, true);
    }
    for (const src of del) {
      cache.delete(src);
      verdict.delete(src);
    }
    for (const src of keep) {
      cache.delete(src);
      verdict.set(src, false);
    }
    if (msg.resetFilter) verdict.clear();
    if (!enabled) return;
    eachRecord((node, name, r) => {
      const cur = name === null ? node.data : node.getAttribute(name);
      if (cur !== r.src && cur !== r.shown) return;
      const write = (v) => (name === null ? (node.data = v) : node.setAttribute(name, v));
      if (keep.has(r.core)) {
        if (name === null) settled(node);
        if (cur !== r.src) write(r.src);
        r.shown = null;
      } else if (Object.hasOwn(set, r.core)) {
        if (name === null) settled(node);
        r.shown = r.a + set[r.core] + r.b;
        if (cur !== r.shown) write(r.shown);
      } else if (del.has(r.core)) {
        if (cur !== r.src) write(r.src);
        r.shown = null;
        want(r.core, node, name, null, null);
      }
    });
    if (msg.resetFilter) {
      work.push({ root: document.body || document.documentElement });
      pump();
    }
  }

  function dispose(restore) {
    if (disposed) return;
    stop(restore);
    disposed = true;
    enabled = false;
    clearTimeout(timer);
    channel.port1.onmessage = null;
    document.removeEventListener(TAKEOVER, onTakeover);
    if (globalThis.__tf === api) delete globalThis.__tf;
  }

  function onTakeover() {
    dispose(true);
  }

  const api = { receive, failed, setEnabled, patch, dispose };
  globalThis.__tf = api;

  document.dispatchEvent(new CustomEvent(TAKEOVER));
  document.addEventListener(TAKEOVER, onTakeover);

  if (enabled) {
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start, { once: true });
    else start();
  }
})();
