---
name: Web 前端 TypeScript 任務規劃
description: 固定 TypeScript 前端型別、編譯設定、模組介面與靜態驗證契約時使用；純 JavaScript 或不涉及型別變更的工作不適用。
metadata:
  work-tags:
    - static-typing
    - type-safety
---

# Web 前端 TypeScript 任務規劃指令

指令分類狀態：已完成
指令邊界：本層只固定 TypeScript 專屬契約；頁面、樣式與框架要求沿用其他適用層級。

1. [強制] TASK 須從實際 manifest、lockfile、tsconfig、建置設定與原始碼確認 TypeScript 版本、module resolution、JSX、path aliases、strictness 與型別檢查入口，不得自行升級或重建設定。
2. [強制] 受影響的 props、資料、事件、函式、設定、環境變數及外部模組邊界須固定型別來源、可空性、窄化、錯誤與相容性行為。
3. [強制] 外部或執行期資料不得只靠 assertion 視為可信；須沿用既有 validator、schema 或明確 guard，缺少方案時列為待確認決策。
4. [預設] 優先使用可推論型別、discriminated union、`unknown` 與精確介面；不得為通過檢查擴散 `any`、非空斷言、忽略註解或不安全 cast。
5. [強制] Generated types、ambient declarations、套件型別與 framework types 須固定來源及更新方式；產生檔不得手動修改。
6. [強制] VAL 須使用專案實際的 typecheck 與 build 入口，並依影響涵蓋 lint、tests、未使用程式與公開型別相容性；不得只以編輯器沒有紅線作為證據。
