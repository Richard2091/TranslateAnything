// 设置窗口通用小部件：元素构造、图标、开关、分段选择、提示与对话框
import { fmtDate, fmtNum, locale, t } from "../i18n";

export type Child = Node | string | null | undefined | false;

/** 创建元素；文字一律走 textContent，页面标题等外部内容不会被当成 HTML */
export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, unknown> = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === null || v === false) continue;
    if (k.startsWith("on") && typeof v === "function") {
      el.addEventListener(k.slice(2), v as EventListener);
    } else if (k === "class") {
      el.className = String(v);
    } else if (k in el && typeof v !== "string") {
      (el as unknown as Record<string, unknown>)[k] = v;
    } else {
      el.setAttribute(k, v === true ? "" : String(v));
    }
  }
  for (const c of children) if (c !== null && c !== undefined && c !== false) el.append(c);
  return el;
}

/** 线条图标（固定的内置 SVG 片段，不含外部内容） */
const ICONS = {
  grid: '<rect x="3.5" y="3.5" width="7" height="7" rx="2"/><rect x="13.5" y="3.5" width="7" height="7" rx="2"/><rect x="3.5" y="13.5" width="7" height="7" rx="2"/><rect x="13.5" y="13.5" width="7" height="7" rx="2"/>',
  frame: '<path d="M4 8V6a2 2 0 0 1 2-2h2M16 4h2a2 2 0 0 1 2 2v2M20 16v2a2 2 0 0 1-2 2h-2M8 20H6a2 2 0 0 1-2-2v-2"/><path d="M8 10h8M8 14h5"/>',
  apps: '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M3 9h18"/><path d="M8 13.5h5M8 16.5h8"/>',
  engine: '<path d="M4 5h7M7.5 3v2M10 5c-.7 3.3-2.8 6-5.5 7.5M6.5 8.5c1 1.6 2.6 3 4.5 3.8"/><path d="m12 21 4.5-10 4.5 10M13.6 17.5h5.8"/>',
  sparkle: '<path d="M12 3.5 13.8 9a2 2 0 0 0 1.2 1.2l5.5 1.8-5.5 1.8a2 2 0 0 0-1.2 1.2L12 20.5 10.2 15a2 2 0 0 0-1.2-1.2L3.5 12 9 10.2A2 2 0 0 0 10.2 9z"/>',
  globe: '<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c2.3 2.4 3.4 5.2 3.4 8.5s-1.1 6.1-3.4 8.5c-2.3-2.4-3.4-5.2-3.4-8.5S9.7 5.9 12 3.5z"/>',
  db: '<ellipse cx="12" cy="5.5" rx="7.5" ry="2.8"/><path d="M4.5 5.5v13c0 1.55 3.36 2.8 7.5 2.8s7.5-1.25 7.5-2.8v-13"/><path d="M4.5 12c0 1.55 3.36 2.8 7.5 2.8s7.5-1.25 7.5-2.8"/>',
  book: '<path d="M5 19V5.5A2.5 2.5 0 0 1 7.5 3H19v14H7.5A2.5 2.5 0 0 0 5 19.5 2.5 2.5 0 0 0 7.5 22H19v-5"/><path d="M9 7.5h6M9 11h4"/>',
  sliders: '<path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2.2"/><circle cx="9" cy="17" r="2.2"/>',
  play: '<path d="M7 4.5v15l12-7.5z"/>',
  swap: '<path d="M7 4 3 8l4 4"/><path d="M3 8h14"/><path d="m17 20 4-4-4-4"/><path d="M21 16H7"/>',
  edit: '<path d="M4 20h4L19 9a2.8 2.8 0 0 0-4-4L4 16z"/><path d="m13.5 6.5 4 4"/>',
  trash: '<path d="M4 7h16"/><path d="M10 11v6M14 11v6"/><path d="M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12"/><path d="M9 7V4h6v3"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  scan: '<path d="M4 8V6a2 2 0 0 1 2-2h2M16 4h2a2 2 0 0 1 2 2v2M20 16v2a2 2 0 0 1-2 2h-2M8 20H6a2 2 0 0 1-2-2v-2"/><path d="M8 12h8"/>',
  search: '<circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2"/>',
  box: '<path d="M21 8 12 3 3 8v8l9 5 9-5z"/><path d="m3 8 9 5 9-5M12 13v8"/>',
  bolt: '<path d="M13 3 5 13.5h6L10 21l8-10.5h-6z"/>',
  x: '<path d="M6 6l12 12M18 6 6 18"/>',
  check: '<path d="m5 12.5 4.5 4.5L19 7.5"/>',
  folder: '<path d="M3.5 7.5A2 2 0 0 1 5.5 5.5h4l2 2.5h7a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/>',
  arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  up: '<path d="m6 15 6-6 6 6"/>',
  down: '<path d="m6 9 6 6 6-6"/>',
  chevL: '<path d="m15 6-6 6 6 6"/>',
  chevR: '<path d="m9 6 6 6-6 6"/>',
  grip: '<g fill="currentColor" stroke="none"><circle cx="9" cy="6" r="1.5"/><circle cx="15" cy="6" r="1.5"/><circle cx="9" cy="12" r="1.5"/><circle cx="15" cy="12" r="1.5"/><circle cx="9" cy="18" r="1.5"/><circle cx="15" cy="18" r="1.5"/></g>',
  eye: '<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="2.8"/>',
  refresh: '<path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3L19.5 9"/><path d="M19.5 4v5h-5"/>',
  copy: '<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>',
} as const;

export type IconName = keyof typeof ICONS;

export function icon(name: IconName): SVGElement {
  const t = document.createElement("template");
  t.innerHTML = `<svg viewBox="0 0 24 24" aria-hidden="true">${ICONS[name]}</svg>`;
  return t.content.firstElementChild as SVGElement;
}

/** 快捷键显示成键帽：Alt+Shift+T → [Alt] + [Shift] + [T] */
export function keys(hotkey: string): HTMLElement {
  const box = h("span", { class: "keys" });
  hotkey
    .split("+")
    .map((k) => k.trim())
    .filter(Boolean)
    .forEach((k, i) => {
      if (i) box.append(h("i", {}, "+"));
      box.append(h("kbd", {}, k));
    });
  return box;
}

/** 页首说明：侧栏选中项已表明当前页，这里不再重复大标题，只留一句说明，随内容滚动 */
export function pageHead(lead: string, ...extra: Child[]): HTMLElement {
  return h("header", { class: "page-head" }, h("p", { class: "lead grow" }, lead), ...extra);
}

export function section(title: string, ...extra: Child[]): HTMLElement {
  return h("h2", { class: "section" }, title, ...extra);
}

/** 应用头像：取名称首字，颜色按名称散列，同一个应用每次颜色一致 */
export function avatar(name: string): HTMLElement {
  let hash = 0;
  for (const ch of name) hash = (hash * 31 + ch.codePointAt(0)!) >>> 0;
  const first = [...name.trim()][0]?.toUpperCase() ?? "?";
  return h("div", { class: "app-avatar", style: `--h:${hash % 360}` }, first);
}

export function emptyState(text: string, ic: IconName = "box"): HTMLElement {
  return h("div", { class: "empty" }, icon(ic), text);
}

export function switchEl(checked: boolean, onChange: (v: boolean) => void, disabled = false): HTMLLabelElement {
  const input = h("input", { type: "checkbox", checked, disabled });
  const label = h("label", { class: "switch" }, input, h("i"));
  input.addEventListener("change", () => {
    lastSwitch = { el: label, checked: input.checked, at: performance.now() };
    onChange(input.checked);
  });
  return label;
}

/** 最近一次拨动的开关；拨动后保存配置会重绘整页、换掉开关元素，靠它让新开关接着把动画放完 */
let lastSwitch: { el: HTMLElement; checked: boolean; at: number } | null = null;
const SWITCH_ANIM_MS = 420;

/** 开关在重绘前后的对应关系：引擎行按行 id，其余按在当前页里的顺序 */
function switchKey(el: HTMLElement): string {
  const row = el.closest<HTMLElement>("[data-id]");
  if (row) return `id:${row.dataset.id}`;
  const scope = el.closest(".tab") ?? document;
  return `n:${[...scope.querySelectorAll(".switch")].indexOf(el)}`;
}

/**
 * 重绘前调用：记下刚拨动、动画还没放完的开关；返回的函数在重绘后调用，
 * 让新开关从已经过去的时间点接着播放（负的 animation-delay）。
 */
export function keepSwitchMotion(): () => void {
  const s = lastSwitch;
  if (!s || !s.el.isConnected) return () => {};
  const elapsed = performance.now() - s.at;
  if (elapsed > SWITCH_ANIM_MS) return () => {};
  const key = switchKey(s.el);
  return () => {
    const scope = document.querySelector(".tab.active") ?? document;
    const next = [...scope.querySelectorAll<HTMLElement>(".switch")].find((el) => el !== s.el && switchKey(el) === key);
    if (!next || next.querySelector("input")?.checked !== s.checked) return;
    next.style.setProperty("--sw-delay", `${-elapsed}ms`);
    next.classList.add(s.checked ? "sw-on" : "sw-off");
    next.addEventListener("animationend", () => next.classList.remove("sw-on", "sw-off"), { once: true });
    lastSwitch = { el: next, checked: s.checked, at: s.at };
  };
}

export function field(label: string, control: HTMLElement, cls = ""): HTMLLabelElement {
  return h("label", { class: `field ${cls}` }, h("span", {}, label), control);
}

/** 分段选择器；`disabled` 里的选项显示但不可点 */
export function segmented<T extends string>(
  options: [T, string][],
  value: T,
  onChange: (v: T) => void,
  disabled: T[] = [],
): HTMLElement {
  let current = value;
  const box = h("div", { class: "segmented", role: "radiogroup" });
  const thumb = h("span", { class: "seg-thumb", "aria-hidden": "true" });
  box.append(thumb);
  const place = () => {
    const on = box.querySelector<HTMLElement>("button.on");
    if (!on) return;
    thumb.style.width = `${on.offsetWidth}px`;
    thumb.style.transform = `translateX(${on.offsetLeft}px)`;
  };
  const segKey = options.map(([v]) => v).join("|");
  for (const [v, label] of options) {
    const b = h(
      "button",
      {
        type: "button",
        role: "radio",
        class: v === value ? "on" : "",
        "aria-checked": String(v === value),
        disabled: disabled.includes(v),
        onclick: () => {
          if (v === current) return;
          current = v;
          // 切换后保存配置多半会重绘整页，记下滑块原位置，让新组件从这里滑过去
          lastSeg = { key: segKey, transform: thumb.style.transform, width: thumb.style.width, at: performance.now() };
          for (const x of box.querySelectorAll("button")) {
            x.classList.toggle("on", x === b);
            x.setAttribute("aria-checked", String(x === b));
          }
          place();
          onChange(v);
        },
      },
      label,
    );
    box.append(b);
  }
  // 放进页面、排版完成后再定位滑块：首次直接就位；若是刚切换引起的重绘，从旧位置滑过去
  requestAnimationFrame(() => {
    const from = lastSeg && lastSeg.key === segKey && performance.now() - lastSeg.at < 450 ? lastSeg : null;
    thumb.style.transition = "none";
    if (from) {
      thumb.style.transform = from.transform;
      thumb.style.width = from.width;
      lastSeg = null;
    } else place();
    void thumb.offsetWidth;
    thumb.style.transition = "";
    if (from) place();
  });
  return box;
}

/** 最近一次分段选择器切换前的滑块位置（见 segmented） */
let lastSeg: { key: string; transform: string; width: string; at: number } | null = null;

export interface DropOption<T extends string> {
  value: T;
  label: string;
  /** 选项下方的小字说明 */
  hint?: string;
  disabled?: boolean;
}

let openDrop: (() => void) | null = null;

/** 弹出层定位：默认在锚点下方，放不下且上方更宽裕时翻到上方 */
function placePop(pop: HTMLElement, anchor: HTMLElement) {
  const r = anchor.getBoundingClientRect();
  const ph = Math.min(pop.scrollHeight, 300);
  const below = window.innerHeight - r.bottom - 12;
  const up = below < ph && r.top > below;
  pop.style.top = pop.style.bottom = "";
  pop.classList.toggle("up", up);
  pop.style.minWidth = `${r.width}px`;
  pop.style.left = `${Math.min(r.left, window.innerWidth - Math.max(r.width, pop.offsetWidth) - 8)}px`;
  pop.style.maxHeight = `${Math.max(120, Math.min(300, up ? r.top - 12 : below))}px`;
  if (up) pop.style.bottom = `${window.innerHeight - r.top + 4}px`;
  else pop.style.top = `${r.bottom + 4}px`;
}

export interface ComboOpts {
  placeholder?: string;
  /** 输入框的 data-k（刷新界面时据此恢复焦点） */
  key?: string;
  /** 固定的候选项 */
  options?: DropOption<string>[];
  /** 打开时异步获取候选项（如模型列表）；失败时把错误信息显示在弹出层里 */
  load?: () => Promise<DropOption<string>[]>;
  /** 已获取过且仍有效的候选项；有则点按钮直接展开，不再调用 `load` */
  cached?: () => DropOption<string>[] | undefined;
  /** 右侧按钮的提示 */
  title?: string;
}

type ComboEl = HTMLElement & { reopen?: (list: DropOption<string>[] | null, err: string) => void };

/**
 * 可输入的下拉框：既能从候选项里挑，也能自己填写（候选项不全或想填别的值时）。
 * 输入时按输入内容筛选候选项；改完一项（失焦、回车或选中候选项）调用 `onCommit`。
 */
export function combo(value: string, onCommit: (v: string) => void, opts: ComboOpts = {}): HTMLElement {
  let committed = value;
  let loaded: DropOption<string>[] | null = opts.options ?? (opts.load ? null : []);
  let loadError = "";
  let loading = false;
  let pop: HTMLElement | null = null;
  let shown: DropOption<string>[] = [];
  let active = -1;

  const input = h("input", { type: "text", value, placeholder: opts.placeholder, "data-k": opts.key, autocomplete: "off", spellcheck: false });
  const btn = h(
    "button",
    { type: "button", class: "input-icon", tabIndex: -1, title: opts.title ?? t("common.showOptions"), "aria-label": opts.title ?? t("common.showOptions") },
    icon(opts.load && !opts.cached?.() ? "refresh" : "down"),
  );
  /** 获取成功后按钮改为展开箭头 */
  const markLoaded = () => btn.replaceChildren(icon("down"));
  const wrap = h("div", { class: "input-wrap combo" }, input, btn);

  const commit = () => {
    const v = input.value.trim();
    if (v === committed) return;
    committed = v;
    onCommit(v);
  };
  const choose = (o: DropOption<string>) => {
    input.value = o.value;
    close();
    commit();
  };

  const close = () => {
    if (!pop) return;
    const p = pop;
    pop = null;
    wrap.classList.remove("open");
    p.classList.add("leaving");
    afterLeave(p, () => p.remove());
    document.removeEventListener("pointerdown", onOutside, true);
    window.removeEventListener("resize", onDismiss);
    document.querySelector("main")?.removeEventListener("scroll", onDismiss);
    if (openDrop === close) openDrop = null;
  };
  const onDismiss = () => close();
  const onOutside = (e: Event) => {
    if (!wrap.contains(e.target as Node) && !pop?.contains(e.target as Node)) close();
  };

  const setActive = (i: number) => {
    active = i;
    if (!pop) return;
    const items = pop.querySelectorAll<HTMLElement>(".dd-item:not(.disabled)");
    items.forEach((el, j) => el.classList.toggle("active", j === i));
    const el = items[i];
    if (el) {
      if (el.offsetTop < pop.scrollTop) pop.scrollTop = el.offsetTop - 5;
      else if (el.offsetTop + el.offsetHeight > pop.scrollTop + pop.clientHeight) pop.scrollTop = el.offsetTop + el.offsetHeight - pop.clientHeight + 5;
    }
  };

  /** 按输入内容筛选并重画弹出层；`all` 为 true 时不筛选（点按钮打开时） */
  const render = (all: boolean) => {
    const q = all ? "" : input.value.trim().toLowerCase();
    const note = (text: string, bad = false) => h("div", { class: `dd-item disabled${bad ? " bad" : ""}` }, h("div", { class: "grow dd-hint" }, text));
    let children: HTMLElement[];
    if (loading) children = [note(t("common.loading"))];
    else if (loadError) children = [note(loadError, true)];
    else {
      shown = (loaded ?? []).filter((o) => !q || `${o.value}\n${o.label}\n${o.hint ?? ""}`.toLowerCase().includes(q));
      if (!shown.length) {
        // 打字筛选没有结果时直接收起，不打扰输入
        if (q) return close();
        children = [note(t("common.noOptions"))];
      } else
        children = shown.map((o) =>
          h(
            "div",
            {
              class: `dd-item${o.value === input.value.trim() ? " selected" : ""}`,
              role: "option",
              onclick: () => choose(o),
            },
            h("div", { class: "grow" }, h("div", {}, o.label), o.hint && h("div", { class: "dd-hint" }, o.hint)),
            o.value === input.value.trim() && h("span", { class: "dd-check" }, icon("check")),
          ),
        );
    }
    if (!pop) {
      openDrop?.();
      pop = h("div", { class: "dd-pop", role: "listbox" });
      // 点选项时不让输入框失焦（否则会先按半截输入提交一次）
      pop.addEventListener("pointerdown", (e) => e.preventDefault());
      document.body.append(pop);
      wrap.classList.add("open");
      openDrop = close;
      document.addEventListener("pointerdown", onOutside, true);
      window.addEventListener("resize", onDismiss);
      document.querySelector("main")?.addEventListener("scroll", onDismiss, { passive: true });
    }
    pop.replaceChildren(...children);
    placePop(pop, wrap);
    active = -1;
  };

  const open = async (all: boolean, reload = false) => {
    if (opts.load && (reload || (!loaded && !loading))) {
      loading = true;
      loadError = "";
      render(true);
      try {
        loaded = await opts.load();
        markLoaded();
      } catch (e) {
        loaded = null;
        loadError = errText(e);
      }
      loading = false;
      if (!pop) return;
      // 获取期间界面重画过（例如先保存了刚改的地址），换到新的输入框上展示结果
      if (!wrap.isConnected) {
        close();
        const next = opts.key ? document.querySelector(`[data-k="${opts.key}"]`)?.closest<HTMLElement>(".combo") : null;
        (next as ComboEl | null)?.reopen?.(loaded, loadError);
        return;
      }
    }
    render(all);
  };
  (wrap as ComboEl).reopen = (list, err) => {
    loaded = list;
    loadError = err;
    if (list && !err) markLoaded();
    input.focus();
    render(true);
  };

  btn.addEventListener("pointerdown", (e) => e.preventDefault());
  btn.addEventListener("click", () => {
    input.focus();
    if (pop) return close();
    if (opts.load) {
      // 已按当前地址 / 密钥获取过就直接展开；没有或已变化才重新获取
      const c = opts.cached?.();
      if (c) {
        loaded = c;
        loadError = "";
        void open(true);
      } else void open(true, true);
    } else void open(true);
  });
  input.addEventListener("input", () => {
    if (loaded && !loading) render(false);
  });
  input.addEventListener("change", commit);
  input.addEventListener("keydown", (e) => {
    const n = pop?.querySelectorAll(".dd-item:not(.disabled)").length ?? 0;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!pop) return void open(true);
      if (n) setActive((active + (e.key === "ArrowDown" ? 1 : n - 1)) % n);
    } else if (e.key === "Enter") {
      if (pop && active >= 0 && shown[active]) choose(shown[active]);
      else {
        close();
        commit();
      }
    } else if (e.key === "Escape" && pop) {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === "Tab") close();
  });
  return wrap;
}

const MOD_KEYS = new Set(["Control", "Alt", "Shift", "Meta"]);

/** 把键盘事件的物理键位换成快捷键里的键名；不支持的键返回空串 */
function keyName(code: string): string {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit\d$/.test(code)) return code.slice(5);
  if (/^F\d{1,2}$/.test(code)) return code;
  if (code.startsWith("Arrow")) return code.slice(5);
  const ok = ["Space", "Enter", "Tab", "Backspace", "Delete", "Insert", "Home", "End", "PageUp", "PageDown", "Minus", "Equal", "BracketLeft", "BracketRight", "Backslash", "Semicolon", "Quote", "Backquote", "Comma", "Period", "Slash"];
  if (ok.includes(code) || /^Numpad\d$/.test(code)) return code;
  return "";
}

/**
 * 快捷键录制框：点一下后直接按下想要的组合键，不用自己拼写"Alt+Shift+T"。
 * 必须带修饰键（F1–F24 除外）；Esc 取消录制。
 */
export function hotkeyInput(value: string, onCommit: (v: string) => void, key?: string): HTMLInputElement {
  let current = value;
  const input = h("input", { type: "text", value, readOnly: true, class: "hotkey-input", placeholder: t("common.hotkeyPress"), "data-k": key, title: t("common.hotkeyHint") });
  const show = (v: string) => (input.value = v);
  input.addEventListener("focus", () => {
    input.classList.add("recording");
    show("");
  });
  input.addEventListener("blur", () => {
    input.classList.remove("recording");
    show(current);
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Tab" && !e.ctrlKey && !e.altKey) return;
    e.preventDefault();
    const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean) as string[];
    if (e.key === "Escape" && !mods.length) {
      show(current);
      return input.blur();
    }
    if (MOD_KEYS.has(e.key)) return show(`${mods.join("+")}+`);
    const k = keyName(e.code);
    if (!k || (!mods.length && !/^F\d{1,2}$/.test(k))) {
      show(mods.length ? `${mods.join("+")}+` : "");
      return toast(t("common.hotkeyNeedMod"), true);
    }
    const v = [...mods, k].join("+");
    show(v);
    if (v !== current) {
      current = v;
      onCommit(v);
    }
    input.blur();
  });
  // 松开修饰键时只显示仍按着的
  input.addEventListener("keyup", (e) => {
    if (!input.classList.contains("recording") || !input.value.endsWith("+")) return;
    const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean);
    show(mods.length ? `${mods.join("+")}+` : "");
  });
  return input;
}

/**
 * 下拉选择框（替代原生 select：原生弹出列表无法定制样式和动效）。
 * 弹出层挂在 body 上、按触发按钮定位，不会被卡片的 overflow 裁掉；空间不够时向上弹出。
 * 键盘：↑↓ 移动，回车/空格选择，Esc 关闭，输入首字母跳转。
 */
export function dropdown<T extends string>(
  options: DropOption<T>[],
  value: T,
  onChange: (v: T) => void,
  opts: { placeholder?: string; width?: string; title?: string } = {},
): HTMLButtonElement {
  let current = value;
  const labelOf = (v: T) => options.find((o) => o.value === v)?.label ?? opts.placeholder ?? v;
  const text = h("span", { class: "dd-text" }, labelOf(current));
  const trigger = h(
    "button",
    { type: "button", class: "dropdown", "aria-haspopup": "listbox", "aria-expanded": "false", title: opts.title, style: opts.width ? `width:${opts.width}` : undefined },
    text,
    icon("down"),
  );

  const close = (focus = false) => {
    const pop = document.querySelector<HTMLElement>(".dd-pop:not(.leaving)");
    trigger.classList.remove("open");
    trigger.setAttribute("aria-expanded", "false");
    openDrop = null;
    document.removeEventListener("pointerdown", onOutside, true);
    window.removeEventListener("resize", onDismiss);
    document.querySelector("main")?.removeEventListener("scroll", onDismiss);
    if (pop) {
      pop.classList.add("leaving");
      afterLeave(pop, () => pop.remove());
    }
    if (focus) trigger.focus();
    window.dispatchEvent(new Event("dropdown-closed"));
  };
  const onDismiss = () => close();
  const onOutside = (e: Event) => {
    const pop = document.querySelector(".dd-pop:not(.leaving)");
    if (!trigger.contains(e.target as Node) && !pop?.contains(e.target as Node)) close();
  };

  const open = () => {
    openDrop?.();
    const pop = h("div", { class: "dd-pop", role: "listbox" });
    let active = Math.max(0, options.findIndex((o) => o.value === current));
    const items = options.map((o, i) =>
      h(
        "div",
        {
          class: `dd-item${o.value === current ? " selected" : ""}${o.disabled ? " disabled" : ""}`,
          role: "option",
          "aria-selected": String(o.value === current),
          onpointerenter: () => setActive(i, false),
          onclick: () => choose(i),
        },
        h("div", { class: "grow" }, h("div", {}, o.label), o.hint && h("div", { class: "dd-hint" }, o.hint)),
        o.value === current && h("span", { class: "dd-check" }, icon("check")),
      ),
    );
    const setActive = (i: number, scroll = true) => {
      active = i;
      items.forEach((el, j) => el.classList.toggle("active", j === i));
      // 只滚动弹出层自己（scrollIntoView 会连带滚动整个页面）
      const el = items[i];
      if (scroll && el) {
        if (el.offsetTop < pop.scrollTop) pop.scrollTop = el.offsetTop - 5;
        else if (el.offsetTop + el.offsetHeight > pop.scrollTop + pop.clientHeight) pop.scrollTop = el.offsetTop + el.offsetHeight - pop.clientHeight + 5;
      }
    };
    const choose = (i: number) => {
      const o = options[i];
      if (!o || o.disabled) return;
      close(true);
      if (o.value !== current) {
        current = o.value;
        text.textContent = o.label;
        onChange(o.value);
      }
    };
    const move = (d: number) => {
      for (let n = 1; n <= options.length; n++) {
        const i = (active + d * n + options.length) % options.length;
        if (!options[i].disabled) return setActive(i);
      }
    };
    pop.addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown") move(1);
      else if (e.key === "ArrowUp") move(-1);
      else if (e.key === "Enter" || e.key === " ") choose(active);
      else if (e.key === "Escape" || e.key === "Tab") return close(e.key === "Escape");
      else if (e.key.length === 1) {
        const k = e.key.toLowerCase();
        const i = options.findIndex((o, j) => j > active && o.label.toLowerCase().startsWith(k));
        const j = i >= 0 ? i : options.findIndex((o) => o.label.toLowerCase().startsWith(k));
        if (j >= 0) setActive(j);
      } else return;
      e.preventDefault();
    });
    pop.tabIndex = -1;
    pop.append(...items);
    document.body.append(pop);

    placePop(pop, trigger);

    trigger.classList.add("open");
    trigger.setAttribute("aria-expanded", "true");
    setActive(active);
    pop.focus({ preventScroll: true });
    openDrop = () => close();
    document.addEventListener("pointerdown", onOutside, true);
    window.addEventListener("resize", onDismiss);
    document.querySelector("main")?.addEventListener("scroll", onDismiss, { passive: true });
  };

  trigger.addEventListener("click", () => (trigger.classList.contains("open") ? close() : open()));
  trigger.addEventListener("keydown", (e) => {
    if ((e.key === "ArrowDown" || e.key === "ArrowUp") && !trigger.classList.contains("open")) {
      e.preventDefault();
      open();
    }
  });
  return trigger;
}

/** 字节数换算成易读的大小 */
export function fileSize(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
  const n = bytes / 1024 ** i;
  return `${n >= 100 || i === 0 ? Math.round(n) : n.toFixed(1)} ${units[i]}`;
}

/**
 * 可折叠区域：高度平滑展开 / 收起（grid 行高 0fr ↔ 1fr，不用量高度），内容略微下滑淡入。
 * 直接渲染出来的是展开状态；用 expand / collapse 播放动画。
 */
export function collapsible(content: HTMLElement): HTMLElement {
  return h("div", { class: "collapse" }, h("div", { class: "collapse-inner" }, content));
}

/** 等 grid 行高过渡结束；动画被关掉（时长为 0）时靠兜底计时 */
function afterRows(el: HTMLElement, fn: () => void) {
  let done = false;
  const finish = () => {
    if (done) return;
    done = true;
    el.removeEventListener("transitionend", onEnd);
    fn();
  };
  const onEnd = (e: TransitionEvent) => e.target === el && e.propertyName === "grid-template-rows" && finish();
  el.addEventListener("transitionend", onEnd);
  window.setTimeout(finish, 450);
}

/** 从收起展开到内容高度 */
export function expand(el: HTMLElement) {
  el.classList.add("closed", "moving");
  void el.offsetHeight;
  el.classList.remove("closed");
  afterRows(el, () => !el.classList.contains("closed") && el.classList.remove("moving"));
}

/** 收起；收完后调用 done（期间又被展开则不调用） */
export function collapse(el: HTMLElement, done?: () => void) {
  el.classList.add("moving");
  void el.offsetHeight;
  el.classList.add("closed");
  afterRows(el, () => el.classList.contains("closed") && done?.());
}

/** 等退场动画放完再执行 fn（动画关闭时立即执行） */
export function afterLeave(el: HTMLElement, fn: () => void) {
  const ms = parseFloat(getComputedStyle(el).animationDuration) * 1000 || 0;
  return window.setTimeout(fn, ms);
}

let toastTimer = 0;
export function toast(msg: string, bad = false) {
  const el = document.getElementById("toast")!;
  el.textContent = msg;
  el.className = bad ? "bad" : "";
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(
    () => {
      el.classList.add("leaving");
      toastTimer = afterLeave(el, () => (el.hidden = true));
    },
    bad ? 6000 : 2500,
  );
}

export interface DialogHandle {
  close: () => void;
  /** 换掉对话框内容（例如从"选择应用"切到"填写信息"），带一次淡入 */
  set: (title: string, body: Child, actions: Child[]) => void;
}

/**
 * 通用对话框。点遮罩空白处或按 Esc 关闭；`onClose` 在关闭时调用一次。
 * 同一时间只有一个对话框，再打开会替换掉上一个。
 */
export function openDialog(title: string, body: Child, actions: Child[], opts: { wide?: boolean; onClose?: () => void } = {}): DialogHandle {
  const modal = document.getElementById("modal")!;
  const box = h("div", { class: `dialog${opts.wide ? " wide" : ""}`, role: "dialog", "aria-modal": "true" });
  let closed = false;
  const close = () => {
    if (closed) return;
    closed = true;
    document.removeEventListener("keydown", onKey);
    modal.onclick = null;
    modal.classList.add("leaving");
    afterLeave(modal, () => {
      if (modal.firstChild !== box) return; // 期间已经打开了新的对话框
      modal.hidden = true;
      modal.classList.remove("leaving");
      modal.replaceChildren();
    });
    opts.onClose?.();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" && !document.querySelector(".dd-pop")) close();
  };
  const set = (t: string, b: Child, a: Child[]) => {
    box.replaceChildren(h("h3", {}, t), h("div", { class: "dialog-body" }, b), h("div", { class: "actions" }, ...a));
    box.classList.remove("swap");
    void box.offsetWidth;
    box.classList.add("swap");
  };
  set(title, body, actions);
  box.classList.remove("swap");
  modal.classList.remove("leaving");
  modal.replaceChildren(box);
  modal.hidden = false;
  modal.onclick = (e) => {
    if (e.target === modal) close();
  };
  document.addEventListener("keydown", onKey);
  // 第一个输入框自动聚焦
  requestAnimationFrame(() => box.querySelector<HTMLElement>("input, textarea")?.focus());
  return { close, set };
}

/** 确认对话框；返回用户是否确认 */
export function confirmDialog(title: string, body: Child, okText: string, danger = false): Promise<boolean> {
  return new Promise((resolve) => {
    let result = false;
    const d = openDialog(
      title,
      body,
      [
        h("button", { class: "btn", onclick: () => d.close() }, t("common.cancel")),
        h("button", { class: `btn ${danger ? "solid-danger" : "primary"}`, onclick: () => ((result = true), d.close()) }, okText),
      ],
      { onClose: () => resolve(result) },
    );
  });
}

export function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

export function relTime(sec: number): string {
  const d = Date.now() / 1000 - sec;
  if (d < 60) return t("common.justNow");
  // “5 分钟前 / 5 minutes ago”交给 Intl 按界面语言生成
  const rtf = new Intl.RelativeTimeFormat(locale(), { numeric: "always" });
  if (d < 3600) return rtf.format(-Math.floor(d / 60), "minute");
  if (d < 86400) return rtf.format(-Math.floor(d / 3600), "hour");
  if (d < 86400 * 30) return rtf.format(-Math.floor(d / 86400), "day");
  return fmtDate(new Date(sec * 1000));
}

export const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v));

export const fmt = (n: number) => fmtNum(n);

/** 毫秒转为两位小数秒字符串，如 0.68 s */
export function fmtSec(ms: number): string {
  return `${(ms / 1000).toFixed(2)} s`;
}

// ───────────────────────── 统一样式悬浮详情卡 ─────────────────────────

let activePop: { trigger: HTMLElement; pop: HTMLElement; close: () => void } | null = null;

/** 重绘后清理已脱离 DOM 的锚点浮层，避免残留 */
export function cleanupStalePopovers() {
  if (activePop && !activePop.trigger.isConnected) {
    activePop.close();
  }
}

function placeHoverPop(pop: HTMLElement, anchor: HTMLElement) {
  const r = anchor.getBoundingClientRect();
  const pw = Math.min(340, window.innerWidth - 16);
  pop.style.maxWidth = `${pw}px`;
  const ph = Math.min(pop.scrollHeight, 260);
  const below = window.innerHeight - r.bottom - 10;
  const up = below < ph && r.top > below;
  pop.style.top = pop.style.bottom = "";
  pop.classList.toggle("up", up);
  const actualW = Math.min(pop.offsetWidth || pw, pw);
  const left = Math.max(8, Math.min(r.left, window.innerWidth - actualW - 8));
  pop.style.left = `${left}px`;
  if (up) pop.style.bottom = `${window.innerHeight - r.top + 6}px`;
  else pop.style.top = `${r.bottom + 6}px`;
}

/**
 * 给元素挂载统一样式的悬停/聚焦详情浮层（挂在 body 上，不裁切、可移入复制文字）
 */
export function attachPopover(trigger: HTMLElement, renderContent: () => HTMLElement | null) {
  let closeTimer = 0;
  let pop: HTMLElement | null = null;

  const clearTimer = () => {
    clearTimeout(closeTimer);
    closeTimer = 0;
  };

  const close = () => {
    clearTimer();
    if (!pop) return;
    const p = pop;
    pop = null;
    if (activePop?.pop === p) activePop = null;
    window.removeEventListener("resize", close);
    document.querySelector("main")?.removeEventListener("scroll", close);
    p.remove();
  };

  const scheduleClose = () => {
    clearTimer();
    closeTimer = window.setTimeout(close, 120);
  };

  const open = () => {
    clearTimer();
    if (pop) return;
    if (activePop && activePop.trigger !== trigger) activePop.close();
    const content = renderContent();
    if (!content) return;
    pop = h("div", { class: "hover-pop", role: "tooltip" }, content);
    pop.addEventListener("pointerenter", clearTimer);
    pop.addEventListener("pointerleave", scheduleClose);
    document.body.append(pop);
    placeHoverPop(pop, trigger);
    activePop = { trigger, pop, close };
    window.addEventListener("resize", close);
    document.querySelector("main")?.addEventListener("scroll", close, { passive: true });
  };

  trigger.addEventListener("pointerenter", open);
  trigger.addEventListener("pointerleave", scheduleClose);
  trigger.addEventListener("focus", open);
  trigger.addEventListener("blur", scheduleClose);
}
