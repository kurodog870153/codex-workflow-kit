# Codex Workflow Kit

Codex Workflow Kit 透過 `$work` 協助你規劃需求、拆分任務並執行工作。

## 功能

1. 使用同一個入口處理 Plan、Task、Revise、Migration 與 Execute。
2. Plan 會依需求推薦適合的工作類型與技能，並說明推薦原因。
3. 推薦內容由你確認後才會使用。
4. Plan 可組合多個技能，例如 UI、frontend 與 backend。
5. Task 以 prepare、status、save、preview、apply、recover 規劃並建立第一版正式 TASK collection；Execute 只使用目標任務需要的技能。
6. Revise 透過內部 Specification 流程修改有效的正式 Plan、TASK 與 Execution，預覽完整變更並在核准後以同一交易發布。
7. Migration 診斷損壞或不相容的既有文件，必要時重建相關 Plan、TASK 與 Execution；選擇性遷移後會核對最終指紋。

## 使用方式

### 語法

```text
$work <mode> -- <request>
```

可用 `<mode>` 模式：

1. `plan`：規劃需求並推薦技能。
2. `task`：依已確認的 Plan 討論任務，建立第一版正式 TASK collection。
3. `revise`：依確認的語意變更，修訂有效的正式 Plan、TASK 與 Execution。
4. `migration`：分析、遷移或重建損壞及不相容的既有文件。
5. `execute`：執行指定任務。

### 範例

```text
$work plan -- 建立一個包含 UI、frontend 與 backend 的網站
$work task -- 依已確認 Plan 拆分網站任務
$work revise -- 調整 example 的 TASK-001 驗收條件並檢查下游影響
$work migration -- 分析 example 的既有 Plan、TASK 與 Execution，重建損壞的關聯
$work execute -- 執行正式 TASK-001
```

Plan 推薦技能後，你可以接受、加入、移除或取消。若沒有合適技能，也可以確認只使用 Work 的基本能力。

一般流程是 Plan → Task → Revise（需要修改正式規格時）→ Execute。Task 的第一版正式文件須先預覽，再以核准的指紋發布；已有有效正式文件的修改使用 Revise。若現有文件無法構成可信的正式基線，先使用 Migration analyze 診斷，確認重建內容後再執行。交易中斷時，只能用該領域的 recover 恢復相同且已核准的變更。

### 保存討論進度

1. 在 Plan 或 Task 討論中說「先保存目前進度」，確認保存內容後即可暫停，不必先完成所有討論。
2. 已確認事項、尚未決定的方案、待回答問題與下次討論位置都會保留。初次保存時，若尚無需求編號，會請你指定。
3. 在同一或新的對話中，使用以下指令繼續討論，將 `example` 換成保存時的需求編號。

```text
$work plan -- resume example
$work task -- resume example
```

保存進度不代表討論已完成，也不會開始執行任務。

## 文件位置

規劃文件、討論進度與執行紀錄預設放在專案的 `outputs/work/` 目錄。保存完成後會提供文件位置。

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
