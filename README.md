# Codex Workflow Kit

Codex Workflow Kit 提供單一 `$work` skill，協助你規劃需求、拆分任務並執行工作。

## 功能

1. 使用同一個入口處理 Plan、Task 與 Execute。
2. Plan 會依需求推薦適合的 instruction hierarchy 與技能，並顯示說明及推薦原因。
3. Instruction hierarchy 與技能只有在使用者確認後才會載入。
4. Plan 可組合多個技能，例如 UI、frontend 與 backend。
5. Task 會把工作拆成最小可執行任務；Execute 只使用目標任務需要的技能。

## 使用方式

### 語法

```text
$work <mode> -- <request>
```

可用 `<mode>` 模式：

1. `plan`：規劃需求並推薦技能。
2. `task`：依已確認的 Plan 拆分任務。
3. `execute`：執行指定任務。

### 範例

```text
$work plan -- 建立一個包含 UI、frontend 與 backend 的網站
$work task -- 依已確認 Plan 拆分網站任務
$work execute -- 執行正式 TASK-001
```

Plan 推薦技能後，你可以接受、加入、移除或取消。若沒有合適技能，也可以確認只使用 Work 的基本能力。

## 必要環境

1. Python 3.10 以上。
2. PyYAML。

安裝器只檢查必要環境，不會自動安裝或升級 Python 套件。

## 安裝

Work skill 會安裝到使用者目錄下的 `.agents/skills/work`。安裝時可以選擇預設或自訂使用者目錄。

可選擇的 instruction hierarchy：

安裝器提供以下選項：

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

可輸入空白分隔的多個編號，或輸入 `all`。選擇較深層項目時會自動包含父層。

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

## 版本限制

本版本刻意不相容舊 Work 架構：

1. 公開入口只有 `$work`，不再使用 `$plan`、`$task` 或 `$execute`。
2. 本版本不相容舊 Work 架構，也不會自動升級舊產物。
3. 不再需要的舊檔案需由使用者確認後自行清理。

## 驗證狀態

1. Work skill 結構與完整 Work 測試已通過。
2. Windows 安裝器手動測試已通過；管線式自動互動測試仍有已知限制。
3. macOS 安裝器具備跨平台靜態驗證，實際整合測試需在 macOS 執行。

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
