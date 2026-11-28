// 沉浸模式：总开关、快捷键、应用列表；添加 / 编辑 / 识别应用都在对话框里完成
import { invoke } from "@tauri-apps/api/core";
import { fmtNum, t } from "../../i18n";
import { S, cfg, loadView, refresh, saveConfig, type ImmersiveTarget, type StateName, type TargetStatus } from "../state";
import {
  avatar,
  confirmDialog,
  emptyState,
  errText,
  field,
  h,
  icon,
  hotkeyInput,
  keys,
  openDialog,
  pageHead,
  section,
  switchEl,
  toast,
  type DialogHandle,
} from "../ui";

interface DetectedApp {
  name: string;
  exe: string;
  kind: "electron" | "cef" | "web_view2";
  port: number | null;
  configured: boolean;
}

/** 应用类型的显示名（产品名，不进语言文件） */
const KIND_LABEL: Record<DetectedApp["kind"], string> = { electron: "Electron", cef: "CEF", web_view2: "WebView2" };

/** 各状态的徽章色调；文字取 immersive.state.<状态> */
const STATE_TONE: Record<StateName, string> = {
  off: "",
  disabled: "",
  not_running: "",
  no_port: "warn",
  conflict: "bad",
  waiting: "warn",
  attached: "ok",
};

/** 状态的显示文字和色调（渲染时取文案，界面语言切换后跟着变） */
export const stateText = (s: StateName): [string, string] => [t(`immersive.state.${s}`), STATE_TONE[s]];

const launching = new Set<number>();

const statusOf = (port: number): TargetStatus | undefined => S.statuses.find((s) => s.port === port);
const attachedList = () => S.statuses.filter((s) => s.state === "attached");

export function badgeOf(port: number): HTMLElement {
  const s = statusOf(port);
  const [label, tone] = s ? stateText(s.state) : [t("immersive.state.checking"), ""];
  return h("span", { class: `badge ${tone}` }, label);
}

/** 全部显示原文 / 译文 */
export function toggleAllButton(cls = "btn"): HTMLElement {
  const attached = attachedList();
  const anyShown = attached.some((s) => s.show);
  return h(
    "button",
    { class: cls, disabled: attached.length === 0, onclick: () => invoke("immersive_set_show", { port: null, show: !anyShown }) },
    icon("swap"),
    anyShown ? t("immersive.showAllSource") : t("immersive.showAllTranslation"),
  );
}

export function setMasterSwitch(v: boolean) {
  return saveConfig((c) => (c.immersive.enabled = v), v ? t("immersive.toast.on") : t("immersive.toast.off"));
}

async function setTargetEnabled(tg: ImmersiveTarget, v: boolean) {
  try {
    await invoke("immersive_set_enabled", { port: tg.port, enabled: v });
    await loadView();
    refresh();
    toast(v ? t("immersive.toast.targetOn", { name: tg.name }) : t("immersive.toast.targetOff", { name: tg.name }));
  } catch (e) {
    toast(errText(e), true);
  }
}

// ───────────────────────── 页面 ─────────────────────────

export function renderImmersive(root: HTMLElement) {
  const im = cfg().immersive;
  const attached = attachedList();
  const translated = attached.reduce((n, s) => n + s.translated, 0);
  const pages = attached.reduce((n, s) => n + s.pages.length, 0);

  const hero = h(
    "div",
    { class: `card hero${im.enabled ? "" : " off"}` },
    h(
      "div",
      { class: "row", style: "gap:16px" },
      h("div", { class: "hero-icon" }, icon("apps")),
      h(
        "div",
        { class: "grow" },
        h("h3", {}, im.enabled ? t("immersive.hero.on") : t("immersive.hero.off")),
        h("div", { class: "desc" }, t("immersive.hero.toggleHotkey"), keys(im.hotkey)),
      ),
      toggleAllButton(),
      switchEl(im.enabled, setMasterSwitch),
    ),
    h(
      "div",
      { class: "metrics" },
      h("div", { class: "metric" }, h("b", {}, `${attached.length} / ${im.targets.length}`), h("span", {}, t("immersive.metric.apps"))),
      h("div", { class: "metric" }, h("b", {}, fmtNum(pages)), h("span", {}, t("immersive.metric.pages"))),
      h("div", { class: "metric" }, h("b", {}, fmtNum(translated)), h("span", {}, t("immersive.metric.translated"))),
    ),
  );

  const hotkey = hotkeyInput(im.hotkey, (v) => saveConfig((c) => (c.immersive.hotkey = v), t("immersive.toast.hotkeySaved", { app: t("app.name") })), "im-hotkey");

  const list = h("div", { class: "list" });
  for (const tg of im.targets) list.append(targetCard(tg));
  if (im.targets.length === 0)
    list.append(
      h(
        "div",
        { class: "card" },
        emptyState(t("immersive.empty"), "apps"),
        h(
          "div",
          { class: "row", style: "justify-content:center; margin-top:-16px; padding-bottom:12px" },
          h("button", { class: "btn primary", onclick: detectDialog }, icon("scan"), t("immersive.detect")),
        ),
      ),
    );

  root.replaceChildren(
    pageHead(t("immersive.lead")),
    hero,
    h(
      "div",
      { class: "card flush" },
      h(
        "div",
        { class: "setting-row" },
        h(
          "div",
          { class: "label" },
          h("div", {}, t("immersive.hotkey.label")),
          h("div", { class: "desc" }, t("immersive.hotkey.desc", { app: t("app.name") })),
        ),
        h("div", { class: "control" }, hotkey),
      ),
    ),
    section(
      t("immersive.apps"),
      h("span", { class: "count" }, String(im.targets.length)),
      h("span", { class: "spacer" }),
      h("button", { class: "btn sm", onclick: detectDialog }, icon("scan"), t("immersive.detect")),
      h("button", { class: "btn ghost sm", onclick: () => appDialog(blankTarget(), null) }, icon("plus"), t("immersive.addManually")),
    ),
    list,
    h("p", { class: "desc", style: "margin:8px 2px 0" }, t("immersive.footnote")),
  );
}

/** 文案里的 {n} 加粗显示：t("…", { n }) 拆成文字 + <b>数字</b> + 文字 */
function boldNum(key: string, n: number): HTMLElement {
  const [before, after = ""] = t(key, { n: "\u0000" }).split("\u0000");
  return h("span", {}, before, h("b", {}, fmtNum(n)), after);
}

function targetCard(tg: ImmersiveTarget): HTMLElement {
  const s = statusOf(tg.port);
  const showLabel = s && s.state === "attached" ? (s.show ? t("immersive.showingTranslation") : t("immersive.showingSource")) : "";

  const actions = h("div", { class: "card-foot" });
  if (s && (s.state === "not_running" || s.state === "no_port")) {
    actions.append(
      h(
        "button",
        { class: "btn primary sm", disabled: launching.has(tg.port), onclick: () => launch(tg) },
        icon("play"),
        launching.has(tg.port) ? t("immersive.launching") : s.state === "no_port" ? t("immersive.relaunch") : t("immersive.launch"),
      ),
    );
  }
  if (s && s.state === "attached") {
    actions.append(
      h(
        "button",
        { class: "btn sm", onclick: () => invoke("immersive_set_show", { port: tg.port, show: !s.show }) },
        icon("swap"),
        s.show ? t("immersive.showSource") : t("immersive.showTranslation"),
      ),
    );
  }
  actions.append(
    h("div", { class: "spacer" }),
    h("button", { class: "btn ghost sm", onclick: () => appDialog({ ...tg }, tg.port) }, icon("edit"), t("common.edit")),
    h("button", { class: "btn ghost danger sm", onclick: () => removeTarget(tg) }, icon("trash"), t("common.delete")),
  );

  const meta = h("div", { class: "meta" }, h("span", { class: "chip" }, t("immersive.port", { port: tg.port })));
  if (s && s.state === "attached") {
    meta.append(
      boldNum("immersive.meta.pages", s.pages.length),
      boldNum("immersive.meta.translated", s.translated),
      boldNum("immersive.meta.batches", s.engine_batches),
    );
  }

  const noteTone = s && (s.state === "conflict" ? "bad" : s.state === "no_port" || s.state === "waiting" ? "warn" : "");
  return h(
    "div",
    { class: `card app-card${tg.enabled && cfg().immersive.enabled ? "" : " dim"}` },
    h(
      "div",
      { class: "row", style: "gap:14px; align-items:flex-start" },
      avatar(tg.name, tg.exe),
      h(
        "div",
        { class: "grow" },
        h("div", { class: "app-title" }, h("h3", {}, tg.name), badgeOf(tg.port), showLabel && h("span", { class: "muted small" }, showLabel)),
        h("div", { class: "path clip", style: "margin-top:2px", title: tg.exe }, tg.exe || t("immersive.noExe")),
      ),
      switchEl(tg.enabled, (v) => setTargetEnabled(tg, v), !cfg().immersive.enabled),
    ),
    meta,
    s && s.message && s.state !== "attached" && h("div", { class: `note ${noteTone}`, style: "margin-top:12px" }, s.message),
    s &&
      s.pages.length > 0 &&
      h(
        "details",
        { class: "pages", style: "margin-top:12px" },
        h("summary", {}, t("immersive.attachedPages", { n: s.pages.length })),
        h("ul", {}, ...s.pages.map((p) => h("li", {}, p.title && h("b", {}, p.title), p.url))),
      ),
    actions,
  );
}

async function removeTarget(tg: ImmersiveTarget) {
  const ok = await confirmDialog(
    t("immersive.remove.title", { name: tg.name }),
    h("p", { class: "muted" }, t("immersive.remove.body")),
    t("common.delete"),
    true,
  );
  if (ok)
    await saveConfig((c) => (c.immersive.targets = c.immersive.targets.filter((x) => x.port !== tg.port)), t("immersive.toast.removed", { name: tg.name }));
}

async function launch(tg: ImmersiveTarget) {
  launching.add(tg.port);
  refresh();
  try {
    toast(await invoke<string>("immersive_launch", { port: tg.port }));
  } catch (e) {
    toast(errText(e), true);
  } finally {
    launching.delete(tg.port);
    await loadView();
    refresh();
  }
}

// ───────────────────────── 对话框 ─────────────────────────

function nextFreePort(): number {
  const used = new Set(cfg().immersive.targets.map((x) => x.port));
  let p = 39222;
  while (used.has(p)) p++;
  return p;
}

const blankTarget = (): ImmersiveTarget => ({
  name: "",
  enabled: true,
  exe: "",
  args: "",
  port: nextFreePort(),
  url_prefixes: [],
  skip_selectors: [],
  auto_relaunch: true,
});

/** 应用表单：返回表单元素和读取结果的函数 */
function appForm(d: ImmersiveTarget) {
  const name = h("input", { type: "text", value: d.name, placeholder: t("immersive.form.namePh") });
  const exe = h("input", { type: "text", value: d.exe, placeholder: t("immersive.form.exePh"), spellcheck: false });
  // 不知道程序装在哪时点"浏览"去选
  const browse = h(
    "button",
    {
      type: "button",
      class: "input-icon",
      title: t("immersive.form.browse"),
      "aria-label": t("immersive.form.browse"),
      onclick: async () => {
        try {
          const p = await invoke<string | null>("pick_exe", { current: exe.value });
          if (p) {
            exe.value = p;
            if (!name.value.trim()) name.value = await invoke<string>("app_name", { exe: p });
          }
        } catch (e) {
          toast(errText(e), true);
        }
      },
    },
    icon("folder"),
  );
  const exeBox = h("div", { class: "input-wrap" }, exe, browse);
  const port = h("input", { type: "number", value: String(d.port), min: "1024", max: "65535" });
  const args = h("textarea", { rows: 2, placeholder: t("immersive.form.argsPh") });
  args.value = d.args;
  const prefixes = h("textarea", { rows: 2, placeholder: t("immersive.form.prefixesPh") });
  prefixes.value = d.url_prefixes.join("\n");
  const skips = h("textarea", { rows: 2, placeholder: t("immersive.form.skipsPh") });
  skips.value = d.skip_selectors.join("\n");
  const auto = h("input", { type: "checkbox", checked: d.auto_relaunch !== false });
  const lines = (v: string) => v.split("\n").map((s) => s.trim()).filter(Boolean);
  const form = h(
    "div",
    { class: "form-grid" },
    field(t("immersive.form.name"), name),
    field(t("immersive.form.port"), port),
    field(t("immersive.form.exe"), exeBox, "wide"),
    h("label", { class: "check wide" }, auto, t("immersive.form.autoRelaunch")),
    h(
      "details",
      { class: "pages wide" },
      h("summary", {}, t("immersive.form.advanced")),
      h(
        "div",
        { class: "form-grid", style: "margin-top:12px" },
        field(t("immersive.form.args"), args, "wide"),
        field(t("immersive.form.prefixes"), prefixes, "wide"),
        field(t("immersive.form.skips"), skips, "wide"),
      ),
    ),
  );
  const read = (): ImmersiveTarget => ({
    ...d,
    name: name.value.trim(),
    exe: exe.value.trim(),
    port: Number(port.value),
    args: args.value.trim(),
    url_prefixes: lines(prefixes.value),
    skip_selectors: lines(skips.value),
    auto_relaunch: auto.checked,
  });
  return { form, read };
}

/** 添加（origPort 为 null）或编辑应用的对话框；可传入已打开的对话框直接切换内容 */
function appDialog(d: ImmersiveTarget, origPort: number | null, existing?: DialogHandle) {
  const isNew = origPort === null;
  const { form, read } = appForm(d);
  let dlg: DialogHandle;
  const saveBtn = h("button", { class: "btn primary" }, isNew ? t("common.add") : t("common.save"));
  saveBtn.onclick = async () => {
    const next = read();
    saveBtn.disabled = true;
    const ok = await saveConfig(
      (c) => {
        if (isNew) c.immersive.targets.push(next);
        else {
          const i = c.immersive.targets.findIndex((x) => x.port === origPort);
          if (i >= 0) c.immersive.targets[i] = next;
        }
      },
      isNew ? t("immersive.toast.added", { name: next.name }) : t("common.saved"),
    );
    saveBtn.disabled = false;
    if (ok) dlg.close();
  };
  const title = isNew ? t("immersive.dialog.addTitle") : t("immersive.dialog.editTitle", { name: d.name });
  const actions = [h("button", { class: "btn", onclick: () => dlg.close() }, t("common.cancel")), saveBtn];
  if (existing) {
    dlg = existing;
    dlg.set(title, form, actions);
    requestAnimationFrame(() => form.querySelector<HTMLInputElement>("input")?.focus());
  } else dlg = openDialog(title, form, actions, { wide: true });
}

/** 识别正在运行、可接管的应用（Electron / WebView2 / CEF），选一个后在同一个对话框里补全信息 */
function detectDialog() {
  const body = h("div", { class: "detect" }, h("div", { class: "loading" }, h("span", { class: "spinner" }), t("immersive.detect.searching")));
  const manual = h("button", { class: "btn ghost", onclick: () => appDialog(blankTarget(), null, dlg) }, icon("plus"), t("immersive.addManually"));
  const dlg = openDialog(
    t("immersive.detect"),
    body,
    [manual, h("span", { class: "spacer" }), h("button", { class: "btn", onclick: () => dlg.close() }, t("common.close"))],
    { wide: true },
  );

  invoke<DetectedApp[]>("immersive_detect")
    .then((apps) => {
      if (apps.length === 0) {
        body.replaceChildren(emptyState(t("immersive.detect.none"), "scan"));
        return;
      }
      body.replaceChildren(
        h("p", { class: "desc", style: "margin:0 0 10px" }, t("immersive.detect.found", { n: apps.length })),
        h(
          "div",
          { class: "pick-list" },
          ...apps.map((a) =>
            h(
              "div",
              { class: `pick-item${a.configured ? " done" : ""}` },
              avatar(a.name, a.exe),
              h(
                "div",
                { class: "grow" },
                h(
                  "div",
                  { class: "row", style: "gap:8px" },
                  h("b", {}, a.name),
                  h("span", { class: "tag" }, KIND_LABEL[a.kind] ?? a.kind),
                  a.port ? h("span", { class: "tag" }, t("immersive.detect.port", { port: a.port })) : null,
                ),
                h("div", { class: "path clip", title: a.exe }, a.exe),
              ),
              a.configured
                ? h("span", { class: "muted small" }, t("immersive.detect.inList"))
                : h(
                    "button",
                    { class: "btn sm", onclick: () => appDialog({ ...blankTarget(), name: a.name, exe: a.exe, port: a.port ?? nextFreePort() }, null, dlg) },
                    t("common.add"),
                  ),
            ),
          ),
        ),
      );
    })
    .catch((e) => body.replaceChildren(h("div", { class: "note bad" }, errText(e))));
}
