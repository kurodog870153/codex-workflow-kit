---
name: Execution 恢復參考指令
description: 處理執行鎖、部分寫入與恢復授權時使用；正常執行流程不適用。
reference-name: execute.general.execution-recovery
metadata:
  work-tags:
    - execution-recovery
---

# Execution 恢復參考指令

## 1. 鎖與寫入防護

1. [強制] index 的持久業務 lock 保存 Attempt／record／Correction 的協作狀態；它不自動過期，只有對應操作完成同步或另行授權的恢復流程才能解除。程序排他 owner 另依第 5 點處理。
2. [強制] 寫入前須只由已核准的本地限定差異，以目標編碼與換行產生完整暫存內容並計算雜湊，確認來源檔仍符合前置證據後才替換；不得重新格式化、重排或重組未變內容。寫入後立即嚴格解碼並檢查限定差異。
3. [強制] 多檔操作部分發布、Attempt 或 index 同步中斷時，不得自動 rollback、刪除或另開 Attempt；保留現況、業務 lock 與 staging 證據，後續須重新摘要並取得恢復授權。受控退出只釋放本次程序 owner。
4. [強制] 新文字檔的編碼、BOM 與換行由 TASK 固定；未固定時唯讀沿用同專案同類慣例，無法確認且非 ASCII 或驗收會受影響時視為規格缺陷，不得依賴 shell 預設編碼。
5. [強制] Work CLI 的程序排他鎖位於 `outputs/work/runtime/locks/<requirement-id>/execution.lock`，由已驗證 TASK 需求與 canonical project root 決定，自訂 Execution 路徑不改變需求鎖身分。排他建立保存唯一 owner；成功與受控錯誤都顯式釋放並只刪除匹配 owner 的鎖檔，不刪業務 index lock、staging 或正式證據。kill／abort、owner 不明與舊 writer lock 殘留保持阻擋，須先停止所有 writer 並完成獨立離線審查；不得根據 age、timeout、PID 或舊 OS 鎖未持有而回收。此程序鎖與前述持久 Execution 業務鎖各自保留其生命週期。

## 2. 恢復與續接

1. [強制] 恢復前須唯讀核對鎖、Formal TASK collection、固定 Source 原 bytes 及 binding、兩個 instruction fingerprints、Execution index、目標 Attempt／Correction、工作區與外部狀態；Attempt／Correction 的原始 Execute instruction baseline 以鎖保存的 `EXECUTE-INSTRUCTIONS-SHA-256` 為準。任何項目不能唯一解釋時保留鎖並交由使用者處理。
2. [強制] 鎖指向的 Attempt 不存在時，只有 ID 仍是下一號、TASK 可執行且沒有不明副作用，才能經授權補建同一 Attempt；不得另取 ID。
3. [強制] Attempt 進行中且目前 Execute instructions hash 未變時，只能續接已由紀錄與現況共同證明的同一 TASK；已完成且仍有效的 CMD／OP／VAL 可續接，證據不足者須重新執行。雜湊已變時不得直接續接，須依 Execute 通用指令取得授權、按鎖中原雜湊停止舊 Attempt、同步並解鎖、確認影響，再以新 Attempt 承接仍有效證據。
4. [強制] Attempt 已結案但 index 未同步時，不修改 Attempt；經授權只同步其最終狀態並解除對應鎖。
5. [強制] Attempt-start 的缺檔或部分新檔，只有在同一完整交易 manifest、原核准 request、固定 before／after bytes 與唯一發布順序皆可驗證時，才能另行授權補齊同一 Attempt；使用 manifest 保存的開始時間，不以恢復當下時間或模型推測補值。缺少 manifest、衝突或未知內容時保留現況停止。
6. [強制] 既有內容衝突、存在多個進行中 Attempt、含無法解釋的紀錄或副作用，或缺少值不能唯一重建時，須列出鎖、路徑、已確認欄位、衝突與期望骨架，由使用者自行處理後再唯讀核對；不得修改、刪除、移動衝突紀錄或解除鎖。
7. [強制] index 指向唯一進行中 Attempt 但缺少執行鎖時，須先唯讀確認 TASK-SPEC、fingerprints 與工作區，再經授權補建對應鎖；補鎖前不得續接 CMD／OP／VAL。
8. [強制] 承接只能指向同一 TASK 最新已結案 Attempt，不得跳過或循環；複製其已核對累積修改檔案，只有紀錄與現況共同證明仍符合目前 TASK 且未被規格變更失效的 CMD／OP／VAL 才能免重做。
9. [強制] 七類 Execution 交易位於 `outputs/work/runtime/staging/<requirement-id>/<operation>/<full-transaction-sha256>/`，完整 `transaction.json` 固定綁定 canonical root、需求、Execution 路徑、核准、業務 IDs、before／after bytes、payload inventory 與發布進度。Attempt-start 中斷時須先審查此完整證據，再以原 `work-attempt-start-request`、明確 TASK／Execution 路徑及 ID 執行 `execute recover-attempt-start --input-file "<request-path>"`；只推進同一交易的唯一 canonical targets，不執行 CMD／OP／VAL。
10. [強制] `record-begin`、`command-correction`、`record-finish` 或 `deviation-record` staging 存在時，保留完整 manifest、payload 與正式狀態並停止一般寫入；不得由正常指令自動恢復。lock 的 `record_id`／`command_correction`、Attempt records／deviations 與 frozen targets 必須一起核對。
11. [強制] `attempt-close` staging 存在、Attempt 已結案但 index 未同步，或正式 targets 完成但清理中斷時，保留 closed Attempt、業務 lock 與剩餘 runtime 證據；另行授權後只依固定 before→after 順序補齊同步及清理，不重寫已完成的關閉結果。
12. [強制] 一般交易恢復須先由 `execute recovery-prepare` 產生 `work-execution-recovery-request`，審查後以原 `data.request` 純 JSON 呼叫 `execute recover --input-file "<request-path>"`。交易類別為 `record_begin|command_correction|record_finish|deviation_record|attempt_close|correction`；請求必備目標 Attempt、專案相對 `transaction_dir`、目錄內排序且完整的實際 `transaction_files` 及 `transaction_evidence_sha256`。清單包含 `transaction.json` 與當下存在的 control tmp／payload；缺檔或可驗證 partial 的狀態亦納入 evidence digest。舊 `.work-*.tmp` 請求不接受，不從 Execution basename 猜需求或拼回舊路徑。
13. [強制] `execute recover` 重新核對當下 inventory／SHA-size digest、正式 TASK／Source、canonical artifacts、原 lock、完整 manifest 與唯一領域 target，再依原交易順序推進；失敗結果仍須新的授權證據。正式檔僅接受固定 before／after graph，部分新 artifact／prepared payload 僅可在完整 frozen proof 下補齊相符前綴；未知正式狀態、foreign entry、缺 manifest、混合交易或核准後 evidence 變更時一律拒絕且保留 bytes。
14. [強制] 進度使用同目錄 `transaction.json.tmp`；僅明確 recover 可在完整 proof 與實際正式 graph 相符時補齊其單調下一狀態的前綴。全部正式 targets 嚴格讀回後才進入 published_verified／cleaning，先刪除本交易列明的 runtime payload，manifest 最後移除；清理失敗保留可重入的完整身分證據。受控錯誤回傳 `recovery_required`、交易目錄與可確認進度，只釋放匹配的程序 owner；不得碰正式收據／history、未知 entry 或用整包／glob 清理。
15. [強制] `execute recovery-prepare --input-file "<preparation-path>"` 為唯讀請求組裝；輸入 `work-execution-recovery-prepare-request`、已確認的 transaction／attempt_id，沿用明確 TASK／Execution 路徑、TASK ID 與 roots。它盤點當下完整 staging、正式來源與 SHA-size，生成含 evidence digest 的 request，不寫入、不取得恢復授權，也不代替 recover 的唯一領域 target 驗證。任一 evidence bytes／size／集合／missing／partial 狀態改變後，原 request 失效，須重新 preview 並核准；外部副作用仍須由使用者核對。

## 3. Correction

1. [強制] Correction 只更正已關閉紀錄或 index 的錯誤事實，不修改 TASK、實作或原 Attempt；每次使用下一個 `<ATTEMPT-ID>-CORRECTION-nnn` 並取得獨立授權與保存原始 `EXECUTE-INSTRUCTIONS-SHA-256` 的 Correction 執行鎖。
2. [強制] Correction 以 canonical `work-correction` 保存建立時間、目標、原 Attempt 的 Task／Execute fingerprints、欄位、正確值及原因，正式 artifact 寫入後不可修改；`invalidates_completion` 與 affected TASK IDs 保存於業務 lock 和交易 manifest 的固定證據，不加入 immutable Correction。
3. [強制] Correction 中斷時使用 recovery-prepare 的 `transaction: correction` request，保留同一目標 Attempt、完整 `transaction_dir`／實際 inventory／evidence digest，再另行核准 recover；Correction ID、canonical artifact、原 fingerprints、lock affected plan 與最終 index bytes 必須完全相符，未知或衝突內容不得覆寫、刪除或解鎖。
4. [強制] 鎖指向的 Correction 尚未發布時，只有完整 manifest 保存的原核准 canonical artifact、ID、lock／final index targets 仍唯一有效才可建立；既有 artifact 完全相符時只完成後續 index／cleanup，不重寫 immutable Correction。缺少完整 manifest 或 frozen bytes 時停止，不由模型補值。
5. [強制] Correction 使完成狀態失效時，目標與已完成下游改為待重新執行並重設其 affected acceptance results；原 Attempt／Correction records 保留且不回寫；只有 TASK 狀態與最新 Attempt 結果不一致時才記狀態差異原因。已取消 TASK 維持已取消，不得因 Correction 重新啟用。
