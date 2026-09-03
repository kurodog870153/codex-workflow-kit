---
name: 任務執行
description: 依正式 TASK 執行前置檢查、授權操作、驗證、紀錄、狀態與恢復時使用；需求或技術決策不適用。
metadata:
  work-tags:
    - task-execution
---

# 任務執行指令

指令分類狀態：已完成
指令邊界：Execute 只控制已核准 TASK 的前置檢查、授權、執行、紀錄、狀態、鎖與恢復；不得建立技術決策、補完 TASK 或重複 Task instructions。

## 1. 載入與 Reference 路由

1. [強制] 只執行正式且已核准的 `TASK-*`；外部技能只依目標 TASK 的 `skill_id` 從來源 Plan 選取一個，`null` 表示 base-only。不得探索、推薦、加入、替換、組合或呼叫其他技能。
2. [強制] 建立、更新或解讀 index、Attempt、狀態、指紋與結果時，載入 `execute.general.execution-records`（`references/execution-records.md`）。
3. [強制] 發現既有鎖、進行中紀錄、中斷、部分寫入、承接或 Correction 時，載入 `execute.general.execution-recovery`（`references/execution-recovery.md`）。
4. [強制] Execute reference 只在需要其正文時載入模型；`EXECUTE-INSTRUCTIONS-SHA-256` 須由工具涵蓋目前觸發且實際適用的 reference，並在 reference 動態加入或移除時重算，即使正文不需載入模型也不得省略其位元組。
5. [強制] 正常執行只載入 TASK 基準、目標 TASK、其引用的 DECISION、index、適用最新 Attempt、承接鏈與 Correction；只有稽核、衝突或使用者要求時才載入完整歷史。

## 2. 唯讀前置檢查

1. [強制] 建立 Attempt 前須確認 TASK-SPEC、TASK SHA、Plan `hierarchy_selection_sha256` 與 `skill_selection_sha256`、index 與目標 TASK 的 hierarchy、`skill_id`、技能 snapshot／bundle、Execute mode、Work instruction fingerprints、TASK 狀態、相依、必要輸入、檔案現況、工作區差異、路徑邊界與既有鎖均符合正式 TASK。
2. [強制] TASK 的目標、檔案、步驟、CMD／OP、VAL、風險、版本、Swagger 或其他適用契約有缺漏、矛盾、占位符或需由 Execute 推論時，視為規格缺陷並停止，不得自行補足。
3. [強制] 目標 TASK 的 instruction sources 或內容與 index 該 TASK 的 `TASK-INSTRUCTIONS-SHA-256` 不同時，不得修改成果或未授權寫入紀錄；先唯讀回報預期值、觀察值、來源差異與影響，再取得使用者明確授權，才可只建立「已停止／指令已變更」Attempt、更新 index 為受阻並交回 Task 稽核。未授權時維持原狀。
4. [強制] 既有差異只有在路徑與內容都可由核准 TASK、有效 Attempt／Correction 或本次已知前序成果逐項解釋時才能繼續；可能屬於使用者或無法判定時停止。
5. [強制] 每次讀寫前須重新套用共用 instruction-loading 的完整需求成品路徑安全檢查；無法確認時不得取得鎖或寫入紀錄。
6. [強制] TASK 狀態只有待執行或待重新執行可建立新 Attempt；進行中只能續接 index 指向的唯一 Attempt，受阻須先確認阻礙解除，已完成或已取消不得執行。
7. [強制] 除有效續接／承接外，建立路徑已存在、修改路徑不存在、移動來源或目的不符、相依未完成、必要輸入不存在或命令入口不可用時，前置檢查失敗並停止，不得改變操作類型。
8. [強制] 一般前置檢查失敗只在對話回報，不建立 Attempt 或修改 index；只有第 3 項的指令已變更依 execution records 建立停止紀錄。
9. [強制] 工作區實作差異檢查只排除目前 execution 目錄，不得排除整個 `outputs/work/`；已核准 CMD 正常產生且不屬交付成果的暫存輸出不視為實作差異。
10. [強制] 續接中的 Attempt 所記 `EXECUTE-INSTRUCTIONS-SHA-256` 與目前適用值不同時，先唯讀列出來源差異、已執行內容及可能影響並取得授權；授權後依原鎖所存雜湊將舊 Attempt 結案為「已停止／指令已變更」、同步 index 並解除舊鎖，再確認影響。只有仍符合目前 TASK 與新 Execute instructions 的證據可在另行授權的新 Attempt 承接；不得在舊 Attempt 直接換用新指令或覆寫其雜湊。
11. [強制] 新 Attempt 的 TASK／index 身分、狀態、直接相依、Task instruction fingerprints、既有 lock、Execute instruction fingerprints、input 來源就緒及目標 TASK 的 create／modify／move 即時狀態須先通過 Work Python CLI 的唯讀 `execute preflight`；`user_provided`／`external` input 只在已有使用者或適用外部證據確認時以 `--confirmed-input <TASK-ID>/<INPUT-ID>` 傳入，不得傳入機密值。非零退出碼立即停止。
12. [強制] preflight 通過後須以相同參數執行唯讀 `execute worktree`；`target_task`、`completed_dependency`、`unrelated` 只表示路徑交集，不證明變更所有權。`review_status: required` 時須依第 4、9 項逐筆核對，任何無法解釋項目都停止。

## 3. 授權與開始

1. [強制] 第一次授權摘要須列出 TASK、目標、檔案、步驟、CMD、OP、VAL、風險，以及將建立 Attempt、更新 index 與記錄結果；摘要內不可分割內容可一次授權，不逐命令重問。
2. [強制] 未列入摘要的檔案、副作用、命令能力或外部操作不得執行；需要擴大範圍時停止並重新確認。
3. [強制] 授權後先取得指向下一個 Attempt 的單一寫入者協作鎖，再建立 Attempt 並同步 index 為進行中；任一步驟失敗時保留鎖，依 recovery reference 處理。
4. [強制] 鎖定、Attempt、成果寫入與 index 更新須維持可判定順序；不得依賴作業系統特定的原子鎖語意或自動過期時間。

## 4. 執行與局部修正

1. [強制] 只能修改 TASK 明列檔案並執行已授權的精確 CMD／OP／VAL。非等價命令替換須回到 Task 升版；只有同一工具、目標、範圍、副作用與通過判準完全不變，且差異只限 shell 語法、引用、跳脫、參數格式或執行檔路徑修正時，才是等價修正。等價修正仍須先取得使用者明確授權，並在同一 Attempt 執行前記錄原完整命令、實際完整命令、原因與授權證據。
2. [強制] 每個 CMD、OP 與 VAL 完成後立即依實際順序記錄最小結果；不得保存機密、完整外部回應或與驗收無關的大量輸出。
3. [強制] 除命令修正須依第 1 項處理外，錯誤原因已確定且修正與重驗完全位於已授權檔案、操作、副作用與驗證範圍內時，須在同一 Attempt 自行修正並以原 ID 加序號重驗。
4. [強制] 原因不確定、規格缺陷、需要新檔案／能力／外部操作、可能覆蓋使用者變更、安全或權限問題時立即停止，不得以重試擴大授權。
5. [強制] 每批不可分割修改後、下一個副作用前，須檢查限定差異，確認沒有非預期刪除、截斷、重排、編碼、換行或無關變更。
6. [預設] 外部操作失敗不自動重試；只有 TASK 已固定可重試條件且目前錯誤符合時才能依核准上限重試，結果不確定時先查證而非重送。
7. [強制] 修改既有文字檔須保留未變內容及 TASK 固定的編碼、BOM 與換行；新檔寫入前須確定同類慣例或由 TASK 固定格式，寫入後立即嚴格解碼並核對非 ASCII 內容。

## 5. 結案

1. [強制] 完成、停止或受阻時，先完成 Attempt 最終紀錄，再以一次 index 更新同步最新 Attempt、TASK 與整體狀態並解除鎖；index 同步失敗時保留鎖並進入恢復流程。
2. [強制] 回報須列出實際修改檔案、CMD／OP／VAL 結果、Attempt 與 TASK 狀態、部分成功或不確定結果、未解決事項及剩餘風險。
3. [強制] 已關閉 Attempt 不得修改；需要更正紀錄時使用獨立 Correction，不得改寫歷史或偽造未執行證據。
4. [強制] 規格缺陷結案後須依影響判定為不需修改規格、只修改 TASK 或修改 Plan 與 TASK；交接須列出實際需求編號、三個路徑、目標 TASK、Attempt／前置檢查資訊、已確認做法、受影響 ID 與驗證要求，不得保留占位符或由後續流程推測。
5. [強制] Attempt 已停止／受阻或前置檢查發現規格缺陷後，須詢問使用者後續方向並每次只確認一項；使用者明確不再處理時停止追問。只影響檔案、CMD、OP、VAL 或執行細節時交接 `$work task -- <完整需求>`，影響 Plan 目標、範圍、成果、驗收或決策時交接 `$work plan -- <完整需求>`；不需修改規格時只列理由、解除條件與下次 Execute 授權，不產生規格交接指令。
6. [強制] 規格交接使用 `execute_to_task` 或 `execute_to_plan`，須包含固定 marker、需求編號、實際路徑、TASK spec、目標 TASK、`skill_id`、`execute_skill_selection_sha256`、TASK 與 instructions SHA、Attempt／前置資訊、已確認做法、修改要求、保留範圍、受影響 ID 與驗證要求；不得保存 target hierarchy。
7. [強制] 交接置於單一 JSON 程式碼區塊且只存在於對話；Execute 不得修改 Plan／TASK 或建立規格鎖，完成對應 Plan／Task 流程前不得建立新 Attempt，交接本身也不授權任何寫入。
