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
const AUTO_LABEL = "خودکار (بهترین سرور)";

const power = el("power");
const statusEl = el("status");
const subEl = el("substatus");
const statsEl = el("stats");
const regionBox = el("regionBox");
const regionBtn = el("regionBtn");
const regionFlag = el("regionFlag");
const regionName = el("regionName");
const picker = el("picker");
const pickerList = el("pickerList");
const pickerSearch = el("pickerSearch");
const uptimeEl = el("uptime");
const engineEl = el("curEngine");
const exitFlag = el("exitFlag");
const exitIp = el("exitIp");

let engine = "aether";
let region = ""; // empty = automatic
let regions = FALLBACK_REGIONS.slice();
let state = "off"; // off | busy | on
let startedAt = 0;
let ticker = null;

const FA_DIGITS = ["۰", "۱", "۲", "۳", "۴", "۵", "۶", "۷", "۸", "۹"];
const fa = (s) => String(s).replace(/\d/g, (d) => FA_DIGITS[+d]);
const nameOf = (code) => COUNTRY[code] || code;

const ENGINES = {
  aether: { label: "Aether", hint: "سریع و پایدار، برای استفاده‌ی روزمره" },
  // Global is carried inside Aether, so it takes two hops to come up and the
  // wait is noticeably longer. Saying so up front stops it looking stuck.
  global: { label: "Global", hint: "با انتخاب کشور خروجی" },
  prowl:  { label: "Prowl",  hint: "هنوز همراه این نسخه نیست" },
};

// ---------------------------------------------------------------------------
// Flags
// ---------------------------------------------------------------------------
const GLOBE =
  '<svg class="globe" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9"/>' +
  '<path d="M3 12h18M12 3c2.5 2.7 3.8 5.7 3.8 9s-1.3 6.3-3.8 9c-2.5-2.7-3.8-5.7-3.8-9S9.5 5.7 12 3z"/></svg>';

// Each flag is its own <img>, so the SVGs never share ids with each other or
// with the page.
function flagHTML(code) {
  const svg = window.FLAGS && window.FLAGS[code];
  if (!svg) return GLOBE;
  const src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(svg);
  return `<img class="flag" alt="" src="${src}" />`;
}

function setFlag(slot, code) {
  slot.innerHTML = code ? flagHTML(code) : GLOBE;
}

// ---------------------------------------------------------------------------
// Engine picker
// ---------------------------------------------------------------------------
document.querySelectorAll(".engine").forEach((btn) => {
  btn.addEventListener("click", () => {
    if (state !== "off") return; // swapping mid-connection would be confusing
    if (btn.classList.contains("soon")) return;
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
function syncRegionBox() {
  regionBox.hidden = engine !== "global";
  regionBtn.disabled = state !== "off";
  setFlag(regionFlag, region);
  regionName.textContent = region ? nameOf(region) : AUTO_LABEL;
}

function renderPicker(filter = "") {
  const q = filter.trim().toLowerCase();
  const sorted = regions
    .slice()
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fa"));
  const items = [["", AUTO_LABEL], ...sorted.map((c) => [c, nameOf(c)])].filter(
    ([code, name]) => !q || name.toLowerCase().includes(q) || code.toLowerCase().includes(q),
  );
  pickerList.innerHTML = "";
  for (const [code, name] of items) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "pick" + (code === region ? " selected" : "") + (code ? "" : " auto");
    b.innerHTML =
      `<span class="flag-slot">${code ? flagHTML(code) : GLOBE}</span>` +
      `<span class="pick-name"></span>` +
      (code ? `<span class="pick-code">${code}</span>` : `<span class="pick-code">پیشنهادی</span>`);
    b.querySelector(".pick-name").textContent = name;
    b.addEventListener("click", () => {
      region = code;
      syncRegionBox();
      closePicker();
    });
    pickerList.appendChild(b);
  }
  if (!items.length) {
    pickerList.innerHTML = '<p class="pick-empty">کشوری پیدا نشد</p>';
  }
}

function openPicker() {
  if (state !== "off") return;
  pickerSearch.value = "";
  renderPicker();
  picker.hidden = false;
  requestAnimationFrame(() => picker.classList.add("open"));
  pickerSearch.focus();
}

function closePicker() {
  picker.classList.remove("open");
  picker.hidden = true;
}

regionBtn.addEventListener("click", openPicker);
el("pickerClose").addEventListener("click", closePicker);
picker.addEventListener("click", (e) => { if (e.target === picker) closePicker(); });
pickerSearch.addEventListener("input", () => renderPicker(pickerSearch.value));
document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !picker.hidden) closePicker(); });

invoke("regions")
  .then((list) => { if (Array.isArray(list) && list.length) regions = list; })
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
  regionBtn.disabled = next !== "off";
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
    exitFlag.innerHTML = "";
    exitIp.textContent = "—";
  }
}

// Asks the engine what the internet actually sees, and shows exactly that.
// A requested country that was not granted is said out loud, not papered over.
async function showExit(requested) {
  exitFlag.innerHTML = "";
  exitIp.textContent = "در حال بررسی…";
  try {
    const x = await invoke("exit_info");
    if (state !== "on") return;
    setFlag(exitFlag, x.country);
    exitIp.textContent = x.ip;
    exitIp.title = nameOf(x.country);
    engineEl.textContent = ENGINES[engine].label;
    if (requested && x.country && x.country !== requested) {
      subEl.textContent = `${nameOf(requested)} الان در دسترس نبود، خروجی: ${nameOf(x.country)}`;
    }
  } catch (e) {
    if (state !== "on") return;
    exitIp.textContent = "نامشخص";
    console.error(e);
  }
}

power.addEventListener("click", async () => {
  if (state === "busy") return;

  if (state === "on") {
    paint("busy", "در حال قطع…", "چند لحظه صبر کن");
    try { await invoke("stop_engine"); } catch {}
    stopClock();
    paint("off", "آماده‌ی اتصال", ENGINES[engine].hint);
    return;
  }

  paint(
    "busy",
    "در حال اتصال…",
    engine === "global"
      ? "مرحله‌ی اول، بعد مرحله‌ی دوم. تا سه دقیقه طول می‌کشد"
      : "بار اول ممکن است تا دو دقیقه طول بکشد",
  );
  try {
    const wanted = engine === "global" ? region : "";
    const st = await invoke("start_engine", { engine, region: wanted });
    engineEl.textContent = st?.engine || ENGINES[engine].label;
    paint("on", "متصل شدی", "همه‌ی برنامه‌ها از تونل رد می‌شوند");
    startClock();
    showExit(wanted);
  } catch (e) {
    const msg = String(e?.message || e);
    let sub = "جزئیات در پوشه‌ی logs برنامه ذخیره شد";
    if (msg.includes("was not found")) sub = "این موتور در این نسخه همراه برنامه نیست";
    else if (msg.includes("tunnel")) sub = "ساخت تونل ممکن نشد. برنامه را با دسترسی Administrator باز کن";
    paint("off", "اتصال برقرار نشد", sub);
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
      paint("on", "متصل شدی", "همه‌ی برنامه‌ها از تونل رد می‌شوند");
      startClock();
      showExit(st.region || "");
    }
  })
  .catch(() => {});

stopClock();
subEl.textContent = ENGINES[engine].hint;
syncRegionBox();
