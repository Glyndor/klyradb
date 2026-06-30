// KlyraDB frontend — vanilla JS. The Tauri backend is reached via window.__TAURI__.
// Tauri bridge. The webview reaches the Rust backend through the global
// `window.__TAURI__` (enabled by withGlobalTauri), so no bundler is needed.
const TAURI = () => window.__TAURI__;
const invoke = (cmd, args = {}) => TAURI().core.invoke(cmd, args);

// Facade matching the former binding surface: each method maps to a Tauri
// command, so the rest of the UI stays unchanged. camelCase argument keys are
// converted to the Rust snake_case parameters by Tauri.
const App = {
  SetLocale: (code) => invoke("set_locale", { code }),
  Strings: () => invoke("strings"),
  Locale: () => invoke("locale"),
  Direction: () => invoke("direction"),
  AvailableLocales: () => invoke("available_locales"),
  ListInstances: () => invoke("list_instances"),
  ListVersions: () => invoke("list_versions"),
  StartInstance: (id) => invoke("start_instance", { id }),
  StopInstance: (id) => invoke("stop_instance", { id }),
  DeleteInstance: (id) => invoke("delete_instance", { id }),
  InstallBinary: (id) => invoke("install_binary", { id }),
  UpgradePatch: (id) => invoke("upgrade_patch", { id }),
  SuggestPort: (dbType) => invoke("suggest_port", { dbType }),
  CreateInstance: (name, dbType, version, port) =>
    invoke("create_instance", { name, dbType, version, port }),
};
const api = () => (TAURI() ? App : null);

// Event bridge: Tauri's `listen` returns an unlisten function and delivers the
// payload wrapped in an event object. Track unlisteners by event name so the
// existing subscribe/unsubscribe-by-key call sites keep working, and unwrap
// the payload to match the old callback signature.
const _unlisten = new Map();
async function eventsOn(key, cb) {
  const un = await TAURI().event.listen(key, (e) => cb(e.payload));
  _unlisten.set(key, un);
}
function eventsOff(key) {
  const un = _unlisten.get(key);
  if (un) {
    un();
    _unlisten.delete(key);
  }
}

const LOCALE_NAMES = {
  ar:"العربية", ca:"Català", cs:"Čeština", da:"Dansk",
  de:"Deutsch", el:"Ελληνικά", en:"English", es:"Español",
  fi:"Suomi", fr:"Français", he:"עברית", hi:"हिन्दी",
  hu:"Magyar", id:"Bahasa Indonesia", it:"Italiano",
  ja:"日本語", ko:"한국어", nb:"Norsk bokmål", nl:"Nederlands",
  pl:"Polski", "pt-br":"Português (Brasil)", pt:"Português",
  ro:"Română", ru:"Русский", sv:"Svenska", tr:"Türkçe",
  uk:"Українська", vi:"Tiếng Việt", "zh-cn":"中文(简体)", "zh-tw":"中文(繁體)",
};

const DB_LABELS = {
  postgres: "PostgreSQL",
  mysql: "MySQL",
  mariadb: "MariaDB",
  redis: "Redis",
  mongodb: "MongoDB",
};

// Official database logos as inline SVG (currentColor = brand color from CSS)
const DB_ICONS = {
  postgres: `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M23.5594 14.7228a.5269.5269 0 0 0-.0563-.1191c-.139-.2632-.4768-.3418-1.0074-.2321-1.6533.3411-2.2935.1312-2.5256-.0191 1.342-2.0482 2.445-4.522 3.0411-6.8297.2714-1.0507.7982-3.5237.1222-4.7316a1.5641 1.5641 0 0 0-.1509-.235C21.6931.9086 19.8007.0248 17.5099.0005c-1.4947-.0158-2.7705.3461-3.1161.4794a9.449 9.449 0 0 0-.5159-.0816 8.044 8.044 0 0 0-1.3114-.1278c-1.1822-.0184-2.2038.2642-3.0498.8406-.8573-.3211-4.7888-1.645-7.2219.0788C.9359 2.1526.3086 3.8733.4302 6.3043c.0409.818.5069 3.334 1.2423 5.7436.4598 1.5065.9387 2.7019 1.4334 3.582.553.9942 1.1259 1.5933 1.7143 1.7895.4474.1491 1.1327.1441 1.8581-.7279.8012-.9635 1.5903-1.8258 1.9446-2.2069.4351.2355.9064.3625 1.39.3772a.0569.0569 0 0 0 .0004.0041 11.0312 11.0312 0 0 0-.2472.3054c-.3389.4302-.4094.5197-1.5002.7443-.3102.064-1.1344.2339-1.1464.8115-.0025.1224.0329.2309.0919.3268.2269.4231.9216.6097 1.015.6331 1.3345.3335 2.5044.092 3.3714-.6787-.017 2.231.0775 4.4174.3454 5.0874.2212.5529.7618 1.9045 2.4692 1.9043.2505 0 .5263-.0291.8296-.0941 1.7819-.3821 2.5557-1.1696 2.855-2.9059.1503-.8707.4016-2.8753.5388-4.1012.0169-.0703.0357-.1207.057-.1362.0007-.0005.0697-.0471.4272.0307a.3673.3673 0 0 0 .0443.0068l.2539.0223.0149.001c.8468.0384 1.9114-.1426 2.5312-.4308.6438-.2988 1.8057-1.0323 1.5951-1.6698zM2.371 11.8765c-.7435-2.4358-1.1779-4.8851-1.2123-5.5719-.1086-2.1714.4171-3.6829 1.5623-4.4927 1.8367-1.2986 4.8398-.5408 6.108-.13-.0032.0032-.0066.0061-.0098.0094-2.0238 2.044-1.9758 5.536-1.9708 5.7495-.0002.0823.0066.1989.0162.3593.0348.5873.0996 1.6804-.0735 2.9184-.1609 1.1504.1937 2.2764.9728 3.0892.0806.0841.1648.1631.2518.2374-.3468.3714-1.1004 1.1926-1.9025 2.1576-.5677.6825-.9597.5517-1.0886.5087-.3919-.1307-.813-.5871-1.2381-1.3223-.4796-.839-.9635-2.0317-1.4155-3.5126zm6.0072 5.0871c-.1711-.0428-.3271-.1132-.4322-.1772.0889-.0394.2374-.0902.4833-.1409 1.2833-.2641 1.4815-.4506 1.9143-1.0002.0992-.126.2116-.2687.3673-.4426a.3549.3549 0 0 0 .0737-.1298c.1708-.1513.2724-.1099.4369-.0417.156.0646.3078.26.3695.4752.0291.1016.0619.2945-.0452.4444-.9043 1.2658-2.2216 1.2494-3.1676 1.0128zm2.094-3.988-.0525.141c-.133.3566-.2567.6881-.3334 1.003-.6674-.0021-1.3168-.2872-1.8105-.8024-.6279-.6551-.9131-1.5664-.7825-2.5004.1828-1.3079.1153-2.4468.079-3.0586-.005-.0857-.0095-.1607-.0122-.2199.2957-.2621 1.6659-.9962 2.6429-.7724.4459.1022.7176.4057.8305.928.5846 2.7038.0774 3.8307-.3302 4.7363-.084.1866-.1633.3629-.2311.5454zm7.3637 4.5725c-.0169.1768-.0358.376-.0618.5959l-.146.4383a.3547.3547 0 0 0-.0182.1077c-.0059.4747-.054.6489-.115.8693-.0634.2292-.1353.4891-.1794 1.0575-.11 1.4143-.8782 2.2267-2.4172 2.5565-1.5155.3251-1.7843-.4968-2.0212-1.2217a6.5824 6.5824 0 0 0-.0769-.2266c-.2154-.5858-.1911-1.4119-.1574-2.5551.0165-.5612-.0249-1.9013-.3302-2.6462.0044-.2932.0106-.5909.019-.8918a.3529.3529 0 0 0-.0153-.1126 1.4927 1.4927 0 0 0-.0439-.208c-.1226-.4283-.4213-.7866-.7797-.9351-.1424-.059-.4038-.1672-.7178-.0869.067-.276.1831-.5875.309-.9249l.0529-.142c.0595-.16.134-.3257.213-.5012.4265-.9476 1.0106-2.2453.3766-5.1772-.2374-1.0981-1.0304-1.6343-2.2324-1.5098-.7207.0746-1.3799.3654-1.7088.5321a5.6716 5.6716 0 0 0-.1958.1041c.0918-1.1064.4386-3.1741 1.7357-4.4823a4.0306 4.0306 0 0 1 .3033-.276.3532.3532 0 0 0 .1447-.0644c.7524-.5706 1.6945-.8506 2.802-.8325.4091.0067.8017.0339 1.1742.081 1.939.3544 3.2439 1.4468 4.0359 2.3827.8143.9623 1.2552 1.9315 1.4312 2.4543-1.3232-.1346-2.2234.1268-2.6797.779-.9926 1.4189.543 4.1729 1.2811 5.4964.1353.2426.2522.4522.2889.5413.2403.5825.5515.9713.7787 1.2552.0696.087.1372.1714.1885.245-.4008.1155-1.1208.3825-1.0552 1.717-.0123.1563-.0423.4469-.0834.8148-.0461.2077-.0702.4603-.0994.7662zm.8905-1.6211c-.0405-.8316.2691-.9185.5967-1.0105a2.8566 2.8566 0 0 0 .135-.0406 1.202 1.202 0 0 0 .1342.103c.5703.3765 1.5823.4213 3.0068.1344-.2016.1769-.5189.3994-.9533.6011-.4098.1903-1.0957.333-1.7473.3636-.7197.0336-1.0859-.0807-1.1721-.151zm.5695-9.2712c-.0059.3508-.0542.6692-.1054 1.0017-.055.3576-.112.7274-.1264 1.1762-.0142.4368.0404.8909.0932 1.3301.1066.887.216 1.8003-.2075 2.7014a3.5272 3.5272 0 0 1-.1876-.3856c-.0527-.1276-.1669-.3326-.3251-.6162-.6156-1.1041-2.0574-3.6896-1.3193-4.7446.3795-.5427 1.3408-.5661 2.1781-.463zm.2284 7.0137a12.3762 12.3762 0 0 0-.0853-.1074l-.0355-.0444c.7262-1.1995.5842-2.3862.4578-3.4385-.0519-.4318-.1009-.8396-.0885-1.2226.0129-.4061.0666-.7543.1185-1.0911.0639-.415.1288-.8443.1109-1.3505.0134-.0531.0188-.1158.0118-.1902-.0457-.4855-.5999-1.938-1.7294-3.253-.6076-.7073-1.4896-1.4972-2.6889-2.0395.5251-.1066 1.2328-.2035 2.0244-.1859 2.0515.0456 3.6746.8135 4.8242 2.2824a.908.908 0 0 1 .0667.1002c.7231 1.3556-.2762 6.2751-2.9867 10.5405zm-8.8166-6.1162c-.025.1794-.3089.4225-.6211.4225a.5821.5821 0 0 1-.0809-.0056c-.1873-.026-.3765-.144-.5059-.3156-.0458-.0605-.1203-.178-.1055-.2844.0055-.0401.0261-.0985.0925-.1488.1182-.0894.3518-.1226.6096-.0867.3163.0441.6426.1938.6113.4186zm7.9305-.4114c.0111.0792-.049.201-.1531.3102-.0683.0717-.212.1961-.4079.2232a.5456.5456 0 0 1-.075.0052c-.2935 0-.5414-.2344-.5607-.3717-.024-.1765.2641-.3106.5611-.352.297-.0414.6111.0088.6356.1851z"/></svg>`,
  mysql:    `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M16.405 5.501c-.115 0-.193.014-.274.033v.013h.014c.054.104.146.18.214.273.054.107.1.214.154.32l.014-.015c.094-.066.14-.172.14-.333-.04-.047-.046-.094-.08-.14-.04-.067-.126-.1-.18-.153zM5.77 18.695h-.927a50.854 50.854 0 00-.27-4.41h-.008l-1.41 4.41H2.45l-1.4-4.41h-.01a72.892 72.892 0 00-.195 4.41H0c.055-1.966.192-3.81.41-5.53h1.15l1.335 4.064h.008l1.347-4.064h1.095c.242 2.015.384 3.86.428 5.53zm4.017-4.08c-.378 2.045-.876 3.533-1.492 4.46-.482.716-1.01 1.073-1.583 1.073-.153 0-.34-.046-.566-.138v-.494c.11.017.24.026.386.026.268 0 .483-.075.647-.222.197-.18.295-.382.295-.605 0-.155-.077-.47-.23-.944L6.23 14.615h.91l.727 2.36c.164.536.233.91.205 1.123.4-1.064.678-2.227.835-3.483zm12.325 4.08h-2.63v-5.53h.885v4.85h1.745zm-3.32.135l-1.016-.5c.09-.076.177-.158.255-.25.433-.506.648-1.258.648-2.253 0-1.83-.718-2.746-2.155-2.746-.704 0-1.254.232-1.65.697-.43.508-.646 1.256-.646 2.245 0 .972.19 1.686.574 2.14.35.41.877.615 1.583.615.264 0 .506-.033.725-.098l1.325.772.36-.622zM15.5 17.588c-.225-.36-.337-.94-.337-1.736 0-1.393.424-2.09 1.27-2.09.443 0 .77.167.977.5.224.362.336.936.336 1.723 0 1.404-.424 2.108-1.27 2.108-.445 0-.77-.167-.978-.5zm-1.658-.425c0 .47-.172.856-.516 1.156-.344.3-.803.45-1.384.45-.543 0-1.064-.172-1.573-.515l.237-.476c.438.22.833.328 1.19.328.332 0 .593-.073.783-.22a.754.754 0 00.3-.615c0-.33-.23-.61-.648-.845-.388-.213-1.163-.657-1.163-.657-.422-.307-.632-.636-.632-1.177 0-.45.157-.81.47-1.085.315-.278.72-.415 1.22-.415.512 0 .98.136 1.4.41l-.213.476a2.726 2.726 0 00-1.064-.23c-.283 0-.502.068-.654.206a.685.685 0 00-.248.524c0 .328.234.61.666.85.393.215 1.187.67 1.187.67.433.305.648.63.648 1.168zm9.382-5.852c-.535-.014-.95.04-1.297.188-.1.04-.26.04-.274.167.055.053.063.14.11.214.08.134.218.313.346.407.14.11.28.216.427.31.26.16.555.255.81.416.145.094.293.213.44.313.073.05.12.14.214.172v-.02c-.046-.06-.06-.147-.105-.214-.067-.067-.134-.127-.2-.193a3.223 3.223 0 00-.695-.675c-.214-.146-.682-.35-.77-.595l-.013-.014c.146-.013.32-.066.46-.106.227-.06.435-.047.67-.106.106-.027.213-.06.32-.094v-.06c-.12-.12-.21-.283-.334-.395a8.867 8.867 0 00-1.104-.823c-.21-.134-.476-.22-.697-.334-.08-.04-.214-.06-.26-.127-.12-.146-.19-.34-.275-.514a17.69 17.69 0 01-.547-1.163c-.12-.262-.193-.523-.34-.763-.69-1.137-1.437-1.826-2.586-2.5-.247-.14-.543-.2-.856-.274-.167-.008-.334-.02-.5-.027-.11-.047-.216-.174-.31-.235-.38-.24-1.364-.76-1.644-.072-.18.434.267.862.422 1.082.115.153.26.328.34.5.047.116.06.235.107.356.106.294.207.622.347.897.073.14.153.287.247.413.054.073.146.107.167.227-.094.136-.1.334-.154.5-.24.757-.146 1.693.194 2.25.107.166.362.534.703.393.3-.12.234-.5.32-.835.02-.08.007-.133.048-.187v.015c.094.188.188.367.274.555.206.328.566.668.867.895.16.12.287.328.487.402v-.02h-.015c-.043-.058-.1-.086-.154-.133a3.445 3.445 0 01-.35-.4 8.76 8.76 0 01-.747-1.218c-.11-.21-.202-.436-.29-.643-.04-.08-.04-.2-.107-.24-.1.146-.247.273-.32.453-.127.288-.14.642-.188 1.01-.027.007-.014 0-.027.014-.214-.052-.287-.274-.367-.46-.2-.475-.233-1.238-.06-1.785.047-.14.247-.582.167-.716-.042-.127-.174-.2-.247-.303a2.478 2.478 0 01-.24-.427c-.16-.374-.24-.788-.414-1.162-.08-.173-.22-.354-.334-.513-.127-.18-.267-.307-.368-.52-.033-.073-.08-.194-.027-.274.014-.054.042-.075.094-.09.088-.072.335.022.422.062.247.1.455.194.662.334.094.066.195.193.315.226h.14c.214.047.455.014.655.073.355.114.675.28.962.46a5.953 5.953 0 012.085 2.286c.08.154.115.295.188.455.14.33.313.663.455.982.14.315.275.636.476.897.1.14.502.213.682.286.133.06.34.115.46.188.23.14.454.3.67.454.11.076.443.243.463.378z"/></svg>`,
  mariadb:  `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M23.157 4.412c-.676.284-.79.31-1.673.372-.65.045-.757.057-1.212.209-.75.246-1.395.75-2.02 1.59-.296.398-1.249 1.913-1.249 1.988 0 .057-.65.998-.915 1.32-.574.713-1.08 1.079-2.14 1.59-.77.36-1.224.524-4.102 1.477-1.073.353-2.133.738-2.367.864-.852.449-1.515 1.036-2.203 1.938-1.003 1.32-.972 1.313-3.042.947a12.264 12.264 0 00-.675-.063c-.644-.05-1.023.044-1.332.334L0 17.193l.177.088c.094.05.353.234.561.398.215.17.461.347.55.391.088.044.17.088.183.101.012.013-.089.17-.228.353-.435.581-.593.871-.574 1.048.019.164.032.17.43.17.517-.006.826-.056 1.261-.208.65-.233 2.058-.94 2.784-1.4.776-.5 1.717-.998 1.956-1.042.082-.02.354-.07.594-.114.58-.107 1.464-.095 2.587.05.108.013.373.045.6.064.227.025.43.057.454.076.026.012.474.037.998.056.934.026 1.104.007 1.3-.189.126-.133.385-.631.498-.985.209-.643.417-.921.366-.492-.113.966-.322 1.692-.713 2.411-.259.499-.663 1.092-.934 1.395-.322.347-.315.36.088.315.619-.063 1.471-.397 2.096-.82.827-.562 1.647-1.691 2.19-3.03.107-.27.22-.22.183.083-.013.094-.038.315-.057.498l-.031.328.353-.202c.833-.48 1.414-1.262 2.127-2.884.227-.518.877-2.922 1.073-3.976a9.64 9.64 0 01.271-1.042c.127-.429.196-.555.48-.858.183-.19.625-.555.978-.808.72-.505.953-.75 1.187-1.205.208-.417.284-1.13.132-1.357-.132-.202-.284-.196-.763.006Z"/></svg>`,
  redis:    `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M22.71 13.145c-1.66 2.092-3.452 4.483-7.038 4.483-3.203 0-4.397-2.825-4.48-5.12.701 1.484 2.073 2.685 4.214 2.63 4.117-.133 6.94-3.852 6.94-7.239 0-4.05-3.022-6.972-8.268-6.972-3.752 0-8.4 1.428-11.455 3.685C2.59 6.937 3.885 9.958 4.35 9.626c2.648-1.904 4.748-3.13 6.784-3.744C8.12 9.244.886 17.05 0 18.425c.1 1.261 1.66 4.648 2.424 4.648.232 0 .431-.133.664-.365a100.49 100.49 0 0 0 5.54-6.765c.222 3.104 1.748 6.898 6.014 6.898 3.819 0 7.604-2.756 9.33-8.965.2-.764-.73-1.361-1.261-.73zm-4.349-5.013c0 1.959-1.926 2.922-3.685 2.922-.941 0-1.664-.247-2.235-.568 1.051-1.592 2.092-3.225 3.21-4.973 1.972.334 2.71 1.43 2.71 2.619z"/></svg>`,
  mongodb:  `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M17.193 9.555c-1.264-5.58-4.252-7.414-4.573-8.115-.28-.394-.53-.954-.735-1.44-.036.495-.055.685-.523 1.184-.723.566-4.438 3.682-4.74 10.02-.282 5.912 4.27 9.435 4.888 9.884l.07.05A73.49 73.49 0 0111.91 24h.481c.114-1.032.284-2.056.51-3.07.417-.296.604-.463.85-.693a11.342 11.342 0 003.639-8.464c.01-.814-.103-1.662-.197-2.218zm-5.336 8.195s0-8.291.275-8.29c.213 0 .49 10.695.49 10.695-.381-.045-.765-1.76-.765-2.405z"/></svg>`,
};

function dbIcon(type) {
  return `<span class="db-icon ${type}">${DB_ICONS[type] || ""}</span>`;
}

const DB_CONN_URI = {
  postgres: (i) => `postgresql://${escape(i.user)}@localhost:${i.port}/postgres`,
  mysql:    (i) => `mysql://${escape(i.user)}@localhost:${i.port}`,
  mariadb:  (i) => `mariadb://${escape(i.user)}@localhost:${i.port}`,
  redis:    (_i) => `redis://localhost:${_i.port}`,
  mongodb:  (i) => `mongodb://${escape(i.user)}@localhost:${i.port}`,
};

const state = {
  instances: [],
  versions: [],
  locales: [],
  view: "instances",
  strings: {},
  locale: "en",
  dir: "ltr",
  filter: "all",
  selectedType: "postgres",
};

function t(key, ...args) {
  let s = state.strings[key] ?? key;
  args.forEach((a) => (s = s.replace("%s", a)));
  return s;
}

// ---------- i18n apply ----------
function applyI18n() {
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    el.textContent = t(el.getAttribute("data-i18n"));
  });
  document.documentElement.lang = state.locale;
  document.documentElement.dir = state.dir || "ltr";
  const nameInput = document.getElementById("f-name");
  if (nameInput) nameInput.placeholder = t("modal.name_ph");
  updateTitles();
}

function updateTitles() {
  const titles = {
    instances: ["title.instances", "sub.instances"],
    versions:  ["title.versions",  "sub.versions"],
    logs:      ["title.logs",      "sub.logs"],
    settings:  ["title.settings",  "sub.settings"],
  };
  const [titleKey, subKey] = titles[state.view];
  document.getElementById("view-title").textContent = t(titleKey);
  document.getElementById("view-sub").textContent   = t(subKey);
}

// ---------- view switching ----------
document.querySelectorAll(".nav-item").forEach((btn) => {
  btn.addEventListener("click", () => switchView(btn.dataset.view));
});

function switchView(name) {
  state.view = name;
  document.querySelectorAll(".nav-item").forEach((b) =>
    b.classList.toggle("active", b.dataset.view === name)
  );
  document.querySelectorAll(".view").forEach((v) => v.classList.add("hidden"));
  document.getElementById(`view-${name}`).classList.remove("hidden");
  document.getElementById("btn-new").classList.toggle("hidden", name === "settings");
  updateTitles();
  if (name === "versions") renderVersions();
  if (name === "settings") renderSettings();
}

// ---------- filter bar ----------
document.querySelectorAll(".filter-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    state.filter = btn.dataset.filter;
    document.querySelectorAll(".filter-btn").forEach((b) =>
      b.classList.toggle("active", b.dataset.filter === state.filter)
    );
    renderInstances();
  });
});

// ---------- data ----------
async function loadStrings() {
  const saved = localStorage.getItem("klyraLocale");
  if (saved) {
    state.strings = (await api().SetLocale(saved)) || {};
    state.locale  = saved;
  } else {
    state.strings = (await api().Strings()) || {};
    state.locale  = (await api().Locale())  || "en";
  }
  try { state.dir = (await api().Direction()) || "ltr"; } catch { state.dir = "ltr"; }
  applyI18n();
}

// ---------- theme ----------
function applyTheme(mode) {
  const actual = mode === "system"
    ? (window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark")
    : mode;
  document.documentElement.dataset.theme = actual;
}

function setTheme(mode) {
  localStorage.setItem("klyraTheme", mode);
  applyTheme(mode);
  document.querySelectorAll(".theme-btn").forEach((b) =>
    b.classList.toggle("active", b.dataset.theme === mode)
  );
}

window.matchMedia("(prefers-color-scheme: light)").addEventListener("change", () => {
  if ((localStorage.getItem("klyraTheme") || "dark") === "system") applyTheme("system");
});

// ---------- settings ----------
function renderSettings() {
  const locSel = document.getElementById("setting-locale");
  locSel.innerHTML = "";
  const sorted = [...state.locales].sort((a, b) =>
    (LOCALE_NAMES[a] || a).localeCompare(LOCALE_NAMES[b] || b)
  );
  sorted.forEach((code) => {
    const o = document.createElement("option");
    o.value = code;
    o.textContent = LOCALE_NAMES[code] || code;
    o.selected = code === state.locale;
    locSel.appendChild(o);
  });

  const saved = localStorage.getItem("klyraTheme") || "dark";
  document.querySelectorAll(".theme-btn").forEach((b) =>
    b.classList.toggle("active", b.dataset.theme === saved)
  );
}

async function changeLocale(code) {
  state.strings = (await api().SetLocale(code)) || {};
  state.locale  = code;
  try { state.dir = (await api().Direction()) || "ltr"; } catch { state.dir = "ltr"; }
  localStorage.setItem("klyraLocale", code);
  applyI18n();
  renderInstances();
  if (state.view === "versions")  renderVersions();
  if (state.view === "settings")  renderSettings();
}

async function refresh() {
  if (!api()) return;
  try {
    state.instances = (await api().ListInstances()) || [];
    state.versions  = (await api().ListVersions())  || [];
    if (!state.locales.length) {
      state.locales = (await api().AvailableLocales()) || [];
    }
    renderInstances();
  } catch (e) {
    toast(e.message || String(e), true);
  }
}

// ---------- render instances ----------
function renderInstances() {
  const wrap  = document.getElementById("cards");
  const empty = document.getElementById("empty");
  wrap.innerHTML = "";

  const filtered = state.filter === "all"
    ? state.instances
    : state.instances.filter((i) => i.type === state.filter);

  if (!filtered.length) {
    empty.classList.remove("hidden");
    return;
  }
  empty.classList.add("hidden");

  filtered.forEach((i, idx) => {
    const running        = i.status === "running";
    const needsInstall   = i.status === "needs_install";
    const installing     = i.status === "installing";
    const hasPatch       = !!i.patchUpdate;
    const uri            = (DB_CONN_URI[i.type] || DB_CONN_URI.postgres)(i);
    const label          = DB_LABELS[i.type] || i.type;
    const el             = document.createElement("div");
    el.className         = "card" + (running ? " running" : "") + (needsInstall ? " needs-install" : "");
    el.dataset.type      = i.type;
    el.style.animationDelay = `${idx * 45}ms`;

    let actionBtn;
    if (needsInstall) {
      actionBtn = `<button class="btn-mini install-btn" data-act="install" data-id="${i.id}">⬇ ${t("btn.install") || "Install"}</button>`;
    } else if (installing) {
      actionBtn = `<button class="btn-mini" disabled>⏳ ${t("btn.installing") || "Installing…"}</button>`;
    } else if (running) {
      actionBtn = `<button class="btn-mini" data-act="stop" data-id="${i.id}">${t("btn.stop")}</button>`;
    } else {
      actionBtn = `<button class="btn-mini" data-act="start" data-id="${i.id}">${t("btn.start")}</button>`;
    }

    el.innerHTML = `
      <div class="card-head">
        <div class="card-name">
          <span class="status-dot ${i.status}"></span>
          ${escape(i.name)}
        </div>
        <div class="card-badges">
          <span class="db-badge ${i.type}">${dbIcon(i.type)}${label}</span>
          <span class="card-version">${t("card.version_prefix")}${escape(i.version)}</span>
          ${i.upgradeVersion ? `<span class="upgrade-badge">${t("card.upgrade", escape(i.upgradeVersion))}</span>` : ""}
          ${hasPatch ? `<span class="patch-badge">↑ ${escape(i.patchUpdate)}</span>` : ""}
        </div>
      </div>
      <div class="card-meta">
        <div class="meta-row">
          <span class="meta-label">${t("card.port")}</span>
          <span class="meta-value">${i.port}</span>
        </div>
        ${i.type !== "redis" ? `
        <div class="meta-row">
          <span class="meta-label">${t("card.user")}</span>
          <span class="meta-value">${escape(i.user)}</span>
        </div>` : ""}
        ${needsInstall ? "" : `
        <div class="meta-row" style="grid-column: 1 / -1">
          <span class="meta-label">${t("card.conn")}</span>
          <span class="meta-value dim">${uri}</span>
        </div>`}
      </div>
      ${needsInstall ? `<div class="card-notice">Binary not installed. Click Install to download and configure automatically.</div>` : ""}
      ${i.lastError && !needsInstall ? `<div class="card-error">${escape(i.lastError)}</div>` : ""}
      <div class="card-actions">
        ${actionBtn}
        ${!needsInstall && !installing ? `<button class="btn-mini" data-act="copy" data-id="${i.id}">${t("btn.copy_uri")}</button>` : ""}
        ${hasPatch ? `<button class="btn-mini patch-btn" data-act="patch_upgrade" data-id="${i.id}">↑ ${t("btn.update") || "Update"} ${escape(i.patchUpdate)}</button>` : ""}
        <button class="btn-mini danger" data-act="delete" data-id="${i.id}">${t("btn.delete")}</button>
      </div>
    `;
    wrap.appendChild(el);
  });

  wrap.querySelectorAll("button[data-act]").forEach((b) => {
    b.addEventListener("click", () => handleAction(b.dataset.act, b.dataset.id));
  });
}

async function handleAction(act, id) {
  const inst = state.instances.find((x) => x.id === id);
  try {
    if (act === "start") {
      await api().StartInstance(id);
      toast(t("toast.started"));
    } else if (act === "stop") {
      await api().StopInstance(id);
      toast(t("toast.stopped"));
    } else if (act === "delete") {
      if (!confirm(t("confirm.delete", inst.name))) return;
      await api().DeleteInstance(id);
      toast(t("toast.deleted"));
    } else if (act === "copy") {
      const uri = (DB_CONN_URI[inst.type] || DB_CONN_URI.postgres)(inst);
      await navigator.clipboard.writeText(uri);
      toast(t("toast.copied"));
      return;
    } else if (act === "install") {
      handleInstall(id);
      return;
    } else if (act === "patch_upgrade") {
      handlePatchUpgrade(id);
      return;
    }
    await refresh();
  } catch (e) {
    toast(e.message || String(e), true);
  }
}

async function handleInstall(id) {
  const inst     = state.instances.find((x) => x.id === id);
  const imodal   = document.getElementById("install-modal");
  const title    = document.getElementById("install-modal-title");
  const log      = document.getElementById("install-log");
  const closeBtn = document.getElementById("install-close");
  const spinner  = document.getElementById("install-spinner");

  title.textContent = `Installing ${DB_LABELS[inst.type] || inst.type} ${inst.version}…`;
  log.innerHTML     = "";
  closeBtn.disabled = true;
  spinner.style.display = "";
  imodal.classList.remove("hidden");

  const appendLine = (line) => {
    const p = document.createElement("div");
    p.className   = "log-line";
    p.textContent = line;
    log.appendChild(p);
    log.scrollTop = log.scrollHeight;
  };

  const evKey = "install:progress:" + id;
  await eventsOn(evKey, appendLine);

  try {
    await api().InstallBinary(id);
    spinner.style.display = "none";
    title.textContent = `${DB_LABELS[inst.type] || inst.type} ${inst.version} installed`;
    closeBtn.textContent = t("btn.close") || "Close";
  } catch (e) {
    spinner.style.display = "none";
    appendLine("Error: " + (e.message || String(e)));
  } finally {
    closeBtn.disabled = false;
    eventsOff(evKey);
    await refresh();
  }
}

async function handlePatchUpgrade(id) {
  const inst     = state.instances.find((x) => x.id === id);
  const imodal   = document.getElementById("install-modal");
  const title    = document.getElementById("install-modal-title");
  const log      = document.getElementById("install-log");
  const closeBtn = document.getElementById("install-close");
  const spinner  = document.getElementById("install-spinner");

  const label = DB_LABELS[inst.type] || inst.type;
  title.textContent = `Updating ${label} ${inst.version} → ${inst.patchUpdate}…`;
  log.innerHTML     = "";
  closeBtn.disabled = true;
  spinner.style.display = "";
  imodal.classList.remove("hidden");

  const appendLine = (line) => {
    const p = document.createElement("div");
    p.className   = "log-line";
    p.textContent = line;
    log.appendChild(p);
    log.scrollTop = log.scrollHeight;
  };

  const evKey = "install:progress:" + id;
  await eventsOn(evKey, appendLine);

  try {
    await api().UpgradePatch(id);
    spinner.style.display = "none";
    title.textContent = `${label} updated to ${inst.patchUpdate}`;
    closeBtn.textContent = t("btn.close") || "Close";
  } catch (e) {
    spinner.style.display = "none";
    appendLine("Error: " + (e.message || String(e)));
  } finally {
    closeBtn.disabled = false;
    eventsOff(evKey);
    await refresh();
  }
}

// ---------- versions view ----------
function renderVersions() {
  const wrap = document.getElementById("versions");
  wrap.innerHTML = "";

  const byType = {};
  state.versions.forEach((v) => {
    if (!byType[v.type]) byType[v.type] = [];
    byType[v.type].push(v);
  });

  const typeOrder = ["postgres", "mysql", "mariadb", "redis"];
  const installHintKey = {
    postgres: "ver.install_pg",
    mysql:    "ver.install_mysql",
    mariadb:  "ver.install_mariadb",
    redis:    "ver.install_redis",
  };

  typeOrder.forEach((type) => {
    const versions = byType[type];
    if (!versions || !versions.length) return;
    const label = DB_LABELS[type] || type;

    const group = document.createElement("div");
    group.className = "version-group";
    group.innerHTML = `<div class="version-group-header">
      ${dbIcon(type)}
      <span class="version-group-label">${label}</span>
    </div>`;

    const grid = document.createElement("div");
    grid.className = "version-cards";

    versions.forEach((v) => {
      const card = document.createElement("div");
      card.className = "version-card";
      const hint = type === "postgres"
        ? `${t(installHintKey[type])}${escape(v.major)}`
        : t(installHintKey[type]);
      card.innerHTML = `
        <div class="major ${type}">${escape(v.major)}</div>
        <div>${v.installed
          ? `<span class="status-ok">● ${t("ver.installed")}</span>`
          : `<span class="status-missing">○ ${t("ver.not_installed")}</span>`}
        </div>
        ${v.installed
          ? `<code>${escape(v.binPath)}</code>`
          : `<code>${hint}</code>`}
      `;
      grid.appendChild(card);
    });

    group.appendChild(grid);
    wrap.appendChild(group);
  });
}

// ---------- modal ----------
const modal = document.getElementById("modal");
document.getElementById("btn-new").addEventListener("click", openModal);
document.getElementById("btn-empty-new").addEventListener("click", openModal);
document.getElementById("m-cancel").addEventListener("click", closeModal);
document.getElementById("install-close").addEventListener("click", () => {
  document.getElementById("install-modal").classList.add("hidden");
});
document.getElementById("m-create").addEventListener("click", submitCreate);

// DB type picker
document.querySelectorAll(".db-type-btn").forEach((btn) => {
  btn.addEventListener("click", async () => {
    document.querySelectorAll(".db-type-btn").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    state.selectedType = btn.dataset.type;
    await updateVersionSelect();
    await updatePortSuggestion();
  });
});

async function openModal() {
  state.selectedType = "postgres";
  document.querySelectorAll(".db-type-btn").forEach((b) =>
    b.classList.toggle("active", b.dataset.type === "postgres")
  );
  document.getElementById("f-name").value = "";
  document.getElementById("modal-error").classList.add("hidden");
  await updateVersionSelect();
  await updatePortSuggestion();
  modal.classList.remove("hidden");
  setTimeout(() => document.getElementById("f-name").focus(), 50);
}

async function updateVersionSelect() {
  const sel = document.getElementById("f-version");
  sel.innerHTML = "";
  const typeVersions = state.versions.filter((v) => v.type === state.selectedType);
  typeVersions.forEach((v) => {
    const o    = document.createElement("option");
    o.value    = v.major;
    o.textContent = `${v.label}${v.installed ? "" : " (" + t("ver.not_installed") + ")"}`;
    sel.appendChild(o);
  });
}

async function updatePortSuggestion() {
  try {
    const port = await api().SuggestPort(state.selectedType);
    document.getElementById("f-port").value = port;
  } catch {
    const defaults = { postgres: 5432, mysql: 3306, mariadb: 3316, redis: 6379 };
    document.getElementById("f-port").value = defaults[state.selectedType] || 5432;
  }
}

function closeModal() { modal.classList.add("hidden"); }

async function submitCreate() {
  const name    = document.getElementById("f-name").value.trim();
  const version = document.getElementById("f-version").value;
  const port    = parseInt(document.getElementById("f-port").value, 10);
  const err     = document.getElementById("modal-error");

  if (!name) return showErr(err, t("error.name_required"));
  if (!version) return showErr(err, t("error.select_version"));

  const btn = document.getElementById("m-create");
  btn.disabled = true;
  try {
    await api().CreateInstance(name, state.selectedType, version, port);
    closeModal();
    await refresh();
    toast(t("toast.created"));
  } catch (e) {
    showErr(err, e.message || String(e));
  } finally {
    btn.disabled = false;
  }
}

function showErr(el, msg) {
  el.textContent = msg;
  el.classList.remove("hidden");
}

// ---------- helpers ----------
function escape(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c])
  );
}

let toastTimer;
function toast(msg, isErr = false) {
  const el    = document.getElementById("toast");
  el.textContent = msg;
  el.className   = "toast" + (isErr ? " error" : "");
  el.classList.remove("hidden");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.add("hidden"), 2500);
}

// ---------- settings events ----------
document.getElementById("setting-locale").addEventListener("change", (e) => {
  changeLocale(e.target.value);
});

document.querySelectorAll(".theme-btn").forEach((b) => {
  b.addEventListener("click", () => setTheme(b.dataset.theme));
});

// ---------- boot ----------
window.addEventListener("DOMContentLoaded", () => {
  const tryBoot = async () => {
    if (api()) {
      await loadStrings();
      await refresh();
      setInterval(refresh, 4000);
    } else {
      setTimeout(tryBoot, 100);
    }
  };
  tryBoot();
});
