<script lang="ts">
// 模块作用域计数器（普通 script 块只执行一次）：
// 不能放 script setup 顶层——那里每实例重新执行，计数器会被重置
let avatarUidCounter = 0;
</script>

<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";

// 气泡式会话头像：user 为通用 person 图标；assistant 为官方彩色品牌矢量标
// （claude/codex/gemini/dsh 路径提取自 @lobehub/icons 5.10.1 的 *.Color 组件，
// workbuddy 为官方应用 icon.png 经 vtracer 矢量化），全部内联零依赖。
const props = defineProps<{
  role: "user" | "assistant";
  model?: string | null;
  cliId?: string | null;
}>();

// 渐变 defs id 必须逐实例唯一：页面同时存在多个 ChatAvatar（消息头像/Tab 徽标/侧栏 Pills），
// 同名 id 会让 url(#...) 解析到别的实例的 defs 上；被 v-show 隐藏时引用失效，渐变丢失只剩底色
const avatarUid = ++avatarUidCounter;
const gradIds = {
  codex: `cb-codex-grad-${avatarUid}`,
  geminiG1: `cb-gemini-g1-${avatarUid}`,
  geminiG2: `cb-gemini-g2-${avatarUid}`,
  geminiG3: `cb-gemini-g3-${avatarUid}`,
  agMask: `cb-ag-mask-${avatarUid}`,
  agF1: `cb-ag-f1-${avatarUid}`,
  agF2: `cb-ag-f2-${avatarUid}`,
  agF3: `cb-ag-f3-${avatarUid}`,
  agF4: `cb-ag-f4-${avatarUid}`,
  agF5: `cb-ag-f5-${avatarUid}`,
  agF6: `cb-ag-f6-${avatarUid}`,
  agF7: `cb-ag-f7-${avatarUid}`,
  agF8: `cb-ag-f8-${avatarUid}`,
  agF9: `cb-ag-f9-${avatarUid}`,
  agF10: `cb-ag-f10-${avatarUid}`,
  agF11: `cb-ag-f11-${avatarUid}`,
};

type ProviderBrand = "claude" | "codex" | "gemini" | "workbuddy" | "dsh" | "antigravity" | "opencode" | "goose" | "generic";

const brand = computed<ProviderBrand>(() => {
  if (props.cliId) {
    const c = props.cliId.toLowerCase();
    if (c === "claude") return "claude";
    if (c === "codex") return "codex";
    if (c === "gemini") return "gemini";
    if (c === "workbuddy") return "workbuddy";
    if (c === "dsh") return "dsh";
    if (c === "antigravity" || c === "agy") return "antigravity";
    if (c === "opencode") return "opencode";
    if (c === "goose") return "goose";
  }
  if (!props.model) return "generic";
  const m = props.model.toLowerCase();
  if (m.includes("claude") || m.includes("anthropic")) return "claude";
  if (m.includes("codex") || m.includes("gpt") || m.includes("o1") || m.includes("o3") || m.includes("openai")) return "codex";
  if (m.includes("antigravity")) return "antigravity";
  if (m.includes("gemini") || m.includes("google")) return "gemini";
  if (m.includes("dsh") || m.includes("deepseek")) return "dsh";
  if (m.includes("workbuddy")) return "workbuddy";
  if (m.includes("goose")) return "goose";
  return "generic";
});

const avatarTitle = computed(() => {
  // 模型名 / 厂商名是身份值，不进语言包，只作为插值传进去
  if (props.role === "user") return t("chat.avatar.roleUser");
  return props.model
    ? t("chat.messageHeader.modelTooltip", { model: props.model })
    : props.cliId
      ? t("chat.avatar.providerTooltip", { provider: props.cliId })
      : t("chat.avatar.roleAssistant");
});

// ─── 官方品牌矢量路径（viewBox 0 0 24 24，workbuddy 为 0 0 1024 1024）───
// OpenCode 官方标记：一个方框挖空（evenodd 让内圈成为洞），故单路径即可。
const OPENCODE_PATH = "M16 6H8v12h8V6zm4 16H4V2h16v20z";
// Goose 官方标记：矢量鹅标
const GOOSE_PATH =
  "M20.9093 19.3861L19.5185 18.2413C18.7624 17.619 18.1189 16.8713 17.6157 16.0313C16.9205 14.8706 15.9599 13.8912 14.8133 13.1735L14.2533 12.8475C14.0614 12.7141 13.9276 12.5062 13.9086 12.2716C13.8963 12.1204 13.9326 11.9852 14.0171 11.8662C14.3087 11.4553 15.896 9.74698 16.1722 9.51845C16.528 9.22442 16.9243 8.97987 17.2921 8.69986C17.3443 8.66 17.3968 8.62035 17.4485 8.57989C17.4503 8.57808 17.4529 8.57668 17.4545 8.57508C17.5725 8.48195 17.6838 8.383 17.7724 8.26563C18.2036 7.76631 18.195 7.3443 18.195 7.3443C18.195 7.3443 18.1954 7.3439 18.1956 7.3437C18.1497 7.23133 17.9847 6.88163 17.6492 6.71759C17.9458 6.71178 18.2805 6.82294 18.4323 6.97156C18.6148 6.68534 18.7328 6.49967 18.9162 6.18762C18.9599 6.11352 18.9831 5.97652 18.8996 5.89981C18.8996 5.89981 18.8992 5.89981 18.8988 5.89981C18.8988 5.89981 18.8988 5.8994 18.8988 5.899C18.8972 5.8974 18.8952 5.8962 18.8936 5.8946C18.892 5.893 18.891 5.89119 18.8892 5.88939C18.8892 5.88939 18.8888 5.88939 18.8884 5.88939C18.8884 5.88899 18.8884 5.88859 18.885 5.88518 18.8812 5.88258 18.8776 5.87938C18.8754 5.87717 18.8736 5.87457 18.8716 5.87217C18.8692 5.87016 18.8665 5.86836 18.8643 5.86616C18.8609 5.86275 18.8587 5.85855 18.8551 5.85534C18.8551 5.85534 18.8545 5.85514 18.8543 5.85534C18.8543 5.85494 18.8543 5.85454 18.8527 5.85294 18.8507 5.85174 18.8491 5.85013C18.8475 5.84853 18.8463 5.84653 18.8447 5.84493C18.8447 5.84493 18.8441 5.84473 18.8439 5.84493C18.8439 5.84453 18.8439 5.84413 18.7672 5.7606 18.6302 5.78384 18.5561 5.8275C18.1503 6.06625 17.7555 6.32322 17.3996 6.54855C17.3996 6.54855 16.9778 6.53973 16.4783 6.97116C16.3607 7.05989 16.2618 7.17125 16.1688 7.28902C16.167 7.29082 16.1654 7.29322 16.164 7.29503C16.1234 7.3465 16.0837 7.39898 16.0441 7.45145C15.7639 7.81939 15.5195 8.21556 15.2255 8.57128C14.9971 8.84768 13.2887 10.4348 12.8777 10.7264C12.7587 10.8109 12.6237 10.8474 12.4723 10.835C12.2379 10.8161 12.0298 10.6821 11.8965 10.4903L11.5704 9.93024C10.8527 8.78318 9.87332 7.82299 8.71264 7.12778C7.87262 6.62466 7.12514 5.98092 6.50264 5.22503L5.35778 3.83421C5.3013 3.76571 5.19314 3.77693 5.15268 3.85585C5.02249 4.10941 4.77393 4.64479 4.58346 5.36483C4.57885 5.38186 4.58286 5.39988 4.59407 5.4135C4.83082 5.69952 5.37901 6.32983 6.03196 6.863C6.07742 6.90005 6.04017 6.97336 5.98369 6.95774C5.42047 6.80432 4.87288 6.55796 4.46308 6.34805C4.42964 6.33103 4.38918 6.35226 4.38437 6.38951C4.32068 6.89985 4.30425 7.46027 4.37155 8.05112C4.37355 8.07035 4.38577 8.08697 4.4036 8.09479C4.87088 8.29808 5.61816 8.59311 6.40269 8.78078C6.45958 8.7944 6.45777 8.87632 6.40029 8.88733C5.78941 9.0023 5.14968 9.02794 4.62973 9.02113C4.59327 9.02073 4.56643 9.05518 4.57625 9.09023C4.6806 9.45896 4.822 9.8339 5.00847 10.2115C5.08559 10.3811 5.16951 10.5475 5.25944 10.7104C5.27486 10.7382 5.3047 10.7548 5.33655 10.7534C5.76577 10.7324 6.28452 10.6871 6.80608 10.595C6.89501 10.5794 6.94268 10.6964 6.86757 10.7466C6.51345 10.9834 6.13571 11.1873 5.7844 11.3551C5.73733 11.3777 5.72211 11.4378 5.75315 11.4797C5.96186 11.7625 6.19139 12.0301 6.44075 12.2794C6.44075 12.2794 7.66853 13.5441 7.70198 13.6432C8.41841 12.9096 9.59612 12.0964 10.8966 11.3864C9.15488 12.8036 8.18387 13.8499 7.69517 14.4444L7.35447 14.9225C7.17742 15.1708 7.02379 15.4346 6.89541 15.7112C6.46579 16.6356 5.75756 18.5051 5.75756 18.5051C5.70328 18.6515 5.74754 18.7959 5.84168 18.89C5.84388 18.8922 5.84609 18.8944 5.84849 18.8964C5.85069 18.8986 5.8527 18.901 5.8549 18.9032C5.94924 18.9976 6.09345 19.0416 6.23986 18.9874C6.23986 18.9874 8.10897 18.2791 9.03371 17.8495C9.31031 17.7211 9.57429 17.5673 9.82245 17.3905L10.349 17.0153C10.6278 16.8166 11.0096 16.8483 11.2517 17.0904L12.4655 18.3042C12.7148 18.5535 12.9824 18.7831 13.2652 18.9918C13.3073 19.0226 13.3672 19.0076 13.3898 18.9605C13.5579 18.6094 13.7618 18.2313 13.9983 17.8774C14.0486 17.8022 14.1657 17.8501 14.1499 17.9388C14.0576 18.4606 14.0127 18.9794 13.9915 19.4084C13.9899 19.44 14.0067 19.4701 14.0345 19.4855C14.1972 19.5756 14.3636 19.6595 14.5335 19.7364C14.911 19.9229 15.2862 20.0645 15.6547 20.1687C15.6897 20.1785 15.7242 20.1516 15.7238 20.1152C15.7168 19.595 15.7424 18.9553 15.8576 18.3446C15.8684 18.2869 15.9503 18.2851 15.9641 18.3422C16.1516 19.127 16.4466 19.8742 16.6501 20.3413C16.6579 20.3591 16.6744 20.3712 16.6938 20.3734C17.2847 20.4407 17.8451 20.4242 18.3554 20.3606C18.3929 20.3559 18.4141 20.3155 18.3969 20.2818C18.187 19.872 17.9406 19.3241 17.7872 18.7612C17.7718 18.7046 17.8449 18.6675 17.8819 18.713C18.4151 19.3659 19.0454 19.9141 19.3314 20.1508C19.345 20.1621 19.3633 20.1659 19.3801 20.1615C20.1003 19.9712 20.6357 19.7226 20.8891 19.5922C20.968 19.5518 20.9792 19.4436 20.9107 19.3871L20.9093 19.3861Z";
const ANTIGRAVITY_PATH =
  "M21.751 22.607c1.34 1.005 3.35.335 1.508-1.508C17.73 15.74 18.904 1 12.037 1 5.17 1 6.342 15.74.815 21.1c-2.01 2.009.167 2.511 1.507 1.506 5.192-3.517 4.857-9.714 9.715-9.714 4.857 0 4.522 6.197 9.714 9.715z";
const CLAUDE_PATH =
  "M4.709 15.955l4.72-2.647.08-.23-.08-.128H9.2l-.79-.048-2.698-.073-2.339-.097-2.266-.122-.571-.121L0 11.784l.055-.352.48-.321.686.06 1.52.103 2.278.158 1.652.097 2.449.255h.389l.055-.157-.134-.098-.103-.097-2.358-1.596-2.552-1.688-1.336-.972-.724-.491-.364-.462-.158-1.008.656-.722.881.06.225.061.893.686 1.908 1.476 2.491 1.833.365.304.145-.103.019-.073-.164-.274-1.355-2.446-1.446-2.49-.644-1.032-.17-.619a2.97 2.97 0 01-.104-.729L6.283.134 6.696 0l.996.134.42.364.62 1.414 1.002 2.229 1.555 3.03.456.898.243.832.091.255h.158V9.01l.128-1.706.237-2.095.23-2.695.08-.76.376-.91.747-.492.584.28.48.685-.067.444-.286 1.851-.559 2.903-.364 1.942h.212l.243-.242.985-1.306 1.652-2.064.73-.82.85-.904.547-.431h1.033l.76 1.129-.34 1.166-1.064 1.347-.881 1.142-1.264 1.7-.79 1.36.073.11.188-.02 2.856-.606 1.543-.28 1.841-.315.833.388.091.395-.328.807-1.969.486-2.309.462-3.439.813-.042.03.049.061 1.549.146.662.036h1.622l3.02.225.79.522.474.638-.079.485-1.215.62-1.64-.389-3.829-.91-1.312-.329h-.182v.11l1.093 1.068 2.006 1.81 2.509 2.33.127.578-.322.455-.34-.049-2.205-1.657-.851-.747-1.926-1.62h-.128v.17l.444.649 2.345 3.521.122 1.08-.17.353-.608.213-.668-.122-1.374-1.925-1.415-2.167-1.143-1.943-.14.08-.674 7.254-.316.37-.729.28-.607-.461-.322-.747.322-1.476.389-1.924.315-1.53.286-1.9.17-.632-.012-.042-.14.018-1.434 1.967-2.18 2.945-1.726 1.845-.414.164-.717-.37.067-.662.401-.589 2.388-3.036 1.44-1.882.93-1.086-.006-.158h-.055L4.132 18.56l-1.13.146-.487-.456.061-.746.231-.243 1.908-1.312-.006.006z";
const CODEX_BG_PATH =
  "M19.503 0H4.496A4.496 4.496 0 000 4.496v15.007A4.496 4.496 0 004.496 24h15.007A4.496 4.496 0 0024 19.503V4.496A4.496 4.496 0 0019.503 0z";
const CODEX_MARK_PATH =
  "M9.064 3.344a4.578 4.578 0 012.285-.312c1 .115 1.891.54 2.673 1.275.01.01.024.017.037.021a.09.09 0 00.043 0 4.55 4.55 0 013.046.275l.047.022.116.057a4.581 4.581 0 012.188 2.399c.209.51.313 1.041.315 1.595a4.24 4.24 0 01-.134 1.223.123.123 0 00.03.115c.594.607.988 1.33 1.183 2.17.289 1.425-.007 2.71-.887 3.854l-.136.166a4.548 4.548 0 01-2.201 1.388.123.123 0 00-.081.076c-.191.551-.383 1.023-.74 1.494-.9 1.187-2.222 1.846-3.711 1.838-1.187-.006-2.239-.44-3.157-1.302a.107.107 0 00-.105-.024c-.388.125-.78.143-1.204.138a4.441 4.441 0 01-1.945-.466 4.544 4.544 0 01-1.61-1.335c-.152-.202-.303-.392-.414-.617a5.81 5.81 0 01-.37-.961 4.582 4.582 0 01-.014-2.298.124.124 0 00.006-.056.085.085 0 00-.027-.048 4.467 4.467 0 01-1.034-1.651 3.896 3.896 0 01-.251-1.192 5.189 5.189 0 01.141-1.6c.337-1.112.982-1.985 1.933-2.618.212-.141.413-.251.601-.33.215-.089.43-.164.646-.227a.098.098 0 00.065-.066 4.51 4.51 0 01.829-1.615 4.535 4.535 0 011.837-1.388zm3.482 10.565a.637.637 0 000 1.272h3.636a.637.637 0 100-1.272h-3.636zM8.462 9.23a.637.637 0 00-1.106.631l1.272 2.224-1.266 2.136a.636.636 0 101.095.649l1.454-2.455a.636.636 0 00.005-.64L8.462 9.23z";
const GEMINI_STAR_PATH =
  "M20.616 10.835a14.147 14.147 0 01-4.45-3.001 14.111 14.111 0 01-3.678-6.452.503.503 0 00-.975 0 14.134 14.134 0 01-3.679 6.452 14.155 14.155 0 01-4.45 3.001c-.65.28-1.318.505-2.002.678a.502.502 0 000 .975c.684.172 1.35.397 2.002.677a14.147 14.147 0 014.45 3.001 14.112 14.112 0 013.679 6.453.502.502 0 00.975 0c.172-.685.397-1.351.677-2.003a14.145 14.145 0 013.001-4.45 14.113 14.113 0 016.453-3.678.503.503 0 000-.975 13.245 13.245 0 01-2.003-.678z";
const DEEPSEEK_PATH =
  "M23.748 4.482c-.254-.124-.364.113-.512.234-.051.039-.094.09-.137.136-.372.397-.806.657-1.373.626-.829-.046-1.537.214-2.163.848-.133-.782-.575-1.248-1.247-1.548-.352-.156-.708-.311-.955-.65-.172-.241-.219-.51-.305-.774-.055-.16-.11-.323-.293-.35-.2-.031-.278.136-.356.276-.313.572-.434 1.202-.422 1.84.027 1.436.633 2.58 1.838 3.393.137.093.172.187.129.323-.082.28-.18.552-.266.833-.055.179-.137.217-.329.14a5.526 5.526 0 01-1.736-1.18c-.857-.828-1.631-1.742-2.597-2.458a11.365 11.365 0 00-.689-.471c-.985-.957.13-1.743.388-1.836.27-.098.093-.432-.779-.428-.872.004-1.67.295-2.687.684a3.055 3.055 0 01-.465.137 9.597 9.597 0 00-2.883-.102c-1.885.21-3.39 1.102-4.497 2.623C.082 8.606-.231 10.684.152 12.85c.403 2.284 1.569 4.175 3.36 5.653 1.858 1.533 3.997 2.284 6.438 2.14 1.482-.085 3.133-.284 4.994-1.86.47.234.962.327 1.78.397.63.059 1.236-.03 1.705-.128.735-.156.684-.837.419-.961-2.155-1.004-1.682-.595-2.113-.926 1.096-1.296 2.746-2.642 3.392-7.003.05-.347.007-.565 0-.845-.004-.17.035-.237.23-.256a4.173 4.173 0 001.545-.475c1.396-.763 1.96-2.015 2.093-3.517.02-.23-.004-.467-.247-.588zM11.581 18c-2.089-1.642-3.102-2.183-3.52-2.16-.392.024-.321.471-.235.763.09.288.207.486.371.739.114.167.192.416-.113.603-.673.416-1.842-.14-1.897-.167-1.361-.802-2.5-1.86-3.301-3.307-.774-1.393-1.224-2.887-1.298-4.482-.02-.386.093-.522.477-.592a4.696 4.696 0 011.529-.039c2.132.312 3.946 1.265 5.468 2.774.868.86 1.525 1.887 2.202 2.891.72 1.066 1.494 2.082 2.48 2.914.348.292.625.514.891.677-.802.09-2.14.11-3.054-.614zm1-6.44a.306.306 0 01.415-.287.302.302 0 01.2.288.306.306 0 01-.31.307.303.303 0 01-.304-.308zm3.11 1.596c-.2.081-.399.151-.59.16a1.245 1.245 0 01-.798-.254c-.274-.23-.47-.358-.552-.758a1.73 1.73 0 01.016-.588c.07-.327-.008-.537-.239-.727-.187-.156-.426-.199-.688-.199a.559.559 0 01-.254-.078c-.11-.054-.2-.19-.114-.358.028-.054.16-.186.192-.21.356-.202.767-.136 1.146.016.352.144.618.408 1.001.782.391.451.462.576.685.914.176.265.336.537.445.848.067.195-.019.354-.25.452z";
// WorkBuddy 官方应用图标（icon.png）vtracer 矢量化：青绿方块底 + 白色猫咪 + 绿圆双点
const WB_BASE_PATH =
  "M121.38,825.88c-25.93-49.51-19.71-106.93-19.71-160.96c-.01-88.75-.13-177.5-.13-266.25c.01-52.93-7.82-120.61,7.38-170.56c13.24-43.5,45.06-81.84,84.58-104.14c18.05-10.19,37.82-16.8,58.29-19.94c38.46-5.89,81.24-2.35,120.24-2.37c74.22-.02,148.43-.08,222.64-.11c35.4-.02,70.81-.08,106.22-.04c73.51,.08,125.33,1.94,177.55,61.8c47.4,54.34,43.84,111.74,43.86,179.12c.01,38.18,.03,76.36,.02,114.54c-.01,69.74-.01,139.48,.06,209.22c.03,37.11,4.09,79.33-3.7,115.57c-5.82,27.07-18.49,52.57-35.98,73.98c-23.68,28.99-56.03,51.14-92.39,60.57c-42.24,10.95-91.99,5.99-135.52,5.99c-82.23,0-164.46,.07-246.69,.06c-51.25,0-130.82,7.55-178.1-7.11c-46.69-14.48-85.34-46.43-108.62-89.37Z";
const WB_TEAL_PATH =
  "M205,906C84.12,846.09,101.72,736.53,101.65,623.6c-.05-83.44-.04-166.89-.11-250.33c-.04-38.38-4.88-83.35,1.4-120.7c4.57-27.13,15.42-53.27,31.55-75.54c23.92-33,58.65-58.47,98.19-69.01c50.38-13.44,136.05-6.36,190.44-6.38c78.87-.04,157.73-.01,236.59-.11c35.86-.05,78.13-4.77,112.93,1.62c29.49,5.42,56.97,17.41,80.52,36.03C932.1,201.63,922.32,276.33,922,366c-4.39,1.77-9.25,4.32-14,2c-5.59-2.73-11.54-9.07-17.01-12.67c-13.34-8.76-27.08-16.37-41.15-23.85c-3.87-2.06-7.65-3.28-10.53-6.8c-9.59-11.68-16.42-30.44-24.45-43.77c-18.22-30.25-77.58-114.46-105.42-130.79c-6.8-3.99-19.32-5.36-26.3-1.76c-33.48,17.22-70.27,114.31-84.01,150.04c-3.32,8.63-7.38,27.24-12.48,33.73c-4.39,5.59-19.64,7.91-26.58,9.97c-56.99,16.94-113.16,48.56-160.78,83.78c-8.94,6.62-56.08,49.23-60.92,50c-11.18,1.77-32.34-4.87-44.15-6.61c-28.06-4.13-154.98-21.47-170.71,3.46c-18.28,28.98,38.11,139.98,53.75,168.89c8.7,16.06,22.29,34.09,28.29,51.02c4.47,12.63,1.03,38.69,1.43,52.43c.44,14.97,5.29,26.88-5.53,39.11c-35.3,39.9,5.48,117.95,3.55,121.82Z";
const WB_CAT_PATH =
  "M922,549c-9.81-12.26-16.57-27.8-24.61-41.33c-28.46-47.9-59.47-83.2-115.59-97.44c-22.09-5.6-46.22-7.22-68.66-2.68c-40.12,8.11-76.43,32.19-111.44,52.35c-38.32,22.06-76.56,44.29-114.86,66.39c-45.58,26.29-102.22,51.14-138.89,88.57c-21.08,21.52-34.36,50.09-41.23,79.17c-16.78,70.92,25.85,134.27,60.3,192.44C369.73,891.04,394,921.52,394,922c-36.24,.15-163.51,7.71-187.39-15.81c-20.35-20.03-24.89-79.56-15.52-105.13c3.48-9.49,14.65-19.11,16.21-27.62c1.91-10.45-.47-69.53-3.3-79.88c-4.96-18.11-19.34-36.94-28.39-53.64c-16.1-29.74-71.03-135.68-53.83-166.17c16.62-29.45,151.16-8.73,181.61-4.19c6.3,.94,31.48,7.51,35.61,5.44c6.48-3.24,16.52-14.32,22.35-19.41c43.58-38,92.95-70.74,145.97-94.13c11.26-4.96,74.93-25.28,78.34-29.53c5.43-6.77,9.94-27.21,13.46-36.13c11.9-30.12,58.45-149.3,93.47-150.15c34.23-.83,104.12,104.78,121.65,133.36c8.24,13.45,15.67,33.1,25.73,44.54c.94,1.07,66.58,42.9,69.67,44.15c2.6,1.05,9.38-1.2,12.36-1.7c0,61,0,122,0,183Z";
const WB_SWOOSH_PATH =
  "M645,922c12.53-13.03,39.57-24,55.66-33.26c37.44-21.56,74.73-43.39,112.24-64.83c31.06-17.76,65.92-34.72,91.42-60.18C906.75,761.3,921.18,744,922,744c3.03,87.69-66.18,160.83-149.63,176.53C735.41,927.48,683.5,922.12,645,922Z";
const WB_PILL1_PATH =
  "M737,704c-7.38-2.31-13.81-6.57-18.92-12.35C707.4,679.55,666,608.61,665,595.84c-1.44-18.55,14.07-36.72,31.36-41.48c10.55-2.91,24.45,.6,32.98,7.32c12.01,9.47,19.53,28.17,27.1,41.35c15.12,26.28,46.11,61.77,18.47,90.1C764.39,703.92,751.45,705.85,737,704Z";
const WB_PILL2_PATH =
  "M518.48,682.01c12.85,10.14,20.98,29.61,29.12,43.65c14.2,24.49,42.29,56.8,20.88,84.69c-5.12,6.67-13.16,12.93-21.51,14.58c-9.79,1.93-21.18,1.26-29.61-4.44c-15.88-10.71-61.23-89.98-60.95-108.55c.21-14.04,9.41-25.38,21.01-32.45c13.36-8.14,28.22-4.9,41.06,2.52Z";
</script>

<template>
  <div
    class="chat-avatar"
    :class="[`role-${role}`, role === 'assistant' ? `brand-${brand}` : '']"
    :title="avatarTitle"
  >
    <!-- User Icon (Refined Silhouette) -->
    <svg
      v-if="role === 'user'"
      class="avatar-svg user-icon"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden="true"
    >
      <path d="M12 12c2.7 0 4.8-2.1 4.8-4.8S14.7 2.4 12 2.4 7.2 4.5 7.2 7.2 9.3 12 12 12zm0 2.4c-3.2 0-9.6 1.6-9.6 4.8v2.4h19.2v-2.4c0-3.2-6.4-4.8-9.6-4.8z" />
    </svg>

    <!-- Claude 官方彩色标（橙） -->
    <svg
      v-else-if="brand === 'claude'"
      class="avatar-svg brand-svg"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path :d="CLAUDE_PATH" fill="#D97757" />
    </svg>

    <!-- Codex 官方彩色标（白底圆角方块 + 紫蓝渐变标） -->
    <svg
      v-else-if="brand === 'codex'"
      class="avatar-svg brand-svg brand-svg-fill"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path :d="CODEX_BG_PATH" fill="#fff" />
      <path :d="CODEX_MARK_PATH" :fill="`url(#${gradIds.codex})`" />
      <defs>
        <linearGradient :id="gradIds.codex" gradientUnits="userSpaceOnUse" x1="12" x2="12" y1="3" y2="21">
          <stop stop-color="#B1A7FF" />
          <stop offset=".5" stop-color="#7A9DFF" />
          <stop offset="1" stop-color="#3941FF" />
        </linearGradient>
      </defs>
    </svg>

    <!-- Gemini 官方彩色标（蓝星 + 绿/红/黄渐变叠层） -->
    <svg
      v-else-if="brand === 'gemini'"
      class="avatar-svg brand-svg"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path :d="GEMINI_STAR_PATH" fill="#3186FF" />
      <path :d="GEMINI_STAR_PATH" :fill="`url(#${gradIds.geminiG1})`" />
      <path :d="GEMINI_STAR_PATH" :fill="`url(#${gradIds.geminiG2})`" />
      <path :d="GEMINI_STAR_PATH" :fill="`url(#${gradIds.geminiG3})`" />
      <defs>
        <linearGradient :id="gradIds.geminiG1" gradientUnits="userSpaceOnUse" x1="7" x2="11" y1="15.5" y2="12">
          <stop stop-color="#08B962" />
          <stop offset="1" stop-color="#08B962" stop-opacity="0" />
        </linearGradient>
        <linearGradient :id="gradIds.geminiG2" gradientUnits="userSpaceOnUse" x1="8" x2="11.5" y1="5.5" y2="11">
          <stop stop-color="#F94543" />
          <stop offset="1" stop-color="#F94543" stop-opacity="0" />
        </linearGradient>
        <linearGradient :id="gradIds.geminiG3" gradientUnits="userSpaceOnUse" x1="3.5" x2="17.5" y1="13.5" y2="12">
          <stop stop-color="#FABC12" />
          <stop offset=".46" stop-color="#FABC12" stop-opacity="0" />
        </linearGradient>
      </defs>
    </svg>

    <!-- WorkBuddy 官方图标（vtracer 矢量化：青绿底 + 白色猫咪） -->
    <svg
      v-else-if="brand === 'workbuddy'"
      class="avatar-svg brand-svg brand-svg-fill"
      viewBox="0 0 1024 1024"
      aria-hidden="true"
    >
      <path :d="WB_BASE_PATH" fill="#25CB89" />
      <path :d="WB_TEAL_PATH" fill="#0AC8A0" />
      <path :d="WB_CAT_PATH" fill="#EAFAF6" />
      <path :d="WB_SWOOSH_PATH" fill="#FDFEFD" />
      <path :d="WB_PILL1_PATH" fill="#FDFEFE" />
      <path :d="WB_PILL2_PATH" fill="#FDFEFE" />
    </svg>

    <!-- DSH 官方彩色标（DeepSeek 蓝鲸） -->
    <svg
      v-else-if="brand === 'dsh'"
      class="avatar-svg brand-svg"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path :d="DEEPSEEK_PATH" fill="#4D6BFE" />
    </svg>

    <!-- Antigravity 官方彩色标（源自 @lobehub/icons Antigravity.Color） -->
    <svg
      v-else-if="brand === 'antigravity'"
      class="avatar-svg brand-svg"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <mask :id="gradIds.agMask" maskUnits="userSpaceOnUse" width="24" height="23" x="0" y="1">
        <path :d="ANTIGRAVITY_PATH" fill="#fff" />
      </mask>
      <g :mask="`url(#${gradIds.agMask})`">
        <g :filter="`url(#${gradIds.agF1})`">
          <path d="M-1.018-3.992c-.408 3.591 2.686 6.89 6.91 7.37 4.225.48 7.98-2.043 8.387-5.633.408-3.59-2.686-6.89-6.91-7.37-4.225-.479-7.98 2.043-8.387 5.633z" fill="#FFE432" />
        </g>
        <g :filter="`url(#${gradIds.agF2})`">
          <path d="M15.269 7.747c1.058 4.557 5.691 7.374 10.348 6.293 4.657-1.082 7.575-5.653 6.516-10.21-1.058-4.556-5.691-7.374-10.348-6.292-4.657 1.082-7.575 5.653-6.516 10.21z" fill="#FC413D" />
        </g>
        <g :filter="`url(#${gradIds.agF3})`">
          <path d="M-12.443 10.804c1.338 4.703 7.36 7.11 13.453 5.378 6.092-1.733 9.947-6.95 8.61-11.652C8.282-.173 2.26-2.58-3.833-.848-9.925.884-13.78 6.1-12.443 10.804z" fill="#00B95C" />
        </g>
        <g :filter="`url(#${gradIds.agF4})`">
          <path d="M-12.443 10.804c1.338 4.703 7.36 7.11 13.453 5.378 6.092-1.733 9.947-6.95 8.61-11.652C8.282-.173 2.26-2.58-3.833-.848-9.925.884-13.78 6.1-12.443 10.804z" fill="#00B95C" />
        </g>
        <g :filter="`url(#${gradIds.agF5})`">
          <path d="M-7.608 14.703c3.352 3.424 9.126 3.208 12.896-.483 3.77-3.69 4.108-9.459.756-12.883C2.69-2.087-3.083-1.871-6.853 1.82c-3.77 3.69-4.108 9.458-.755 12.883z" fill="#00B95C" />
        </g>
        <g :filter="`url(#${gradIds.agF6})`">
          <path d="M9.932 27.617c1.04 4.482 5.384 7.303 9.7 6.3 4.316-1.002 6.971-5.448 5.93-9.93-1.04-4.483-5.384-7.304-9.7-6.301-4.316 1.002-6.971 5.448-5.93 9.93z" fill="#3186FF" />
        </g>
        <g :filter="`url(#${gradIds.agF7})`">
          <path d="M2.572-8.185C.392-3.329 2.778 2.472 7.9 4.771c5.122 2.3 11.042.227 13.222-4.63 2.18-4.855-.205-10.656-5.327-12.955-5.122-2.3-11.042-.227-13.222 4.63z" fill="#FBBC04" />
        </g>
        <g :filter="`url(#${gradIds.agF8})`">
          <path d="M-3.267 38.686c-5.277-2.072 3.742-19.117 5.984-24.83 2.243-5.712 8.34-8.664 13.616-6.592 5.278 2.071 11.533 13.482 9.29 19.195-2.242 5.713-23.613 14.298-28.89 12.227z" fill="#3186FF" />
        </g>
        <g :filter="`url(#${gradIds.agF9})`">
          <path d="M28.71 17.471c-1.413 1.649-5.1.808-8.236-1.878-3.135-2.687-4.531-6.201-3.118-7.85 1.412-1.649 5.1-.808 8.235 1.878s4.532 6.2 3.119 7.85z" fill="#749BFF" />
        </g>
        <g :filter="`url(#${gradIds.agF10})`">
          <path d="M18.163 9.077c5.81 3.93 12.502 4.19 14.946.577 2.443-3.612-.287-9.727-6.098-13.658-5.81-3.931-12.502-4.19-14.946-.577-2.443 3.612.287 9.727 6.098 13.658z" fill="#FC413D" />
        </g>
        <g :filter="`url(#${gradIds.agF11})`">
          <path d="M-.915 2.684c-1.44 3.473-.97 6.967 1.05 7.804 2.02.837 4.824-1.3 6.264-4.772 1.44-3.473.97-6.967-1.05-7.804-2.02-.837-4.824 1.3-6.264 4.772z" fill="#FFEE48" />
        </g>
      </g>
      <defs>
        <filter :id="gradIds.agF1" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="17.587" width="19.838" x="-3.288" y="-11.917">
          <feFlood flood-opacity="0" result="BackgroundImageFix" />
          <feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
          <feGaussianBlur result="effect1_foregroundBlur" stdDeviation="1.117" />
        </filter>
        <filter :id="gradIds.agF2" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="38.565" width="38.9" x="4.251" y="-13.493">
          <feFlood flood-opacity="0" result="BackgroundImageFix" />
          <feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
          <feGaussianBlur result="effect1_foregroundBlur" stdDeviation="5.4" />
        </filter>
        <filter :id="gradIds.agF3" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="36.517" width="40.955" x="-21.889" y="-10.592">
          <feFlood flood-opacity="0" result="BackgroundImageFix" />
          <feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
          <feGaussianBlur result="effect1_foregroundBlur" stdDeviation="4.591" />
        </filter>
        <filter :id="gradIds.agF4" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="36.517" width="40.955" x="-21.889" y="-10.592">
          <feFlood flood-opacity="0" result="BackgroundImageFix" />
          <feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
          <feGaussianBlur result="effect1_foregroundBlur" stdDeviation="4.591" />
        </filter>
        <filter :id="gradIds.agF5" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="36.595" width="36.632" x="-19.099" y="-10.278">
          <feFlood flood-opacity="0" result="BackgroundImageFix" />
          <feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
          <feGaussianBlur result="effect1_foregroundBlur" stdDeviation="4.591" />
        </filter>
        <filter :id="gradIds.agF6" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="34.087" width="33.533" x=".981" y="8.758">
          <feFlood flood-opacity="0" result="BackgroundImageFix" />
          <feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
          <feGaussianBlur result="effect1_foregroundBlur" stdDeviation="4.363" />
        </filter>
        <filter :id="gradIds.agF7" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="35.276" width="35.978" x="-6.143" y="-21.659">
          <feFlood flood-opacity="0" result="BackgroundImageFix" />
          <feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
          <feGaussianBlur result="effect1_foregroundBlur" stdDeviation="3.954" />
        </filter>
        <filter :id="gradIds.agF8" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="46.523" width="45.114" x="-11.96" y="-.46">
          <feFlood flood-opacity="0" result="BackgroundImageFix" />
          <feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
          <feGaussianBlur result="effect1_foregroundBlur" stdDeviation="3.531" />
        </filter>
        <filter :id="gradIds.agF9" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="24.054" width="25.094" x="10.485" y=".58">
          <feFlood flood-opacity="0" result="BackgroundImageFix" />
          <feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
          <feGaussianBlur result="effect1_foregroundBlur" stdDeviation="3.159" />
        </filter>
        <filter :id="gradIds.agF10" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="30.007" width="33.508" x="5.833" y="-12.467">
          <feFlood flood-opacity="0" result="BackgroundImageFix" />
          <feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
          <feGaussianBlur result="effect1_foregroundBlur" stdDeviation="2.669" />
        </filter>
        <filter :id="gradIds.agF11" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse" height="26.151" width="22.194" x="-8.355" y="-8.876">
          <feFlood flood-opacity="0" result="BackgroundImageFix" />
          <feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
          <feGaussianBlur result="effect1_foregroundBlur" stdDeviation="3.303" />
        </filter>
      </defs>
    </svg>

    <!-- OpenCode 官方标（青）：方框挖空，单路径 evenodd -->
    <svg
      v-else-if="brand === 'opencode'"
      class="avatar-svg brand-svg"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path :d="OPENCODE_PATH" fill="#22d3ee" />
    </svg>

    <!-- Goose 官方标（绿）：矢量鹅标 -->
    <svg
      v-else-if="brand === 'goose'"
      class="avatar-svg brand-svg"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path :d="GOOSE_PATH" fill="#0da678" />
    </svg>

    <!-- Generic AI Sparkle -->
    <svg
      v-else
      class="avatar-svg ai-icon"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="m12 3-1.912 5.813a2 2 0 0 1-1.275 1.275L3 12l5.813 1.912a2 2 0 0 1 1.275 1.275L12 21l1.912-5.813a2 2 0 0 1 1.275-1.275L21 12l-5.813-1.912a2 2 0 0 1-1.275-1.275L12 3Z" fill="currentColor" fill-opacity="0.25" />
    </svg>
  </div>
</template>

<style scoped>
.chat-avatar {
  flex-shrink: 0;
  width: 30px;
  height: 30px;
  border-radius: 10px; /* Modern Desktop Squircle */
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  -webkit-user-select: none;
  user-select: none;
  transition: transform var(--transition-fast), box-shadow var(--transition-fast);
}

.chat-avatar:hover {
  transform: scale(1.04);
}

.role-user {
  background: var(--color-bg-hover);
  border: 1px solid var(--color-border);
  color: var(--color-primary);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.04);
}

/* assistant 品牌头像：容器透明、无描边投影，展示官方彩色标本身 */
.role-assistant {
  background: transparent;
}

.brand-codex,
.brand-workbuddy {
  /* 这两款图标自带满血圆角方块底色，铺满容器 */
  border-radius: 10px;
}

.avatar-svg {
  width: 16px;
  height: 16px;
}

.brand-svg {
  width: 20px;
  height: 20px;
}

/* 自带底色的图标铺满整个头像容器 */
.brand-svg-fill {
  width: 30px;
  height: 30px;
}

.user-icon {
  width: 15px;
  height: 15px;
}
</style>
