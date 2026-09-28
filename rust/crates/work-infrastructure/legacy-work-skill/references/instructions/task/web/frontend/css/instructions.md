---
name: Web 前端 CSS 任務規劃
description: 固定 CSS cascade、佈局、tokens、響應式、互動狀態與樣式驗證契約時使用；不涉及樣式變更的前端工作不適用。
metadata:
  work-tags:
    - visual-styling
    - responsive-layout
---

# Web 前端 CSS 任務規劃指令

指令分類狀態：已完成
指令邊界：本層固定框架中立的 CSS 契約；具體 utility framework 規則由子層補足。

1. [強制] TASK 須確認現有 global、scoped、module、CSS-in-JS、preprocessor、reset、cascade layer、selector 與 import 順序，不得無證據引入另一套架構。
2. [強制] 受影響樣式須固定 token 來源、繼承、specificity、狀態、容器或 viewport 條件、內容溢出與不同文字長度結果；禁止以大量 `!important` 掩蓋未分析的 cascade。
3. [強制] 佈局須固定 grid、flex、flow、position、尺寸與斷點責任，並涵蓋最小與最大代表性 viewport、縮放及內容成長，不以固定像素假設內容不變。
4. [強制] Focus、hover、active、disabled、invalid、loading、reduced motion、forced colors 與列印樣式只在適用時固定，且不得移除可見焦點或依顏色單獨傳達狀態。
5. [強制] 新增或修改 animation、transition、字型、圖片及 effects 時須固定降級、載入、效能與系統偏好行為。
6. [強制] VAL 依專案能力涵蓋 style lint、build、代表性 viewport、鍵盤焦點、縮放、overflow、主題與支援瀏覽器；人工視覺驗證須列出明確比較條件。
