# Execution 紀錄參考指令

參考名稱：execute.general.execution-records
適用層級：execute.general
指令分類狀態：已完成

## 1. 路徑、TASK 與指紋

1. [強制] Execute 只使用正式 TASK 或完整交接所確定的需求編號與 Plan、TASK、execution 三路徑，並於每次讀寫前套用共用 instruction-loading 的完整需求成品路徑安全檢查；不得由單一路徑推測、替換或重新決定。
2. [強制] TASK SHA-256 由工具直接讀取 TASK 位元組，完全依共用 instruction-loading 的 canonical TASK fingerprint 計算；不得為計算而把全文載入模型或另訂正規化方式。
3. [強制] Execute 依目標 TASK 的正式 `instruction_selection` 與共用 canonical instruction fingerprint 重新計算該 TASK 的 `TASK-INSTRUCTIONS-SHA-256`，第一個來源固定為共用 instruction-loading，並另核對 index 的文件層聯集值；無關 TASK 的來源變更不得阻擋目標 TASK。
4. [強制] `EXECUTE-INSTRUCTIONS-SHA-256` 只涵蓋 Work Execute instructions 與適用 references；外部技能另以目標 TASK 的 `skill_id` 與 `execute_skill_selection_sha256` 固定身分，不得混入 Work hierarchy fingerprint。
5. [強制] 來源只在 raw-byte duplicate 比對確認時省略；不得依字典排序、正規化內容相等或父 instructions 相等去重。
6. [強制] 建立新 Attempt 前使用 Work Python CLI 的 `execute preflight` 重新驗證 canonical TASK、canonical index、指定 TASK、直接相依、Task instruction fingerprints、既有 lock、目前 Execute instruction fingerprints、input 來源及指定 TASK 的檔案生命週期；`user_provided`／`external` input 使用已確認的完整 INPUT ID，不傳入值。成功結果固定使用 `work-execute-preflight/v1` 與 `eligibility: passed`，且不寫入任何資料。
7. [強制] preflight 後使用相同輸入執行 `execute worktree`，以 Git porcelain v1 `-z` 唯讀列出 staged、unstaged、untracked，且只排除目前 execution 目錄。`work-execute-worktree/v1` 的路徑分類只供核對，不是所有權證據；任何無法由正式紀錄與已知成果解釋的差異都不得建立 Attempt。

## 2. Index

1. [強制] index 固定位於 execution 目錄的 `index.md`，保存 TASK spec、TASK SHA、Plan `hierarchy_selection_sha256` 與 `skill_selection_sha256`、文件層 Task instructions SHA、選用 audit／lock、整體狀態及 TASK rows；不保存技能全文。
2. [強制] TASK row 固定包含 TASK ID、狀態、`skill_id` 與該 TASK 的 instructions SHA；最新 Attempt、Correction 或狀態原因只在存在時加入。
3. [強制] TASK 狀態只使用「待執行」、「進行中」、「待重新執行」、「受阻」、「已完成」及「已取消」；全部取消時整體為已取消，否則忽略已取消 TASK 後精確判定待執行、已完成、受阻或進行中。
4. [強制] 初始 Attempt execution lock 使用 `kind: execution`、`task_id`、`attempt_id`、`execute_instructions_sha256`；開始執行 CMD／OP／VAL 前才由後續紀錄交易加入 `record_id`。規格鎖與執行鎖互斥，任一執行鎖存在時不得建立其他 Attempt、Correction 或規格鎖。恢復與結案必須使用鎖所存原始 Execute 雜湊解讀該操作。
5. [強制] 整體狀態算法固定為：全部 TASK 已取消時為已取消；否則忽略已取消 TASK，全部待執行時為待執行，全部必要 TASK 已完成時為已完成，沒有進行中或可執行 TASK 且未完成項目均受直接或相依阻礙時為受阻，其餘為進行中。已取消 TASK 不視為完成或驗收證據。
6. [強制] 規格鎖存在時不得執行，Execute 不得解除規格鎖；Attempt 鎖從建立 Attempt 前持續至結案紀錄與 index 同步完成，Correction 鎖持續至 Correction、TASK、下游及 index 全部同步完成。

```markdown
# Execution

TASK-SPEC：TASK-SPEC-001
TASK-SHA-256：<task-sha>
TASK-INSTRUCTIONS-SHA-256：<task-instructions-sha>
整體狀態：待執行

TASK-001：待執行；TASK-INSTRUCTIONS-SHA-256：<task-001-instructions-sha>
```

## 3. Attempt 建立與內容

1. [強制] Attempt 位於 `<execution-dir>/<TASK-ID>/<ATTEMPT-ID>.md`，每個 TASK 由 `ATTEMPT-001` 遞增；同一 TASK 只能有一個進行中 Attempt，鎖寫入成功後才能建立檔案。
2. [強制] 新 Attempt 使用 canonical `work-attempt/v1`，依序保存 schema、Attempt ID、TASK spec、TASK ID、`skill_id`、狀態、TASK SHA、Task instructions SHA、Execute instructions SHA、`hierarchy_selection_sha256`、`execute_skill_selection_sha256`、開始時間、選用承接資料及 records。
3. [強制] 執行紀錄依實際順序追加；同一 ID 首次使用原 ID，重複執行才依序使用 `#1`、`#2`。CMD 記退出碼與一行關鍵結果或最小錯誤，OP 記成功／失敗及必要外部狀態且不保存完整回應，VAL 記通過／失敗與最小證據或足夠的前項 ID。
4. [強制] 有檔案修改或承接成果時維護「本 Attempt 累積修改檔案」，保存有效承接與目前 Attempt 的路徑聯集，不保存 diff 或檔案雜湊。
5. [強制] 結案加入結束時間、最終狀態、適用類型與具體原因；時間使用含偏移的 `YYYY-MM-DDTHH:mm±HH:mm`，結案後內容不可修改。
6. [強制] TASK-SPEC 或 TASK SHA 已變更時先依原基準結案舊 Attempt，再依新規格建立下一 Attempt；外部阻礙解除後可在新授權中直接建立 Attempt 並設為進行中，不先單獨改為待重新執行。

```json
{
  "schema": "work-attempt/v1",
  "attempt_id": "ATTEMPT-001",
  "task_spec_id": "TASK-SPEC-001",
  "task_id": "TASK-001",
  "skill_id": null,
  "status": "in_progress",
  "task_sha256": "<task-sha>",
  "task_instructions_sha256": "<task-instructions-sha>",
  "execute_instructions_sha256": "<execute-instructions-sha>",
  "hierarchy_selection_sha256": "<hierarchy-selection-sha>",
  "execute_skill_selection_sha256": "<execute-skill-selection-sha>",
  "started_at": "<YYYY-MM-DDTHH:mm±HH:mm>",
  "records": []
}
```

## 4. 狀態與結果

1. [強制] `status` 只使用 `in_progress`、`completed`、`stopped` 與 `blocked`。`stopped` 的 `final_type` 只使用 `specification_defect`、`instructions_changed`、`validation_failed`、`unexpected_change`、`external_operation_failed`、`user_stopped`、`other`；`blocked` 只使用 `environment`、`external_service`、`permission`、`required_input`、`other`。
2. [強制] Attempt 對 TASK 的映射為：進行中→進行中、已完成→已完成、一般已停止→待重新執行、規格缺陷／Task instructions 變更／外部操作失敗→受阻、受阻→受阻。進行中 Attempt 因 Execute instructions 變更而停止時，影響尚未確認前為受阻；確認仍可依目前 TASK 重新執行後轉為待重新執行，並只能在新授權的新 Attempt 使用新指令。
3. [強制] 外部或多步結果固定記 `整體結果：完整成功／部分成功／失敗／結果不確定`，並依實況加入 `已生效`、`未生效`、`未知`；部分成功與結果不確定不得標示 Attempt 已完成。每個 OP 的成功、失敗或未知須分別映射到上述三類明細，且整體結果不得與 OP 紀錄矛盾。
4. [強制] 紀錄驗證依實際欄位是否存在判定適用 schema；既有已關閉 Attempt 缺少後來新增欄位仍是有效歷史且不得回寫，新 Attempt 使用目前完整 schema，index 在觸及時延遲補齊。
5. [強制] 目前 instructions 與 fingerprints 只約束未來的新 Attempt 或續接判定；已關閉歷史中當時有效的證據不因指令更新失效，除非另有已確認決策要求重審。
6. [強制] 完成只依實際通過的 VAL 與有效承接判定；未執行、退出碼未知、只編譯未驗收、零測試或主觀推測不得記為通過。
7. [強制] 「已取消」只由 Plan／Task 流程設定；Execute 不得將 TASK 設為、移出或重新啟用已取消。所有未取消必要 TASK 完成時代表 Plan 驗收已有 Attempt 證據，不重讀全部 Attempt 或建立 `completion.md`；全部取消只代表目前無需執行，不代表驗收已完成。

## 5. 固定紀錄格式

1. [強制] Index 的 TASK 行依 TASK 文件順序排列，固定為 `TASK-001：<狀態>；TASK-INSTRUCTIONS-SHA-256：<hash>；最新 Attempt：<ATTEMPT-ID>；最新 Correction：<CORRECTION-ID>；阻礙：<ID>；狀態差異原因：<ID>`，選用欄位只在存在時依此順序加入，不輸出空值或占位符。
2. [強制] `records` 依序保存 discriminated object：CMD 使用 `id`、`kind: command`、選用 `correction`、整數 `exit_code`、單行 `result`；OP 使用 `id`、`kind: operation`、`outcome: success|failure|unknown`、最小 `state`；VAL 使用 `id`、`kind: validation`、`outcome: passed|failed`、最小 `evidence`。累積修改檔案使用 normalized project-relative `modified_files`，不保存 diff 或雜湊。CMD `correction` 固定保存 canonical `original_command`、`actual_command`、`reason` 與 `authorization_evidence`，兩個命令須維持相同 `argv|shell` mode 且不得相同。
3. [強制] `completed` 加入 `ended_at`；`stopped` 或 `blocked` 依序加入 `final_type`、`reason`、`ended_at`。有 OP 的已關閉 Attempt 必須加入 `overall_result`，其 `effective`、`not_effective`、`unknown` 精確列出對應 OP ID；`partial_success`、`failure`、`uncertain_result` 不得搭配 `completed`。
4. [強制] 承接 Attempt 使用 `continued_from`，選用 `carried_records` 依原 TASK ID 順序保存 `source_attempt_id`、`record_id` 與目前仍有效的最小 `evidence`；未承接的 ID 省略，不得以摘要取代來源、ID 或有效性證據。重跑已承接 ID 時接續 `#1`、`#2` 序號。
5. [強制] 在任何 Attempt 寫入前，純 JSON 必須通過 Work Python CLI `attempt validate --stdin`；`attempt render --stdin` 回傳 canonical 欄位順序，既有檔案使用 `attempt validate --path <attempt-path>` 驗證 H1、檔名、TASK 父目錄與 canonical bytes。這些指令唯讀，不建立 Attempt、lock 或 index 狀態。
6. [強制] Correction 使用 canonical `work-correction/v1` fenced JSON，固定使用下列英文欄位與順序，沒有額外摘要或同義欄位；`correction validate` 驗證純 JSON 或既有檔案，`correction render` 只回傳 canonical JSON：

````markdown
# ATTEMPT-001-CORRECTION-001

```json
{
  "schema": "work-correction/v1",
  "correction_id": "ATTEMPT-001-CORRECTION-001",
  "created_at": "YYYY-MM-DDTHH:mm±HH:mm",
  "target_attempt_id": "ATTEMPT-001",
  "task_instructions_sha256": "<task-instructions-sha>",
  "execute_instructions_sha256": "<execute-instructions-sha>",
  "field": "<corrected-field>",
  "correct_value": "<value>",
  "reason": "<reason-and-required-evidence>"
}
```
````

7. [強制] Correction 建立使用 `work-correction-create-request/v1` 純 JSON及 `execute correction-create --stdin`；`invalidates_completion` 是必填 boolean 交易判定，不寫入 immutable Correction。Python 自動推導下一個 ID、目前含偏移分鐘時間與原 Attempt fingerprints，依序 atomic replacement Correction lock、exclusive canonical Correction 與最終 index；需要失效完成狀態時只把目標及已完成下游改為 `pending_retry`，已取消 TASK 不變。

## 6. Attempt start transaction

1. [強制] `execute worktree` 以未分類 Git 狀態與 execution 目錄排除指令產生 `work-execute-worktree-snapshot/v1` SHA-256。使用者須核對完整 worktree 結果；hash 只綁定核對狀態，不取代明確授權。
2. [強制] Attempt 建立使用 `work-attempt-start-request/v1` 純 JSON及 `execute attempt-start --stdin`。Python 自動推導下一個 Attempt ID、目前含偏移分鐘時間、TASK 身分與三個 fingerprints；重新驗證 preflight 與 snapshot 後，依序安裝 execution lock、exclusive create canonical Attempt、同步 TASK 為 `in_progress` 與 `latest_attempt`，全程保留 lock，且不執行 CMD／OP／VAL。
3. [強制] `pending_retry` request 必須提供最新來源 Attempt 與逐筆承接 record ID／目前有效證據；`pending` 不得提供 continuation。來源必須是同一 TASK 最新已關閉 Attempt，承接 ID 必須存在，並複製其累積修改檔案。
4. [強制] start 中斷時保留 lock、Attempt 與固定 transaction temporary file，回傳 `recovery_required` 及精確階段；不得自動 rollback、刪除、解鎖或續作。
5. [強制] 每個正式 CMD／OP／VAL 執行前，必須以明確授權的 base ID 呼叫 `execute record-begin`。Python 驗證目前 TASK、Attempt、index lock 與 Task／Execute fingerprints，依承接及既有 records 推導原 ID 或下一個 `#n`，再以 atomic index replacement 將實際 `record_id` 加入 lock；成功固定回傳 `work-record-begin/v1` 與 `lock_status: record_reserved`，且不執行或追加該 record。
6. [強制] record-begin 發現既有 `record_id`、指令變更、非正式 ID 或 transaction conflict 時停止。寫入中斷保留原 lock 與 `.work-record-begin-*.tmp`，回傳 `recovery_required`；不得自動重試、執行 record、刪除暫存檔或解除 lock。
7. [強制] 已保留 CMD 需要等價修正時，執行前以 `work-command-correction-request/v1` 純 JSON呼叫 `execute command-correction --stdin`，保存 lock 完全一致的 `record_id`、與正式 TASK 完全相符的 `original_command`、使用者核准的 `actual_command`、原因及最小授權證據。工具只 atomic replacement index 保存 `command_correction`，不執行命令或判斷語意等價；同一 record 只能保存一次。
8. [強制] command-correction 成功須回傳 `work-command-correction/v1`、`correction_status: recorded` 及 `lock_status: record_reserved` 後才能執行實際命令。中斷時保留 lock 與 `.work-command-correction-*.tmp` 並回傳 `recovery_required`；不得執行、重送、刪除、rollback 或解鎖。
9. [強制] 已保留 record 完成後，使用 `work-record-finish-request/v1` 純 JSON呼叫 `execute record-finish --stdin`。`record.id` 必須與 lock 完全一致；command 使用 `exit_code`／`result` 且不得由 request 傳入 correction，operation 使用 `outcome`／`state`，validation 使用 `outcome`／`evidence`，有檔案變更時另傳 normalized `modified_files`。
10. [強制] record-finish 先將 lock 的 command correction 併入 CMD，再 atomic replacement canonical Attempt，依序追加 record、更新累積修改檔案與 OP overall result；再 atomic replacement index，移除 lock 的 `record_id` 與 `command_correction`，保留 Attempt lock。工具不執行 record。任一步驟中斷時保留已寫入內容、lock 與 `.work-record-finish-*.tmp` 並回傳 `recovery_required`；不得重送結果、開始其他 record、rollback、刪除或解鎖。
11. [強制] Attempt 結案使用 `work-attempt-close-request/v1` 純 JSON呼叫 `execute attempt-close --stdin`。`completed` 不傳 final details 且每個正式 VAL 的最新 current／carried 結果必須通過；`stopped`／`blocked` 必須傳入其允許的 `final_type` 與具體 `reason`。結束時間由 Python 產生，不接受 request 指定。
12. [強制] attempt-close 必須確認沒有 `record_id` 或 `command_correction` 保留，依序 atomic replacement canonical closed Attempt，再 atomic replacement index 同步 TASK／整體狀態並移除 execution lock。`completed` 映射已完成；一般 `stopped` 映射待重新執行，規格缺陷／指令變更／外部操作失敗映射受阻；`blocked` 映射受阻。工具不執行 record。
13. [強制] attempt-close 任一步驟中斷時保留已寫入 Attempt、原 lock 與 `.work-attempt-close-*.tmp` 並回傳 `recovery_required`；不得重送、rollback、刪除或手動解鎖。只有 Attempt 與 index 完全同步時回傳 `work-attempt-close/v1` 及 `lock_status: released`。
