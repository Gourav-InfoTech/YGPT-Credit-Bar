const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const BUCKETS = [
  { key: "credits", title: "Credits" },
  { key: "voice_credits", title: "Voice Credits" },
  { key: "voice_lite_credits", title: "Voice Lite Credits" },
  { key: "campaign_credits", title: "Campaign Credits" },
  { key: "chatbot", title: "Chatbots" },
  { key: "members", title: "Team Members" },
  { key: "document", title: "Documents" },
  { key: "webpages", title: "Webpages" },
];

function severityClass(pct) {
  if (pct >= 90) return "bucket--severity-alert";
  if (pct >= 70) return "bucket--severity-warn";
  return "bucket--severity-ok";
}

function heroLabel(pct) {
  if (pct >= 100) return "At limit";
  if (pct >= 90) return "Near limit";
  if (pct >= 70) return "Approaching limit";
  return "Highest usage";
}

function bucketPct(data) {
  const limit = Number(data?.limit ?? 0);
  if (limit <= 0) return 0;
  const usage = Number(data?.usage ?? 0);
  return Math.min(100, (usage / limit) * 100);
}

function formatNum(n) {
  if (n == null) return "—";
  const num = Number(n);
  if (Number.isNaN(num)) return String(n);
  const abs = Math.abs(num);
  // Trim to at most one decimal place, drop a trailing ".0" for cleaner output.
  const trim = (v) => v.toFixed(1).replace(/\.0$/, "");
  if (abs >= 1e9) return trim(num / 1e9) + "B";
  if (abs >= 1e6) return trim(num / 1e6) + "M";
  if (abs >= 1e3) return trim(num / 1e3) + "K";
  return String(Math.round(num));
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

function renderHeroBucket(def, data) {
  const usage = Number(data.usage ?? 0);
  const limit = Number(data.limit ?? 0);
  const pct = bucketPct(data);
  const sev = severityClass(pct);

  return `
    <div class="hero-bucket ${sev}">
      <div class="hero-bucket__head">
        <span class="hero-bucket__label">${heroLabel(pct)}</span>
      </div>
      <div class="hero-bucket__main">
        <span class="hero-bucket__pct">${pct.toFixed(0)}%</span>
        <span class="hero-bucket__title">${def.title}</span>
      </div>
      <div class="hero-bucket__bar"><div class="hero-bucket__fill" style="width:${pct}%"></div></div>
      <div class="hero-bucket__detail">
        <span>${formatNum(usage)} used</span>
        <span>of ${formatNum(limit)}</span>
      </div>
    </div>
  `;
}

function renderQuotaRow(def, data) {
  const usage = Number(data.usage ?? 0);
  const limit = Number(data.limit ?? 0);
  const pct = bucketPct(data);
  const sev = severityClass(pct);
  return `
    <li class="quota-row ${sev}">
      <div class="quota-row__top">
        <span class="quota-row__title">${def.title}</span>
        <span class="quota-row__meta">${formatNum(usage)} / ${formatNum(limit)}</span>
      </div>
      <div class="quota-row__bar"><div class="quota-row__fill" style="width:${pct}%"></div></div>
    </li>
  `;
}

function planLabel(plan) {
  if (!plan) return "";
  const status = (plan.subscription_status || "").toLowerCase();
  const name = plan.plan_name || "Trial";
  if (status === "trialing" || status === "free_trial" || status === "trial") {
    // Skip the "(Trial)" suffix when the plan name itself already says "Trial"
    // (otherwise we render "Trial (Trial)").
    return /trial/i.test(name) ? name : `${name} (Trial)`;
  }
  return plan.plan_name || "Active plan";
}

function statusLabel(s) {
  if (!s) return "—";
  // Mirrors yourgpt-chatbot's SubscriptionStatus enum (utils/constants/plans.ts).
  // The server emits both `cancel` (in-flight cancellation) and `canceled` (final state).
  const map = {
    active: "Active",
    trialing: "Trial",
    free_trial: "Free trial",
    trial: "Trial",
    cancel: "Canceling",
    canceled: "Canceled",
    past_due: "Past due",
    expired: "Expired",
    paused: "Paused",
    incomplete: "Incomplete",
    incomplete_expired: "Expired",
    complete: "Completed",
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
      const features = document.getElementById("empty-features");
      const tokenHelp = document.getElementById("empty-token-help");

      // Feature list + token-help link are pure onboarding affordances —
      // only show them in the initial "no token yet" welcome state.
      const isWelcome = !state?.has_token;
      features.classList.toggle("hidden", !isWelcome);
      tokenHelp.classList.toggle("hidden", !isWelcome);

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
          "Live YourGPT usage right in your menu bar. See credits, voice, and team caps at a glance.";
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

    // Pick the worst-pct bucket as the hero, render the rest in a compact list.
    const present = BUCKETS.filter((b) => snap.usage?.[b.key]);
    let worst = null;
    let worstP = -1;
    for (const b of present) {
      const p = bucketPct(snap.usage[b.key]);
      if (p > worstP) {
        worstP = p;
        worst = b;
      }
    }

    const heroHtml = worst ? renderHeroBucket(worst, snap.usage[worst.key]) : "";
    const restHtml = present
      .filter((b) => b !== worst)
      .map((b) => renderQuotaRow(b, snap.usage[b.key]))
      .join("");

    bucketsRoot.innerHTML = `
      ${heroHtml}
      ${restHtml ? `<h3 class="other-quotas-title">Other quotas</h3><ul class="other-quotas">${restHtml}</ul>` : ""}
    `;

    document.getElementById("cost-plan").textContent = snap.plan_name || "—";
    document.getElementById("cost-status").textContent = statusLabel(snap.subscription_status);
    // Mirror the dashboard's "Plan expired on" vs "Next payment on" labels.
    const s = (snap.subscription_status || "").toLowerCase();
    const isTrialing = s === "trialing" || s === "trial" || s === "free_trial";
    // "cancel" = in-flight cancellation, "canceled" = final state, both show plan-expired.
    const isCancelled = s === "canceled" || s === "cancel" || s === "expired" || s === "incomplete_expired";

    let renewalLabel;
    let renewalDate;
    if (isTrialing && snap.trial_expiry) {
      // Trial accounts get a "Trial ends" label sourced from subscriptionData.trail_plan.expiry_date.
      renewalLabel = "Trial ends";
      renewalDate = snap.trial_expiry;
    } else if (isCancelled) {
      renewalLabel = "Plan expired on";
      renewalDate = snap.current_period_end;
    } else {
      renewalLabel = "Next payment";
      renewalDate = snap.current_period_end;
    }

    document.getElementById("cost-renewal-label").textContent = renewalLabel;
    document.getElementById("cost-renewal").textContent = formatNextPayment(renewalDate);
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
  document.getElementById("empty-token-help").addEventListener("click", () => {
    invoke("open_external", { url: "https://chatbot.yourgpt.ai/settings/api-tokens" });
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

  // Close dropdowns when clicking elsewhere.
  document.addEventListener("click", (e) => {
    const orgMenu = document.getElementById("org-menu");
    if (!orgMenu.classList.contains("hidden")) {
      if (!orgMenu.contains(e.target) && !switcher.contains(e.target)) {
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

        if (String(id) === String(currentOrgId)) {
          closeOrgMenu();
          return;
        }

        closeOrgMenu();
        document.getElementById("org-name").textContent = name;
        document.getElementById("plan-name").textContent = "Loading usage…";

        try {
          await invoke("switch_org", {
            organizationId: id,
            organizationName: name,
          });
          // Update local cache so subsequent dropdowns mark the right row
          currentOrgId = id;
          // Render once now; the snapshot-updated event will re-render with real data
          await render();
        } catch (err) {
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
