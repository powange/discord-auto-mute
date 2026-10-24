// Settings window: reads and writes through the Rust commands (src-tauri/src/settings.rs).
// Profile changes are saved at once; the Discord and General tabs have Save / Cancel.
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

let data = null; // get_settings
let settings = null; // saved settings, as the engine uses them
let state = null; // get_state, polled
let selected = null; // id of the profile shown, or null for the "add a game" panel
let layout = null; // the keyboard layout's labels, when the web view knows them
let capture = null; // key capture in progress: { kind: "profile" | "shortcut", pressed, order }
let shortcutDraft = null; // shortcut keys being edited in the Discord tab

const t = (text, values = {}) => {
  let out = (data && data.texts[text]) || text;
  for (const [k, v] of Object.entries(values)) out = out.replaceAll(`{${k}}`, v);
  return out;
};

const NAMED_KEYS = {
  AltLeft: "Left Alt", AltRight: "Right Alt (AltGr)", ControlLeft: "Left Ctrl", ControlRight: "Right Ctrl",
  ShiftLeft: "Left Shift", ShiftRight: "Right Shift", CapsLock: "Caps Lock", Space: "Space", Tab: "Tab",
  Enter: "Enter", Backspace: "Backspace", Escape: "Escape", Insert: "Insert", Delete: "Delete", Home: "Home",
  End: "End", PageUp: "Page Up", PageDown: "Page Down", ArrowUp: "Up arrow", ArrowDown: "Down arrow",
  ArrowLeft: "Left arrow", ArrowRight: "Right arrow", NumpadEnter: "Numpad Enter", NumpadAdd: "Numpad +",
  NumpadSubtract: "Numpad -", NumpadMultiply: "Numpad *", NumpadDivide: "Numpad /", NumpadDecimal: "Numpad .",
  NumLock: "Num Lock", ScrollLock: "Scroll Lock", Pause: "Pause", PrintScreen: "Print Screen",
  ContextMenu: "Menu", MouseMiddle: "Middle mouse button", Mouse4: "Mouse button 4", Mouse5: "Mouse button 5",
};
const META_KEYS = {
  windows: { MetaLeft: "Left Windows", MetaRight: "Right Windows" },
  linux: { MetaLeft: "Left Super", MetaRight: "Right Super" },
};
const MODIFIERS = ["ControlLeft", "ControlRight", "ShiftLeft", "ShiftRight", "AltLeft", "AltRight", "MetaLeft", "MetaRight"];
const MOUSE_BUTTONS = { 1: "MouseMiddle", 3: "Mouse4", 4: "Mouse5" };

function keyLabel(code) {
  const meta = META_KEYS[data.platform][code];
  if (meta) return t(meta);
  if (NAMED_KEYS[code]) return t(NAMED_KEYS[code]);
  const fromLayout = layout && layout.get(code);
  if (fromLayout && fromLayout.trim()) return fromLayout.toUpperCase();
  let m;
  if ((m = /^Key(.)$/.exec(code))) return m[1];
  if ((m = /^Digit(.)$/.exec(code))) return m[1];
  if ((m = /^Numpad(\d)$/.exec(code))) return t("Numpad {n}", { n: m[1] });
  return code;
}

function el(tag, props = {}, ...children) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function chip(text, onRemove, className = "") {
  const node = el("span", { className: `chip ${className}`.trim() }, el("span", { textContent: text }));
  if (onRemove) {
    const remove = el("button", { textContent: "×", title: t("Remove") });
    remove.addEventListener("click", onRemove);
    node.append(remove);
  } else {
    node.classList.add("static");
  }
  return node;
}

function showError(id, message) {
  $(id).textContent = message || "";
}

/* ---------- live state ---------- */

function problemText() {
  if (!state) return "";
  if (state.input_error === "no-permission") {
    return t("Discord Auto-Mute cannot read the keyboard. Run “sudo usermod -aG input $USER”, then log out and back in.");
  }
  if (state.input_error) return t("Keys cannot be read: {error}", { error: state.input_error });
  if (settings.method === "rpc") {
    const s = state.discord.state;
    if (s === "not_configured") return t("Discord application not configured: see the Discord tab.");
    if (s === "needs_authorization") return t(state.discord.detail);
  }
  return "";
}

function discordText() {
  const d = state ? state.discord : { state: "connecting" };
  switch (d.state) {
    case "connected": return [t("Connected"), "good"];
    case "connecting": return [t("Connecting…"), ""];
    case "authorizing": return [t("Waiting for your approval in Discord or in your browser…"), ""];
    case "not_configured": return [t("Not configured: enter your Discord application below."), "bad"];
    case "needs_authorization": return [t(d.detail), "bad"];
    case "unavailable": return [t("Discord is not running or cannot be reached ({reason}).", { reason: d.detail }), "bad"];
    default: return ["", ""];
  }
}

function renderLive() {
  const status = state ? state.status : null;
  let text = t("No game in the foreground");
  if (status && status.paused) text = t("Paused");
  else if (status && status.profile) {
    text = status.muting
      ? t("{name}: Discord microphone muted", { name: status.profile[1] })
      : t("Profile: {name}", { name: status.profile[1] });
  }
  $("live").textContent = text;
  const problem = problemText();
  $("problem").textContent = problem;
  $("problem").hidden = !problem;
  const [discord, kind] = discordText();
  $("discord_state").textContent = discord;
  $("discord_state").className = kind;
  const recent = state ? state.recent : [];
  $("recent").replaceChildren(...recent.map((exe) => el("option", { value: exe })));
  for (const li of $("profiles").children) {
    const active = status && status.profile && status.profile[0] === li.dataset.id;
    li.classList.toggle("active", Boolean(active));
    li.classList.toggle("muting", Boolean(active && status.muting));
  }
}

async function poll() {
  try {
    state = await invoke("get_state");
    renderLive();
  } catch (e) {
    console.error(e);
  }
}

/* ---------- profiles ---------- */

function currentProfile() {
  return settings.profiles.find((p) => p.id === selected) || null;
}

function renderList() {
  const items = settings.profiles.map((p) => {
    const li = el("li", { className: p.enabled ? "" : "off" },
      el("span", { className: "dot" }), el("span", { className: "name", textContent: p.name }));
    li.dataset.id = p.id;
    li.setAttribute("aria-selected", p.id === selected);
    li.title = p.enabled ? p.name : `${p.name} (${t("disabled")})`;
    li.addEventListener("click", () => select(p.id));
    return li;
  });
  $("profiles").replaceChildren(...items);
  $("no-profiles").hidden = settings.profiles.length > 0;
  renderLive();
}

function select(id) {
  stopCapture();
  selected = id;
  renderList();
  renderEditor();
}

function renderEditor() {
  const profile = currentProfile();
  $("editor").hidden = !profile;
  $("adder").hidden = Boolean(profile);
  showError("profile_error", "");
  showError("adder_error", "");
  if (!profile) {
    renderPresets();
    return;
  }
  $("name").value = profile.name;
  $("enabled").checked = profile.enabled;

  const preset = profile.preset && data.presets.find((p) => p.id === profile.preset.id);
  let info = preset ? t("Preset: {name}", { name: preset.name }) : t("Custom profile");
  if (preset && profile.preset && JSON.stringify(profile.preset.base) !== JSON.stringify(pick(profile))) {
    info += ` — ${t("modified")}`;
  }
  $("preset_info").textContent = info;
  if (preset && !preset.key_verified) {
    $("preset_info").append(el("span", {
      className: "unverified",
      textContent: ` — ${t("default key not verified: check it in the game")}`,
    }));
  }
  $("reset").hidden = !preset;

  $("executables").replaceChildren(...profile.executables.map((exe, i) =>
    chip(exe, () => saveProfile((p) => p.executables.splice(i, 1)), "key")));
  if (!profile.executables.length) $("executables").append(el("span", { className: "note bad", textContent: t("No executable: the profile never applies.") }));
  $("keys").replaceChildren(...profile.keys.map((code, i) =>
    chip(keyLabel(code), () => saveProfile((p) => p.keys.splice(i, 1)))));
  if (!profile.keys.length) $("keys").append(el("span", { className: "note bad", textContent: t("No key: the profile never mutes Discord.") }));
  for (const radio of document.querySelectorAll("input[name=mode]")) radio.checked = radio.value === profile.mode;
}

/** The fields a preset sets, to tell whether the profile was modified. */
function pick(profile) {
  return { executables: profile.executables, keys: profile.keys, mode: profile.mode };
}

async function profileAction(action) {
  // The editor and the "add a game" panel each show the errors of their own actions.
  const errorId = selected ? "profile_error" : "adder_error";
  showError(errorId, "");
  try {
    settings = await invoke("profile_action", { action });
    return true;
  } catch (e) {
    showError(errorId, String(e));
    return false;
  }
}

async function saveProfile(change) {
  const profile = structuredClone(currentProfile());
  change(profile);
  if (await profileAction({ kind: "update", profile })) {
    renderList();
    renderEditor();
  }
}

function renderPresets() {
  const query = $("search").value.trim().toLowerCase();
  const used = new Set(settings.profiles.map((p) => p.preset && p.preset.id));
  const items = data.presets
    .filter((p) => !query || p.name.toLowerCase().includes(query))
    .map((preset) => {
      const keys = preset.keys.length ? preset.keys.map(keyLabel).join(", ") : t("no default key");
      const details = el("span", { className: "note", textContent: keys });
      if (!preset.key_verified) details.append(el("span", { className: "unverified", textContent: ` (${t("to check")})` }));
      const add = el("button", { textContent: used.has(preset.id) ? t("Add again") : t("Add") });
      add.addEventListener("click", async () => {
        const before = new Set(settings.profiles.map((p) => p.id));
        if (await profileAction({ kind: "add_preset", preset: preset.id })) {
          const created = settings.profiles.find((p) => !before.has(p.id));
          select(created ? created.id : null);
        }
      });
      return el("li", {}, el("span", { className: "name", textContent: preset.name }), details, add);
    });
  $("presets").replaceChildren(...items);
}

/* ---------- key capture ---------- */

function startCapture(kind) {
  stopCapture();
  capture = { kind, pressed: new Set(), order: [] };
  const [button, hint] = kind === "profile" ? ["add_key", "capture_hint"] : ["record_shortcut", "shortcut_hint"];
  $(button).classList.add("capturing");
  $(hint).hidden = false;
}

function stopCapture() {
  if (!capture) return;
  capture = null;
  for (const id of ["add_key", "record_shortcut"]) $(id).classList.remove("capturing");
  for (const id of ["capture_hint", "shortcut_hint"]) $(id).hidden = true;
}

function addProfileKey(code) {
  stopCapture();
  saveProfile((p) => {
    if (!p.keys.includes(code)) p.keys.push(code);
  });
}

window.addEventListener("keydown", (e) => {
  if (!capture) return;
  e.preventDefault();
  e.stopPropagation();
  if (e.repeat) return;
  if (e.code === "Escape" && !capture.order.length) return stopCapture();
  if (capture.kind === "profile") return addProfileKey(e.code);
  capture.pressed.add(e.code);
  if (!capture.order.includes(e.code)) capture.order.push(e.code);
  renderShortcut(capture.order);
}, true);

window.addEventListener("keyup", (e) => {
  if (!capture || capture.kind !== "shortcut") return;
  e.preventDefault();
  capture.pressed.delete(e.code);
  if (capture.pressed.size || !capture.order.length) return;
  const keys = [...capture.order].sort((a, b) => (MODIFIERS.includes(b) ? 1 : 0) - (MODIFIERS.includes(a) ? 1 : 0));
  stopCapture();
  shortcutDraft = keys;
  renderShortcut(keys);
}, true);

// Mouse buttons 4 and 5 would navigate back and forward in the web view.
for (const type of ["mousedown", "mouseup", "auxclick"]) {
  window.addEventListener(type, (e) => {
    if (e.button === 3 || e.button === 4) e.preventDefault();
    if (type !== "mousedown" || !capture || capture.kind !== "profile") return;
    const code = MOUSE_BUTTONS[e.button];
    if (code) {
      e.preventDefault();
      addProfileKey(code);
    } else if (e.button === 0 && e.target.id !== "add_key") {
      stopCapture();
    }
  }, true);
}

/* ---------- Discord and General tabs ---------- */

function renderShortcut(keys) {
  $("shortcut").replaceChildren(...keys.map((code) => chip(keyLabel(code), null, "key")));
}

function showMethod() {
  const method = document.querySelector("input[name=method]:checked").value;
  $("method-shortcut").hidden = method !== "shortcut";
  $("method-rpc").hidden = method !== "rpc";
}

function fillForms() {
  for (const radio of document.querySelectorAll("input[name=method]")) radio.checked = radio.value === settings.method;
  showMethod();
  $("action").value = settings.shortcut.action;
  shortcutDraft = [...settings.shortcut.keys];
  renderShortcut(shortcutDraft);
  $("redirect").value = data.redirect_uri;
  $("client_id").value = settings.rpc.client_id;
  $("client_secret").value = data.client_secret;
  $("oauth_port").value = settings.rpc.oauth_port;
  $("language").replaceChildren(...data.languages.map(([code, label]) =>
    el("option", { value: code, textContent: code === "auto" ? t(label) : label, selected: code === settings.language })));
  $("autostart").checked = data.autostart;
  showError("error", "");
}

async function save() {
  showError("error", "");
  const payload = {
    language: $("language").value,
    method: document.querySelector("input[name=method]:checked").value,
    shortcut: { keys: shortcutDraft, action: $("action").value },
    client_id: $("client_id").value.trim(),
    client_secret: $("client_secret").value.trim(),
    oauth_port: Number($("oauth_port").value) || 0,
    autostart: $("autostart").checked,
  };
  try {
    await invoke("save_general", { payload });
  } catch (e) {
    showError("error", String(e));
    return;
  }
  // Reload: the language may have changed.
  const tab = document.querySelector(".tabs button[aria-selected=true]").dataset.tab;
  sessionStorage.setItem("tab", tab);
  location.reload();
}

function selectTab(name) {
  stopCapture();
  for (const b of document.querySelectorAll(".tabs button")) b.setAttribute("aria-selected", b.dataset.tab === name);
  for (const s of document.querySelectorAll("main section")) s.hidden = s.id !== `tab-${name}`;
  $("footer").hidden = name === "profiles";
}

/* ---------- start ---------- */

async function load() {
  data = await invoke("get_settings");
  settings = data.settings;
  try {
    if (navigator.keyboard && navigator.keyboard.getLayoutMap) layout = await navigator.keyboard.getLayoutMap();
  } catch {
    layout = null;
  }
  document.documentElement.lang = data.language;
  for (const node of document.querySelectorAll("[data-i18n]")) node.textContent = t(node.dataset.i18n);
  for (const node of document.querySelectorAll("[data-i18n-placeholder]")) node.placeholder = t(node.dataset.i18nPlaceholder);
  document.title = t("{app} – Settings", { app: "Discord Auto-Mute" });
  $("autostart_label").textContent = t(data.platform === "windows" ? "Start with Windows" : "Start on login");
  $("linux_shortcut").hidden = data.platform !== "linux";
  $("version").textContent = `Discord Auto-Mute ${data.version}`;
  fillForms();
  selected = settings.profiles.length ? settings.profiles[0].id : null;
  renderList();
  renderEditor();
  selectTab(sessionStorage.getItem("tab") || location.hash.slice(1) || "profiles");
  sessionStorage.removeItem("tab");
  await poll();
  setInterval(poll, 1000);
}

for (const b of document.querySelectorAll(".tabs button")) b.addEventListener("click", () => selectTab(b.dataset.tab));
$("add").addEventListener("click", () => select(null));
$("search").addEventListener("input", renderPresets);
$("add_custom").addEventListener("click", async () => {
  const before = new Set(settings.profiles.map((p) => p.id));
  if (await profileAction({ kind: "add_custom", name: $("custom_name").value })) {
    $("custom_name").value = "";
    const created = settings.profiles.find((p) => !before.has(p.id));
    select(created ? created.id : null);
  }
});
$("custom_name").addEventListener("keydown", (e) => {
  if (e.key === "Enter") $("add_custom").click();
});
$("name").addEventListener("change", () => {
  const name = $("name").value.trim();
  if (!name) {
    showError("profile_error", t("The profile needs a name."));
    $("name").value = currentProfile().name;
    return;
  }
  saveProfile((p) => (p.name = name));
});
$("enabled").addEventListener("change", () => {
  profileAction({ kind: "set_enabled", id: selected, enabled: $("enabled").checked }).then((ok) => {
    if (ok) {
      renderList();
      renderEditor();
    }
  });
});
function addExecutable() {
  const exe = $("new_exe").value.trim();
  if (!exe) return;
  $("new_exe").value = "";
  saveProfile((p) => p.executables.push(exe));
}
$("add_exe").addEventListener("click", addExecutable);
$("new_exe").addEventListener("keydown", (e) => {
  if (e.key === "Enter") addExecutable();
});
$("add_key").addEventListener("click", () => (capture ? stopCapture() : startCapture("profile")));
for (const radio of document.querySelectorAll("input[name=mode]")) {
  radio.addEventListener("change", () => saveProfile((p) => (p.mode = radio.value)));
}
$("reset").addEventListener("click", () => profileAction({ kind: "reset", id: selected }).then((ok) => {
  if (ok) {
    renderList();
    renderEditor();
  }
}));
$("remove").addEventListener("click", async () => {
  const profile = currentProfile();
  if (!confirm(t("Delete the profile “{name}”?", { name: profile.name }))) return;
  if (await profileAction({ kind: "remove", id: profile.id })) select(settings.profiles.length ? settings.profiles[0].id : null);
});

for (const radio of document.querySelectorAll("input[name=method]")) radio.addEventListener("change", showMethod);
$("record_shortcut").addEventListener("click", () => (capture ? stopCapture() : startCapture("shortcut")));
$("send_shortcut").addEventListener("click", async () => {
  $("send_result").className = "note";
  try {
    await invoke("send_shortcut", { codes: shortcutDraft });
    $("send_result").textContent = t("Sending in 3 seconds: switch to Discord's keybind field now.");
  } catch (e) {
    $("send_result").className = "note bad";
    $("send_result").textContent = String(e);
  }
});
$("portal").addEventListener("click", () => invoke("open_external", { url: "https://discord.com/developers/applications" }));
$("copy").addEventListener("click", () => navigator.clipboard.writeText($("redirect").value));
$("oauth_port").addEventListener("input", () => {
  $("redirect").value = `http://127.0.0.1:${$("oauth_port").value}/callback`;
});
$("show_secret").addEventListener("change", () => ($("client_secret").type = $("show_secret").checked ? "text" : "password"));
$("check").addEventListener("click", async () => {
  $("check_result").className = "note";
  $("check_result").textContent = t("Checking…");
  const [ok, message] = await invoke("check_credentials", { clientId: $("client_id").value, clientSecret: $("client_secret").value });
  $("check_result").className = `note ${ok ? "good" : "bad"}`;
  $("check_result").textContent = message;
});
$("reauthorize").addEventListener("click", () => invoke("reauthorize"));
$("open_log").addEventListener("click", () => invoke("open_log_folder"));
$("save").addEventListener("click", save);
$("cancel").addEventListener("click", fillForms);

load().catch((e) => {
  document.body.textContent = String(e);
});
