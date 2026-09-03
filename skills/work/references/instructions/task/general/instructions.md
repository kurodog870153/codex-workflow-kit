---
name: 任務規劃
description: 將已確認需求轉成技術決策、檔案、步驟、命令、操作與驗證時使用；實際執行不適用。
metadata:
  work-tags:
    - task-specification
---

# 任務規劃指令

指令分類狀態：已完成
指令邊界：Task 是正式 `work-task/v1` 可表達之技術決策、檔案、步驟、命令、操作與驗證的唯一權威來源；Execute 不得補充或推論缺漏。

## 1. 載入與所有權

1. [強制] 建立、修改、審查或維護正式 TASK 時，只使用來源 Plan 已確認且未漂移的技能；不得在 Task 階段探索、推薦、新增、替換或移除技能。
2. [強制] 正式 TASK、index、狀態、欄位或指令來源一律載入 `task.general.task-records`（`references/task-records.md`）。
3. [強制] 出現 `external_state` OP 時載入 `task.general.external-operations`（`references/external-operations.md`）。
4. [強制] 新增、移動、拆分、合併或分類 Work instruction／reference 時載入 `task.general.instruction-maintenance`（`references/instruction-maintenance.md`）。
5. [強制] Reference 只在觸發條件成立時載入；正式 TASK 只保存 globally unique logical name，不保存絕對路徑。
6. [強制] 每個 TASK 綁定一個 `skill_id`；不需要外部技能時使用 `null`。同一 TASK 不得組合多個外部技能。
7. [強制] 每個可執行技能使用一個隔離短暫 subagent 並依 TASK 順序處理；Plan-only 技能不得建立執行型 TASK，skill subagent 不得再委派。

## 2. 需求確認與最小 TASK

1. [強制] 目標、範圍、限制、交付成果、輸入、檔案、副作用、風險與驗證必須可唯一判定；資訊不足時逐項詢問，不得自行假設。
2. [強制] 每個 `TASK-*` 產生一個可獨立驗收的單一成果；可分割成果須拆分，不可分割多檔修改可保留在同一 TASK。
3. [強制] Execute 所需的所有檔案、順序、`CMD-*`、`OP-*`、`VAL-*` 與授權邊界必須在 Task 階段確定。
4. [強制] 新決策改變已確認內容、references 或副作用時，列出影響並重新取得使用者確認。
5. [強制] 全部決策完成後展示完整候選 JSON；使用者核准及 Python 驗證前不得建立正式檔案。

## 3. 正式 TASK contract

1. [強制] 正式文件只包含單一 H1 與唯一 fenced JSON，固定使用 `schema: work-task/v1`、`status: confirmed` 與 `TASK-SPEC-nnn`。
2. [強制] keys、enums、IDs、statuses、paths、references 及 hashes 使用英文；語意字串可使用繁體中文。所有欄位、條件結構與引用規則以 Work Python validator 為唯一機器規格。
3. [強制] 對話候選直接呈現預計正式化的 `confirmed` JSON；不存在 `draft` schema、`TASK-SPEC-DRAFT` 或草案檔。
4. [強制] 每個 TASK 的 `instruction_selection.selected_paths` 只能是來源 Plan 已確認葉節點、其祖先或空子集，且非空路徑必須完整存在於 Task 與 Execute catalog；`sources` 只保存 `kind`、`logical_name`、`canonical_sha256`，不得保存來源 layer 或絕對路徑。
5. [強制] 每個 TASK 保存 `skill_id`、`instruction_selection`、`traceability`、單一 `goal`、有序 `steps` 與至少一個 `VAL-*`；只有 `skill_id` 可用 `null` 表示 base-only，沒有內容的選用欄位省略。
6. [強制] TASK 集合完整覆蓋來源 Plan 的 `GOAL-*`、`DELIVERABLE-*`、`ACCEPTANCE-*` 與存在的 `MILESTONE-*`；每個 acceptance 至少由一個最終 VAL 覆蓋。
7. [強制] TASK 文件只保存正式指令選擇與 canonical fingerprints，不保存執行狀態、Attempt、lock、驗證證據或執行期 instruction audit 紀錄。

## 4. 決定性驗證

1. [強制] stdin 只接受純 JSON；path 模式只接受由共用 renderer 產生的 canonical H1＋JSON 文件。
2. [強制] validator 每次重新驗證來源 Plan、`hierarchy_selection_sha256`、TASK hierarchy 子集與雙模式可用性、技能選擇與 bundle 漂移、三個 artifact paths、Plan SHA、每 TASK 的單一技能綁定、Work references、DAG、ID、引用、檔案生命週期與 acceptance coverage。
3. [強制] Python 不執行任意 CMD 或 OP；Task agent 必須在對話展示命令入口、語法、專案現況及驗證分類等唯讀證據。
4. [強制] 同一路徑由多個 TASK 處理時必須有明確相依順序；`create`、`modify` 與 `move` 的生命週期必須符合目前專案狀態。
5. [強制] 任一確定性或就緒檢查未通過時維持對話候選，不得正式化或交給 Execute。

## 5. 完成與交接

1. [強制] 正式 TASK 核准後才能建立或同步 execution index；Task 不建立 Attempt，也不執行成果。
2. [強制] Plan 交入需求時使用 `plan_to_task`，Execute 發現正式 TASK 缺漏時使用 `execute_to_task`；兩者都必須是已由 Work Python CLI 驗證、含固定 `WORK-HANDOFF` marker 的純 JSON 交接，Task 不修改既有 Attempt。
3. [強制] TASK 正式化後使用 `task_to_execute`，需要修改 Plan 時使用 `task_to_plan`；交接須由 Work Python CLI render，只存在於對話，且本身不授權修改 Plan、TASK、index 或 Attempt。
4. [強制] 完成回報列出 TASK spec、每 TASK 的 `skill_id`、變更檔案、Work references、驗證結果及仍需 Execute 處理的事項。
