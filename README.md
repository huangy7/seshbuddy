<p align="center">
  <img src="src/assets/mascot.png" alt="SeshBuddy" width="120">
</p>

<h1 align="center">SeshBuddy</h1>

<p align="center">
  <b>Your desktop companion and session explorer for AI coding agents</b>
</p>

<p align="center">
  <a href="README.md"><img src="https://img.shields.io/badge/lang-English-blue?style=flat-square" alt="English"></a>
  <a href="README.zh-CN.md"><img src="https://img.shields.io/badge/lang-简体中文-lightgrey?style=flat-square" alt="简体中文"></a>
</p>

<p align="center">
  <a href="https://seshbuddy.huangy.top/en/"><b>Download for macOS, Windows or Linux</b></a>
</p>

<p align="center">
  <a href="https://seshbuddy.huangy.top/en/"><img src="https://img.shields.io/badge/website-seshbuddy.huangy.top-blue?style=flat-square" alt="Website"></a>
  <a href="https://github.com/huangy7/seshbuddy/releases/latest"><img src="https://img.shields.io/github/v/release/huangy7/seshbuddy?style=flat-square&label=latest" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/huangy7/seshbuddy?style=flat-square" alt="License"></a>
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-blue?style=flat-square" alt="Platform">
  <a href="https://github.com/huangy7/seshbuddy/releases"><img src="https://img.shields.io/github/downloads/huangy7/seshbuddy/total?style=flat-square&color=success" alt="Downloads"></a>
  <img src="https://komarev.com/ghpvc/?username=huangy7-seshbuddy&label=Views&color=0071e3&style=flat-square" alt="Views">
</p>

<p align="center">
  <img src="assets/screenshot-main.png" alt="SeshBuddy browsing sessions from multiple CLIs in one tree" width="880">
</p>

SeshBuddy is a native desktop workbench for the sessions you build with AI coding agents. It reads the session data and logs that Claude Code, Codex, Gemini, Antigravity, WorkBuddy, DSH, OpenCode, Cursor, Pi, Aider, Kimi, Goose and Grok already write to disk, indexes them locally, and gives you one place to browse, search, archive and inspect everything — plus a local reverse proxy that shows you exactly what your agent sends over the wire.

## Features

| Feature | Description |
| :--- | :--- |
| **Multi-CLI Session Explorer** | Browse, organize, bookmark and archive sessions from **Claude Code / Codex / Gemini / Antigravity / WorkBuddy / DSH / OpenCode / Cursor / Pi / Aider / Kimi / Goose / Grok** in a single tree. |
| **Unified Full-Text Search** | A local **Tantivy** index over every prompt, reply, tool call and code block, with session-title matching folded into the same result set. |
| **API Reverse Proxy & Traffic Inspector** | `seshbuddy-proxy` sits between your CLI and the vendor endpoint (**Claude Code** and **Codex**) so you can read request/response payloads, latency and token usage. One click to enable, one click to restore the original endpoint. |
| **Monaco Editor & Git Diff** | Review what an agent changed with side-by-side diffs and syntax highlighting, without leaving the app. |
| **Desktop Assistant** | Ask questions about past sessions, summarize work, or pull out insights. Answers cite their sources and link back to the exact session. |

## Privacy

Sessions never leave your machine. There is no account and no telemetry. The only outbound requests are ones you trigger or configure — the optional model-pricing catalog (`models.dev`), your own API endpoint, and the update check against GitHub Releases.

## Build from Source

You need Node.js v20+, a stable Rust toolchain, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
git clone https://github.com/huangy7/seshbuddy.git
cd seshbuddy
npm install
npm run tauri dev
```

`npm run tauri dev` builds the `seshbuddy-proxy` sidecar first — the desktop app spawns it at runtime.

See [DEVELOPMENT.md](DEVELOPMENT.md) for dependencies, debugging, tests and the release process.

## Contributors

<p align="center">
  <a href="https://github.com/huangy7/seshbuddy/graphs/contributors">
    <img src="https://contrib.rocks/image?repo=huangy7/seshbuddy&max=100" alt="Contributors" />
  </a>
</p>

## License

Released under the [MIT](LICENSE) License.

---

<p align="center">
  Developed and maintained by <a href="https://github.com/huangy7">huangy7</a>
</p>
