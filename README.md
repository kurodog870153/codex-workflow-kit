# Codex Workflow Kit

Codex Workflow Kit 透過 `$work` 協助你規劃需求、拆分任務並執行工作。

## 功能

1. 使用 task、revise、migration、execute 四種模式，從原始需求規劃並驗證工作。
2. Task 先捕捉一次不可變 Source，獨立推薦並確認工作類型與外部技能；你可以接受、調整或選擇基本能力。
3. Task 保存需求層與子任務驗收，規劃相依、步驟及 VAL，預覽後以核准指紋建立正式 TASK collection 與 Execution index。
4. Execute 只使用指定 TASK 的指令與技能，先驗證來源、工作區與授權，再保存不可改寫的 Attempt／Correction 證據。
5. Revise 透過 Specification 流程修訂有效的 Source／TASK／Execution 集合，審查整體及下游影響，核准後發布或復原同一交易。
6. Migration 診斷既有不相容或損壞文件，比較原始 bytes 與目前契約，保留歷史證據，經核准後重建並驗證完整 Task／Execution 綁定；必要時可明確核准無 Source 的 migration provenance。

## 使用方式

```text
$work <mode> -- <request>
```

1. `task`：捕捉需求 Source、確認技能與驗收，規劃並建立第一版 TASK collection。
2. `revise`：修訂已驗證的正式規格與相關 Execution 狀態。
3. `migration`：診斷並遷移或重建不相容的既有證據。
4. `execute`：在驗證與授權後執行指定正式 TASK。

```text
$work task -- 建立一個包含 UI、frontend 與 backend 的網站
$work revise -- 調整 example 的 TASK-001 驗收條件並檢查下游影響
$work migration -- 分析 example 的既有文件，重建損壞的 Task 與 Execution 關聯
$work execute -- 執行 example 的正式 TASK-001
```

一般流程是 Source → Task → Revise（需要修改正式規格時）→ Execute。Task 只捕捉一次原始需求；後續討論、進度及草稿沿用同一 Source 和已確認選擇。第一版正式集合經 preview／apply 建立，既有有效規格使用 Revise；無法建立可信基線時，先使用 Migration analyze。中斷只能由該領域的 recover 恢復同一核准集合，不能手寫 JSON 或改寫歷史。

一般對話也可以收到 Work 使用建議。只有你確認精確模式與需求後，才會透過 confirmed 入口啟動；系統保留 implicit_confirmed 證據，不偽造明示指令。啟動 Work 本身不授權檔案寫入或執行。

### 保存討論進度

1. 在 Task 討論中要求保存進度，核對保存內容與核准指紋後即可暫停，不必先完成全部決策。
2. 已確認事項、未決方案、問題與續談位置，以及同一 Source 與選擇 context 都會保留。
3. 用下列指令恢復；進度僅為歷史討論，不代表正式驗證或執行授權。

```text
$work task -- resume example
```

## 文件位置

1. Source：`outputs/work/sources/<requirement-id>/SRC-NNN/`，包含 manifest、完成標記與精確原始內容。 File capture metadata 的 `source` 必須包含 `kind: file`、原始 `path` 及 host 提供的 `media_type`（如 `application/pdf`；僅 type/subtype，不含參數），原樣保存且不從副檔名猜測；capture time 保存於 `captured_at`。
2. 正式 TASK：`outputs/work/tasks/<requirement-id>/index.json` 與 `tasks/TASK-NNN.json`，保存主驗收及各 TASK 子驗收。
3. Execution：`outputs/work/executions/<requirement-id>/`，保存 index、不可變 Attempt／Correction 及衍生交易紀錄。
4. 討論進度與 draft 不替代正式 TASK；自訂路徑需完整確認並通過跨平台安全檢查。

## 必要環境

1. Windows 或 macOS；本 Issue 不提供 Linux 安裝器或正式相容性保證。
2. 已安裝 Rust 與 Cargo 1.85 或更新版本，以及本機可用的 linker 與 SDK。macOS 需要 Xcode Command Line Tools；Windows 需要對應 MSVC 或 GNU Rust 目標的建置工具。
3. 首次編譯需要可取得 `rust/Cargo.lock` 指定的 crate；Cargo 可下載未快取的 crate。

安裝器會在 `rust/` 目錄從原始碼執行 `cargo build --release --locked -p work-cli`，不安裝工具鏈或永久修改 PATH。macOS 安裝器在目前 PATH 找不到 Rust 時，會尋找使用者目錄與常見 Homebrew 位置的 Rust 工具鏈；Windows 安裝器則會尋找 `CARGO_HOME` 與使用者目錄中的 Cargo 工具鏈。因此可直接點擊 `.command` 或 `.bat` 執行。

## 安裝

Work skill 預設安裝到使用者目錄下的 `.agents/skills/work`。選擇自訂安裝目錄時，會安裝到該目錄下的 `skills/work`，不會再加上 `.agents`。自訂安裝目錄須已存在。

安裝時可依專案選擇適用的工作類型：

1. `general only`。
2. `web`。
3. `backend`。
4. `java`。
5. `jpa`。
6. `mybatis`。
7. `frontend`。
8. `typescript`。
9. `astro`。
10. `css`。
11. `tailwind`。
12. `spring-boot`。

可輸入空白分隔的多個編號，或輸入 `all`。選擇較深層項目時會自動包含父層。程式語言、Web 後端與 Spring Boot 可獨立選取；需要 Web Java 後端時請同時選擇適用的路徑。TypeScript 前端工作須同時選取 `typescript` 與 `frontend`；`astro` 只代表前端框架。

### macOS

執行：

```bash
os-scripts/mac/install-work.command
```

若檔案沒有執行權限，可由使用者自行執行：

```bash
chmod +x os-scripts/mac/*.command
```

### Windows

執行：

```text
os-scripts\windows\install-work.bat
```

同名檔案會覆寫，但安裝器不會自動刪除舊檔案。

重新選擇工作類型不會移除已安裝的分支。例如先安裝 `all`，再選 `general only`，先前的 web 分支仍會保留；其中已安裝的官方指引會更新，本次未選且原本不存在的分支不會新增。使用者自加檔案及舊版 `work.py`／`worklib/` 也會保留。逐檔複製若中途失敗，目錄可能暫時混合新舊版本；此風險已接受，應修正原因後重跑安裝器。

安裝器會在寫入前檢查必要的 workflow、subagent、Cargo manifest 與所選 instruction，並完成本機編譯及 `--help` 啟動檢查。編譯失敗不會改動已安裝的 binary。安裝位置的入口是 `<skill-root>/scripts/work`（macOS）或 `<skill-root>\scripts\work.exe`（Windows）；從不同工作目錄使用時，以已解析的 skill root 組成完整路徑，不依賴 PATH。指引檔逐檔複製失敗時不提供自動回復。

## 清理本機 Codex 資料

> [!WARNING]
> 清理腳本會永久刪除本機 Codex 工作階段、封存工作階段、產生的圖片、歷史紀錄與相關狀態資料。執行前請先關閉 Codex，並確認不需要保留這些資料。

1. Windows：`os-scripts/windows/clean-codex-data.bat`
2. macOS：`os-scripts/mac/clean-codex-data.command`

執行前請先關閉 Codex，確認畫面顯示的刪除範圍，再依提示操作。

## License

This project is licensed under the
[PolyForm Noncommercial License 1.0.0](LICENSE).

You may use, modify, and distribute this project only for purposes permitted
under the License.

Commercial use is not licensed under the PolyForm Noncommercial License 1.0.0.
A separate commercial license or other written permission from the licensor is
required before any commercial use.

Examples of commercial use that require separate authorization include,
but are not limited to:

- Using this project as part of paid software development or consulting work
- Using this project to provide services to paying clients
- Incorporating this project into a commercial product, service, or SaaS
- Selling access to, or commercially distributing, this project or a derivative work

These examples are provided for clarification only and do not modify or replace
the terms of the License. In the event of any inconsistency, the terms in the
[LICENSE](LICENSE) file govern.

For commercial licensing inquiries, please contact the project author.
