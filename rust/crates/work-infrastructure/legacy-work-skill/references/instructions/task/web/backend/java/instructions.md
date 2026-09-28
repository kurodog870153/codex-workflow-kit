---
name: Java Web 後端任務規劃
description: 固定 Java 或 Spring Web 後端的版本、建置、型別、實作與測試決策時使用；其他後端語言不適用。
metadata:
  work-tags:
    - java
    - spring
---

# Java Web 後端任務規劃指令

指令分類狀態：已完成
指令邊界：本層固定 Java／Spring 共通技術決策；關聯式資料、Swagger、JPA 與 MyBatis 細節由其唯一 reference 或子層負責。

## 1. Reference 路由

1. [強制] TASK 涉及關聯式資料庫、schema、migration、SQL、交易、索引或資料映射時，載入 `task.web.backend.java.relational-data`（`references/relational-data.md`）。
2. [強制] TASK 新增或修改 Controller、Endpoint、HTTP 參數、Request Body、Response、DTO、OpenAPI 或 Swagger 產生來源時，先唯讀確認目標專案已存在 Swagger／OpenAPI 依賴與有效設定；兩者成立才載入 `task.web.backend.java.swagger`（`references/swagger.md`）。任一不存在時不得由 AI 新增依賴、設定、產生工具或驗證工具，且正常 Controller 變更不因此受阻。
3. [強制] 使用 JPA 或 MyBatis／MyBatis-Plus 時，依實際受影響流程載入對應子層；兩者同時適用時維持各自邊界，不得互套 API 或預設只能選一種。

## 2. 平台與專案慣例

1. [強制] TASK 核准前須確認目標模組、實際 JDK、編譯目標、Spring 與相關依賴版本、建置工具、測試入口、程式碼風格及受影響鏈；版本證據與使用者確認結果寫入 TASK。
2. [強制] 原始碼、測試、設定與依賴須遵循專案既有架構、命名、格式與責任邊界；不得為套用指令主動重構未觸及內容或引入新層級。
3. [預設] Lombok 沿用目標模組既有風格；只有新增用法會隱藏可變性、生命週期、框架建構、相等性或序列化風險時，TASK 才須限制或改用明確程式碼，不維護固定版本矩陣。
4. [預設] 區域變數是否使用 `var` 依專案風格、目標 JDK 與該行型別可讀性決定；公開簽章、欄位及型別語意不得以 `var` 取代，未觸及程式不主動改寫。
5. [強制] Maven、Gradle 或其他建置操作依專案入口與驗證目的固定；只有需排除舊產物影響時才包含 clean，不得把 `clean` 當成所有驗證的固定前置。
6. [強制] TASK 須確認實際 Wrapper 或系統執行檔、工作目錄、模組／project path、goal／task、參數與測試篩選；不得假設通用 `test`、`check` 或 `build` 一定存在。產生或升級 Wrapper、Gradle、plugin、JDK／toolchain，或修改建置 DSL／設定時，須另列檔案、版本、步驟、CMD、VAL 與授權，不得留給 Execute 順帶修正。
7. [強制] 使用專案產生器時須固定產生器版本、全部參數與必要建置設定，並以目前實際回應或輸出清單逐項核對建立路徑；不得依歷史模板推測產物。
8. [強制] 沿用目標模組既有且適用的單一建置系統，不得在 Maven／Gradle 間切換或為同一變更新增第二套設定；子模組不得被要求自備 Wrapper。Maven 可使用適用 Wrapper 或 TASK 已預檢的系統執行檔；Gradle 有適用 Wrapper 時使用 Wrapper，沒有時只能使用 TASK 已固定版本的系統 Gradle。
9. [強制] TASK 須固定目標 shell、工作目錄、Wrapper 根目錄或系統執行檔、Maven 模組或 Gradle project path、建置 DSL、實際 goal／task、參數／property、測試篩選、Java／JDK／toolchain 與測試位置；固定執行檔、`JAVA_HOME` 或 toolchain 時必須使用精確值，只有 TASK 明確指定使用 `PATH` 解析時才可回退至 PATH。不得由 Execute 補上或替換，可能初始化、下載或寫入的預檢仍須先取得授權。
10. [強制] Maven reactor 指定測試只有在命令使用 `-am` 且實際會執行不含指定測試的上游模組時，才可依需要加入 `failIfNoSpecifiedTests=false`；未使用 `-am` 或上游不會執行時不得加入。Gradle task 必須由既有設定、實際定義或已核准預檢確認，不得假設通用 task 存在。

## 2.1 版本基準

1. [強制] TASK 新增或變更 Java／Spring、編譯 release、Wrapper、Maven／Gradle、依賴／外掛、JUnit、Surefire／Failsafe，或需求涉及相容性時，核准前須依官方第一方資料固定唯一相容組合；其他 TASK 沿用可唯讀確認的既有固定版本，不另建版本基準。
2. [強制] 適用版本須使用精確值或由精確 parent、BOM、Wrapper 唯一推導，不得使用 `latest`、範圍或候選值；來源不足、衝突或仍有候選時維持草案，搜尋結果或單獨 `BUILD SUCCESS` 不構成相容證據。
3. [強制] 版本基準只影響一個 TASK 時記於該 TASK `決策`，影響至少兩個 TASK 時建立文件層 `DECISION-*`，不得另增平行 TASK 欄位。決策內每個元件固定使用 `元件：<名稱>；版本：<精確版本或唯一推導值>；來源：<來源>` 子項，並另用 `官方相容性來源：<可重現定位>；支持結論：<結論>` 子項記錄相容性證據；不得壓成無法逐元件核對的單行、建立第二組版本值或保留候選。

## 3. Java 契約與實作

1. [強制] 型別、null、例外、泛型、序列化與並行契約須在受影響邊界可觀察且一致；不得以未檢查轉型、原始型別或吞例外規避契約。
2. [強制] Collection 宣告使用能表達所有必要操作與語意的最高層介面；只迭代且無其他語意時使用 `Iterable<T>`，一般集合操作使用 `Collection<T>`，唯一性使用 `Set<T>`，索引或 `List#set` 使用 `List<T>`，一般鍵值契約使用 `Map<K, V>`。順序、排序、並行或框架限制只有實際需要時才降低抽象層級，不能依賴具體實作的偶然行為。
3. [強制] 既有公開 API 或框架邊界的集合型別以相容性優先；型別變更須先套用 API 相容性 reference，未確認外部呼叫者時不得主動提升或收窄。
4. [預設] Spring MVC `@RequestParam` 多值參數使用 `Collection<T>`；需要順序或索引時使用 `List<T>`，需要唯一性時使用 `Set<T>`，不得以 `Iterable<T>` 取代框架已確認的綁定契約。Request Body／Response 等序列化邊界須依實際框架版本與往返證據決定。
5. [強制] 日期時間依業務語意選擇 `java.time` 型別並固定時區、格式、時間來源及邊界轉換；只有日期、無時區日期時間、時間點與帶偏移時間不得互相替代。
6. [強制] 資源、Stream、執行緒、Future 與交易生命週期須有明確擁有者、關閉或取消結果；不得建立無界並行或把阻塞工作放入未確認的共用執行器。
7. [強制] 涉及關聯式資料時須交叉確認相依與目標流程的註解、介面、設定、查詢或映射以判定 JPA、MyBatis 或 MyBatis-Plus；既有流程沿用實際鏈路，新流程有多個同樣適用選項時展示證據並由使用者選擇，不得把候選留給 Execute。
8. [強制] 技術選擇不授權新增或升級相依、processor、外掛、設定或 migration；同一不可分割 TASK 可涵蓋多個既有技術分支，但須分別固定流程邊界與指令層級。
9. [強制] 關聯式技術判定須交叉確認建置相依與實際設定、註解、介面、實作、查詢或映射；不得只因父模組、BOM、starter、傳遞相依或未引用程式碼存在就判定。JPA 至少核對 Persistence namespace、Spring Data／Hibernate 相依及 Entity／Repository／EntityManager／查詢；MyBatis 至少核對相依及 Mapper／XML／SqlSession／BaseMapper／實際查詢。
10. [強制] 既有流程依實際資料存取鏈路選擇，不能用 repository 其他位置的技術覆蓋；新流程有多個同樣適用選項、兩者皆未使用或證據衝突時，須展示證據並由使用者指定。正式 TASK 分別固定每個受影響流程的技術、模組、證據、實際版本、namespace／provider 與資料架構，不得留給 Execute。
11. [強制] 集合與 Map 型別核准前須依目標 JDK 確認可用介面階層，並檢查受影響鏈路的實例方法、參數、指派、回傳、泛型、overload、靜態方法與框架入口；不得使用目標 JDK 尚未提供的介面。
12. [強制] 正常流程需要下層方法或語意時直接宣告能同時滿足操作與契約的最高層介面，不得先宣告高層後向下轉型。只有沒有介面可表達必要契約，或框架／外部 API 明確要求時才能使用具體類別或轉型，並固定原因、最小作用域、型別檢查、安全失敗及直接測試。
13. [強制] 既有公開 API、外部呼叫者或序列化邊界無法完整確認時以相容性優先；必要語意須同時固定宣告介面、實作類別及直接測試，VAL 涵蓋受影響宣告與使用點、靜態分析、編譯及相關測試，不得為此掃描或重構未觸及流程。
14. [強制] 僅日期且無時間與時區使用 `LocalDate`；日期時間且無時區使用 `LocalDateTime`；純時間、時間點、偏移量或時區使用對應 `java.time` 型別。需要目前時間時固定時間來源；序列化、API、JDBC、資料庫、外部系統或框架邊界另固定格式、目標型別、轉換與往返驗證。

## 4. Lombok 風險邊界

1. [預設] 只有專案已具備 Lombok 與有效 annotation processing，或 TASK 已另行核准新增設定時才使用；原本未使用 Lombok 的專案若要新增，TASK 須固定採用原因、精確版本或唯一來源、建置設定、影響範圍及 CMD／VAL，Execute 不得自行新增或調整。現有 `lombok.config` 與專案慣例任一禁止即不得使用。
2. [強制] 正式碼與測試碼的 Lombok 白名單只有 `@Getter`、`@Setter`、`@RequiredArgsConstructor`、`@NoArgsConstructor`、`@AllArgsConstructor`、`@Value`、`@Builder`、`@Builder.Default`、`@Singular`、`@With`、`@EqualsAndHashCode`、`@ToString`、`@Slf4j` 與 `@Jacksonized`；未列註解預設禁止，例外須由 TASK 固定註解、目標、完整產生行為、原因與直接驗證。
3. [強制] `@Data`、`@NonNull`、類別層級 `@Setter` 與 `@NoArgsConstructor(force = true)` 預設禁止；例外須由 TASK 個別核准其完整產生行為與物件有效性。Entity 相等性／toString 及類別層級 Builder 亦屬風險觸發項目，未完成核准時使用明確程式碼。
4. [強制] Lombok 產生的 public／protected 方法與建構子視為正式 API；跨模組或公開類別須評估來源與二進位相容性，不主動改寫未觸及用法。
5. [強制] `@Getter` 類別層級使用前須確認每個 getter 都應公開；`@Setter` 只用於確實需要可變性的個別欄位。`@RequiredArgsConstructor` 只用於必要依賴或不可變欄位，`@NoArgsConstructor` 只限框架要求且採最小權限，`@AllArgsConstructor` 只限所有欄位構成建構契約的 DTO／value object。
6. [強制] `@Value` 只用於需要不可變、全欄位相等性、全欄位 `toString` 與 final 類別語意的 value object／DTO，適用時優先考慮 `record`；`@With` 只用於不可變物件並驗證替換後契約與相等性。
7. [強制] `@Builder` 優先標註明確建構子或靜態工廠；類別層級例外須固定全部輸入、可見性、預設值與驗證。`@Builder.Default`、`@Singular` 只在 Builder 已核准時使用，並驗證未賦值、單筆、多筆與空集合行為；`@Jacksonized` 只在專案實際使用 Jackson、Builder 已核准且序列化契約與實際版本相容時使用，既有版本或設定不相容且 TASK 未核准調整時改用明確 Jackson 註解。
8. [強制] `@EqualsAndHashCode` 須固定欄位、包含方式與 `callSuper`；Entity 例外須驗證識別碼、未持久化狀態與代理。`@ToString` 使用欄位白名單，排除機密、個資、Entity 關聯、延遲欄位與循環；`@Slf4j` 只限已使用 SLF4J 的專案，不得為註解新增或替換日誌依賴，其他 Lombok 日誌註解須逐項核准。
9. [強制] Entity 只允許經檢查的 getter、框架要求的最小權限無參數建構子及必要個別 setter；其他 Lombok 行為須逐項核准。新程式碼與 TASK 觸及的既有 Lombok 使用都須符合本節，未觸及程式不主動重構；合規需要擴大檔案或成果範圍時維持草案並重新確認。建構子、Builder、相等性、toString 或序列化受影響時，VAL 除編譯與相關測試外須直接覆蓋產生行為。

## 5. 測試與驗證

1. [預設] 新增或修改可測試行為採 Red／Green：先取得會因缺少目標行為而失敗的證據，再以最小實作通過；既有覆蓋已能直接證明缺口、純規格／產生碼變更或工具限制有具體證據時可例外。
2. [建議] Green 後只有存在可觀察的重複、命名或結構問題時才 Refactor；不得為完成形式循環而擴大修改。
3. [強制] VAL 依實際影響選擇既有單元、切片、整合、契約、靜態分析與編譯入口；不得自行新增框架、依賴或把未執行測試宣稱通過。
4. [強制] 版本、框架、資料庫或環境特有行為無法直接驗證時，須記錄已驗證層級、未驗證項目與剩餘風險。
5. [強制] 測試 VAL 須確認目標模組與指定測試實際被執行且測試數大於零；單獨 `BUILD SUCCESS`、零測試或只編譯不得作為行為通過證據。
