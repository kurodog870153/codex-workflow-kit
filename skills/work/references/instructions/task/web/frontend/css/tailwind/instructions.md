---
name: Web 前端 Tailwind 任務規劃
description: 固定 utility-first 樣式、主題 tokens、class 掃描、組合與建置驗證時使用；非 Tailwind 樣式工作不適用。
metadata:
  work-tags:
    - utility-first
    - design-tokens
---

# Web 前端 Tailwind 任務規劃指令

指令分類狀態：已完成
指令邊界：本層只固定 Tailwind 專屬契約，不產生品牌、內容、區塊或介面文案。

1. [強制] TASK 須從 manifest、lockfile、CSS entry、PostCSS 或 bundler integration、config 與既有程式確認實際版本能力及設定方式；不得套用其他版本的慣例或自行升級。
2. [強制] Theme、tokens、variants、plugins、prefix、important、dark mode 與 preflight 須沿用實際專案來源；新增或改變全域行為須固定影響範圍與相容性驗證。
3. [強制] Utility 組合須保留可讀的元件責任與狀態變體；抽取 class、component 或 helper 只在重複、條件組合或專案慣例需要時進行，不建立無用途抽象。
4. [強制] 動態 class 必須可被實際 content scanning 或明確 safelist 發現；不得以無界字串拼接產生可能被移除的 utility。
5. [強制] Arbitrary values 只在現有 token 無法正確表達且需求已確認時使用；可重用設計值須依專案既有 token 機制管理，不散落魔術數值。
6. [強制] Class merge、variant helper 或 formatting 工具只有專案已使用或另經確認時才能納入；不得為整理 class 新增依賴。
7. [強制] VAL 須執行實際 build，證明受影響 utilities、variants、responsive 與 state classes 存在於產出且無掃描遺漏；適用時再執行 lint、tests 與代表性視覺驗證。
