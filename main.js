// Panther Desktop — UI logic. Talks to the Rust side through Tauri commands
// when running inside the app, and degrades to a harmless preview in a plain
// browser so the layout can be checked without a build.

const TELEGRAM = "https://t.me/parsv2r";

const invoke = (cmd, args) =>
  window.__TAURI__?.core?.invoke
    ? window.__TAURI__.core.invoke(cmd, args)
    : Promise.reject(new Error("not running inside Tauri"));

const el = (id) => document.getElementById(id);
const power = el("power");
const statusEl = el("status");
const subEl = el("substatus");
const statsEl = el("stats");
const uptimeEl = el("uptime");
const engineEl = el("curEngine");
const socksEl = el("socks");

let engine = "aether";
let state = "off"; // off | busy | on
let startedAt = 0;
let ticker = null;

const FA_DIGITS = ["۰", "۱", "۲", "۳", "۴", "۵", "۶", "۷", "۸", "۹"];
const fa = (s) => String(s).replace(/\d/g, (d) => FA_DIGITS[+d]);

const ENGINES = {
  aether: { label: "Aether", hint: "اتصال از راه WARP / MASQUE" },
  global: { label: "Global", hint: "اتصال از راه Psiphon" },
  prowl:  { label: "Prowl",  hint: "اتصال از راه Xray" },
};

// ---------------------------------------------------------------------------
// Engine picker
// ---------------------------------------------------------------------------
document.querySelectorAll(".engine").forEach((btn) => {
  btn.addEventListener("click", () => {
    if (state !== "off") return; // swapping mid-connection would be confusing
    document.querySelectorAll(".engine").forEach((b) => b.classList.remove("selected"));
    btn.classList.add("selected");
    engine = btn.dataset.engine;
    subEl.textContent = ENGINES[engine].hint;
  });
});

// ---------------------------------------------------------------------------
// Connect / disconnect
// ---------------------------------------------------------------------------
function paint(next, text, sub) {
  state = next;
  power.classList.toggle("busy", next === "busy");
  power.classList.toggle("on", next === "on");
  statusEl.classList.toggle("on", next === "on");
  statsEl.classList.toggle("live", next === "on");
  statusEl.textContent = text;
  if (sub !== undefined) subEl.textContent = sub;
}

function startClock() {
  startedAt = Date.now();
  stopClock(false);
  ticker = setInterval(() => {
    const s = Math.floor((Date.now() - startedAt) / 1000);
    const mm = String(Math.floor(s / 60)).padStart(2, "0");
    const ss = String(s % 60).padStart(2, "0");
    uptimeEl.textContent = fa(`${mm}:${ss}`);
  }, 1000);
}

function stopClock(reset = true) {
  if (ticker) clearInterval(ticker);
  ticker = null;
  if (reset) {
    uptimeEl.textContent = fa("00:00");
    engineEl.textContent = "—";
    socksEl.textContent = "—";
  }
}

power.addEventListener("click", async () => {
  if (state === "busy") return;

  if (state === "on") {
    paint("busy", "در حال قطع…", "چند لحظه صبر کن");
    try { await invoke("stop_engine"); } catch {}
    stopClock();
    paint("off", "آماده‌ی اتصال", "یک موتور انتخاب کن و دکمه را بزن");
    return;
  }

  paint("busy", "در حال اتصال…", `${ENGINES[engine].label} — ${ENGINES[engine].hint}`);
  try {
    const st = await invoke("start_engine", { engine });
    engineEl.textContent = st?.engine || ENGINES[engine].label;
    socksEl.textContent = st?.socks || "";
    paint("on", "متصل شدی", "پروکسی را در مرورگرت وارد کن");
    startClock();
  } catch (e) {
    const msg = String(e?.message || e);
    // A missing core is the one failure worth naming plainly, because the fix
    // is different from "try another engine".
    const missing = msg.includes("was not found");
    paint(
      "off",
      "اتصال برقرار نشد",
      missing ? "این موتور در این نسخه همراه برنامه نیست" : "یک موتور دیگر را امتحان کن",
    );
    console.error(e);
  }
});

// ---------------------------------------------------------------------------
// Window chrome + footer
// ---------------------------------------------------------------------------
el("minBtn").addEventListener("click", () => {
  window.__TAURI__?.window?.getCurrentWindow?.().minimize();
});

el("closeBtn").addEventListener("click", async () => {
  try { await invoke("stop_engine"); } catch {}
  window.__TAURI__?.window?.getCurrentWindow?.().close();
});

el("tg").addEventListener("click", (e) => {
  e.preventDefault();
  if (window.__TAURI__?.opener?.openUrl) window.__TAURI__.opener.openUrl(TELEGRAM);
  else window.open(TELEGRAM, "_blank");
});

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------
invoke("app_version")
  .then((v) => { if (v) el("ver").textContent = `v${v}`; })
  .catch(() => {});

// If the app was reopened while an engine is still up, show the truth.
invoke("engine_status")
  .then((st) => {
    if (st?.running) {
      engineEl.textContent = st.engine || "";
      socksEl.textContent = st.socks || "";
      paint("on", "متصل شدی", "پروکسی را در مرورگرت وارد کن");
      startClock();
    }
  })
  .catch(() => {});

stopClock();
subEl.textContent = ENGINES[engine].hint;
