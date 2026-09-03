---
name: Web 前端 TypeScript 任務執行
description: 執行並驗證 TASK 已固定的 TypeScript 前端型別、模組與編譯要求時使用；純 JavaScript 或不涉及型別變更的工作不適用。
metadata:
  work-tags:
    - static-typing
    - type-safety
---

# Web 前端 TypeScript 任務執行指令

指令分類狀態：已完成
指令邊界：本層只執行 TASK 已固定的 TypeScript 契約，不提高 strictness、升級版本或擴大型別重構。

1. [強制] 修改前核對 TASK 與實際 tsconfig、manifest、lockfile、module resolution、型別來源及檢查入口一致；差異會影響做法時停止交回 Task。
2. [強制] 只在正式範圍內維持 props、資料、事件、設定與外部邊界的型別安全，不得以 `any`、忽略註解、非空斷言或不安全 cast 隱藏錯誤。
3. [強制] 執行期外部資料須使用 TASK 指定且專案已有的 validator、schema 或 guard；不得把靜態型別當作執行期驗證。
4. [強制] Generated types 與 declaration files 依正式來源重建或修改，產生檔不得手動修補。
5. [強制] 執行 TASK 指定的 typecheck、build、lint 與 tests，保存最小有效證據；未涵蓋的公開型別或相容性風險須明確回報。
