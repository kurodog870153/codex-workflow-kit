---
name: Web 前端 CSS 任務執行
description: 執行並驗證 TASK 已固定的 CSS cascade、佈局、響應式、狀態與樣式要求時使用；不涉及樣式變更的前端工作不適用。
metadata:
  work-tags:
    - visual-styling
    - responsive-layout
---

# Web 前端 CSS 任務執行指令

指令分類狀態：已完成
指令邊界：本層只執行正式 CSS 契約，不重新設計、改變品牌內容或引入另一套樣式架構。

1. [強制] 修改前核對 global、scoped、module、preprocessor、cascade layer、tokens、import 與 selector 現況符合 TASK；未記錄的全域影響須停止。
2. [強制] 僅依正式範圍修改 cascade、layout、responsive、state、theme、motion 與資產樣式，不得以 `!important`、固定尺寸或 selector 擴散掩蓋根因。
3. [強制] 維持鍵盤焦點、對比、縮放、reduced motion、forced colors、內容溢出與不同文字長度的已確認結果。
4. [強制] 執行 TASK 指定的 style lint、build、viewport、瀏覽器及人工視覺 VAL；不得以單一截圖或主觀相似取代明確條件。
5. [強制] 實際 token、斷點、瀏覽器或設計來源與 TASK 不符時，記錄差異並交回 Task，不自行決定替代值。
