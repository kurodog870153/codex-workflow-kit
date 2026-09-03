# Execution 恢復參考指令

參考名稱：execute.general.execution-recovery
適用層級：execute.general
指令分類狀態：已完成

## 1. 鎖與寫入防護

1. [強制] 執行鎖是單一寫入者的協作契約，不宣稱具作業系統原子性；鎖一旦建立就不自動過期，只能在對應操作完成同步或經重新授權的恢復流程解除。
2. [強制] 寫入前須只由已核准的本地限定差異，以目標編碼與換行產生完整暫存內容並計算雜湊，確認來源檔仍符合前置證據後才替換；不得重新格式化、重排或重組未變內容。寫入後立即嚴格解碼並檢查限定差異。
3. [強制] 多檔操作部分寫入、Attempt 或 index 同步中斷時，不得自動 rollback、刪除或另開 Attempt；保留現況、鎖與可確認證據，後續須重新摘要並取得恢復授權。
4. [強制] 新文字檔的編碼、BOM 與換行由 TASK 固定；未固定時唯讀沿用同專案同類慣例，無法確認且非 ASCII 或驗收會受影響時視為規格缺陷，不得依賴 shell 預設編碼。

## 2. 恢復與續接

1. [強制] 恢復前須唯讀核對鎖、TASK、兩個 instruction fingerprints、index、目標 Attempt／Correction、工作區與外部狀態；Attempt／Correction 的原始 Execute instruction baseline 以鎖保存的 `EXECUTE-INSTRUCTIONS-SHA-256` 為準。任何項目不能唯一解釋時保留鎖並交由使用者處理。
2. [強制] 鎖指向的 Attempt 不存在時，只有 ID 仍是下一號、TASK 可執行且沒有不明副作用，才能經授權補建同一 Attempt；不得另取 ID。
3. [強制] Attempt 進行中且目前 Execute instructions hash 未變時，只能續接已由紀錄與現況共同證明的同一 TASK；已完成且仍有效的 CMD／OP／VAL 可續接，證據不足者須重新執行。雜湊已變時不得直接續接，須依 Execute 通用指令取得授權、按鎖中原雜湊停止舊 Attempt、同步並解鎖、確認影響，再以新 Attempt 承接仍有效證據。
4. [強制] Attempt 已結案但 index 未同步時，不修改 Attempt；經授權只同步其最終狀態並解除對應鎖。
5. [強制] Attempt 內容為空或只是目前建立骨架的無衝突子集，且路徑、ID、TASK-SPEC、兩個 instruction fingerprints、承接、工作區及缺少值均可唯一確認，且沒有執行紀錄或不明副作用時，才能經授權保留正確值並補齊同一 Attempt；缺少開始時間使用恢復當下時間。補齊後重新核對骨架並保留鎖續接。
6. [強制] 既有內容衝突、存在多個進行中 Attempt、含無法解釋的紀錄或副作用，或缺少值不能唯一重建時，須列出鎖、路徑、已確認欄位、衝突與期望骨架，由使用者自行處理後再唯讀核對；不得修改、刪除、移動衝突紀錄或解除鎖。
7. [強制] index 指向唯一進行中 Attempt 但缺少執行鎖時，須先唯讀確認 TASK-SPEC、fingerprints 與工作區，再經授權補建對應鎖；補鎖前不得續接 CMD／OP／VAL。
8. [強制] 承接只能指向同一 TASK 最新已結案 Attempt，不得跳過或循環；複製其已核對累積修改檔案，只有紀錄與現況共同證明仍符合目前 TASK 且未被規格變更失效的 CMD／OP／VAL 才能免重做。
9. [強制] Attempt-start 中斷以固定 `.work-attempt-start-<TASK-ID>-<ATTEMPT-ID>-<stage>.tmp`、index lock 與 Attempt 是否存在判定階段；一般 preflight 發現暫存檔時停止。只有重新取得授權後，才能以原路徑、input IDs 及 `work-attempt-start-request/v1` 執行 `execute recover-attempt-start --stdin`。工具只安裝內容完全相符的預備 index、補建同一 canonical Attempt 或同步同一 index；任何衝突均保留現況並停止。
10. [強制] `.work-record-begin-*.tmp`、`.work-command-correction-*.tmp` 或 `.work-record-finish-*.tmp`、lock `record_id`／`command_correction` 與 Attempt record 不一致時，視為 record transaction 中斷；保留全部現況並停止。只有後續既有 recovery 任務可在重新授權後推進內容完全相符的預備檔或同步 lock；不得由 begin／command-correction／finish 指令自動恢復。
11. [強制] `.work-attempt-close-*.tmp` 存在、Attempt 已結案但 index 仍為進行中，或 index 已同步但 lock 未依序解除時，視為 attempt-close 中斷；保留 closed Attempt、index、lock 與全部 temporary files。只有後續既有 recovery 任務可在重新授權後驗證完全相符的最終狀態並推進 index；attempt-close 不得自動恢復、重寫已關閉 Attempt 或再次解鎖。
12. [強制] 一般交易恢復須在使用者另行核准當下完整狀態後，以 `work-execution-recovery-request/v1` 純 JSON呼叫 `execute recover --stdin`，固定傳入 `record_begin|command_correction|record_finish|attempt_close|correction`、目標 Attempt ID，以及排序後完整 `.work-*.tmp` 檔名清單；檔案集合在執行前有任何改變即停止。Attempt-start 只能使用既有 `recover-attempt-start`。
13. [強制] `execute recover` 重新驗證正式 TASK identity、canonical Attempt、index、原 lock fingerprint、transaction identity 與所有 prepared bytes，只能依原交易順序推進唯一 canonical target。只有 record-finish／attempt-close 的 Attempt 已寫入且 lock 足以唯一重建 index 時可接受空檔案清單；其他缺檔、內容衝突、多重交易或模糊狀態均保留現況停止。
14. [強制] Recovery 需要補建下一階段 temporary file 時，須先 exclusive prepare 並核對 bytes，再依序 atomic replacement；失敗時保留全部來源、lock 與 temporary files 並再次回傳 `recovery_required`。不得自動重試、rollback、刪除、覆寫已關閉 Attempt、處理未知交易或手動解鎖。

## 3. Correction

1. [強制] Correction 只更正已關閉紀錄或 index 的錯誤事實，不修改 TASK、實作或原 Attempt；每次使用下一個 `<ATTEMPT-ID>-CORRECTION-nnn` 並取得獨立授權與保存原始 `EXECUTE-INSTRUCTIONS-SHA-256` 的 Correction 執行鎖。
2. [強制] Correction 只以 canonical `work-correction/v1` 英文欄位記錄建立時間、目標、原 Attempt 的 Task／Execute instruction fingerprints、欄位、正確值、原因與必要證據，寫入後不可修改；`invalidates_completion` 與已計算 affected TASK IDs 只保存在交易 lock，完成同步後才解除鎖。
3. [強制] Correction 中斷時以 `transaction: correction`、同一目標 Attempt、完整排序 `.work-*.tmp` 清單呼叫 `execute recover --stdin`；工具只接受同一 Correction ID、canonical artifact、原 fingerprints、lock affected plan 與最終 index bytes 全部相符的狀態。內容衝突或無法唯一確認時不得覆寫、刪除或解除鎖。
4. [強制] 鎖指向的 Correction 不存在時，只有原核准 canonical artifact temporary file、內容與 ID 仍唯一有效才能建立；已存在且 bytes 完全相符時不得重寫，只同步 index。缺少 artifact、lock plan 或最終 index 任一 prepared bytes 時停止，不由模型補值。
5. [強制] Correction 使完成狀態失效時，目標與已完成下游改為待重新執行；只有 TASK 狀態與最新 Attempt 結果不一致時才記狀態差異原因。已取消 TASK 維持已取消，不得因 Correction 重新啟用。
