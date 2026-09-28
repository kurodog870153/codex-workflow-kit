---
name: Java Web 後端 JPA 任務執行
description: 執行 TASK 已固定的 JPA 或 Hibernate 修改、資料驗證與證據紀錄時使用；非 JPA 資料存取不適用。
metadata:
  work-tags:
    - jpa
    - hibernate
---

# Java Web 後端 JPA 任務執行指令

指令分類狀態：已完成
指令邊界：本層只執行 TASK 已固定的 JPA／Hibernate 修改與證據，不新增 metamodel、映射、關聯、交易或鎖定決策。

1. [強制] 執行前核對實際 JPA／Hibernate 版本、namespace、Entity、Repository、schema 來源與 TASK 一致；不一致時停止，不自行遷移或改版。
2. [強制] 只有 TASK 已要求靜態 metamodel 時才驗證 processor、產生來源與編譯；產生檔不得手動修改，未要求時不得臨時新增 processor。
3. [強制] 依 TASK 執行映射、關聯、query count、分頁、交易、鎖、schema 與 migration 的適用 VAL；不得用全域 EAGER、額外 cascade 或新架構修補失敗。
4. [強制] 連線資料庫或改變資料前確認該 OP 已授權；未連線時只回報實際證據與未驗證的 dialect、鎖、原生 SQL 或 migration 風險。
5. [強制] TASK 適用的 JPA instructions 須在執行前完整核對；映射、關聯、查詢、分頁、交易、鎖、bulk、batch、audit、schema 或驗證任何一項缺漏時停止，不得以 JPA／Hibernate 預設、Open Session in View、額外 save、全域 EAGER 或擴大 cascade 補足。
