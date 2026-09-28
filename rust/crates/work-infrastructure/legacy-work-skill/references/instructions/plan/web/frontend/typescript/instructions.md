---
name: Web 前端 TypeScript 需求規劃
description: 規劃需要靜態型別、型別安全整合或既有 TypeScript 相容性的前端成果時使用；純 JavaScript 或不涉及型別契約的工作不適用。
metadata:
  work-tags:
    - static-typing
    - type-safety
---

# Web 前端 TypeScript 需求規劃指令

指令分類狀態：已完成
指令邊界：本層只確認型別安全與相容性成果，不指定 TypeScript 版本、tsconfig 選項、套件或檔案結構。

1. [強制] Plan 須確認哪些外部資料、元件介面、事件、設定與建置邊界需要可驗證的型別安全結果。
2. [強制] 涉及既有 JavaScript 或 TypeScript 時須記錄允許的相容性、漸進採用或嚴格化結果，不得預設可全面轉換或提高 strictness。
3. [強制] 版本與編譯能力由 Task 依專案 manifest、lockfile、tsconfig 及實際工具鏈確認；Plan 不寫死版本或新增工具。
4. [強制] 驗收只描述型別錯誤必須被發現及既有建置仍可成立的結果，不指定檢查命令或測試框架。
