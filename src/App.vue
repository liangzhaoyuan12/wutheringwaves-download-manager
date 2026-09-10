<!--
  鸣潮 (Wuthering Waves) 下载管理器 GUI
-->
<script setup>
import { ref, onMounted, onUnmounted, nextTick, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, ask } from "@tauri-apps/plugin-dialog";

// ── State ──
const gamePath = ref("");
const server = ref("cn");
const status = ref(null);
const logs = ref([]);
const progress = ref({ eventType: "", message: "", current: 0, total: 0, fileName: null });
const running = ref(false);
const theme = ref("system"); // "system" | "dark" | "light"
const dxMode = ref(""); // "" | "dx11" | "dx12"

// ── CDN ──
const cdnNodes = ref([]); // 可选 CDN 节点列表
const cdnUrl = ref(""); // 当前选中的 CDN 基址 URL
const cdnDropdownOpen = ref(false);
const cdnDropdownRef = ref(null);

function cdnHost(url) {
  try { return new URL(url).host; } catch (_) { return url; }
}

function currentCdnLabel() {
  const n = cdnNodes.value.find((x) => x.url === cdnUrl.value);
  if (!n) return cdnUrl.value || "未选择";
  return cdnHost(n.url) + (n.recommended ? " (推荐)" : "");
}

function selectCdn(n) {
  cdnUrl.value = n.url;
  localStorage.setItem("cdn:" + server.value, n.url);
  cdnDropdownOpen.value = false;
}

// 获取当前服务器对应的 CDN 列表，默认选中推荐（优先级最高）节点，
// 但若用户此前手动选过则保留其选择。
async function loadCdnList() {
  try {
    const list = await invoke("get_cdn_list", { server: server.value });
    cdnNodes.value = list;
    const saved = localStorage.getItem("cdn:" + server.value);
    const recommended = list.find((n) => n.recommended) || list[0];
    if (saved && list.some((n) => n.url === saved)) {
      cdnUrl.value = saved;
    } else {
      cdnUrl.value = recommended ? recommended.url : "";
    }
  } catch (e) {
    addLog("获取 CDN 列表失败: " + e, "error");
    cdnNodes.value = [];
    cdnUrl.value = "";
  }
}

function applyTheme(t) {
  theme.value = t;
  const isDark = t === "dark" || (t === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  document.body.classList.toggle("light", !isDark);
  localStorage.setItem("theme", t);
}


const logContainer = ref(null);
const dropdownOpen = ref(false);
const dropdownRef = ref(null);
const dxDropdownRef = ref(null);
const dxDropdownOpen = ref(false);

const servers = [
  { key: "cn", label: "国服 (CN)" },
  { key: "global", label: "国际服 (Global)" },
  { key: "bilibili", label: "B站服 (Bilibili)" },
];

function selectServer(s) {
  server.value = s.key;
  dropdownOpen.value = false;
  loadCdnList();
}

const currentServerLabel = computed(() => {
  return servers.find(s => s.key === server.value)?.label || server.value;
});

function toggleDropdown() {
  if (!running.value) dropdownOpen.value = !dropdownOpen.value;
}

// ── Init ──
onMounted(async () => {
  theme.value = localStorage.getItem("theme") || "system";
  applyTheme(theme.value);
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  mq.addEventListener("change", () => { if (theme.value === "system") applyTheme("system"); });

  try {
    const cfg = await invoke("get_app_config");
    if (cfg.default_path) gamePath.value = cfg.default_path;
    if (cfg.dx_mode) dxMode.value = cfg.dx_mode;
  } catch (_) {}

  const unlisten = await listen("ww:progress", (event) => {
    progress.value = event.payload;
    if (event.payload.eventType === "log" || event.payload.eventType === "error") {
      addLog(event.payload.message, event.payload.eventType);
    } else if (
      event.payload.eventType === "verify_progress" ||
      event.payload.eventType === "download_file_done" ||
      event.payload.eventType === "delete_progress"
    ) {
      addLog(event.payload.message, "info");
    }
    autoScroll();
  });
  onUnmounted(() => unlisten());

  // Click-outside to close dropdown
  document.addEventListener("click", onDocumentClick);
  onUnmounted(() => document.removeEventListener("click", onDocumentClick));

  if (gamePath.value) await refreshStatus();

  // 拉取默认服务器的 CDN 列表
  loadCdnList();
});

function onDocumentClick(e) {
  if (dropdownRef.value && !dropdownRef.value.contains(e.target)) {
    dropdownOpen.value = false;
  }
  if (dxDropdownRef.value && !dxDropdownRef.value.contains(e.target)) {
    dxDropdownOpen.value = false;
  }
  if (cdnDropdownRef.value && !cdnDropdownRef.value.contains(e.target)) {
    cdnDropdownOpen.value = false;
  }
}

// ── Helpers ──
function addLog(msg, level = "info") {
  const time = new Date().toLocaleTimeString("zh-CN", { hour12: false });
  logs.value.push({ time, msg, level });
  if (logs.value.length > 500) logs.value.shift();
}

function autoScroll() {
  nextTick(() => {
    if (logContainer.value) logContainer.value.scrollTop = logContainer.value.scrollHeight;
  });
}

async function withRunning(fn) {
  if (running.value) return;
  running.value = true;
  progress.value = { eventType: "", message: "", current: 0, total: 0, fileName: null };
  try {
    await fn();
  } catch (e) {
    addLog(String(e), "error");
  }
  running.value = false;
}

// ── Commands ──
async function refreshStatus() {
  try {
    const result = await invoke("get_status", { gamePath: gamePath.value || undefined });
    status.value = result;
    if (result.server && result.server !== "未知") {
      const srv = servers.find((s) => s.key === result.server);
      if (srv && srv.key !== server.value) {
        server.value = srv.key;
        loadCdnList();
      }
    }
  } catch (e) {
    addLog("获取状态失败: " + e, "error");
  }
}

async function selectPath() {
  const selected = await open({ directory: true, multiple: false });
  if (selected) {
    gamePath.value = selected;
    await onPathChange();
  }
}

async function startSync() {
  await withRunning(async () => {
    addLog("开始检验游戏完整性...", "info");
    await invoke("start_sync", { gamePath: gamePath.value || undefined, server: server.value, cdnUrl: cdnUrl.value || undefined });
    addLog("检验完成！", "info");
    await refreshStatus();
  });
}

async function startDownload() {
  await withRunning(async () => {
    addLog(`开始下载 ${server.value} 服游戏...`, "info");
    await invoke("start_download", { gamePath: gamePath.value || undefined, server: server.value, cdnUrl: cdnUrl.value || undefined });
    addLog("下载完成！", "info");
    await refreshStatus();
  });
}

async function startUpdate() {
  await withRunning(async () => {
    addLog(`正在检查 ${server.value} 服游戏更新...`, "info");
    await invoke("start_update", { gamePath: gamePath.value || undefined, server: server.value, cdnUrl: cdnUrl.value || undefined });
    addLog("更新流程完成！", "info");
    await refreshStatus();
  });
}

async function startCheckout() {
  await withRunning(async () => {
    addLog(`正在切换到 ${server.value} 服...`, "info");
    await invoke("start_checkout", {
      gamePath: gamePath.value || undefined,
      server: server.value,
      cdnUrl: cdnUrl.value || undefined,
    });
    addLog(`已切换到 ${server.value} 服！`, "info");
    await refreshStatus();
  });
}

async function startPredownload() {
  await withRunning(async () => {
    addLog("开始预下载...", "info");
    await invoke("start_predownload", { gamePath: gamePath.value || undefined, server: server.value, cdnUrl: cdnUrl.value || undefined });
    addLog("预下载完成！", "info");
  });
}

async function onPathChange() {
  // Persist path
  try {
    const cfg = await invoke("get_app_config");
    cfg.default_path = gamePath.value;
    await invoke("save_app_config_cmd", { configData: cfg });
  } catch (_) {}
  await refreshStatus();
}

const showLinuxGuide = ref(false);

async function launchGame() {
  try {
    const result = await invoke("launch_game", {
      gamePath: gamePath.value || undefined,
      dxMode: dxMode.value || undefined,
    });
    if (result.platform === "linux") {
      showLinuxGuide.value = true;
    } else {
      addLog("游戏已启动", "info");
    }
  } catch (e) {
    addLog(String(e), "error");
  }
}

async function deleteGame() {
  const ok = await ask(
    "确定要完全卸载游戏吗？\n\n此操作将删除所有游戏文件，包括预下载内容，且不可恢复！",
    { title: "删除游戏", kind: "error" }
  );
  if (!ok) return;
  await withRunning(async () => {
    addLog("开始删除游戏文件...", "info");
    await invoke("delete_game", { gamePath: gamePath.value || undefined });
    addLog("游戏已删除！", "info");
    status.value = null;
  });
}

// ── Computed ──
const progressPct = computed(() => {
  if (!progress.value.total) return 0;
  if (progress.value.current >= progress.value.total) return 100;
  return Math.floor((progress.value.current / progress.value.total) * 100);
});

const progressLabel = computed(() => {
  if (!progress.value.eventType) return "";
  if (progress.value.eventType === "verify_progress") {
    return `校验进度: ${progress.value.current} / ${progress.value.total}`;
  }
  if (progress.value.eventType === "delete_progress") {
    return `清理进度: ${progress.value.current} / ${progress.value.total}`;
  }
  if (progress.value.eventType === "download_file_done") {
    return `下载进度: ${progress.value.current} / ${progress.value.total}`;
  }
  return progress.value.message;
});
</script>

<template>
  <main class="app-container">
    <header class="app-header">
      <h1>鸣潮 (Wuthering Waves) 下载管理器</h1>
      <div class="theme-selector">
        <button
          v-for="t in [{ key: 'system', label: '🌗 跟随系统' }, { key: 'dark', label: '🌙 暗色' }, { key: 'light', label: '☀ 浅色' }]"
          :key="t.key"
          class="theme-btn"
          :class="{ active: theme === t.key }"
          @click="applyTheme(t.key)"
        >{{ t.label }}</button>
      </div>
    </header>

    <!-- ── Config Panel ── -->
    <section class="config-panel">
      <div class="field">
        <label>游戏路径:</label>
        <input
          v-model="gamePath"
          type="text"
          placeholder="输入游戏安装目录路径"
          @change="onPathChange"
          :disabled="running"
        />
        <button class="btn-sm" @click="selectPath" :disabled="running">
          📂 选择路径
        </button>
        <button class="btn-sm" @click="refreshStatus" :disabled="running || !gamePath">
          ↻ 刷新状态
        </button>
      </div>
      <div class="field">
        <label>服务器:</label>
        <div class="custom-select" ref="dropdownRef" :class="{ open: dropdownOpen, disabled: running }" v-click-outside="() => dropdownOpen = false">
          <div class="select-trigger" @click="toggleDropdown">
            <span>{{ currentServerLabel }}</span>
            <span class="arrow">&#9662;</span>
          </div>
          <div class="select-dropdown" v-show="dropdownOpen">
            <div
              v-for="s in servers"
              :key="s.key"
              class="select-option"
              :class="{ active: server === s.key }"
              @click="selectServer(s)"
            >{{ s.label }}</div>
          </div>
        </div>
        <button class="btn-sm" @click="startCheckout" :disabled="running || !gamePath">
          ✅ 应用服务器更改
        </button>
      </div>
      <div class="field">
        <label>CDN 节点:</label>
        <div class="custom-select" ref="cdnDropdownRef" :class="{ open: cdnDropdownOpen, disabled: running }">
          <div class="select-trigger" @click="!running && (cdnDropdownOpen = !cdnDropdownOpen)">
            <span>{{ currentCdnLabel() }}</span>
            <span class="arrow">&#9662;</span>
          </div>
          <div class="select-dropdown cdn-dropdown" v-show="cdnDropdownOpen && cdnNodes.length">
            <div
              v-for="n in cdnNodes"
              :key="n.url"
              class="select-option"
              :class="{ active: cdnUrl === n.url }"
              @click="selectCdn(n)"
            >
              <span class="cdn-host">{{ cdnHost(n.url) }}</span>
              <span v-if="n.recommended" class="cdn-badge">推荐</span>
              <span class="cdn-prio">优先级 {{ n.priority }}</span>
            </div>
          </div>
          <div class="select-dropdown cdn-dropdown" v-show="cdnDropdownOpen && !cdnNodes.length">
            <div class="select-option disabled-opt">无法获取 CDN 列表</div>
          </div>
        </div>
      </div>
    </section>

    <!-- ── Status Panel ── -->
    <section class="status-panel" v-if="status">
      <div class="status-grid">
        <div class="status-item">
          <span class="status-label">目录</span>
          <span class="status-value">{{ status.path }}</span>
        </div>
        <div class="status-item">
          <span class="status-label">服务器</span>
          <span class="status-value">{{ status.server }}</span>
        </div>
        <div class="status-item">
          <span class="status-label">版本</span>
          <span class="status-value">{{ status.version }}</span>
        </div>
      </div>
      <div class="status-actions">
        <button class="btn-sm" @click="launchGame" :disabled="!gamePath">
          ▶ 启动游戏
        </button>
        <label class="dx-label">DX:</label>
        <div class="dx-select-wrap" ref="dxDropdownRef">
          <div class="select-trigger dx-trigger" @click="dxDropdownOpen = !dxDropdownOpen">
            <span>{{ dxMode === 'dx11' ? 'DX11' : 'DX12' }}</span>
            <span class="arrow">&#9662;</span>
          </div>
          <div class="select-dropdown dx-dropdown" v-show="dxDropdownOpen">
            <div
              v-for="opt in [{ key: '', label: 'DX12' }, { key: 'dx11', label: 'DX11' }]"
              :key="opt.key"
              class="select-option"
              :class="{ active: dxMode === opt.key }"
              @click="dxMode = opt.key; dxDropdownOpen = false"
            >{{ opt.label }}</div>
          </div>
        </div>
      </div>
    </section>
    <section class="status-panel" v-else-if="gamePath">
      <p class="hint">点击「刷新状态」查看当前客户端信息</p>
    </section>

    <!-- ── Operations ── -->
    <section class="ops-panel">
      <h3>操作</h3>
      <div class="ops-grid">
        <button class="btn-primary" @click="startDownload" :disabled="running || !gamePath">
          ⬇ 下载游戏
        </button>
        <button class="btn-primary" @click="startUpdate" :disabled="running || !gamePath">
          🔄 更新游戏
        </button>
        <button class="btn-primary" @click="startSync" :disabled="running || !gamePath">
          🔍 检验游戏完整性
        </button>
        <button class="btn-danger" @click="deleteGame" :disabled="running || !gamePath">
          🗑 删除游戏
        </button>
        <button class="btn-secondary push-right" @click="startPredownload" :disabled="running || !gamePath" title="提前将更新包下载到本地，点击'更新游戏'时自动应用">
          📦 预下载
        </button>
      </div>
    </section>

    <!-- ── Progress ── -->
    <section class="progress-panel">
      <div class="progress-info" v-if="progress.eventType">
        <span>{{ progressLabel }}</span>
        <span v-if="progress.fileName" class="filename">{{ progress.fileName }}</span>
      </div>
      <div class="progress-row">
        <div class="progress-bar-bg" v-if="progress.eventType">
          <div class="progress-bar-fill" :style="{ width: progressPct + '%' }" :class="{ done: progressPct >= 100 }"></div>
        </div>
        <div class="progress-pct" v-if="progress.eventType && progress.total">{{ progressPct }}%</div>
        <div class="spinner" :class="{ active: running }">
          <div class="spinner-ring">
            <span></span>
            <span></span>
            <span></span>
            <span></span>
            <span></span>
            <span></span>
          </div>
        </div>
      </div>
      <div class="progress-message" v-if="progress.eventType === 'download_file_done' && progress.message">
        {{ progress.message }}
      </div>
    </section>

    <!-- ── Log ── -->
    <section class="log-panel">
      <div class="log-header">
        <h3>日志</h3>
        <button class="btn-sm" @click="logs = []">清空</button>
      </div>
      <div class="log-container" ref="logContainer">
        <div
          v-for="(log, i) in logs"
          :key="i"
          class="log-line"
          :class="'log-' + log.level"
        >
          <span class="log-time">{{ log.time }}</span>
          <span class="log-msg">{{ log.msg }}</span>
        </div>
        <div v-if="logs.length === 0" class="log-empty">暂无日志，开始操作后将在此显示。</div>
      </div>
    </section>

    <!-- ── Linux 启动指南 ── -->
    <div class="modal-overlay" v-if="showLinuxGuide" @click.self="showLinuxGuide = false">
      <div class="modal-content">
        <div class="modal-header">
          <h2>🎮 在 Linux 上启动游戏</h2>
          <button class="btn-sm" @click="showLinuxGuide = false">✕ 关闭</button>
        </div>
        <div class="modal-body">
          <p class="modal-desc">本工具仅负责文件管理。启动游戏<strong>推荐通过 Steam + Proton</strong> 进行。</p>

          <h3>一、通过 Steam 启动（推荐）</h3>

          <h4>1. 添加游戏到 Steam</h4>
          <ol>
            <li>打开 <strong>Steam → 左下角「添加游戏」→「添加非 Steam 游戏」</strong></li>
            <li>点击「浏览」，选择你的游戏可执行文件：<br/>
              <code>Client/Binaries/Win64/Client-Win64-Shipping.exe</code>
            </li>
          </ol>

          <h4>2. 启用 Proton 兼容层</h4>
          <ol>
            <li>右键游戏 → <strong>属性</strong></li>
            <li>勾选 <strong>「强制使用特定 Steam Play 兼容性工具」</strong></li>
            <li>选择以下任一兼容层：<strong>GE-Proton</strong> 或 <strong>dwproton</strong></li>
          </ol>

          <h4>3. 设置启动参数</h4>
          <p>在 Steam 启动选项中添加：（默认不添加即为 DX12 模式）</p>
          <pre><code>steamdeck=1</code></pre>
          <p>如需切换为 DX11 模式，改为：</p>
          <pre><code>steamdeck=1 -dx11</code></pre>

          <blockquote>如果遇到 ACE 反作弊环境警告，可尝试在 <strong>GE-Proton</strong> 和 <strong>dwproton</strong> 之间切换。</blockquote>
        </div>
      </div>
    </div>

  </main>
</template>

<style>
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

:root {
  --bg-primary: #1a1a2e;
  --bg-secondary: #16213e;
  --border-color: #0f3460;
  --accent: #533483;
  --accent-hover: #6b4c9a;
  --accent-bg: #2a2a4e;
  --accent-bg-hover: #3a3a5e;
  --text-primary: #e0e0e0;
  --text-secondary: #c0c0c0;
  --text-muted: #a0a0b0;
  --text-muted2: #888;
  --input-bg: #1a1a2e;
  --progress-done: #2e7d32;
  --progress-done-light: #4caf50;
  --danger: #7a1a1a;
  --danger-hover: #a03030;
  --modal-overlay: rgba(0, 0, 0, 0.7);
  --log-error: #ef5350;

  font-family: "SF Mono", "Cascadia Code", "Consolas", "Monaco", monospace;
  font-size: 13px;
  line-height: 1.5;
  color: var(--text-secondary);
  background-color: var(--bg-primary);
}

body.light {
  --bg-primary: #f0f0f5;
  --bg-secondary: #ffffff;
  --border-color: #d0d0e0;
  --accent: #3366cc;
  --accent-hover: #4477dd;
  --accent-bg: #e8e8f0;
  --accent-bg-hover: #d8d8e8;
  --text-primary: #111122;
  --text-secondary: #222233;
  --text-muted: #444455;
  --text-muted2: #666677;
  --input-bg: #ffffff;
  --progress-done: #2e7d32;
  --progress-done-light: #4caf50;
  --danger: #cc3333;
  --danger-hover: #dd4444;
  --modal-overlay: rgba(0, 0, 0, 0.4);
  --log-error: #cc3333;
}

body {
  overflow: hidden;
  background-color: var(--bg-primary);
}

#app {
  height: 100vh;
  background-color: var(--bg-primary);
}
</style>

<style scoped>
.app-container {
  display: flex;
  flex-direction: column;
  height: 100vh;
  padding: 16px;
  gap: 12px;
  overflow: hidden;
}

.app-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.app-header h1 {
  font-size: 1.3em;
  font-weight: 600;
  color: var(--text-primary);
}

.theme-selector {
  display: flex;
  gap: 4px;
}

.theme-btn {
  padding: 4px 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: transparent;
  color: var(--text-muted);
  font-family: inherit;
  font-size: 0.8em;
  cursor: pointer;
  transition: all 0.2s;
}

.theme-btn:hover {
  background: var(--accent-bg);
  color: var(--text-primary);
}

.theme-btn.active {
  background: var(--accent);
  color: #fff;
  border-color: var(--accent);
}

/* ── Config ── */
.config-panel {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  background: var(--bg-secondary);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  padding: 12px;
}

.field {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
  min-width: 280px;
}

.field label {
  font-weight: 600;
  color: var(--text-muted);
  white-space: nowrap;
}

.field input {
  flex: 1;
  padding: 6px 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-primary);
  font-family: inherit;
  font-size: 0.95em;
  outline: none;
  transition: border-color 0.2s;
}

.field input:focus {
  border-color: var(--accent);
}

.field input:disabled {
  opacity: 0.5;
}

/* ── Custom Select ── */
.custom-select {
  flex: 1;
  position: relative;
  user-select: none;
}

.custom-select.disabled {
  opacity: 0.5;
  pointer-events: none;
}

.select-trigger {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-primary);
  cursor: pointer;
  transition: border-color 0.2s;
}

.custom-select.open .select-trigger {
  border-color: var(--accent);
}

.arrow {
  font-size: 0.7em;
  color: var(--text-muted2);
  transition: transform 0.2s;
}

.custom-select.open .arrow {
  transform: rotate(180deg);
}

.select-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  right: 0;
  z-index: 100;
  background: var(--bg-secondary);
  border: 1px solid var(--border-color);
  border-radius: 6px;
  overflow: hidden;
}

.select-option {
  padding: 8px 12px;
  cursor: pointer;
  color: var(--text-secondary);
  transition: background 0.15s;
}

.select-option:hover {
  background: var(--accent-bg);
  color: var(--text-primary);
}

.select-option.active {
  background: var(--accent);
  color: #fff;
}

/* ── Status ── */
.status-panel {
  background: var(--bg-secondary);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  padding: 12px;
}

.status-grid {
  display: flex;
  gap: 24px;
}

.status-item {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.status-label {
  font-size: 0.8em;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.status-value {
  font-size: 1.05em;
  color: var(--text-primary);
  font-weight: 600;
}

.hint {
  color: var(--text-muted);
  font-style: italic;
}

.status-actions {
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--border-color);
  display: flex;
  align-items: center;
  gap: 8px;
}

.dx-label {
  font-size: 0.8em;
  color: var(--text-muted);
  margin-left: 8px;
}

.dx-select-wrap {
  position: relative;
  user-select: none;
}

.dx-trigger {
  padding: 3px 8px;
  font-size: 0.82em;
  min-width: 70px;
}

.dx-dropdown {
  min-width: 70px;
}

/* ── CDN Select ── */
.cdn-dropdown {
  max-height: 260px;
  overflow-y: auto;
}

.cdn-host {
  flex: 1;
  word-break: break-all;
}

.cdn-badge {
  font-size: 0.72em;
  padding: 1px 6px;
  border-radius: 4px;
  background: var(--accent);
  color: #fff;
  margin-left: 6px;
  white-space: nowrap;
}

.cdn-prio {
  font-size: 0.72em;
  color: var(--text-muted2);
  margin-left: 6px;
  white-space: nowrap;
}

.select-option {
  display: flex;
  align-items: center;
  gap: 4px;
}

.disabled-opt {
  color: var(--text-muted2);
  cursor: default;
  font-style: italic;
}

.disabled-opt:hover {
  background: transparent;
  color: var(--text-muted2);
}

/* ── Operations ── */
.ops-panel {
  background: var(--bg-secondary);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  padding: 12px;
}

.ops-panel h3 {
  font-size: 0.85em;
  color: var(--text-muted2);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  margin-bottom: 8px;
}

.ops-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.push-right {
  margin-left: auto;
}

.btn-primary,
.btn-secondary,
.btn-danger,
.btn-sm {
  padding: 8px 16px;
  border: 1px solid transparent;
  border-radius: 6px;
  font-family: inherit;
  font-size: 0.95em;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s;
}

.btn-primary {
  background: var(--accent);
  color: #fff;
  border-color: var(--accent);
}

.btn-primary:hover:not(:disabled) {
  background: var(--accent-hover);
  border-color: var(--accent-hover);
}

.btn-secondary {
  background: transparent;
  color: var(--text-muted);
  border-color: var(--border-color);
}

.btn-secondary:hover:not(:disabled) {
  background: var(--accent-bg);
  color: var(--text-primary);
}

.btn-danger {
  background: var(--danger);
  color: #fff;
  border-color: var(--danger);
}

.btn-danger:hover:not(:disabled) {
  background: var(--danger-hover);
  border-color: var(--danger-hover);
}

.btn-sm {
  padding: 4px 10px;
  font-size: 0.85em;
  background: var(--accent-bg);
  color: var(--text-muted);
  border-color: var(--border-color);
  width: auto;
}

.btn-sm:hover:not(:disabled) {
  background: var(--accent-bg-hover);
  color: var(--text-primary);
}

button:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

/* ── Progress ── */
.progress-panel {
  background: var(--bg-secondary);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  padding: 12px;
}

.progress-info {
  display: flex;
  justify-content: space-between;
  margin-bottom: 6px;
  font-size: 0.9em;
  color: var(--text-primary);
}

.filename {
  color: var(--text-secondary);
  font-size: 0.85em;
  max-width: 60%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.progress-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.progress-bar-bg {
  flex: 1;
  height: 12px;
  background: var(--input-bg);
  border-radius: 6px;
  overflow: hidden;
  border: 1px solid var(--border-color);
}

.progress-bar-fill {
  height: 100%;
  background: linear-gradient(90deg, var(--accent), var(--accent-hover));
  border-radius: 6px;
  transition: width 0.3s ease;
}

.progress-bar-fill.done {
  background: linear-gradient(90deg, var(--progress-done), var(--progress-done-light));
}

.progress-pct {
  font-size: 0.85em;
  color: var(--text-secondary);
  min-width: 36px;
  text-align: right;
}

.spinner {
  width: 32px;
  height: 32px;
  flex-shrink: 0;
  position: relative;
  overflow: visible;
  opacity: 0;
  transition: opacity 0.2s;
}

.spinner.active {
  opacity: 1;
}

.spinner-ring {
  position: absolute;
  top: calc(50% - 16px);
  left: calc(50% - 16px);
  width: 32px;
  height: 32px;
  display: block;
}

.spinner span {
  position: absolute;
  width: 100%;
  height: 100%;
  opacity: 0;
}

.spinner span:after {
  content: "";
  display: block;
  position: absolute;
  left: 0;
  top: 0;
  width: 5px;
  height: 5px;
  background: var(--accent);
  border-radius: 50%;
}

.spinner span:nth-child(1) { animation: s_i1 5.5s 0.2s infinite; }
.spinner span:nth-child(2) { animation: s_i2 5.5s 0.4s infinite; }
.spinner span:nth-child(3) { animation: s_i3 5.5s 0.6s infinite; }
.spinner span:nth-child(4) { animation: s_i4 5.5s 0.8s infinite; }
.spinner span:nth-child(5) { animation: s_i5 5.5s 1.0s infinite; }
.spinner span:nth-child(6) { animation: s_i6 5.5s 1.2s infinite; }

@keyframes s_i1 {
  0%   { opacity:1; transform:rotate(190deg); animation-timing-function:cubic-bezier(0.29,0.44,0.32,0.74); }
  7%   { opacity:1; transform:rotate(300deg); animation-timing-function:linear; }
  30%  { opacity:1; transform:rotate(450deg); animation-timing-function:cubic-bezier(0.53,0.27,0.37,0.81); }
  39%  { opacity:1; transform:rotate(645deg); animation-timing-function:linear; }
  63%  { opacity:1; transform:rotate(800deg); animation-timing-function:cubic-bezier(0.5,0.32,0.82,0.54); }
  68%  { opacity:1; transform:rotate(920deg); animation-timing-function:ease-in; }
  69%  { opacity:0; transform:rotate(930deg); }
}
@keyframes s_i2 {
  0%   { opacity:1; transform:rotate(180deg); animation-timing-function:cubic-bezier(0.29,0.44,0.32,0.74); }
  7%   { opacity:1; transform:rotate(300deg); animation-timing-function:linear; }
  30%  { opacity:1; transform:rotate(450deg); animation-timing-function:cubic-bezier(0.53,0.27,0.37,0.81); }
  39%  { opacity:1; transform:rotate(645deg); animation-timing-function:linear; }
  63%  { opacity:1; transform:rotate(800deg); animation-timing-function:cubic-bezier(0.5,0.32,0.82,0.54); }
  68%  { opacity:1; transform:rotate(910deg); animation-timing-function:ease-in; }
  69%  { opacity:0; transform:rotate(920deg); }
}
@keyframes s_i3 {
  0%   { opacity:1; transform:rotate(170deg); animation-timing-function:cubic-bezier(0.29,0.44,0.32,0.74); }
  7%   { opacity:1; transform:rotate(300deg); animation-timing-function:linear; }
  30%  { opacity:1; transform:rotate(450deg); animation-timing-function:cubic-bezier(0.53,0.27,0.37,0.81); }
  39%  { opacity:1; transform:rotate(645deg); animation-timing-function:linear; }
  63%  { opacity:1; transform:rotate(800deg); animation-timing-function:cubic-bezier(0.5,0.32,0.82,0.54); }
  68%  { opacity:1; transform:rotate(900deg); animation-timing-function:ease-in; }
  69%  { opacity:0; transform:rotate(910deg); }
}
@keyframes s_i4 {
  0%   { opacity:1; transform:rotate(160deg); animation-timing-function:cubic-bezier(0.29,0.44,0.32,0.74); }
  7%   { opacity:1; transform:rotate(300deg); animation-timing-function:linear; }
  30%  { opacity:1; transform:rotate(450deg); animation-timing-function:cubic-bezier(0.53,0.27,0.37,0.81); }
  39%  { opacity:1; transform:rotate(645deg); animation-timing-function:linear; }
  63%  { opacity:1; transform:rotate(800deg); animation-timing-function:cubic-bezier(0.5,0.32,0.82,0.54); }
  68%  { opacity:1; transform:rotate(890deg); animation-timing-function:ease-in; }
  69%  { opacity:0; transform:rotate(900deg); }
}
@keyframes s_i5 {
  0%   { opacity:1; transform:rotate(150deg); animation-timing-function:cubic-bezier(0.29,0.44,0.32,0.74); }
  7%   { opacity:1; transform:rotate(300deg); animation-timing-function:linear; }
  30%  { opacity:1; transform:rotate(450deg); animation-timing-function:cubic-bezier(0.53,0.27,0.37,0.81); }
  39%  { opacity:1; transform:rotate(645deg); animation-timing-function:linear; }
  63%  { opacity:1; transform:rotate(800deg); animation-timing-function:cubic-bezier(0.5,0.32,0.82,0.54); }
  68%  { opacity:1; transform:rotate(880deg); animation-timing-function:ease-in; }
  69%  { opacity:0; transform:rotate(880deg); }
}
@keyframes s_i6 {
  0%   { opacity:1; transform:rotate(140deg); animation-timing-function:cubic-bezier(0.29,0.44,0.32,0.74); }
  7%   { opacity:1; transform:rotate(300deg); animation-timing-function:linear; }
  30%  { opacity:1; transform:rotate(450deg); animation-timing-function:cubic-bezier(0.53,0.27,0.37,0.81); }
  39%  { opacity:1; transform:rotate(645deg); animation-timing-function:linear; }
  63%  { opacity:1; transform:rotate(800deg); animation-timing-function:cubic-bezier(0.5,0.32,0.82,0.54); }
  68%  { opacity:1; transform:rotate(870deg); animation-timing-function:ease-in; }
  69%  { opacity:0; transform:rotate(880deg); }
}

.progress-message {
  margin-top: 4px;
  font-size: 0.85em;
  color: var(--text-secondary);
}

/* ── Log ── */
.log-panel {
  flex: 1;
  min-height: 0;
  background: var(--bg-secondary);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.log-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
}

.log-header h3 {
  font-size: 0.85em;
  color: var(--text-muted2);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.log-container {
  flex: 1;
  overflow-y: auto;
  padding: 8px 12px;
  font-size: 0.85em;
  line-height: 1.6;
}

.log-line {
  display: flex;
  gap: 10px;
  padding: 1px 0;
}

.log-time {
  color: var(--text-muted2);
  flex-shrink: 0;
}

.log-msg {
  color: var(--text-secondary);
  word-break: break-all;
}

.log-error .log-msg {
  color: var(--log-error);
}

.log-error .log-time {
  color: var(--log-error);
}

.log-info .log-msg {
  color: var(--text-secondary);
}

.log-empty {
  color: var(--text-muted2);
  font-style: italic;
  padding: 20px;
  text-align: center;
}

/* ── Linux Guide Modal ── */
.modal-overlay {
  position: fixed;
  inset: 0;
  background: var(--modal-overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.modal-content {
  background: var(--bg-secondary);
  border: 1px solid var(--border-color);
  border-radius: 12px;
  max-width: 650px;
  max-height: 80vh;
  overflow-y: auto;
  padding: 0;
  margin: 20px;
}

.modal-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 16px 20px;
  border-bottom: 1px solid var(--border-color);
}

.modal-header h2 {
  font-size: 1.1em;
  color: var(--text-primary);
  margin: 0;
}

.modal-body {
  padding: 20px;
  color: var(--text-secondary);
  font-size: 0.9em;
  line-height: 1.7;
}

.modal-body h3 {
  color: var(--text-primary);
  font-size: 1em;
  margin: 16px 0 8px;
}

.modal-body h4 {
  color: var(--text-muted);
  font-size: 0.95em;
  margin: 12px 0 6px;
}

.modal-body p {
  margin: 8px 0;
}

.modal-body ol {
  padding-left: 20px;
  margin: 6px 0;
}

.modal-body li {
  margin: 4px 0;
}

.modal-body code {
  background: var(--input-bg);
  padding: 2px 6px;
  border-radius: 4px;
  font-size: 0.9em;
  color: var(--accent-hover);
}

.modal-body pre {
  background: var(--input-bg);
  padding: 10px 14px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  overflow-x: auto;
}

.modal-body pre code {
  background: none;
  padding: 0;
  font-size: 0.9em;
  color: var(--text-secondary);
}

.modal-body blockquote {
  border-left: 3px solid var(--accent);
  padding: 6px 12px;
  margin: 12px 0;
  color: var(--text-muted2);
  font-size: 0.85em;
}

.modal-desc {
  color: var(--text-muted) !important;
  margin-bottom: 12px !important;
}
</style>
