// 机器翻译与 AI 翻译页面共用的表单控件、测试按钮、拖动排序与状态横幅
import { invoke } from "@tauri-apps/api/core";
import { S, cfg, engineInfo, kindOf, refresh, saveConfig, saving, settled, testEngine, type ProviderConfig } from "../state";
import { t } from "../../i18n";
import { combo, dropdown, field, fmtSec, h, icon, type DropOption } from "../ui";

/** 可配置的参数；标签和提示文字取 engines.field.<参数>.label / .ph */
const FIELDS = new Set(["base_url", "api_key", "region", "model", "proxy"]);
const SECRET = new Set(["api_key"]);
const LLM_PLACEHOLDER = new Set(["base_url"]);

export const blankProvider = (): ProviderConfig => ({ name: "", base_url: "", api_key: "", region: "", model: "", proxy: null });

/** DeepL 免费版地址（免费版密钥以 :fx 结尾） */
const DEEPL_FREE = "https://api-free.deepl.com";

export function setParam(id: string, key: string, raw: string) {
  const v = raw.trim();
  return saveConfig((c) => {
    const p = (c.engines[id] ??= blankProvider());
    if (key === "proxy") p.proxy = v || null;
    else if (key === "proxy") {
      grid.append(field(label, proxyField(value, id, save), wide));
      continue;
    }
    const ph = llm && LLM_PLACEHOLDER.has(key) ? t(`engines.field.${key}.llmPh`) : t(`engines.field.${key}.ph`);
    if (key === "region") {
      grid.append(field(label, combo(value, save, { key: `eng-${id}-region`, placeholder: ph, options: regionOptions() }), wide));
      continue;
    }
    const presets = key === "base_url" ? presetOptions(kind) : null;
    if (presets) {
      grid.append(field(label, combo(value, save, { key: `eng-${id}-base_url`, placeholder: ph, options: presets }), wide));
      continue;
    }
    const input = h("input", {
      type: secret ? "password" : "text",
      value,
      placeholder: ph,
      "data-k": `eng-${id}-${key}`,
      autocomplete: "off",
      spellcheck: false,
    });
    input.addEventListener("change", () => save(input.value));
    let control: HTMLElement = input;
    if (secret) {
      const eye = h(
        "button",
        { type: "button", class: "input-icon", title: t("engines.toggleSecret"), onclick: () => (input.type = input.type === "password" ? "text" : "password") },
        icon("eye"),
      );
      control = h("div", { class: "input-wrap" }, input, eye);
    }
    grid.append(field(label, control, wide));
  }
  return grid;
}

/** "测试"按钮：显示耗时（秒，精确到小数点后两位）或失败 */
export function testButton(id: string, cls = "btn sm"): HTMLElement {
  const r = S.tests[id];
  const label = r?.busy
    ? t("engines.test.busy")
    : r?.result
      ? fmtSec(r.result.ms)
      : r?.error
        ? t("engines.test.failed")
        : t("engines.test.label");
  const tone = r?.busy ? "" : r?.result ? (r.result.ms > 2000 ? " slow" : " good") : r?.error ? " fail" : "";
  const title = r?.result ? `${r.result.source}\n→ ${r.result.text}` : r?.error ?? t("engines.test.hint");
  return h("button", { class: `${cls} test-btn${tone}`, disabled: r?.busy, title, onclick: () => void testEngine(id) }, icon("bolt"), label);
}

/** 测试结果的说明行（失败原因或示例译文） */
export function testNote(id: string, style = "margin-top:12px"): HTMLElement | null {
  const r = S.tests[id];
  if (!r || r.busy) return null;
  if (r.error) return h("div", { class: "note bad", style }, r.error);
  if (r.result) return h("div", { class: "note ok", style }, `${r.result.source} → ${r.result.text}`);
  return null;
}

/**
 * 按住手柄拖动排序（只在启用的行之间移动）。
 */
export function sortable(list: HTMLElement, onDrop: (ids: string[]) => void) {
  const enabledRows = () => [...list.querySelectorAll<HTMLElement>(".engine-row.on")];
  const order = () => enabledRows().map((r) => r.dataset.id!);

  const flip = (el: HTMLElement, mutate: () => void) => {
    const a = el.offsetTop;
    mutate();
    const b = el.offsetTop;
    el.style.transition = "none";
    el.style.transform = `translateY(${a - b}px)`;
    void el.offsetHeight;
    el.style.transition = "";
    el.style.transform = "";
  };

  list.addEventListener("pointerdown", (e) => {
    const grip = (e.target as Element).closest<HTMLButtonElement>(".engine-grip");
    if (!grip || grip.disabled || e.button !== 0) return;
    const row = grip.closest<HTMLElement>(".engine-row")!;
    e.preventDefault();
    grip.setPointerCapture(e.pointerId);
    const before = order().join();
    const listTop = () => list.getBoundingClientRect().top;
    const offset = e.clientY - listTop() - row.offsetTop;
    row.classList.add("dragging");
    list.classList.add("sorting");

    const place = (clientY: number) => {
      const rows = enabledRows();
      const first = rows[0];
      const lastRow = rows[rows.length - 1];
      let top = clientY - listTop() - offset;
      top = Math.max(first.offsetTop, Math.min(top, lastRow.offsetTop + lastRow.offsetHeight - row.offsetHeight));
      for (let guard = 0; guard < rows.length; guard++) {
        const cur = enabledRows();
        const i = cur.indexOf(row);
        const prev = cur[i - 1];
        const next = cur[i + 1];
        if (prev && top < prev.offsetTop + prev.offsetHeight / 2) flip(prev, () => list.insertBefore(prev, row.nextSibling));
        else if (next && top + row.offsetHeight > next.offsetTop + next.offsetHeight / 2) flip(next, () => list.insertBefore(next, row));
        else break;
      }
      row.style.transform = `translateY(${top - row.offsetTop}px)`;
    };

    const move = (ev: PointerEvent) => place(ev.clientY);
    const end = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      row.style.transform = "";
      row.classList.remove("dragging");
      list.classList.remove("sorting");
      const after = order();
      if (after.join() !== before) onDrop(after);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  });
}

/** 子页顶部角色状态横幅：显示当前是否默认，非默认时带「设为默认」按钮 */
export function statusBanner(opts: {
  active: boolean;
  title: string;
  desc: string;
  actionDisabled?: boolean;
  actionTitle?: string;
  onAction?: () => void;
}): HTMLElement {
  return h(
    "div",
    { class: `card status-banner${opts.active ? " active" : ""}` },
    h(
      "div",
      { class: "row spread", style: "gap:16px" },
      h(
        "div",
        { class: "grow" },
        h(
          "div",
          { class: "row", style: "gap:8px" },
          h("h3", {}, opts.title),
          opts.active && h("span", { class: "tag accent" }, t("engines.banner.defaultBadge")),
        ),
        h("div", { class: "desc" }, opts.desc),
      ),
      !opts.active &&
        opts.onAction &&
        h(
          "button",
          {
            type: "button",
            class: "btn primary sm",
            disabled: opts.actionDisabled,
            title: opts.actionTitle,
            onclick: opts.onAction,
          },
          t("engines.banner.setDefault"),
        ),
    ),
  );
}
