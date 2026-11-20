// 沉浸模式注入脚本
//
// 运行在目标页面的隔离环境（isolated world）里：和页面自己的脚本互相看不见，只共享 DOM，
// 不会污染页面的全局变量，也不受页面内容安全策略限制。
//
// 工作方式（对应 GText 的"H5 页面"通道）：
//   遍历 DOM 文本节点和 placeholder/title 属性 → 需要翻译的原文先查页面内缓存，命中当场替换；
//   没命中的攒成一批，通过 CDP binding（__tfSend）交给随译主程序，
//   主程序查本地译文库 / 调翻译引擎后调用 __tf.receive() 回填。
//   MutationObserver 持续监听新增节点和文字变化，界面怎么刷新都能跟上。
//
// 性能（脚本和页面共用主线程，切页时页面一次挂载成千上万个节点）：
//   - 分片处理：观察者回调里先同步处理一小片（命中缓存的当场替换，首屏不闪原文），
//     其余放到后续任务里接着做，每片不超过 SLICE 毫秒，不把页面卡住
//   - 节点的记录挂在节点自身（隔离环境里的属性，页面看不见），节点被页面丢弃后随之回收，
//     来回切页不会越积越多
//   - "是否值得翻译"的判断结果按原文记忆，相同文字只算一次
(() => {
  'use strict';
  // 同一隔离环境里已有旧版脚本（主程序重新接管时）：先让它恢复原文并卸载
  if (globalThis.__tf) globalThis.__tf.dispose(true);

  // 主程序注入时替换为实际配置：{ enabled, skipCjk, skipSelectors, preload }
  const CFG = /*__TF_CONFIG__*/ {};

  // 文字不翻译的区域：代码、输入框、可编辑区（不能改用户正在输入的内容）、显式声明不翻译的元素
  // select 整个跳过：没有 value 属性的 option 改了文字会连带改掉提交值
  const SKIP_TAGS = new Set([
    'script', 'style', 'noscript', 'template', 'textarea', 'input', 'select',
    'code', 'pre', 'kbd', 'samp', 'var', 'svg', 'math', 'canvas',
  ]);
  const EDITABLE = new Set(['', 'true', 'plaintext-only']);
  // 声明不翻译的元素。写在 <html> / <body> 上的不算：很多应用（如 Apifox）在根节点写 translate="no"
  // 只是为了不让浏览器弹自带的翻译提示，照它跳过就整页都不翻了；用户装随译就是要翻它
  const NO_TRANSLATE = [':not(html):not(body)[translate="no"]', ':not(html):not(body).notranslate'];
  // 内置跳过：代码编辑器和终端的正文。它们不用 contenteditable，按标签认不出来，翻了等于改用户的代码
  const BUILTIN_SKIP = [
    '.monaco-editor .view-lines', // VS Code 及其衍生（Trae、Cursor 等）、Monaco
    '.xterm-rows', '.xterm-accessibility', // xterm.js 终端
    '.CodeMirror-code', // CodeMirror 5（CodeMirror 6 是 contenteditable，已跳过）
    '.ace_text-layer', // Ace
  ];
  const CUSTOM_SKIP = [...BUILTIN_SKIP, ...(CFG.skipSelectors || [])].join(',');
  const SKIP = [
    ...SKIP_TAGS,
    '[contenteditable=""]', '[contenteditable="true"]', '[contenteditable="plaintext-only"]',
    ...NO_TRANSLATE,
    ...BUILTIN_SKIP,
    ...(CFG.skipSelectors || []),
  ].join(',');
  // 属性的跳过范围宽一些：输入框的 placeholder 要翻，但可编辑区和声明不翻译的不碰
  const ATTR_SKIP = [
    '[contenteditable=""]', '[contenteditable="true"]', '[contenteditable="plaintext-only"]',
    ...NO_TRANSLATE,
    ...(CFG.skipSelectors || []),
  ].join(',');
  const ATTRS = ['placeholder', 'title'];
  const BATCH = 200;
  /** 每片处理最多占用主线程的毫秒数 */
  const SLICE = 8;
  const CJK = /[぀-ヿ㐀-䶿一-鿿豈-﫿]/g;
  const LETTER = /\p{L}/gu;
  // 节点上的记录：Text 节点 → { src, a, core, b, shown }；元素 → { 属性名: 同上 }
  const REC = Symbol('tf');
  const AREC = Symbol('tfa');

  let enabled = CFG.enabled !== false;
  const cache = new Map(Object.entries(CFG.preload || {})); // 原文 → 译文
  const verdict = new Map(); // 原文 → 是否值得翻译（记忆 worth 的结果）
  const waiting = new Map(); // 原文 → Set(等待回填的目标)
  const queue = new Set(); // 待发送的原文
  const inflight = new Set(); // 已发送、等待译文的原文
  let timer = 0;
  let disposed = false;
  // 主程序重启或重新接管时会在新的隔离环境里再注入一份；隔离环境之间互相看不见，
  // 只能通过共享的 DOM 事件通知旧脚本让位（恢复原文并卸载），避免两份脚本同时改写页面
  // 事件名沿用改名前的旧名，不要改：目标应用里可能还留着旧版注入的脚本，它只认这个名字
  const TAKEOVER = '__translate_filter_takeover__';

  /** 是否值得翻译（与悬浮窗模式 layout.rs 的 needs_translation 保持一致，再多过滤掉标识符） */
  function worth(t) {
    if (t.length < 2 || t.length > 5000) return false;
    const letters = (t.match(LETTER) || []).length;
    if (letters < 2) return false;
    const cjk = (t.match(CJK) || []).length;
    if (CFG.skipCjk && cjk * 2 >= letters) return false;
    // 单个"词"且像路径、文件名、版本号、变量名的不翻
    if (!/\s/.test(t)) {
      // 中日文不用空格分词，整句都是一个"词"：句末的"…"、"："、数字不能当成标识符的证据，
      // 只认明显的路径、文件扩展名（"海鸥3.0.md"、"C:\文档"）；句中的驼峰多是产品名（"VibCode分支原型：…"），不算
      if (cjk ? /[\/\\@#=<>{}\[\]$|]|\.[A-Za-z0-9]{1,6}$/.test(t) : /[\/\\._:@#=<>{}\[\]$|]|\d|[a-z][A-Z]/.test(t)) {
        return false;
      }
    }
    // 带版本号的短名字（"Opus 5.5"、"Node 22"）多是产品/型号名，翻了反而错；
    // 中日文按空格数词没有意义（"空格: 4"、"第 1 页"），不用这条
    if (!cjk && t.split(/\s+/).length <= 3 && /\d/.test(t)) return false;
    return true;
  }

  /** 已去掉首尾空白的原文是否值得翻译，结果记住（页面里相同的文字反复出现） */
  function worthCore(core) {
    let v = verdict.get(core);
    if (v === undefined) {
      if (verdict.size > 50000) verdict.clear();
      verdict.set(core, (v = worth(core)));
    }
    return v;
  }

  /** 拆出首尾空白，只翻中间部分，写回时原样补上 */
  function record(v, core) {
    const lead = v.length - v.trimStart().length;
    return { src: v, a: v.slice(0, lead), core, b: v.slice(lead + core.length), shown: null };
  }

  /** 元素自身是否属于不翻译的区域（子树整个跳过） */
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

  // ───────────── 段落等待提示（不改变现有文本节点翻译粒度） ─────────────

  const PROSE = 'p, li, blockquote';
  const NOT_PROSE = 'nav, aside, menu, table, button, [role="navigation"], [role="menu"], [role="menuitem"], [role="tab"], [role="button"]';
  const loaders = new Map(); // 正文块 → 同一段唯一的指示器
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
    // 不设置 inset：使用段尾的静态位置，但绝对定位不参与正文换行。
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
    // 引擎长时间不响应或回填丢失时，别让提示永久留在页面上。
    entry.timer = setTimeout(() => clearLoader(entry), 45000);
  }

  // ───────────── 文字节点 ─────────────

  function handleText(node, checked) {
    const v = node.data;
    const old = node[REC];
    if (old && v === old.shown) return; // 我们自己写进去的
    const loader = nodeLoaders.get(node);
    if (loader && (v !== old?.src || !loader.block.contains(node))) settled(node);
    const core = v.trim();
    if (!worthCore(core) || (!checked && skippedText(node.parentElement))) {
      settled(node);
      if (old) node[REC] = undefined;
      return;
    }
    const r = record(v, core);
    node[REC] = r;
    const tr = cache.get(core);
    if (tr !== undefined) {
      settled(node);
      applyText(node, r, tr);
    } else want(core, node, null);
  }

  function applyText(node, r, tr) {
    if (node[REC] !== r || node.data !== r.src) return; // 期间被页面改掉了
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
    else want(core, el, name);
  }

  function applyAttr(el, name, r, tr) {
    const m = el[AREC];
    if (!m || m[name] !== r || el.getAttribute(name) !== r.src) return;
    r.shown = r.a + tr + r.b;
    if (r.shown !== r.src) el.setAttribute(name, r.shown);
  }

  /** 遍历页面（含开放的 Shadow DOM）里所有带记录的文字节点和属性：fn(节点, 属性名或 null, 记录) */
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

  function want(core, target, attr) {
    let set = waiting.get(core);
    if (!set) waiting.set(core, (set = new Set()));
    set.add(attr ? { el: target, attr } : { node: target });
    if (!attr) trackLoading(target);
    if (!inflight.has(core)) {
      queue.add(core);
      if (!timer) timer = setTimeout(flush, 30);
    }
  }

  function flush() {
    timer = 0;
    if (!queue.size || !enabled) return;
    const texts = [...queue].slice(0, BATCH);
    for (const t of texts) {
      queue.delete(t);
      inflight.add(t);
    }
    try {
      globalThis.__tfSend(JSON.stringify({ texts }));
    } catch (e) {
      // binding 尚未就绪：稍后再发
      for (const t of texts) {
        inflight.delete(t);
        queue.add(t);
      }
      timer = setTimeout(flush, 500);
      return;
    }
    if (queue.size) timer = setTimeout(flush, 30);
  }

  /** 主程序回填译文：{ map: { 原文: 译文 } } */
  function receive(msg) {
    if (disposed) return;
    for (const [src, tr] of Object.entries(msg.map || {})) {
      cache.set(src, tr);
      inflight.delete(src);
      const set = waiting.get(src);
      waiting.delete(src);
      if (!set || !enabled) continue;
      for (const t of set) {
        if (t.node) {
          const r = t.node[REC];
          if (r && r.core === src && t.node.data === r.src) {
            settled(t.node);
            applyText(t.node, r, tr);
          }
        } else {
          const r = t.el[AREC]?.[t.attr];
          if (r && r.core === src) applyAttr(t.el, t.attr, r, tr);
        }
      }
    }
  }

  /** 翻译失败：过一会儿重试仍在等待的原文 */
  function failed(msg) {
    const texts = msg.texts || [];
    for (const t of texts) {
      inflight.delete(t);
      for (const { node } of waiting.get(t) || []) {
        if (node?.[REC]?.core === t) settled(node);
      }
    }
    setTimeout(() => {
      for (const t of texts) if (waiting.has(t) && !inflight.has(t)) queue.add(t);
      if (queue.size && !timer) timer = setTimeout(flush, 30);
    }, 10000);
  }

  // ───────────── 遍历与监听（分片） ─────────────

  // 待处理的工作，按先后顺序：{ text: 节点 } | { attr: [元素, 属性名] } | { root: 新增子树 } | { walker: 遍历到一半的子树 }
  let work = [];
  let head = 0;
  let scheduled = false;
  // 用 MessageChannel 排下一片：比 setTimeout 及时（没有 4ms 下限），又让出主线程给页面渲染和响应输入
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

  /** 处理元素自身的属性和开放的 Shadow DOM；返回是否继续深入它的子节点 */
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

  /** 开始处理一棵新增子树：先确认它不在跳过区域内；需要深入遍历时返回遍历器 */
  function startRoot(n) {
    if (!n.isConnected) return null;
    if (n.nodeType === 3) {
      handleText(n, false);
      return null;
    }
    if (n.nodeType === 1) {
      if (n.parentElement && n.parentElement.closest(SKIP)) {
        // 跳过区域里的元素仍可能带 placeholder/title
        for (const a of ATTRS) if (n.hasAttribute(a)) handleAttr(n, a);
        return null;
      }
      if (!visitElement(n)) return null;
    } else if (n.nodeType !== 11) {
      return null; // 注释等
    }
    return document.createTreeWalker(n, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, FILTER);
  }

  /** 处理待办工作，最多 SLICE 毫秒，没做完的排到下一片 */
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
        // 我们自己写入引起的变化直接跳过，不进队列
        const r = m.target[REC];
        if (!r || m.target.data !== r.shown) work.push({ text: m.target });
      } else if (m.type === 'attributes') {
        const r = m.target[AREC]?.[m.attributeName];
        if (!r || m.target.getAttribute(m.attributeName) !== r.shown) work.push({ attr: [m.target, m.attributeName] });
      } else {
        for (const n of m.addedNodes) {
          if (!loaderElements.has(n)) work.push({ root: n });
        }
        // 遍历移除的子树，避免动态页面上的旧等待状态留到超时。
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
    // 先同步做一片：命中缓存的文字在这一帧画出来之前就换好，不闪原文
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

  /**
   * 主程序推送的修改（改了术语表、译文库）：
   * set：原文 → 新译文，已显示的立即换成新译文；del：忘掉这些原文的旧译文，重新请求翻译
   */
  function patch(msg) {
    if (disposed) return;
    const set = msg.set || {};
    const del = new Set(msg.del || []);
    for (const [src, tr] of Object.entries(set)) cache.set(src, tr);
    for (const src of del) cache.delete(src);
    if (!enabled) return;
    eachRecord((node, name, r) => {
      const cur = name === null ? node.data : node.getAttribute(name);
      if (cur !== r.src && cur !== r.shown) return; // 已被页面改成别的内容，由观察者处理
      const write = (v) => (name === null ? (node.data = v) : node.setAttribute(name, v));
      if (Object.hasOwn(set, r.core)) {
        if (name === null) settled(node);
        r.shown = r.a + set[r.core] + r.b;
        if (cur !== r.shown) write(r.shown);
      } else if (del.has(r.core)) {
        if (cur !== r.src) write(r.src);
        r.shown = null;
        want(r.core, node, name);
      }
    });
  }

  /** 卸载：停止监听、清掉定时器；restore 为真时把页面恢复成原文 */
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

  // 先通知其他隔离环境里的旧脚本让位，再开始监听（自己发的事件不会被自己收到）
  document.dispatchEvent(new CustomEvent(TAKEOVER));
  document.addEventListener(TAKEOVER, onTakeover);

  if (enabled) {
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start, { once: true });
    else start();
  }
})();
