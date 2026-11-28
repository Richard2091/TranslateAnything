// 新版本对话框：版本信息、更新说明，下载进度与"立即更新 / 跳过 / 稍后"都在对话框里
import { invoke } from "@tauri-apps/api/core";
import { S, cfg, refresh, saveConfig, type UpdateInfo } from "./state";
import { fmtDate, isZh, t } from "../i18n";
import { errText, fileSize, h, openDialog, toast, type DialogHandle } from "./ui";

let dialog: DialogHandle | null = null;
/** 对话框里的进度条与状态，下载进度事件直接改它们，不重建对话框 */
let bar: HTMLElement | null = null;
let barText: HTMLElement | null = null;

const SEEN_KEY = "update-dialog-seen";

/** 手动检查更新：有新版本时弹出对话框 */
export async function checkUpdate() {
  S.update.checking = true;
  S.update.error = "";
  refresh();
  try {
    S.update.info = await invoke<UpdateInfo>("update_check");
    if (S.update.info.newer) openUpdateDialog();
    else toast(t("general.update.latestAlready"));
  } catch (e) {
    S.update.error = errText(e);
  } finally {
    S.update.checking = false;
    refresh();
  }
}

/**
 * 后台检查到新版本时自动弹出：每个版本只自动弹一次，跳过的版本不弹
 * （记在本窗口的浏览器存储里，读不到时最多多弹一次）
 */
export function autoOpenUpdateDialog() {
  const info = S.update.info;
  if (!info?.newer || info.latest === cfg().update?.skip_version || dialog) return;
  try {
    if (localStorage.getItem(SEEN_KEY) === info.latest) return;
    localStorage.setItem(SEEN_KEY, info.latest);
  } catch {
    /* 存储不可用时照常弹出 */
  }
  openUpdateDialog();
}

/** 下载进度（0–100） */
export function onUpdateProgress(n: number) {
  S.update.progress = n;
  if (bar) bar.style.width = `${n}%`;
  if (barText) barText.textContent = t("general.update.downloading", { n });
}

async function install() {
  S.update.installing = true;
  S.update.progress = 0;
  S.update.error = "";
  render();
  refresh();
  try {
    // 下载、校验后启动安装程序（便携版就地替换），本程序随即退出并自动重启
    await invoke("update_install");
  } catch (e) {
    S.update.error = errText(e);
  } finally {
    S.update.installing = false;
    render();
    refresh();
  }
}

/** 打开（或刷新）新版本对话框 */
export function openUpdateDialog() {
  const info = S.update.info;
  if (!info?.newer) return;
  const [title, body, actions] = content(info);
  if (dialog) dialog.set(title, body, actions);
  else
    dialog = openDialog(title, body, actions, {
      wide: true,
      onClose: () => {
        dialog = null;
        bar = barText = null;
      },
    });
}

function render() {
  const info = S.update.info;
  if (dialog && info?.newer) {
    const [title, body, actions] = content(info);
    dialog.set(title, body, actions);
  }
}

function content(info: UpdateInfo): [string, HTMLElement, HTMLElement[]] {
  const u = S.update;
  const pageOnly = !info.asset_name;
  const date = info.published_at ? fmtDate(new Date(info.published_at)) : "";
  const meta = [`v${info.current} → v${info.latest}`, date, info.asset_size ? fileSize(info.asset_size) : ""].filter(Boolean).join(" · ");

  const notes = pickNotes(info.notes, info.latest);
  const pageLink = h("button", { type: "button", class: "link", onclick: () => void invoke("update_open_page").catch((e) => toast(errText(e), true)) }, t("general.update.viewPage"));

  bar = h("div", { class: "update-bar-fill", style: `width:${u.progress}%` });
  barText = h("span", {}, t("general.update.downloading", { n: u.progress }));
  const body = h(
    "div",
    { class: "update-dialog" },
    h("div", { class: "update-meta" }, h("span", { class: "muted" }, meta), pageLink),
    notes ? markdown(notes) : h("div", { class: "update-notes muted" }, t("general.update.noNotes")),
    u.installing &&
      h(
        "div",
        { class: "update-progress" },
        h("div", { class: "update-bar" }, bar),
        h("div", { class: "row spread small muted" }, barText, info.via ? h("span", {}, t("general.update.channelVia", { via: info.via })) : null),
      ),
    u.error ? h("div", { class: "note bad" }, u.error) : null,
  );

  const skipped = info.latest === cfg().update?.skip_version;
  const actions: HTMLElement[] = [
    h(
      "button",
      {
        class: "btn ghost",
        disabled: u.installing || skipped,
        onclick: async () => {
          if (await saveConfig((x) => (x.update = { ...x.update, skip_version: info.latest }), t("general.update.skipped"))) dialog?.close();
        },
      },
      t("general.update.skip"),
    ),
    h("button", { class: "btn", onclick: () => dialog?.close() }, u.installing ? t("general.update.background") : t("general.update.later")),
    h(
      "button",
      { class: "btn primary", disabled: u.installing, onclick: () => (pageOnly ? void invoke("update_open_page").catch((e) => toast(errText(e), true)) : void install()) },
      pageOnly ? t("general.update.openPage") : u.error ? t("general.update.retry") : t("general.update.install"),
    ),
  ];
  return [t("general.update.dialogTitle", { version: info.latest }), body, actions];
}

/**
 * 从 Release 正文里取当前界面语言那一半（上中文、下英文，用单独一行 `---` 分隔），
 * 去掉重复的版本标题和下载说明
 */
function pickNotes(raw: string, version: string): string {
  if (!raw.trim()) return "";
  const parts = raw.split(/^\s*---\s*$/m).map((s) => s.trim()).filter(Boolean);
  const text = (isZh() ? parts[0] : parts[1] ?? parts[0]) ?? "";
  const out: string[] = [];
  let skipping = false;
  for (const line of text.split(/\r?\n/)) {
    const m = /^(#{1,6})\s+(.*)$/.exec(line.trim());
    if (m) {
      skipping = /下载说明|^downloads?$/i.test(m[2].trim());
      if (skipping || m[2].includes(version)) continue;
    }
    if (!skipping) out.push(line);
  }
  return out.join("\n").trim();
}

/** 极简 Markdown：标题、列表、段落、**粗体**、`代码`。只建文本节点，不插入 HTML */
function markdown(text: string): HTMLElement {
  const box = h("div", { class: "update-notes" });
  let list: HTMLUListElement | null = null;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) {
      list = null;
      continue;
    }
    const head = /^(#{1,6})\s+(.*)$/.exec(line);
    if (head) {
      list = null;
      box.append(h("h4", {}, ...inline(head[2])));
      continue;
    }
    const item = /^[-*]\s+(.*)$/.exec(line);
    if (item) {
      if (!list) box.append((list = h("ul")));
      list.append(h("li", {}, ...inline(item[1])));
      continue;
    }
    list = null;
    box.append(h("p", {}, ...inline(line)));
  }
  return box;
}

function inline(s: string): (Node | string)[] {
  return s
    .split(/(\*\*[^*]+\*\*|`[^`]+`)/)
    .filter(Boolean)
    .map((p) => (p.startsWith("**") && p.endsWith("**") ? h("strong", {}, p.slice(2, -2)) : p.startsWith("`") && p.endsWith("`") && p.length > 1 ? h("code", {}, p.slice(1, -1)) : p));
}
