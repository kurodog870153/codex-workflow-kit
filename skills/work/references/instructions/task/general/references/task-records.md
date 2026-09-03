# 正式 TASK 與索引參考指令

參考名稱：task.general.task-records
適用層級：task.general
指令分類狀態：已完成

## 1. 正式 TASK

1. [強制] 一份已確認 Plan 對應一份持續維護的 TASK 文件；初版使用 `TASK-SPEC-001`，canonical TASK JSON 實質改變時每次核准只遞增一次。
2. [強制] 必要頂層欄位依序為 `schema`、`requirement_id`、`spec_id`、`status`、`title`、`summary`、`artifacts`、`source_plan`、`instruction_selection`、`tasks`、`readiness`；`execution_defaults`、`decisions`、`changes` 僅在有內容時加入其固定位置。
3. [強制] `source_plan.canonical_sha256` 與 `source_plan.hierarchy_selection_sha256` 必須分別等於同一需求正式 Plan 的 canonical SHA 與 hierarchy selection SHA；三個 `artifacts` 必須與 Plan 完全一致。
4. [強制] 文件層 `instruction_selection` 保存所有 TASK Work 指令來源與 routed reference 的第一出現聯集；每個 TASK 保存自己的 Work instruction selection 與 `instructions_sha256`。外部技能身分只由來源 Plan `skill_selection` 與 TASK `skill_id` 表達。
5. [強制] 每個 TASK 必要欄位為 `id`、`title`、`skill_id`、`instruction_selection`、`traceability`、`goal`、`steps`、`validations`；選用 `dependencies`、`inputs`、`decisions`、`files`、`risks`、`commands`、`operations`。

## 2. ID、相依與追溯

1. [強制] `TASK-*` 在文件內唯一；每個 TASK 的 `INPUT-*`、`TASK-DECISION-*`、`FILE-*`、`RISK-*`、`STEP-*`、`CMD-*`、`OP-*`、`VAL-*` 各自由 `001` 開始。文件層共用決策使用 `DECISION-*`。
2. [強制] TASK 內引用使用短 ID；跨 TASK 引用使用 `<TASK-ID>/<ITEM-ID>`。既有 ID 不得重編或重用。
3. [強制] `dependencies` 只列直接相依 TASK，禁止自我相依、循環、未知 ID 與可由其他直接相依推導的冗餘相依。
4. [強制] `traceability` 必須包含非空的 `goal_ids`、`deliverable_ids`、`acceptance_ids`；Plan 有 milestone 時以 `milestone_ids` 完整覆蓋。
5. [強制] 影響至少兩個 TASK 的共用決策置於文件層並保存 `task_ids`；只影響單一 TASK 的已確認決策置於該 TASK。

## 3. 輸入、檔案與步驟

1. [強制] `inputs[]` 使用 `id`、`kind`、`source`、`precondition`；`kind` 只允許 `task_output`、`project_state`、`user_provided`、`external`。機密值不得寫入 TASK。
2. [強制] `task_output.source` 使用 `<TASK-ID>/<FILE-ID>`，且來源 TASK 必須是直接相依。
3. [強制] `project_state.source` 使用 normalized project-relative path；`user_provided` 與 `external` 的 source 只保存非機密識別或描述，不保存實際機密值。
4. [強制] `files[]` 的 `action` 只允許 `create`、`modify`、`move`；前兩者使用 `path`，move 使用 `source` 與 `destination`，不支援 delete。
5. [強制] `risks[]` 使用 `condition`、`impact`、`mitigation`。
6. [強制] `steps[]` 使用 `id`、`action`、非空 `references`；陣列順序就是執行順序。所有 FILE、CMD、OP、VAL 必須至少被一個 STEP 引用。

## 4. CMD、OP 與 VAL

1. [強制] `execution_defaults` 與命令 `execution` 完整覆寫都使用 `working_directory`、`os`、`shell`；OS 只允許 `windows`、`macos`、`linux`，shell 只允許 `powershell`、`pwsh`、`cmd`、`bash`、`zsh`、`sh`。
2. [強制] `commands[]` 使用 `mode: argv` 加 `argv` 字串陣列，或 `mode: shell` 加 `script`；兩種專屬欄位互斥。
3. [強制] `operations[]` 只描述非檔案副作用，使用 `kind: local_state` 或 `external_state`、`action`、`target`、`validation_id`，由命令執行時加入 `command_id`。
4. [強制] `validations[]` 使用 `kind: automated` 加 `command_ids`、`pass_condition`，或 `kind: manual` 加 `confirmer`、`criteria`；直接驗收成果時加入 `acceptance_ids`。
5. [強制] Python validator 不執行 CMD 或 OP；具副作用預檢仍須另行授權。

## 5. 升版與就緒

1. [強制] 初版省略 `changes`；升版使用 `TASK-CHANGE-*`，保存 `spec_id`、ISO date、reason、affected IDs、選用 Plan change IDs 與結構化 edits。
2. [強制] edit `operation` 為 `add` 時只保存 `after`，`replace` 保存 `before` 與 `after`，`remove` 只保存 `before`；`path` 使用 JSON Pointer。
3. [強制] 純需求編號及三路徑重新命名不升版；其他 canonical TASK 實質變更皆升版。指令來源身分或順序改變屬 TASK 變更；只有相同來源內容改變且 TASK 仍完整有效時可走 instruction audit。
4. [強制] 正式 `readiness` 固定為 `status: passed` 且 `spec_id` 等於頂層 spec；詳細探索證據只在核准前對話展示。

## 6. Index 與交易邊界

1. [強制] execution index 使用 `work-execution-index/v1` canonical H1＋JSON，保存 TASK spec、TASK SHA、Plan `hierarchy_selection_sha256` 與 `skill_selection_sha256`、文件與每 TASK instructions SHA、每 TASK `skill_id`、狀態及選用 lock／audit reference；不得複製 TASK 規格或技能全文。
2. [強制] TASK 狀態只使用 `pending`、`in_progress`、`pending_retry`、`blocked`、`completed`、`cancelled`；`overall_status` 必須由 Python 推導。
3. [強制] 初版 TASK 與 index 由 `task create` 使用同一已核准 JSON 建立；create 要求 TASK 與 requirement-specific execution 目錄都不存在，先 exclusive create TASK，再建立 execution 目錄與 canonical `index.md`。
4. [強制] 規格鎖與 execution lock 互斥；部分失敗時保留現況與 lock，不自動回復或覆寫。
5. [強制] 初版 index 的所有 TASK 狀態與 `overall_status` 均為 `pending`，不建立 `latest_attempt`、`status_reason`、lock、audit 或其他 execution record。
6. [強制] create 部分失敗時不刪除或覆寫已完成內容；`task recover-create` 只有在相同 canonical TASK 已存在且 execution 目錄不存在、為空或只含完全相同初始 index 時可使用，且須先取得使用者授權。
7. [強制] Validator 只驗證 contract；create／recover-create 都不執行 CMD 或 OP，也不建立 Attempt、execution lock、instruction audit 或規格升版交易。
