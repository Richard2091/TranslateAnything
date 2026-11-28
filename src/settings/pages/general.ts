// 常规：界面语言、目标语言、网络代理、译文库开关、文件位置（显示大小，译文库可搬家）、版本与更新
import { invoke } from "@tauri-apps/api/core";
import { S, cfg, langs, loadStats, loadView, refresh, saveConfig, type UpdateInfo } from "../state";
import { combo, confirmDialog, dropdown, errText, fileSize, h, icon, pageHead, section, switchEl, toast, type IconName } from "../ui";
import { t, uiLocales } from "../../i18n";
import { proxyField } from "./engineShared";

let moving = false;

type AutostartStatus = "disabled" | "enabled" | "blocked" | "stale" | "portable";
let autostart: AutostartStatus | null = null;
let autostartBusy = false;
let autostartError = "";
let autostartLoad: Promise<void> | undefined;

/** 读取 Windows 状态，查询期间停用开关。 */
async function readAutostart() {
  autostart = null;
  autostartError = "";
  try {
    autostart = await invoke<AutostartStatus>("autostart_status");
  } catch (e) {
    autostartError = errText(e);
  }
}

/** 每次进入常规页时重新读取，反映用户在 Windows 中的更改。 */
export function loadAutostart(): Promise<void> | undefined {
  if (autostartBusy) return autostartLoad;
  autostartBusy = true;
  autostartLoad = readAutostart().finally(() => {
    autostartBusy = false;
    autostartLoad = undefined;
  });
  return autostartLoad;
}

/** 用系统回读结果更新开关，保存失败时也重新读取状态。 */
async function setAutostart(enabled: boolean) {
  if (autostartBusy || autostart === null || autostart === "portable") return;
  autostartBusy = true;
  refresh();
  try {
    autostart = await invoke<AutostartStatus>("autostart_set", { enabled });
  } catch (e) {
    toast(errText(e), true);
    await readAutostart();
  } finally {
    autostartBusy = false;
    refresh();
  }
}

async function checkUpdate() {
  S.update.checking = true;
  S.update.error = "";
  refresh();
  try {
    S.update.info = await invoke<UpdateInfo>("update_check");
    if (!S.update.info.newer) toast(t("general.update.latestAlready"));
  } catch (e) {
    S.update.error = errText(e);
  } finally {
    S.update.checking = false;
    refresh();
  }
}

async function installUpdate() {
  S.update.installing = true;
  S.update.progress = 0;
  S.update.error = "";
  refresh();
  try {
    // 安装版：下载完成后启动安装程序，本程序随即退出；便携版：打开下载页
    await invoke("update_install");
  } catch (e) {
    S.update.error = errText(e);
  } finally {
    S.update.installing = false;
    refresh();
  }
}

/** 选了“自定义镜像”但还没填地址时，保持显示输入框 */
let customChannel = false;

/** 更新通道：自动 / 官方 / 内置镜像 / 自定义镜像前缀 */
function channelField(value: string, mirrors: string[]): HTMLElement {
  const v = value.trim();
  const host = (m: string) => m.replace(/^https:\/\//, "").replace(/\/$/, "");
  const builtin = v === "" || v === "auto" || v === "github" || mirrors.includes(v);
  const mode = customChannel || !builtin ? "custom" : v === "auto" ? "" : v;
  const save = (ch: string) => saveConfig((x) => (x.update = { ...x.update, channel: ch }));
  const pick = dropdown(
    [
      { value: "", label: t("general.update.channelAuto") },
      { value: "github", label: t("general.update.channelGithub"), hint: "github.com" },
      ...mirrors.map((m) => ({ value: m, label: host(m), hint: t("general.update.channelMirror") })),
      { value: "custom", label: t("general.update.channelCustom") },
    ],
    mode,
    (m) => {
      customChannel = m === "custom";
      if (customChannel) refresh();
      else save(m);
    },
    { width: "100%" },
  );
  if (mode !== "custom") return pick;
  const addr = combo(
    builtin ? "" : v,
    (a) => {
      if (a) customChannel = false;
      save(a);
    },
    { key: "update-channel", placeholder: t("general.update.channelCustomPh") },
  );
  return h("div", { class: "proxy-field" }, pick, addr);
}

/** 版本与更新卡片 */
function updateCard(row: (label: string, desc: string, control: HTMLElement, auto?: boolean) => HTMLElement): HTMLElement {
  const c = cfg();
  const v = S.version;
  const u = S.update;
  const info = u.info;
  const skipped = !!info && info.latest === c.update?.skip_version;
  const versionText = v ? `v${v.version} · ${v.arch}${v.portable ? ` · ${t("general.update.portable")}` : ""}` : "";

  let status: HTMLElement;
  if (u.installing) {
    status = h("span", { class: "tag" }, t("general.update.downloading", { n: u.progress }));
  } else if (info?.newer) {
    status = h("span", { class: "tag" }, t("general.update.available", { version: info.latest }));
  } else if (info) {
    status = h("span", { class: "muted" }, t("general.update.latest"));
  } else {
    status = h("span");
  }

  const actions: HTMLElement[] = [];
  if (info?.newer && !u.installing) {
    // 没有匹配本机的安装包时只能打开下载页；安装版和便携版都能就地更新
    const pageOnly = !info.asset_name;
    actions.push(
      h("button", { class: "btn sm primary", onclick: installUpdate }, pageOnly ? t("general.update.openPage") : t("general.update.install")),
      skipped
        ? h("span")
        : h("button", {
            class: "btn ghost sm",
            onclick: () => saveConfig((x) => (x.update = { ...x.update, skip_version: info.latest }), t("general.update.skipped")),
          }, t("general.update.skip")),
    );
  }
  actions.push(
    h("button", { class: "btn sm", disabled: u.checking || u.installing, onclick: checkUpdate },
      u.checking ? t("general.update.checking") : t("general.update.check")),
  );

  return h(
    "div",
    { class: "card flush" },
    row(
      t("general.update.version"),
      versionText,
      h("div", { class: "row", style: "gap:8px;justify-content:flex-end;flex-wrap:wrap" }, status, ...actions),
      true,
    ),
    u.error ? h("div", { class: "note bad", style: "margin:0 16px 12px" }, u.error) : h("span"),
    row(
      t("general.update.channel"),
      info?.via ? `${t("general.update.channelDesc")} ${t("general.update.channelVia", { via: info.via })}` : t("general.update.channelDesc"),
      channelField(c.update?.channel ?? "", v?.mirrors ?? []),
    ),
    row(
      t("general.update.auto"),
      t("general.update.autoDesc"),
      switchEl(c.update?.auto_check ?? true, (on) => saveConfig((x) => (x.update = { ...x.update, auto_check: on }))),
      true,
    ),
  );
}

async function reveal(which: "config" | "store") {
  try {
    await invoke("reveal_file", { which });
  } catch (e) {
    toast(errText(e), true);
  }
}

async function relocate(reset: boolean) {
  if (reset) {
    const ok = await confirmDialog(
      t("general.resetConfirmTitle"),
      h("p", { class: "muted" }, t("general.resetConfirmBody")),
      t("general.resetConfirmOk"),
    );
    if (!ok) return;
  }
  moving = true;
  refresh();
  try {
    const path = reset ? await invoke<string>("store_reset_location") : await invoke<string | null>("store_pick_and_relocate");
    if (path) toast(t("general.storeMoved"));
  } catch (e) {
    toast(errText(e), true);
  } finally {
    moving = false;
    await loadView();
    await loadStats();
    refresh();
  }
}

export function renderGeneral(root: HTMLElement) {
  const c = cfg();
  const v = S.view;
  const row = (label: string, desc: string, control: HTMLElement, auto = false) =>
    h(
      "div",
      { class: "setting-row" },
      h("div", { class: "label" }, h("div", {}, label), desc && h("div", { class: "desc" }, desc)),
      h("div", { class: `control${auto ? " auto" : ""}` }, control),
    );

  const fileRow = (ic: IconName, label: string, path: string, size: number, note: string, ...actions: HTMLElement[]) =>
    h(
      "div",
      { class: "file-row" },
      h("span", { class: "file-icon" }, icon(ic)),
      h(
        "div",
        { class: "grow" },
        h("div", { class: "row", style: "gap:8px" }, h("span", { class: "file-name" }, label), h("span", { class: "tag" }, fileSize(size))),
        h("div", { class: "path clip", title: path }, path),
        note && h("div", { class: "desc" }, note),
      ),
      ...actions,
    );

  const autostartLabel = t("general.autostart");
  const autostartDesc = autostartError || t(
    autostart === "portable" ? "general.autostartPortable" :
    autostart === "blocked" ? "general.autostartBlocked" :
    autostart === "stale" ? "general.autostartStale" : "general.autostartDesc",
  );
  const autostartSwitch = switchEl(
    autostart === "enabled", setAutostart, autostartBusy || autostart === null || autostart === "portable",
  );
  const autostartInput = autostartSwitch.querySelector("input")!;
  autostartInput.setAttribute("aria-label", autostartLabel);
  autostartInput.setAttribute("role", "switch");
  const autostartControl = autostart === "stale" || autostart === "blocked"
    ? h("div", { class: "row" },
        h("button", { class: "btn small", type: "button", disabled: autostartBusy, onclick: () => void setAutostart(false) }, t("general.autostartRemove")),
        autostartSwitch)
    : autostartSwitch;

  root.replaceChildren(
    pageHead(t("general.lead")),
    h(
      "div",
      { class: "card flush" },
      row(
        t("general.uiLang"),
        t("general.uiLangDesc"),
        dropdown(
          [{ value: "", label: t("general.uiLangSystem") }, ...uiLocales().map(([tag, name]) => ({ value: tag, label: name, hint: tag }))],
          c.ui_lang ?? "",
          // 保存后后端会发 locale-changed，窗口随之重新载入，所以不提示
          (val) => saveConfig((x) => (x.ui_lang = val), ""),
          { width: "100%" },
        ),
      ),
      row(
        t("general.targetLang"),
        t("general.targetLangDesc"),
        dropdown(
          langs().map(([val, n]) => ({ value: val, label: n, hint: val })),
          c.target_lang,
          (val) => saveConfig((x) => (x.target_lang = val), t("general.targetLangChanged")),
          { width: "100%" },
        ),
      ),
      row(
        t("general.proxy"),
        t("general.proxyDesc"),
        proxyField(c.proxy ?? "", null, (val) => saveConfig((x) => (x.proxy = val))),
      ),
      row(autostartLabel, autostartDesc, autostartControl, true),
      row(
        t("general.useStore"),
        t("general.useStoreDesc"),
        switchEl(c.persist_cache, (on) => saveConfig((x) => (x.persist_cache = on), on ? t("general.storeOn") : t("general.storeOff"))),
        true,
      ),
    ),
    section(t("general.files")),
    h(
      "div",
      { class: "card flush" },
      fileRow(
        "sliders",
        t("general.configFile"),
        v.config_path,
        v.config_size,
        t("general.configFileDesc", { app: t("app.name") }),
        h("button", { class: "btn ghost sm", onclick: () => reveal("config") }, icon("folder"), t("general.openLocation")),
      ),
      fileRow(
        "db",
        t("nav.library"),
        v.db_path,
        v.db_size,
        v.db_custom ? t("general.storeCustom") : "",
        h("button", { class: "btn sm", disabled: moving, onclick: () => relocate(false) }, moving ? t("general.moving") : t("general.changeLocation")),
        v.db_custom ? h("button", { class: "btn ghost sm", disabled: moving, onclick: () => relocate(true) }, t("common.restoreDefault")) : h("span"),
        h("button", { class: "btn ghost sm", onclick: () => reveal("store") }, icon("folder"), t("general.openLocation")),
      ),
    ),
    h("p", { class: "desc", style: "margin:8px 2px 0" }, t("general.enginesHint", { mt: t("nav.mt"), ai: t("nav.ai") })),
    section(t("general.update.title")),
    updateCard(row),
  );
}
