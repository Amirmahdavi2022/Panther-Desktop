// Panther Desktop — UI logic. Talks to the Rust side through Tauri commands
// when running inside the app, and degrades to a harmless preview in a plain
// browser so the layout can be checked without a build.

const TELEGRAM = "https://t.me/parsv2r";

const invoke = (cmd, args) =>
  window.__TAURI__?.core?.invoke
    ? window.__TAURI__.core.invoke(cmd, args)
    : Promise.reject(new Error("not running inside Tauri"));

const el = (id) => document.getElementById(id);

// Country names in Persian; anything the engine reports that is not listed
// here is shown by its code rather than hidden.
const COUNTRY = {
  AT:"اتریش", AU:"استرالیا", BE:"بلژیک", BG:"بلغارستان", BR:"برزیل", CA:"کانادا",
  CH:"سوئیس", CL:"شیلی", CZ:"چک", DE:"آلمان", DK:"دانمارک", EE:"استونی",
  ES:"اسپانیا", FI:"فنلاند", FR:"فرانسه", GB:"بریتانیا", GR:"یونان", HK:"هنگ‌کنگ",
  HR:"کرواسی", HU:"مجارستان", ID:"اندونزی", IE:"ایرلند", IL:"اسرائیل", IN:"هند",
  IS:"ایسلند", IT:"ایتالیا", JP:"ژاپن", KR:"کره جنوبی", LT:"لیتوانی", LU:"لوکزامبورگ",
  LV:"لتونی", MD:"مولداوی", MX:"مکزیک", MY:"مالزی", NL:"هلند", NO:"نروژ",
  NZ:"نیوزیلند", PH:"فیلیپین", PL:"لهستان", PT:"پرتغال", RO:"رومانی", RS:"صربستان",
  SE:"سوئد", SG:"سنگاپور", SI:"اسلوونی", SK:"اسلواکی", TH:"تایلند", TR:"ترکیه",
  TW:"تایوان", UA:"اوکراین", US:"آمریکا", ZA:"آفریقای جنوبی",
};
const FALLBACK_REGIONS = [
  "AT","BE","BG","CA","CH","CZ","DE","DK","EE","ES","FI","FR","GB","HU","IE",
  "IN","IT","JP","LV","NL","NO","PL","RO","RS","SE","SG","SK","UA","US",
];
const power = el("power");
const statusEl = el("status");
const subEl = el("substatus");
const statsEl = el("stats");
const regionBox = el("regionBox");
const regionSel = el("region");
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
  aether: { label: "Aether", hint: "سریع و پایدار، برای استفاده‌ی روزمره" },
  // Global is carried inside Aether, so it takes two hops to come up and the
  // wait is noticeably longer. Saying so up front stops it looking stuck.
  global: { label: "Global", hint: "دو مرحله‌ای است، کمی بیشتر طول می‌کشد" },
  prowl:  { label: "Prowl",  hint: "هنوز همراه این نسخه نیست" },
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
    syncRegionBox();
  });
});

// ---------------------------------------------------------------------------
// Exit country — only the Global engine can honour one
// ---------------------------------------------------------------------------
function fillRegions(codes) {
  const keep = regionSel.value;
  regionSel.length = 1; // leaves the "automatic" option in place
  for (const code of codes) {
    const o = document.createElement("option");
    o.value = code;
    o.textContent = COUNTRY[code] ? `${COUNTRY[code]} (${code})` : code;
    regionSel.appendChild(o);
  }
  if (keep) regionSel.value = keep;
}

function syncRegionBox() {
  regionBox.hidden = engine !== "global";
  regionSel.disabled = state !== "off";
}

fillRegions(FALLBACK_REGIONS);
invoke("regions")
  .then((list) => { if (Array.isArray(list) && list.length) fillRegions(list); })
  .catch(() => {});

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
  regionSel.disabled = next !== "off";
}

// Shows the country alongside the engine, so a chosen exit is visible at a
// glance rather than buried in a menu.
function labelFor(name, region) {
  if (!region) return name;
  return `${name} · ${COUNTRY[region] || region}`;
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

  paint(
    "busy",
    "در حال اتصال…",
    engine === "global"
      ? "مرحله‌ی اول، بعد مرحله‌ی دوم. تا یک دقیقه طول می‌کشد"
      : ENGINES[engine].hint,
  );
  try {
    const wanted = engine === "global" ? regionSel.value : "";
    const st = await invoke("start_engine", { engine, region: wanted });
    engineEl.textContent = labelFor(st?.engine || ENGINES[engine].label, st?.region);
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
      engineEl.textContent = labelFor(st.engine || "", st.region);
      socksEl.textContent = st.socks || "";
      paint("on", "متصل شدی", "پروکسی را در مرورگرت وارد کن");
      startClock();
    }
  })
  .catch(() => {});

stopClock();
subEl.textContent = ENGINES[engine].hint;
syncRegionBox();
