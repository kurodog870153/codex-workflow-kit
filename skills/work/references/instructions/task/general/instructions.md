---
name: 任務規劃
description: 將已確認需求轉成技術決策、檔案、步驟、命令、操作與驗證時使用；實際執行不適用。
metadata:
  work-tags:
    - task-specification
---

# 任務規劃指令

指令邊界：Task 從固定 Source Snapshot 完成需求與技術規劃，是正式 TASK collection 之成果、驗收、技術決策、檔案、步驟、命令、操作與驗證的唯一權威來源；Execute 不得補充或推論缺漏。

## 1. 固定來源與所有權

1. [強制] 需求來源可為 host 已完整取得的 GitHub Issue、檔案原始 bytes 或使用者原樣 UTF-8 文字；使用已驗證的 Source Snapshot，核心不得自行刷新外部來源或解析 PDF 代替原始檔案。
2. [強制] 一份正常 TASK 集合綁定一份 Snapshot；來源的身分、原始內容、hash 與 size 必須完整且可驗證。重新擷取只建立新版本，不改寫舊 Snapshot；來源替換須列出對成果、技術決策、TASK 邊界、驗收及 selection 的影響並取得確認。
3. [強制] 正式 TASK、index、狀態、欄位或指令來源一律載入 `task.general.task-records`（`references/task-records.md`）；出現 `external_state` OP 時載入 `task.general.external-operations`（`references/external-operations.md`）。Reference 只在觸發條件成立時載入，正式 TASK 只保存 globally unique logical name，不保存絕對路徑。
4. [強制] Task 依 repository evidence 自行完成 hierarchy、skill discovery／selection 與 instruction selection；只使用使用者已確認且未漂移的選擇。每個 TASK 綁定一個 `skill_id`，不需要外部技能時使用 `null`，同一 TASK 不得組合多個外部技能。
5. [強制] 初版規劃與建立由 Task 負責；既有正式 TASK 修訂交由 `$work revise`，無法驗證的舊成品依 migration 流程處理，不在 planning 直接發布。原始 migration evidence 可以沒有有效 Snapshot，但不得當作正常 TASK 的來源。

## 2. 成果、證據與確認

1. [強制] 先確認目標、範圍、限制、交付成果、輸入、相容性及可觀察驗收；每項成果至少對應一項驗收。只納入完成需求所需的最小範圍，候選改善不得在未確認前併入。
2. [強制] 資訊不足、來源與 repository evidence 衝突或正確結果無法唯一判定時逐項提問，不得自行補足需求。每次只確認一項決策，同時展示相關證據、影響及有編號的選項。
3. [強制] 技術選型、版本、檔案、副作用、風險與驗證依實際專案證據決定；使用者提供的細節視為待核對輸入或已確認限制，不擴充其周邊設計。影響正式成果的判定須展示證據並取得確認。
4. [強制] 新決策可能影響已確認內容時，先列出受影響的成果、驗收、references、selection 與副作用，再逐項重新確認；沒有影響也須明示。每次回答後更新目前目標、範圍、限制、成果、驗收與未決事項。
5. [預設] 唯讀探索可依同一決策所需範圍批次進行；同一對話中未改變的證據可重用，新對話須先核對保存來源與目前專案狀態。依賴、風險與時程只在適用時加入。

## 3. 最小 TASK 與正式化

1. [強制] 每個 `TASK-*` 產生一個可獨立驗收的單一成果；可分割成果須拆分，不可分割多檔修改視為同一完整變更集。相依順序須讓每項任務具備所需輸入及可執行的前置成果。
2. [強制] Execute 所需的全部檔案、有序步驟、`CMD-*`、`OP-*`、`VAL-*`、失敗處理與授權邊界必須在 Task 確定；同一路徑由多個 TASK 處理時明列相依順序，檔案生命週期符合目前專案狀態。
3. [強制] 目前正式規格由單一 `work-task-index` 純 JSON index 與每個 TASK 的 `work-task-item` 純 JSON item 組成，固定使用 `status: confirmed` 與 `TASK-SPEC-nnn`；item 位於 index 旁的 `tasks/TASK-NNN.json`，index 不重複 item 內容。
4. [強制] keys、enums、IDs、statuses、paths、references 及 hashes 使用英文，語意字串可使用繁體中文。必要欄位、optional fields、nested contract references、canonical key order 與引用規則以 Work CLI registry／validator 為準，不另訂同義欄位或複製完整 JSON 結構。
5. [強制] 每個 TASK 保存單一 `goal`、技能與 instruction 選擇、traceability、有序 `steps` 及至少一個 `VAL-*`；驗收由 TASK 自有的穩定識別與 coverage 表達，完整覆蓋固定來源，不依賴另一份需求成品的 ordinal。沒有內容的選用欄位省略。
6. [強制] TASK 的 hierarchy 路徑須為已確認 Task selection 的有效子集，非空路徑完整存在於 Task 與 Execute catalog；instruction sources 只保存 `kind`、`logical_name`、`canonical_sha256`，不保存來源 layer 或絕對路徑。
7. [強制] Source、正式 TASK 入口與 Execution 預設位於 `outputs/work/sources/<requirement-id>/`、`outputs/work/tasks/<requirement-id>/index.json`、`outputs/work/executions/<requirement-id>/`。非預設路徑須同時確認三個專案相對位置與同一需求編號；每次讀寫完整套用共用路徑安全檢查。
8. [強制] 需求編號使用合法且可攜的識別；沿用使用者目前輸入或正式交接明列的值，不由檔名或其他對話推測。首次 Source capture 前缺少編號時，先詢問使用者提供並確認，再進行 capture 預覽與核准；不得延到完整候選後才確認。進度保存或編號確認不代表 capture 或正式核准。
9. [強制] 每個需求的所有 TASK 共用單一 `work-discussion-session`，保存在 `outputs/work/discussions/<requirement-id>/session.json` 與不可變 history；先確認範圍保存授權，再於每輪讀取已提交狀態。問題映射須先保存並驗證後才提問；保存、決策確認及規劃完成均不是正式核准，不得交給 Execute。
10. [強制] 所有必要決策完成後以 `task preview` 驗證並展示完整初版候選；正式核准綁定 `approval_sha256`，再以 `task apply` 一次發布完整集合，不由 AI 手動寫入正式 JSON。

## 4. 驗證與交接

1. [強制] validator 核對 Snapshot 完整性、需求 coverage、Task-owned hierarchy／skill／instruction selection、技能 bundle、三個 artifact paths、每 TASK 的單一技能、Work references、DAG、ID、檔案生命週期與驗收 coverage；核准後來源漂移仍須拒絕寫入。
2. [強制] 純 JSON request 依公開契約讀取，正式 path 文件由共用 renderer 產生。Task 不執行任意 CMD 或 OP，須在對話展示命令入口、語法、專案現況及驗證分類的唯讀證據。
3. [強制] 任何決定性或就緒檢查未通過時維持候選，不正式化；TASK index 與 item 不保存執行狀態、Attempt、lock、驗證證據或執行期 instruction audit。
4. [強制] 正式 TASK 核准後才建立或同步 Execution index；Task 不建立 Attempt、不執行成果、不修改既有 Attempt。交給 Execute 或從 Execute 返回時使用已由 CLI 驗證與渲染、含固定 `WORK-HANDOFF` marker 的公開交接，交接本身不授權修改任何成品。
5. [強制] 完成回報列出 TASK spec、各 TASK 的技能、變更檔案、Work references、實際驗證結果及仍需 Execute 處理的事項；不得聲稱未執行的檢查已通過。
