# Swagger／OpenAPI 參考指令

參考名稱：task.web.backend.java.swagger
適用層級：task.web.backend.java
指令分類狀態：已完成

## 0. 適用前提

1. [強制] 本 reference 只在目標專案已存在 Swagger／OpenAPI 依賴與可用設定時適用；任一不存在時不得由 AI 新增依賴、設定、產生器或檢查工具，且不得以缺少 Swagger 為由阻擋正常 Controller 變更。
2. [強制] 適用後，任何新增或修改的 Controller、Endpoint 方法、參數、Request Body 或 Response Body 都必須依本檔補齊說明；不得只處理此次新增的 annotation 或忽略同一受影響 Endpoint 的既有缺漏。

## 1. 完整契約

1. [強制] 所有適用 Controller 類別與每個 Endpoint 方法都須具有專案版本支援的 Swagger／OpenAPI 註解，完整描述用途、操作識別、標籤、授權需求及適用的 deprecated 語意。
2. [強制] 每個 path、query、header、cookie、form、multipart 參數及 Request Body 都須具有註解，固定名稱、位置、用途、必要性、型別／schema、格式、允許值、範圍及適用範例。
3. [強制] 每個可能回傳的成功與錯誤 Response 都須具有註解，固定狀態碼、語意、媒體型別、schema、header 及適用範例；不得只註解成功回應或以模糊 default 取代已知錯誤。
4. [強制] 每個受影響 Endpoint 直接或巢狀引用的 request／response DTO，以及其每個可序列化欄位，都須具有註解，固定語意、必要性、null、型別、格式、允許值、範圍、讀寫模式及適用範例；DTO 即使原本不在修改範圍，也須納入同一 TASK 的檔案與原子變更集後補齊。
5. [強制] 「完整」表示所有會影響契約語意的適用屬性均明確且彼此一致，不要求填入與該元素無關的所有可選 annotation 屬性。

## 2. Task 階段確定

1. [強制] TASK 核准前須確認實際 Swagger／OpenAPI 函式庫、版本、annotation namespace、產生入口、客製設定、契約消費者與既有文件風格；證據須展示並由使用者確認。
2. [強制] TASK 須逐 Endpoint 展示並固定 Controller、Parameter、Request Body、所有可確認成功／錯誤 Response、DTO 與欄位的完整契約矩陣，包含狀態碼、媒體型別、schema 與 body，並在正式核准前取得使用者確認；不得使用 TODO、TBD、空描述、候選值或要求 Execute 依程式碼自行推論。
3. [強制] Swagger 契約須與實際驗證、序列化、授權、錯誤處理、泛型、分頁及 API 相容性決策一致；衝突時先修改 TASK 決策，不得只修改文件掩蓋行為。
4. [強制] 產生碼只能由正式規格、模板或 generator 設定產生註解；TASK 須修改其唯一來源與再生驗證，不得規劃手動編輯產生檔。

## 3. 驗證

1. [強制] 自動 VAL 須實際產生或載入 OpenAPI 契約，確認所有適用 Endpoint、Parameter、Request Body、Response、DTO 與欄位存在，且 `$ref`、schema、狀態碼及媒體型別可解析並與 TASK 矩陣一致。
2. [強制] 專案既有契約檢查、lint 或 consumer 驗證只有實際存在且適用時才使用；不得為套用本指令自行新增工具或外部服務。
3. [強制] 描述是否易懂、範例是否符合使用情境等主觀品質由人工 VAL 確認；自動結構通過不得取代人工語意確認。
