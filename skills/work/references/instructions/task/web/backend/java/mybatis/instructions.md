---
name: Java Web 後端 MyBatis 任務規劃
description: 固定 MyBatis 或 MyBatis-Plus 流程的 Mapper、SQL、外掛與驗證契約時使用；非 MyBatis 資料存取不適用。
metadata:
  work-tags:
    - mybatis
    - mybatis-plus
---

# Java Web 後端 MyBatis 任務規劃指令

指令分類狀態：已完成
指令邊界：本層只固定受影響 MyBatis／MyBatis-Plus 流程的專屬契約；共通 SQL、交易與資料指令沿用父層。

## 1. 適用分支

1. [強制] TASK 須依目標模組與 Mapper／流程確認實際 MyBatis、MyBatis-Plus、Spring 整合、設定、Mapper、XML／annotation、`SqlSession` 與持久層架構；影響兩種技術時分別列出邊界。
2. [強制] 單一 Mapper／流程只能使用其已確認分支的 API、外掛與分頁契約；不得因專案另有 MyBatis-Plus 就遷移純 MyBatis，反之亦然。
3. [強制] 固定技術分支不代表授權新增、升級或遷移依賴、設定、外掛或架構；未觸及流程不主動重構。
4. [強制] 正式 TASK 須分別固定各受影響 Mapper／流程的分支、目標模組、判定證據、實際 MyBatis core／MyBatis-Plus 版本、Spring 整合或 `SqlSession` 管理、Mapper 掃描、XML 資源、設定、SQL 來源、交易參與及資料存取架構；證據不足或衝突時維持草案。
5. [預設] 專案既有且有效慣例優先；沒有明確慣例時採本檔安全預設。偏離時須固定原因、適用邊界、替代方案及直接驗證。

## 2. Mapper 與 SQL

1. [強制] 依既有慣例固定 Mapper 方法、參數名稱與型別、綁定、回傳、SQL 來源及結果映射；XML namespace、statement ID、`resultMap`／`resultType` 與 Mapper 必須一致。
2. [強制] SQL 來源須固定為既有 annotation、XML 或 provider 方式；只有既有方式無法正確表達需求時才能改用其他方式，並固定原因、邊界與測試。實際涉及時另固定自動映射、alias、`jdbcType`、`notNullColumn`、column prefix、巢狀或建構子映射。
3. [強制] `#{}` 用於值綁定；`${}` 或可接受 SQL 內容的 API 不得接收不可信輸入，動態 identifier、表名、排序與片段須由後端允許清單產生。確需原始插值時固定原因、可信來源、允許清單、邊界與直接驗證。
4. [強制] Null、enum、TypeHandler、動態 WHERE／SET、空集合 IN、LIKE 跳脫及空值語意只有實際受影響時才固定並直接驗證，不得加入未核准的隱含轉換。
5. [強制] 單筆查詢不得用任意第一筆、任意 `LIMIT 1` 或忽略重複掩蓋資料異常；分頁須依既有機制固定參數、回傳契約、單頁上限、含唯一 tie-breaker 的穩定排序、count、總筆數與溢出結果。
6. [強制] 更新與刪除須固定條件、範圍及預期影響筆數，預設禁止全表操作；確有必要時另固定原因、保護及驗證。主鍵來源、型別、產生策略與回填須依分支固定，適用時包含 `useGeneratedKeys`、`keyProperty`、`keyColumn`、複合鍵、sequence 與批次回填。
7. [強制] 多筆或跨 Mapper 寫入須在已確認架構的協調層固定交易邊界；大量寫入固定既有版本支援的批次機制、資料量、批次大小、失敗、回滾、部分成功及主鍵回填，不得以未受控逐筆迴圈取代。
8. [強制] 資料庫日期時間映射須依 Java 共通指令固定資料庫型別、JDBC 行為、必要 TypeHandler／轉換及往返驗證。

## 3. 純 MyBatis

1. [強制] 純 MyBatis 流程不得新增或使用 MyBatis-Plus API；分頁須固定既有機制、dialect、參數、回傳、count 與攔截器，不得套用 `Page<T>`／`IPage<T>`。
2. [強制] Batch executor 或手動 `SqlSession` 只有需求實際採用時才固定 executor type、session／交易生命週期、flush、批次大小與錯誤處理，不得為套用指令臨時新增。

## 4. MyBatis-Plus 與架構

1. [預設] MyBatis-Plus 流程在專案架構與實際版本適用時，Mapper 優先繼承 `BaseMapper`；只有 TASK 選用 Persistence Service 層時，其介面優先繼承 `IService`、實作優先繼承 `ServiceImpl`。不得為套用指令新增層級、遷移未觸及流程或替換既有公開 API。
2. [預設] TASK 已採用 Persistence Service／`IService` 且查詢可清楚表達時優先使用 `lambdaQuery()`；複雜 join、聚合、特殊 SQL 或架構不適用時，依 TASK 固定原因、邊界與測試使用自訂 Mapper／XML。四層架構不得省略業務 Service 或 Persistence Service，Controller 不得直接呼叫 Mapper；其他架構沿用已確認鏈路。
3. [強制] Controller、RPC 或其他不可信邊界不得傳入 Wrapper 或 SQL 片段；`last()`、`apply()`、`inSql()`、`exists()` 等 API 必須符合允許清單與參數綁定要求。
4. [強制] `getOne(..., false)` 不得用於掩蓋重複資料。分頁須分別檢查 Controller、Service 與 Mapper 各層既有契約或慣例並逐層沿用；某層沒有既有指令時，該層預設以 `Page<T>` 作為 request／parameter、以 `IPage<T>` 作為 response／return。不得因此破壞既有 Controller 或其他公開 API，並須固定最大筆數、溢出、穩定排序與總筆數語意。
5. [強制] 分頁、租戶、邏輯刪除、資料權限、樂觀鎖與全表防護外掛只有受影響時才固定實際版本、設定、InnerInterceptor 順序、作用範圍、排除與直接驗證；自訂 SQL 不得無意繞過既有控制。
6. [強制] 多 Mapper 寫入、batch executor 或手動 `SqlSession` 須依既有架構固定交易責任、session 生命週期、flush、批次大小、回滾、部分成功與主鍵回填。
7. [強制] VAL 依實際影響涵蓋 Mapper XML 載入、映射、動態 SQL、null／唯一性、主鍵回填、分頁／count、交易／回滾、批次、樂觀鎖、邏輯刪除、租戶、資料權限與外掛；資料庫連線與剩餘風險依關聯式資料 reference 處理。
