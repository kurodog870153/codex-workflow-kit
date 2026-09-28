---
name: Web 前端 Astro 任務執行
description: 執行並驗證 TASK 已固定的 Astro 頁面、元件、渲染、islands 與建置要求時使用；非 Astro 前端工作不適用。
metadata:
  work-tags:
    - islands-architecture
    - static-rendering
---

# Web 前端 Astro 任務執行指令

指令分類狀態：已完成
指令邊界：本層只執行正式 Astro 契約，不新增頁面內容、route、framework integration、adapter 或 hydration 決策。

1. [強制] 修改前核對實際版本、Astro config、adapter、integrations、輸出模式、目錄、route 與命令均符合 TASK；任何影響契約的差異須停止交回 Task。
2. [強制] 僅修改正式列出的 page、layout、Astro component、framework component、content source 與資產，並維持 server-only、client bundle 與環境變數邊界。
3. [強制] Client directives、prerender、dynamic paths 與資料取得完全依 TASK 實作，不得為排除錯誤增加 hydration、改變輸出模式或搬移執行邊界。
4. [強制] 不得以 placeholder、臨時 metadata、未確認內容或未授權 integration 補足缺漏；規格不足時停止。
5. [強制] 執行正式 typecheck 或 Astro check、build 與 route 驗證；適用時檢查 hydration、產出檔、404、資產及 adapter 結果並記錄證據。
