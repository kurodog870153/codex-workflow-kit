---
name: 需求規劃
description: 確定需求目標、範圍、限制、交付成果、決策與可觀察驗收時使用；技術實作與執行細節不適用。
metadata:
  work-tags:
    - requirements-planning
---

# 需求規劃指令

指令分類狀態：已完成
指令邊界：Plan 只確定目標、範圍、限制、交付成果、決策與可觀察驗收；可由正式 TASK 欄位表達的技術或執行指令不屬於 Plan。

## 1. 職責與內容

1. [強制] Plan 只記錄需求層的 `GOAL-*`、`SCOPE-*`、`CONSTRAINT-*`、`DEPENDENCY-*`、`RISK-*`、`MILESTONE-*`、`DELIVERABLE-*`、`ACCEPTANCE-*`、`PLAN-DECISION-*` 與 `PLAN-CHANGE-*`，不得包含實作步驟、命令、工具、測試方法或檔案修改細節。
2. [強制] 每項交付成果須對應至少一項可由使用者觀察的驗收條件；驗收條件描述結果，不指定如何產生證據。
3. [強制] 技術選型、版本、檔案、操作、驗證方法及例外若不改變需求結果，只能在 Task 階段確定。
4. [預設] Plan 採完成需求所需的最小範圍；候選改善不得在未確認前併入正式內容。
5. [預設] 非目標、依賴、風險及時程只在適用時加入；`MILESTONE-*` 只用於多階段、重要檢查點或具有階段相依的計畫。
6. [強制] 初次正式化時各類識別碼須各自由 `001` 連續遞增；後續變更可因保留歷史而留下缺號，但既有識別碼不得重新編號或重用，新項目只取該類下一號。

## 1.1 對話與草案邊界

1. [強制] Plan 邊界適用於本階段的全部對話、證據摘要、釐清問題、選項、決策、狀態摘要、候選內容與正式 Plan，不得只在正式寫入時移除先前已決定的不適用內容。
2. [強制] 討論中出現不屬於 Plan 的細節時，只能判定其是否會改變需求結果；會改變結果時改寫為需求層的成果、限制、相容性或可觀察驗收，不會改變結果時標示由 Task 依專案證據確定，且不得繼續展開候選設計。
3. [強制] 使用者主動提供的細節只有在其為不可變限制且會改變需求結果時才能納入 Plan；否則只能視為 Task 階段待核對的輸入，不得擴充其周邊設計或轉成 Plan 決策。

## 2. 證據與決策

1. [強制] 資訊不足、證據衝突或結果無法唯一判定時須提問，不得自行補足需求。
2. [強制] 任何會影響正式 Plan 結果的證據都須向使用者展示並取得確認，即使證據看似明確；確認只代表使用者接受本次判定，不代表證據永久正確。
3. [強制] 唯讀探索可依同一決策所需範圍批次進行；每次問題只能確認一項決策，並同時列出其相關證據、影響與有編號的選項。
4. [強制] 新決策可能影響已確認內容時，須先列出受影響項目，再逐項重新確認；沒有影響時也須在狀態摘要中明示。
5. [預設] 同一對話中內容與來源均未改變的證據可重用；新對話開始後須先唯讀重新核對，才能再次引用。
6. [強制] 每次回答後更新目前目標、範圍、限制、交付成果、驗收與未決事項；全部決策完成後先提供完整草案，取得確認後才能正式化。
7. [強制] 全部必要確認完成前，草案只能存在於對話，不得建立 Plan 檔案或相關目錄；正式寫入時一次建立 `status` 為 `confirmed` 的 Plan。

## 3. 正式 Plan

1. [強制] 正式文件使用單一 H1，後接唯一的 fenced JSON contract；JSON 是唯一正式內容，固定使用 `schema: work-plan/v1` 與 `status: confirmed`，不得包含其他 Markdown。
2. [強制] JSON keys、enums、IDs、statuses、paths、references 及 hashes 使用英文；`summary`、`statement`、`reason` 等不由 Python 解讀的語意文字可使用繁體中文。Required keys、optional keys、nested fields、ID 格式、references、雙向 Deliverable／Acceptance 關聯及 instruction fingerprint 完全以 Work Python validator 為準，不得另訂同義欄位。
3. [強制] 需求編號只能使用小寫英數、`.`、`_`、`-`，並須通過共用 instruction-loading 的跨 Windows、macOS、Linux 檔名限制；只能沿用使用者目前輸入或正式交接明列的值，不得由檔名、目錄或其他對話推測。
4. [強制] Plan、TASK 與 execution 預設分別位於 `outputs/work/plans/<requirement-id>.md`、`outputs/work/tasks/<requirement-id>.md`、`outputs/work/executions/<requirement-id>/`；任一項改用非預設路徑時，使用者須同時確認三個專案相對路徑與同一需求編號，不得由單一路徑推導其他路徑。
5. [強制] 每次讀寫前須完整套用共用 instruction-loading 的需求成品路徑安全檢查，不得在本層縮減、另訂或只沿用前一階段的檢查結果；無法確認時維持對話草案。
6. [強制] 已確認完整草案後，需求編號必須是寫入前最後一項問題；使用者選定有效編號即授權建立該正式 Plan，不得再追加未揭露內容。
7. [強制] 初次寫入只確認一次需求編號；後續 Task 只有在使用者目前輸入或正式規格交接已明列有效需求編號時才能沿用且不得重問，未明列時仍須詢問。
8. [強制] `artifacts` 永遠明列 Plan、TASK 與 execution 三個 project-relative paths；`hierarchy_selection` 保存已確認跨模式葉節點、各模式 metadata、推薦原因與選擇雜湊；`work_instruction_selection` 保存原始 `selected_paths`、Plan 實際載入的 `resolved_paths`、`sources`、`references` 與 `instructions_sha256`；`skill_selection` 保存外部技能快照。每個 instruction source 只保存 `kind`、`logical_name` 與 `canonical_sha256`，不得保存絕對路徑。
9. [強制] 正式寫入前以 stdin 模式驗證純 JSON，核准後由 Work Python CLI 的 `plan create` 使用固定欄位順序、二格縮排、UTF-8、NFC、LF、無 BOM 與恰好一個尾端換行渲染並 exclusive create；不得由 AI 手動組合或寫入 Markdown。寫入後以 path 模式重新驗證，兩次 canonical Plan 與 instruction fingerprints 必須一致。任一次失敗立即停止，不得自行修補。

````markdown
# <Plan title>

```json
{
  "schema": "work-plan/v1",
  "requirement_id": "example",
  "status": "confirmed",
  "title": "<Plan title>",
  "summary": "<summary>",
  "artifacts": {
    "plan": "outputs/work/plans/example.md",
    "task": "outputs/work/tasks/example.md",
    "execution": "outputs/work/executions/example"
  },
  "hierarchy_selection": {
    "schema": "work-hierarchy-selection/v1",
    "decision": "general_only",
    "selected_paths": [],
    "entries": [],
    "catalog_sha256": "<64-lowercase-hex>",
    "selection_sha256": "<64-lowercase-hex>"
  },
  "work_instruction_selection": {
    "selected_paths": [],
    "resolved_paths": [
      "general"
    ],
    "sources": [
      {
        "kind": "workflow",
        "logical_name": "work.instruction-loading",
        "canonical_sha256": "<64-lowercase-hex>"
      },
      {
        "kind": "instruction",
        "logical_name": "plan.general",
        "canonical_sha256": "<64-lowercase-hex>"
      }
    ],
    "references": [],
    "instructions_sha256": "<64-lowercase-hex>"
  },
  "skill_selection": {
    "schema": "work-skill-selection/v1",
    "decision": "base_only",
    "skills": [],
    "selection_sha256": "<64-lowercase-hex>"
  },
  "goals": [
    {
      "id": "GOAL-001",
      "statement": "<goal>"
    }
  ],
  "scope": [
    {
      "id": "SCOPE-001",
      "kind": "in_scope",
      "statement": "<scope>",
      "goal_ids": [
        "GOAL-001"
      ]
    }
  ],
  "deliverables": [
    {
      "id": "DELIVERABLE-001",
      "statement": "<deliverable>",
      "goal_ids": [
        "GOAL-001"
      ],
      "acceptance_ids": [
        "ACCEPTANCE-001"
      ]
    }
  ],
  "acceptance_criteria": [
    {
      "id": "ACCEPTANCE-001",
      "statement": "<observable result>",
      "deliverable_ids": [
        "DELIVERABLE-001"
      ]
    }
  ]
}
```
````

## 4. 變更與同步

1. [強制] 已確認 Plan 的變更須先在對話核准並維持 `status: confirmed`；既有 Plan 不得由 Plan 流程直接覆寫，須交由 Task 流程在同一規格交易中同步 Plan、TASK 與 execution index；不得建立平行 Plan 或把完成狀態寫入 Plan。
2. [強制] `PLAN-CHANGE-*` 由 `PLAN-CHANGE-001` 遞增，只記 ISO `date`、`location`、`before`、`after`、`reason` 及 `affected_ids`；相同原因與同一成果合併一筆，沒有變更時省略，不把執行證據或狀態寫入變更紀錄。
3. [強制] 變更時須指出受影響的目標、成果、驗收及正式 TASK；只影響 Plan 時不修改 TASK，影響既有 TASK 時依 Task instructions 在同一已授權邏輯原子更新中同步 TASK 與 execution index。
4. [強制] 需求編號或三個對應路徑重新命名須另行授權並同步處理；未受影響內容不得順便改寫。
5. [強制] Plan 不保存完成狀態；實際狀態與證據只由 execution index 與 Attempt 管理。
6. [強制] Task 或 Execute 要求修改 Plan 時，只接受已由 Work Python CLI 驗證、含固定 `WORK-HANDOFF` marker 的 `task_to_plan` 或 `execute_to_plan` 純 JSON 交接；交接只存在於對話，且本身不授權修改任何成品。

## 5. 完成回報

1. [強制] 回報實際建立或修改的 Plan、狀態、驗證結果、未決事項及是否需要進入 Task 階段。
2. [強制] 不得聲稱未執行的檢查已通過，也不得把 Task 或 Execute 的成果列為 Plan 已完成成果。
