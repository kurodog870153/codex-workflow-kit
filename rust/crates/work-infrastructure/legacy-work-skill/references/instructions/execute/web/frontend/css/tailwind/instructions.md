---
name: Web 前端 Tailwind 任務執行
description: 執行並驗證 TASK 已固定的 utility-first 樣式、tokens、class 掃描與建置要求時使用；非 Tailwind 樣式工作不適用。
metadata:
  work-tags:
    - utility-first
    - design-tokens
---

# Web 前端 Tailwind 任務執行指令

指令分類狀態：已完成
指令邊界：本層只執行正式 Tailwind 契約，不升級版本、重設主題、產生品牌內容或新增 class 工具依賴。

1. [強制] 修改前核對實際版本、CSS entry、integration、config、content scanning、theme、plugins、prefix 與 preflight 均符合 TASK；不得套用其他版本語法。
2. [強制] 僅依正式元件責任組合 utilities、variants 與 responsive states，沿用既有 token 與 class helper；不得為縮短字串建立無需求抽象。
3. [強制] 動態 classes 必須依 TASK 使用可掃描的完整值或明確 safelist，不得以無界拼接或未記錄設定避免產出移除。
4. [強制] Arbitrary values、全域 utilities、theme 或 plugin 變更只限正式列出的例外；發現可重用值缺少已確認 token 時交回 Task。
5. [強制] 執行實際 build 並驗證受影響 utilities、states、responsive variants 與產出；適用時執行 lint、tests 及代表性視覺 VAL，記錄掃描或版本限制。
