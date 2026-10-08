# Codex Workflow Kit

Codex Workflow Kit 透過 `$work` 協助你規劃需求、拆分任務並執行工作。

## 功能

1. 整理原始需求，確認工作類型、需要的技能與驗收條件。
2. 將需求拆分為可執行的任務，安排相依關係與驗證步驟。
3. 在正式建立或修訂任務前，提供預覽供你審查與核准。
4. 依授權執行指定任務，保存執行結果與驗證紀錄。
5. 保存討論進度，方便稍後接續規劃。
6. 分析不相容或損壞的既有工作資料，保留原始紀錄並協助遷移。

## 使用方式

```text
$work <mode> -- <request>
```

在 Codex 對話中輸入以下指令，選擇適合目前工作的模式：

1. `task`：從新需求開始，確認驗收條件並建立任務。
2. `revise`：修改已建立的需求或任務，檢查對後續工作的影響。
3. `migration`：分析不相容或損壞的既有工作資料，審查遷移方案。
4. `execute`：在驗證與授權後執行指定任務。

```text
$work task -- 建立一個包含 UI、frontend 與 backend 的網站
$work revise -- 調整 example 的 TASK-001 驗收條件並檢查下游影響
$work migration -- 分析 example 的既有文件，重建損壞的 Task 與 Execution 關聯
$work execute -- 執行 example 的正式 TASK-001
```

建議先用 `task` 規劃，審查並核准任務後再用 `execute` 執行。需求改變時使用 `revise`；舊資料無法正常使用時，先用 `migration` 分析。

規劃時會保存原始需求，後續討論沿用同一份紀錄。Work 會請你提供並確認需求編號，例如 `example`，用來區分不同需求的資料。

Work 也可能在一般對話中建議適合的模式，經你確認模式與需求後才啟動。啟動 Work 本身不代表授權寫入檔案或執行任務；請在審查範圍與操作內容後給予授權。

### 保存討論進度

1. 確認需求編號、專案位置及討論進度的保存範圍。
2. Work 會保存已確認事項、待決定的方案、問題與續談位置。
3. 用下列指令接續已保存的討論；將 `example` 換成你的需求編號。保存討論的授權不代表授權建立正式任務或執行工作。

```text
$work task -- resume example
```

## 工作資料與中斷處理

工作資料保存在專案的 `outputs/work/` 下，依需求編號分類：

1. `sources/<requirement-id>/`：原始需求與附件。
2. `tasks/<requirement-id>/`：正式任務及驗收條件。
3. `executions/<requirement-id>/`：執行結果、修正與驗證紀錄。
4. `discussions/<requirement-id>/`：目前討論進度與歷史版本。
5. `transactions/`：各次操作的輸入、請求與回應，包含尚未確認需求編號的操作。
6. `runtime/`：執行期間的鎖定與暫存資料，由 Work 管理。

備份時請保留完整的工作資料。原始需求、討論歷史與執行紀錄會持續保存；專案的 Git ignore 預設排除 `transactions/`，若要備份操作輸入與回應，請一併保存該目錄。

操作中斷時，請提供需求編號與錯誤訊息，讓 Work 檢查並透過對應流程復原。不要手動修改工作紀錄或刪除鎖定檔；部分執行證據可能表示命令已經啟動，需要先確認結果，才能決定後續處理。

舊版工作資料若遭到拒絕，請使用 `migration` 分析，依審查結果保留原始資料並進行遷移。

## 必要環境

1. Windows 或 macOS；目前提供這兩個平台的安裝器。
2. 已安裝 Rust 與 Cargo 1.85 或更新版本，以及本機可用的 linker 與 SDK。macOS 需要 Xcode Command Line Tools；Windows 需要對應 MSVC 或 GNU Rust 目標的建置工具。
3. 首次編譯需要可取得 `rust/Cargo.lock` 指定的 crate；Cargo 可下載未快取的 crate。

安裝器會從原始碼編譯 Work，請先準備好 Rust 與必要的建置工具。安裝器不會替你安裝工具鏈或永久修改 PATH。macOS 與 Windows 安裝器皆可直接點擊執行。

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

### 更新既有安裝

重新執行安裝器即可更新，並重新選擇需要的工作類型。安裝器會先編譯與檢查新版本，再替換既有安裝；準備或編譯失敗時，舊安裝保持不變。

更新會替換整個 Work skill 目錄。未選取的工作類型、自行加入的檔案及舊版檔案不會保留在新安裝中；完整舊檔案會保存於畫面顯示的復原目錄 `previous/`。替換失敗時，安裝器會嘗試還原，請保留復原目錄直到確認新版本可用。

## 進一步閱讀

1. [Work 流程與操作指引](skills/work/references/instruction-loading.md)。
2. [既有資料診斷與遷移指引](skills/work/references/instruction-loading/artifact-migration.md)。
3. [Rust 架構與執行產物管理](docs/work/rust/architecture.md)。
4. [工作指引架構](docs/work/instruction/architecture.md)。

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
