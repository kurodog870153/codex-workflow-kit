---
name: Java Web 後端 JPA 任務規劃
description: 固定 JPA 或 Hibernate 流程的 Entity、Repository、查詢、映射與驗證契約時使用；非 JPA 資料存取不適用。
metadata:
  work-tags:
    - jpa
    - hibernate
---

# Java Web 後端 JPA 任務規劃指令

指令分類狀態：已完成
指令邊界：本層只固定受影響 JPA／Hibernate 流程的專屬契約；關聯式資料與 Java 共通要求沿用父層，不在此重複。

1. [強制] TASK 須確認目標模組實際 Spring Data JPA、Hibernate、Persistence namespace、Repository 抽象、annotation processing 與受影響 Entity／查詢；不得自行遷移 `javax.persistence` 與 `jakarta.persistence`。
2. [預設] 只有受影響 Criteria／Specification 或專案既有建置契約實際需要靜態 metamodel 時，才固定相容的 annotation processor、產生目錄與編譯驗證；不適用時不得強制新增 processor。
3. [強制] 靜態 metamodel 適用時，processor 座標與版本須依官方資料和實際 Hibernate core／BOM 對齊，並固定 Maven／Gradle 宣告位置、annotation processing、產生來源目錄、編譯整合及清理行為；VAL 須證明 processor 執行、目標 `*_` 類別產生且參與編譯，Criteria／Specification 對已產生屬性不得改用字串路徑。
4. [強制] Entity 映射依實際欄位固定主鍵、產生策略、access type、Entity／table／column 名稱、null、長度、precision／scale、唯一性、預設值、enum、日期時間、識別碼不可變性、schema 與相等性；產生檔不得手動修改。
5. [預設] Enum 使用明確字串映射或已確認 converter，不依 ordinal；複合主鍵須固定 `@EmbeddedId` 或 `@IdClass`、值相等性與查詢驗證，Entity 相等性須固定代理、未持久化與持久化行為，不得只依可變欄位或資料庫產生 ID。
6. [強制] 每個受影響關聯須依真實生命週期固定 cardinality、擁有端、`mappedBy`／join column、可空性、集合型別、雙向同步、fetch plan、cascade、`orphanRemoval` 與生命週期測試；cascade 採最小集合，`orphanRemoval` 只限父實體私有擁有的子實體，不預設 `CascadeType.ALL`。
7. [預設] 查詢所需關聯優先由個別 fetch join、Entity Graph、projection、batch fetch、subselect 或既有方案取得；不得為避免 N+1 全域改為 EAGER。每個可能載入關聯的查詢依風險固定 SQL／query count 上限；高資料量另固定 projection、分頁／scroll、索引、記憶體邊界與代表性資料驗證。
8. [強制] Repository 與交易責任沿用專案既有架構，不預設四層；只有既有抽象無法表達需求時才能使用自訂 Repository／`EntityManager`，並固定原因、邊界與測試。
9. [強制] 簡單條件沿用既有 derived query，組合條件依慣例使用 Specification／Criteria，複雜固定查詢使用明確 JPQL／HQL；所有值參數綁定，動態欄位與排序只接受後端白名單。Native query 只在標準查詢不能正確表達或有已確認效能需求時使用，並固定 dialect、SQL、結果映射、count query、限制條件與驗證。
10. [強制] 單筆查詢固定唯一性、找不到資料及 `Optional`／例外契約；分頁固定 `Page`／`Slice`／`List`／scroll、單頁上限、溢出、總筆數及含唯一 tie-breaker 的穩定排序。Collection fetch join、native query 或複雜 projection 併用分頁時，須固定正確的資料與 count 方案及測試。
11. [強制] 交易邊界置於完整業務工作單元；讀取優先 `readOnly = true`，寫入固定回滾。非預設 propagation、isolation、timeout 或 rollback 須固定理由與測試；延遲載入與 DTO 轉換位於已固定邊界，不得依賴 Open Session in View。
12. [預設] `@Version` 只在可能並行更新且樂觀鎖符合衝突契約時使用；不需要時不得強制加入，需一致性保護時固定替代方案。合併 detached Entity 亦須固定一致性行為；悲觀鎖另固定 lock mode、順序、timeout、死鎖／重試、交易邊界與驗證。
13. [強制] 更新、刪除與 Bulk DML 須固定條件、目標範圍、預期影響筆數、`@Modifying`、flush／clear 與 persistence context 過期狀態，且不得無意繞過 version、listener、audit、邏輯刪除、租戶或資料權限；全表操作預設禁止。
14. [強制] Batch 須固定資料量、JDBC batch 設定、主鍵策略相容性、batch size、排序、定期 flush／clear、交易大小、記憶體上限及失敗／回滾／部分成功，不得因 `saveAll` 就宣稱批次化或改用未受控逐筆寫入。
15. [強制] Auditing、Entity listener、邏輯刪除、租戶或資料權限受影響時，須固定建立／更新者、時間與主體來源、查詢條件、bulk／native 行為及測試；Entity 不得直接讀取 Controller、HTTP Session 或安全框架上下文。
16. [強制] 正式環境不得以未核准的 `ddl-auto=create`、`create-drop` 或 `update` 取代 migration；Entity、constraint、index、foreign key 與 sequence 須依核准 schema 來源及 migration 驗證。
17. [強制] VAL 依實際影響涵蓋映射、constraint、Repository、null／唯一性、關聯擁有端、cascade／orphan、fetch／query count、分頁／count、交易／回滾、鎖衝突、bulk persistence context、batch、audit／邏輯刪除／租戶及 migration；`@DataJpaTest` 只有專案既有且適用時才使用，資料庫連線依關聯式資料 reference 授權並揭露未驗證風險。
18. [強制] 偏離適用安全預設時，TASK 須固定例外原因、資料量與生命週期邊界、替代方案及直接驗證；不得為套用指令主動重構未觸及 JPA 程式。
