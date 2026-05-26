const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

const patInput = document.getElementById("pat-input");
const orgSelect = document.getElementById("org-select");
const orgStatus = document.getElementById("org-status");
const intervalInput = document.getElementById("interval-input");
const saveBtn = document.getElementById("save-btn");
const cancelBtn = document.getElementById("cancel-btn");
const removeBtn = document.getElementById("remove-btn");
const howToLink = document.getElementById("how-to-link");
const toast = document.getElementById("toast");

let debouncedListOrgs = null;
let lastListedToken = "";

function showToast(msg, ms = 1800) {
  toast.textContent = msg;
  toast.classList.remove("hidden");
  setTimeout(() => toast.classList.add("hidden"), ms);
}

function setOrgStatus(text, isError = false) {
  orgStatus.textContent = text;
  orgStatus.style.color = isError ? "var(--danger)" : "var(--fg-secondary)";
}

function evaluateCanSave() {
  const hasToken = patInput.value.trim().length > 0;
  const hasOrg = orgSelect.value.length > 0;
  saveBtn.disabled = !(hasToken && hasOrg);
}

async function loadCurrentSettings() {
  try {
    const settings = await invoke("get_settings");
    if (settings?.has_token) {
      patInput.placeholder = "Token saved. Paste a new one to replace.";
      removeBtn.classList.remove("hidden");
    }
    if (settings?.interval_secs) {
      intervalInput.value = settings.interval_secs;
    }
    if (settings?.has_token && settings?.organization_id) {
      // Populate org dropdown with saved org name only (we won't fetch the full list unless they re-paste the token)
      orgSelect.disabled = true;
      orgSelect.innerHTML = `<option value="${settings.organization_id}" selected>${
        settings.organization_name || settings.organization_id
      }</option>`;
      setOrgStatus("Paste a token above to switch organizations.");
    }
    evaluateCanSave();
  } catch (err) {
    console.error("loadCurrentSettings", err);
  }
}

async function tryListOrgs() {
  const token = patInput.value.trim();
  if (token.length < 8) {
    orgSelect.disabled = true;
    orgSelect.innerHTML = `<option value="">Paste token to load organizations…</option>`;
    setOrgStatus("");
    evaluateCanSave();
    return;
  }
  if (token === lastListedToken) return;
  lastListedToken = token;

  orgSelect.disabled = true;
  orgSelect.innerHTML = `<option value="">Loading organizations…</option>`;
  setOrgStatus("Validating token…");

  try {
    const result = await invoke("list_orgs", { token });
    if (!result || !result.orgs || result.orgs.length === 0) {
      orgSelect.innerHTML = `<option value="">No organizations found</option>`;
      setOrgStatus("Token is valid but no organizations are accessible.", true);
      evaluateCanSave();
      return;
    }
    orgSelect.innerHTML = result.orgs
      .map(
        (o) =>
          `<option value="${o.id}" data-name="${o.name}">${o.name}</option>`
      )
      .join("");
    orgSelect.disabled = false;
    setOrgStatus(
      result.orgs.length === 1
        ? "1 organization available."
        : `${result.orgs.length} organizations available — pick one.`
    );
    evaluateCanSave();
  } catch (err) {
    console.error(err);
    orgSelect.innerHTML = `<option value="">Invalid token</option>`;
    setOrgStatus(typeof err === "string" ? err : err?.message || "Token rejected.", true);
    evaluateCanSave();
  }
}

function debounce(fn, ms) {
  let t;
  return (...args) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...args), ms);
  };
}

patInput.addEventListener("input", () => {
  if (debouncedListOrgs) debouncedListOrgs();
});

orgSelect.addEventListener("change", evaluateCanSave);

saveBtn.addEventListener("click", async () => {
  saveBtn.disabled = true;
  saveBtn.textContent = "Saving…";
  try {
    const selected = orgSelect.options[orgSelect.selectedIndex];
    const payload = {
      token: patInput.value.trim() || null,
      organizationId: orgSelect.value,
      organizationName: selected?.dataset?.name || selected?.text || "",
      intervalSecs: Math.max(15, Math.min(300, Number(intervalInput.value) || 30)),
    };
    await invoke("save_settings", payload);
    showToast("Saved");
    setTimeout(() => getCurrentWindow().close(), 600);
  } catch (err) {
    showToast(typeof err === "string" ? err : err?.message || "Save failed");
    saveBtn.disabled = false;
    saveBtn.textContent = "Save";
  }
});

cancelBtn.addEventListener("click", () => {
  getCurrentWindow().close();
});

removeBtn.addEventListener("click", async () => {
  await invoke("clear_account");
  showToast("Account removed");
  setTimeout(() => getCurrentWindow().close(), 500);
});

howToLink.addEventListener("click", (e) => {
  e.preventDefault();
  invoke("open_external", { url: "https://chatbot.yourgpt.ai/dashboard" });
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.preventDefault();
    getCurrentWindow().close();
  }
  if (e.metaKey && e.key === "s") {
    e.preventDefault();
    if (!saveBtn.disabled) saveBtn.click();
  }
});

window.addEventListener("DOMContentLoaded", () => {
  debouncedListOrgs = debounce(tryListOrgs, 600);
  loadCurrentSettings();
  patInput.focus();
});
