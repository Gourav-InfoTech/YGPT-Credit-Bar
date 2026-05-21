const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const BUCKETS = [
  { key: "credits", title: "Credits", type: "bar" },
  { key: "voice_credits", title: "Voice Credits", type: "bar" },
  { key: "voice_lite_credits", title: "Voice Lite Credits", type: "bar" },
  { key: "campaign_credits", title: "Campaign Credits", type: "bar" },
  { key: "chatbot", title: "Chatbots", type: "count" },
  { key: "members", title: "Team Members", type: "count" },
  { key: "document", title: "Documents", type: "count" },
  { key: "webpages", title: "Webpages", type: "count" },
];

function severityClass(pct) {
  if (pct >= 90) return "bucket--severity-alert";
  if (pct >= 70) return "bucket--severity-warn";
  return "bucket--severity-ok";
}

function formatNum(n) {
  if (n == null) return "—";
  const num = Number(n);
  if (Number.isNaN(num)) return String(n);
  return num.toLocaleString();
}

/// "dd MMM yyyy, HH:mm" — matches the dashboard's formatDateTime output.
function formatNextPayment(periodEnd) {
  if (!periodEnd) return "—";
  const d = new Date(periodEnd * 1000);
  if (Number.isNaN(d.getTime())) return "—";
  const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
  const pad = (n) => String(n).padStart(2, "0");
  return `${pad(d.getDate())} ${months[d.getMonth()]} ${d.getFullYear()}, ${pad(
    d.getHours()
  )}:${pad(d.getMinutes())}`;
}

function formatResetIn(periodEnd) {
  if (!periodEnd) return "";
  const now = Date.now() / 1000;
  const diff = periodEnd - now;
  if (diff <= 0) return "Renewing…";
  const days = Math.floor(diff / 86400);
  const hours = Math.floor((diff % 86400) / 3600);
  const mins = Math.floor((diff % 3600) / 60);
  if (days > 0) return `Resets in ${days}d ${hours}h`;
  if (hours > 0) return `Resets in ${hours}h ${mins}m`;
  return `Resets in ${mins}m`;
}

function renderBucket(bucket, data, periodEnd) {
  const usage = Number(data.usage ?? 0);
  const limit = Number(data.limit ?? 0);
  const pct = limit > 0 ? Math.min(100, (usage / limit) * 100) : 0;
  const sev = severityClass(pct);

  if (bucket.type === "count") {
    return `
      <div class="bucket bucket--count">
        <span class="bucket__title">${bucket.title}</span>
        <span class="bucket__meta">${formatNum(usage)} / ${formatNum(limit)}</span>
      </div>
    `;
  }

  return `
    <div class="bucket ${sev}">
      <div class="bucket__head">
        <span class="bucket__title">${bucket.title}</span>
        <span class="bucket__pct">${pct.toFixed(0)}%</span>
      </div>
      <div class="bucket__bar"><div class="bucket__fill" style="width:${pct}%"></div></div>
      <div class="bucket__detail">
        <span>${formatNum(usage)} / ${formatNum(limit)}</span>
        <span>${formatResetIn(periodEnd)}</span>
      </div>
    </div>
  `;
}

function planLabel(plan) {
  if (!plan) return "";
  const status = (plan.subscription_status || "").toLowerCase();
  if (status === "trialing" || status === "free_trial" || status === "trial") return `${plan.plan_name || "Trial"} (Trial)`;
  return plan.plan_name || "Active plan";
}

function statusLabel(s) {
  if (!s) return "—";
  const map = {
    active: "Active",
    trialing: "Trial",
    free_trial: "Free trial",
    trial: "Trial",
    canceled: "Canceled",
    past_due: "Past due",
    expired: "Expired",
    paused: "Paused",
    incomplete: "Incomplete",
  };
  return map[s.toLowerCase()] || s;
}

let isFetching = false;

function updateFreshness() {
  const label = document.getElementById("updated-label");
  if (!label) return;
  label.textContent = isFetching ? "Syncing…" : "";
}

async function render() {
  try {
    const state = await invoke("get_plan_state");
    currentOrgId = state?.organization_id ?? null;

    const empty = document.getElementById("empty-state");
    const bucketsRoot = document.getElementById("buckets");
    const cost = document.getElementById("cost-section");

    if (!state || !state.snapshot) {
      // Clear any previously rendered buckets and show the empty state.
      bucketsRoot.innerHTML = "";
      empty.classList.remove("hidden");
      cost.classList.add("hidden");

      // Keep optimistic org name if we have one configured, otherwise show "Not connected".
      const optimisticOrg = document.getElementById("org-name").textContent;
      const orgFallback =
        optimisticOrg && optimisticOrg !== "Not connected" && state?.has_org
          ? optimisticOrg
          : "Not connected";
      document.getElementById("org-name").textContent = orgFallback;

      let subtitle;
      if (!state?.has_token) subtitle = "Connect your YourGPT account";
      else if (!state?.has_org) subtitle = "Open settings to pick an organization";
      else if (state?.last_error) subtitle = "Fetch failed";
      else subtitle = "Loading usage…";

      document.getElementById("plan-name").textContent = subtitle;

      const heroTitle = empty.querySelector(".empty__title");
      const heroDesc = empty.querySelector(".empty__desc");
      const heroBtn = empty.querySelector("#empty-connect");
      if (state?.has_token && state?.has_org && state?.last_error) {
        heroTitle.textContent = "Couldn't reach YourGPT";
        heroDesc.innerHTML = `<code style="font-family:ui-monospace,monospace;font-size:11px;color:var(--danger);">${escapeHtml(
          state.last_error
        )}</code>`;
        heroBtn.textContent = "Open Settings";
        heroBtn.classList.remove("hidden");
      } else if (state?.has_token && !state?.has_org) {
        heroTitle.textContent = "Pick an organization";
        heroDesc.textContent =
          "Your token is saved. Open settings to select which organization to monitor.";
        heroBtn.textContent = "Open Settings";
        heroBtn.classList.remove("hidden");
      } else if (state?.has_token && state?.has_org && !state?.last_error) {
        heroTitle.textContent = "Fetching usage…";
        heroDesc.textContent = "First poll in progress. This usually takes under a second.";
        heroBtn.classList.add("hidden");
      } else {
        heroTitle.textContent = "Welcome to YGPTCreditBar";
        heroDesc.textContent =
          "Paste your YourGPT API token to start monitoring credits, voice usage, and team caps right from the menu bar.";
        heroBtn.textContent = "Connect account";
        heroBtn.classList.remove("hidden");
      }

      document.getElementById("updated-label").textContent = "";
      return;
    }

    const snap = state.snapshot;
    empty.classList.add("hidden");

    document.getElementById("org-name").textContent = snap.org_name || "—";
    document.getElementById("plan-name").textContent = planLabel(snap);
    updateFreshness();

    const html = BUCKETS.map((b) => {
      const bucket = snap.usage?.[b.key];
      if (!bucket) return "";
      return renderBucket(b, bucket, snap.current_period_end);
    }).join("");

    bucketsRoot.innerHTML = html;

    document.getElementById("cost-plan").textContent = snap.plan_name || "—";
    document.getElementById("cost-status").textContent = statusLabel(snap.subscription_status);
    // Mirror the dashboard's "Plan expired on" vs "Next payment on" labels.
    const isCancelled = (snap.subscription_status || "").toLowerCase() === "canceled";
    document.getElementById("cost-renewal-label").textContent = isCancelled
      ? "Plan expired on"
      : "Next payment";
    document.getElementById("cost-renewal").textContent = formatNextPayment(snap.current_period_end);
    cost.classList.remove("hidden");
  } catch (err) {
    console.error("render failed", err);
  }
}

let currentOrgId = null;

function bindActions() {
  document.getElementById("empty-connect").addEventListener("click", () => {
    invoke("open_settings_window");
  });
  document.getElementById("action-open-dashboard").addEventListener("click", () => {
    invoke("open_external", { url: "https://chatbot.yourgpt.ai/dashboard" });
  });
  document.getElementById("action-manage-billing").addEventListener("click", () => {
    const url = currentOrgId
      ? `https://chatbot.yourgpt.ai/settings/billing?org=${encodeURIComponent(currentOrgId)}`
      : "https://chatbot.yourgpt.ai/settings/billing";
    invoke("open_external", { url });
  });
  document.getElementById("action-refresh").addEventListener("click", async () => {
    await invoke("refresh_now");
    render();
  });
  document.getElementById("action-settings").addEventListener("click", () => {
    invoke("open_settings_window");
  });
  document.getElementById("action-quit").addEventListener("click", () => {
    invoke("quit_app");
  });

  // Org switcher: toggle the dropdown and load orgs on demand
  const switcher = document.getElementById("org-switcher");
  switcher.addEventListener("click", async (e) => {
    e.stopPropagation();
    await toggleOrgMenu();
  });

  // Close the dropdown when clicking elsewhere
  document.addEventListener("click", (e) => {
    const menu = document.getElementById("org-menu");
    if (!menu.classList.contains("hidden")) {
      if (!menu.contains(e.target) && !switcher.contains(e.target)) {
        closeOrgMenu();
      }
    }
  });

  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      closeOrgMenu();
      return;
    }
    if (!e.metaKey) return;
    if (e.key === "r") {
      e.preventDefault();
      invoke("refresh_now").then(render);
    } else if (e.key === ",") {
      e.preventDefault();
      invoke("open_settings_window");
    } else if (e.key === "q") {
      e.preventDefault();
      invoke("quit_app");
    }
  });
}

async function toggleOrgMenu() {
  const menu = document.getElementById("org-menu");
  const switcher = document.getElementById("org-switcher");
  if (!menu.classList.contains("hidden")) {
    closeOrgMenu();
    return;
  }
  // Require a token before showing the list
  const state = await invoke("get_plan_state").catch(() => null);
  if (!state?.has_token) {
    invoke("open_settings_window");
    return;
  }

  menu.classList.remove("hidden");
  switcher.setAttribute("aria-expanded", "true");

  const list = document.getElementById("org-menu-list");
  list.innerHTML = `<div class="org-menu__loading">Loading organizations…</div>`;

  try {
    const result = await invoke("list_orgs", {});
    const orgs = result?.orgs || [];
    if (orgs.length === 0) {
      list.innerHTML = `<div class="org-menu__empty">No organizations found</div>`;
      return;
    }
    list.innerHTML = orgs
      .map(
        (o) => `
        <button class="org-menu__item ${
          String(o.id) === String(currentOrgId) ? "org-menu__item--active" : ""
        }" data-id="${escapeHtml(o.id)}" data-name="${escapeHtml(o.name)}">
          <svg class="org-menu__check" viewBox="0 0 12 12" aria-hidden="true">
            <path d="M2 6.5l2.5 2.5L10 3.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>
          </svg>
          <span class="org-menu__name">${escapeHtml(o.name)}</span>
        </button>
      `
      )
      .join("");

    list.querySelectorAll(".org-menu__item").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const id = btn.dataset.id;
        const name = btn.dataset.name;
        console.log("[org-switch] click", { id, name, currentOrgId });

        if (String(id) === String(currentOrgId)) {
          console.log("[org-switch] same org, no-op");
          closeOrgMenu();
          return;
        }

        closeOrgMenu();
        document.getElementById("org-name").textContent = name;
        document.getElementById("plan-name").textContent = "Loading usage…";

        try {
          const result = await invoke("switch_org", {
            organizationId: id,
            organizationName: name,
          });
          console.log("[org-switch] switch_org returned", result);
          // Update local cache so subsequent dropdowns mark the right row
          currentOrgId = id;
          // Render once now; the snapshot-updated event will re-render with real data
          await render();
        } catch (err) {
          console.error("[org-switch] failed:", err);
          document.getElementById("plan-name").textContent = `Error: ${
            typeof err === "string" ? err : err?.message || "switch failed"
          }`;
        }
      });
    });
  } catch (err) {
    list.innerHTML = `<div class="org-menu__error">${escapeHtml(
      typeof err === "string" ? err : err?.message || "Failed to load orgs"
    )}</div>`;
  }
}

function closeOrgMenu() {
  const menu = document.getElementById("org-menu");
  const switcher = document.getElementById("org-switcher");
  menu.classList.add("hidden");
  switcher.setAttribute("aria-expanded", "false");
}

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

window.addEventListener("DOMContentLoaded", () => {
  bindActions();
  render();
  // Re-render on focus (window shown)
  window.addEventListener("focus", render);
  // Show "Loading…" the moment a fetch starts
  listen("fetch-started", () => {
    isFetching = true;
    updateFreshness();
  }).catch(() => {});
  // Re-render the moment the Rust poller finishes a fetch
  listen("snapshot-updated", () => {
    isFetching = false;
    render();
  }).catch(() => {});
  // Light poll for state changes from the Rust poller
  setInterval(render, 3000);
});
