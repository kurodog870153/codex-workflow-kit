---
name: Web 前端 Astro 任務規劃
description: 固定 Astro 專案的頁面、元件、渲染模式、islands、整合與建置驗證時使用；非 Astro 前端工作不適用。
metadata:
  work-tags:
    - islands-architecture
    - static-rendering
---

# Web 前端 Astro 任務規劃指令

指令分類狀態：已完成
指令邊界：本層只固定 Astro 專屬實作契約，不決定產品內容、版面區塊或介面文案。

1. [強制] TASK 須從實際 manifest、lockfile、Astro config、adapter、integrations 與目錄確認版本能力、輸出模式、路由慣例及可用命令；不得自行升級或切換 adapter。
2. [強制] 每個受影響 route、layout、page、Astro component、framework component 與 content source 須固定檔案、輸入、輸出、資料取得與錯誤邊界。
3. [強制] Static、server、hybrid、prerender 與動態 route 行為須依既有部署契約及需求固定；不得為方便任意改變整站輸出模式。
4. [強制] Client directives 只用於已確認需要瀏覽器執行的互動，並固定 framework、hydration 時機、fallback 與 JavaScript 成本；純靜態內容不得無理由 hydrate。
5. [強制] Head metadata、資產、圖片處理、環境變數、integration 與 server-only code 須遵循現有 Astro 邊界，不得把機密資料送入 client bundle。
6. [強制] Dynamic paths、content collections 或資料來源適用時須固定 schema、缺漏資料、建置時與請求時行為及代表性測試資料。
7. [強制] VAL 須至少執行實際 typecheck 或 Astro check、build 與受影響 route 驗證；適用時另涵蓋 hydration、輸出檔、adapter、404、資產及部署相容性。
