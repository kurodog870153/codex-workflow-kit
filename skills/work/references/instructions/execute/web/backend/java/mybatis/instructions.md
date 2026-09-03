---
name: Java Web 後端 MyBatis 任務執行
description: 執行 TASK 已固定的 MyBatis 或 MyBatis-Plus Mapper、SQL 與外掛驗證時使用；非 MyBatis 資料存取不適用。
metadata:
  work-tags:
    - mybatis
    - mybatis-plus
---

# Java Web 後端 MyBatis 任務執行指令

指令分類狀態：已完成
指令邊界：本層只執行 TASK 已固定的 MyBatis／MyBatis-Plus 分支、Mapper、SQL 與外掛驗證，不替換持久層 API 或架構。

1. [強制] 執行前逐 Mapper／流程核對實際 MyBatis 或 MyBatis-Plus 分支、版本、設定與 TASK 一致；證據不同時停止，不得跨用另一分支 API。
2. [強制] 依 TASK 修改 Mapper、XML／annotation、綁定與結果映射，限定差異須確認 namespace、statement、參數與回傳契約未出現未核准變化。
3. [強制] 依 TASK 執行 Mapper 載入、動態 SQL、唯一性、分頁、交易、批次與外掛 VAL；只有 TASK 已依各層既有 instructions 或 MyBatis-Plus 預設固定 `BaseMapper`、`IService`、`ServiceImpl`、`lambdaQuery()`、`Page<T>` 或 `IPage<T>` 時才能使用，不得為通過驗證臨時改變架構、公開 API 或新增外掛。
4. [強制] 連線資料庫或改變資料前確認 OP 已授權；未連線時揭露未驗證的 dialect、SQL、交易、批次與外掛行為。
5. [強制] TASK 適用的 MyBatis instructions 須在執行前完整核對；Mapper／SQL 來源、綁定、映射、主鍵、分頁、交易、批次或外掛任何一項缺漏時停止，不得改變 SQL 來源、跨用分支 API、加入 `LIMIT 1`、放寬不可信 SQL 或新增外掛補足。
