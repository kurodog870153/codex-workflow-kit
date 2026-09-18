# worklib Python 架構規範

## 1. 文件目的

1. 本文件是 `skills/work/scripts/worklib` 的長期架構規範。
2. 新功能、Issue #41 的遷移工作及後續重構都必須遵守本文件。
3. 本文件定義模組責任與允許依賴；公開 CLI、Contract、檔案格式及執行安全規則仍以實際產品規格與測試為準。
4. 若需求必須違反本文件，應先更新架構決策與驗證規則，不得只在程式碼中加入未記錄例外。

## 2. 目標架構

`worklib` 的主要呼叫方向固定為：

```text
controllers
    |
    v
business_services
    |
    v
services
    |
    v
models
```

1. `controllers/`：CLI 邊界。
2. `business_services/`：業務流程聚合與跨功能編排。
3. `services/`：單一功能的實作。
4. `models/`：資料 class。
5. `protocol/` 是各層可讀取的穩定協定常數邊界，不是額外的業務層。
6. `foundation/` 與 `infrastructure/` 是受限制的技術支援模組，不是額外的業務層。
7. 主要業務呼叫不得逆向或跳層；Protocol 與技術支援依賴依第 7 節處理。

## 3. Controller

### 3.1 責任

1. 註冊 CLI command、argument 與 help。
2. 接收並轉換 CLI 輸入。
3. 呼叫一個對應的 Business Service 公開入口。
4. 將成功結果或既有錯誤交給統一 CLI envelope 處理。

### 3.2 禁止事項

1. 不包含業務決策、跨檔案驗證、交易或復原邏輯。
2. 不直接呼叫單一功能 Service。
3. 不直接呼叫 Model 的驗證、render 或 storage 操作。
4. 不直接存取 `artifacts/`、`contracts/`、`execution/`、`foundation/` 或 `infrastructure/` 的產品功能。
5. 不自行建立 Catalog、Selection、fingerprint 或交易資料。

### 3.3 允許依賴

1. Python 標準函式庫。
2. `controllers/` 內的 CLI 共用型別及註冊輔助元件。
3. 對應的 `business_services/<business>/` 公開入口。
4. `protocol/` 公開常數。

## 4. Business Service

### 4.1 責任

1. 依業務範圍聚合多個單一功能 Service。
2. 固定流程順序、交易邊界與跨功能決策。
3. 將一個 Service 的結果明確傳給下一個 Service。
4. 管理公開 Use Case，例如 Plan 建立、Task 發布、Handoff 驗證及 Execution lifecycle。
5. 回傳 Model 或可直接序列化的結果，不處理 CLI formatting。

### 4.2 禁止事項

1. 不實作可獨立重用的解析、驗證、排序、fingerprint、storage 或檔案掃描算法。
2. 不直接執行低階檔案替換、subprocess 或 OS lock。
3. 不引用 Controller。
4. 不以全域可變狀態保存流程資料。

### 4.3 允許依賴

1. Python 標準函式庫。
2. 同一業務目錄內的 Business Service 私有組件。
3. 一個或多個 `services/<feature>/` 公開入口。
4. `models/<feature>/` 中作為輸入、輸出或流程狀態的 class。
5. `protocol/` 公開常數。
6. Business Service 不得依賴其他業務範圍的 Business Service；共用能力應由上層流程明確聚合，或下沉為單一功能 Service。

## 5. 單一功能 Service

### 5.1 定義

1. 一個 Service 只完成一項可命名、可獨立驗證的功能。
2. 功能名稱應描述能力，例如 `validation`、`ordering`、`catalog`、`selection`、`storage`、`fingerprint` 或 `parsing`。
3. 同一功能可以包含多個內部模組，但只能提供一個明確的功能邊界。
4. Service 的輸入必須包含完成工作所需資料；不得為取得資料而呼叫另一個 Service。

### 5.2 禁止 Service 互相引用

1. `services/<feature-a>/` 不得匯入 `services/<feature-b>/`。
2. 同一功能目錄內的私有模組可以互相匯入。
3. Catalog 與 Selection 必須分離時，由 Business Service 先呼叫 Catalog，再把結果傳給 Selection。
4. Validation 需要多種來源資料時，由 Business Service 取得完整資料後一次傳入 Validation。
5. 不得以 `__init__.py` 重匯出、延遲匯入、動態匯入或 dependency locator 規避此規則。

### 5.3 責任範例

1. Parsing：bytes 或 text 轉為 Model，不讀取不相關來源。
2. Validation：驗證完整輸入並回傳 Model 或診斷，不自行載入其他功能資料。
3. Ordering：以純輸入輸出產生 canonical 順序。
4. Catalog：掃描指定根目錄並建立 catalog，不執行 selection。
5. Selection：以既有 catalog 和決策建立 selection，不重新掃描 catalog。
6. Storage：實作一種資產的讀寫、原子替換或 lock，不做跨資產流程決策。
7. Fingerprint：對明確輸入產生穩定 identity，不決定業務狀態。

### 5.4 允許依賴

1. Python 標準函式庫。
2. 同一 `services/<feature>/` 內的私有模組。
3. `models/<feature>/` 與必要的共用 Model。
4. 不含業務規則的 `foundation/` 純技術函式。
5. 對應的 `infrastructure/` adapter；此依賴只允許低階 I/O、OS 或 subprocess 能力。
6. `protocol/` 公開常數。

## 6. Model

### 6.1 允許內容

1. Python class。
2. Pydantic `BaseModel`、`RootModel` 或專案 Model base class 的子類別。
3. `Enum`、例外 class 及表達資料狀態所需的 class。
4. class 欄位、class-level schema metadata、validator 及純 instance／class method。
5. 建立上述 class 所需的 import 與 module docstring。

### 6.2 禁止內容

1. Module-level 業務函式。
2. 檔案、目錄、環境變數、Git、subprocess、網路或時間來源存取。
3. Storage、transaction、recovery 或 orchestration。
4. 對 Controller、Business Service、Service、Infrastructure 或舊業務模組的依賴。
5. 可變的 module-level 狀態。
6. 為了重用驗證流程而呼叫 Service。

### 6.3 Model 方法界線

1. Model 可以執行只依賴自身欄位的結構驗證與序列化。
2. 需要其他檔案、catalog、歷史紀錄或跨 Model 集合的驗證屬於 Service。
3. Canonical ordering 若涉及多筆資料或外部 Contract，屬於 Service。
4. Model 不得因方便而成為 Business Service 或 storage facade。
5. Model 可以匯入 `protocol/` 公開常數，但 Protocol 不得匯入 Model。

## 7. 技術支援邊界

### 7.1 Foundation

1. `foundation/` 只保留與業務功能無關的純技術能力。
2. 允許內容包括 UTF-8 decoding、canonical JSON、通用 path safety 及無業務語意的 fingerprint primitive。
3. Foundation 不得依賴 Controller、Business Service、Service、Model 或 Infrastructure。
4. 含 `requirement_id`、Task layout、Workflow path 等產品語意的規則應遷移至對應 Model 或 Service，不能因多處使用就留在 Foundation。

### 7.2 Infrastructure

1. `infrastructure/` 封裝檔案系統、原子替換、writer lock、subprocess 及其他 OS adapter。
2. Infrastructure 不決定業務流程、狀態轉換或跨資產一致性政策。
3. Infrastructure 可以依賴 Foundation 與 I/O 所需的 Model。
4. Infrastructure 不得依賴 Controller、Business Service 或 Service。
5. Service 可以呼叫 Infrastructure adapter；Business Service 與 Controller 不得直接呼叫 Infrastructure。

### 7.3 外部操作

1. Subprocess、Git 及 OS 操作必須由 Infrastructure adapter 執行。
2. 是否執行、執行順序、授權與結果處置由 Business Service 決定。
3. 命令解析、preview、approval identity 與 receipt 各自維持單一功能邊界。

### 7.4 Protocol

1. `protocol/` 只保存穩定、無副作用且可由所有層讀取的協定常數。
2. 跨兩個以上功能且語意完全相同的常數放在 `protocol/shared.py`。
3. 只在單一功能內跨模組共用的常數放在 `protocol/<feature>.py`。
4. 只在單一模組使用的常數保留為該模組的私有常數。
5. Schema ID、狀態及錯誤碼必須先證明語意相同；不得只因字串相同就集中。
6. 一般 JSON key、欄位名稱與一次性示例值不得常數化。
7. Protocol 不得匯入 Controller、Business Service、Service、Model、Foundation、Infrastructure 或舊業務模組。
8. Protocol 不得執行 I/O、驗證、序列化、流程判斷或保存可變狀態。
9. `protocol/__init__.py` 是穩定公開入口，只能重匯出明確列入 `__all__` 的常數。

## 8. 功能目錄與命名

1. Controller：`controllers/<business>.py` 或 `controllers/<business>/`。
2. Business Service：`business_services/<business>/`。
3. 單一功能 Service：`services/<feature>/`。
4. Model：`models/<feature>/`。
5. Protocol：`protocol/shared.py` 或 `protocol/<feature>.py`。
6. `<business>` 描述使用者可辨識的流程，例如 `plan`、`task`、`handoff`、`execution`。
7. `<feature>` 描述單一能力，例如 `plan_validation`、`instruction_catalog`、`task_ordering`。
8. 不以 `common`、`utils`、`helpers` 或 `misc` 隱藏未分類責任。
9. `__init__.py` 只定義穩定公開入口，不包含業務邏輯，也不得用來規避依賴檢查。

## 9. 允許依賴矩陣

| 來源 | Controller | Business Service | Service | Model | Protocol | Foundation | Infrastructure |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Controller | 同層 CLI 共用元件 | 允許 | 禁止 | 禁止 | 允許 | 禁止 | 禁止 |
| Business Service | 禁止 | 僅同一業務內部 | 允許 | 允許 | 允許 | 禁止 | 禁止 |
| Service | 禁止 | 禁止 | 僅同一功能內部 | 允許 | 允許 | 允許 | 允許 |
| Model | 禁止 | 禁止 | 禁止 | 允許 | 允許 | 禁止 | 禁止 |
| Protocol | 禁止 | 禁止 | 禁止 | 禁止 | 允許 | 禁止 | 禁止 |
| Foundation | 禁止 | 禁止 | 禁止 | 禁止 | 允許 | 允許 | 禁止 |
| Infrastructure | 禁止 | 禁止 | 禁止 | 僅 I/O 所需 | 允許 | 允許 | 允許 |

1. 表中的「允許」表示架構上可以依賴，不表示每個模組都應建立該依賴。
2. Python 標準函式庫及明確核准的第三方套件不列入矩陣。
3. 測試可以匯入其測試目標，但產品程式不得藉由測試入口形成反向依賴。

## 10. 現有目錄的目標歸屬

### 10.1 `artifacts/`

1. 檔案讀寫、原子替換與 lock 移至對應 Infrastructure adapter 或 Storage Service。
2. Draft、assembly、source check、status 等單一能力移至 `services/<feature>/`。
3. 跨 Draft、Task Collection、Specification 的流程編排移至 `business_services/task/`。
4. Model 建構資料移至對應 `models/` class，不保留 Artifact 業務層。

### 10.2 `contracts/`

1. 純 Pydantic class 移至 `models/<feature>/`。
2. Validation、canonicalization、ordering、render 與 diagnostics 移至單一功能 Service。
3. Registry 的資料 class 移至 Model；catalog／scaffold 行為移至 Service。
4. 遷移完成後，`contracts/` 只可暫時保留相容匯出，不再新增實作。

### 10.3 `execution/`

1. Attempt、Command、Record、Correction、Deviation、Recovery 等資料 class 移至 `models/execution/` 下的功能範圍。
2. Preflight、worktree、authorization、receipt、recovery 等單一能力移至 `services/<feature>/`。
3. Execution lifecycle、交易順序及跨功能復原移至 `business_services/execution/`。
4. 低階 subprocess、Git、lock 與原子檔案操作移至 Infrastructure adapter。

### 10.4 `foundation/`

1. 無業務語意的純技術能力可以保留。
2. 產品錯誤 class 移至 Model。
3. 含 Plan、Task、Requirement、Workflow 或 Execution 語意的 path、transaction 與 validation 移至對應 Service 或 Model。
4. Foundation 不得成為跨層捷徑。

### 10.5 `infrastructure/`

1. 保留低階 I/O 與 OS adapter。
2. 業務狀態判定、Contract validation 及流程復原移至 Service 或 Business Service。
3. 依功能拆分 adapter，避免單一 Infrastructure 模組知道多個業務流程。
4. Infrastructure 的公開 API 應以資料或 Model 表達，不回呼上層。

### 10.6 現有 `services/`

1. 聚合多個功能的模組移至 `business_services/`。
2. 真正單一功能的模組移入 `services/<feature>/`。
3. 現有 Service 對 Service 匯入必須改由 Business Service 聚合。
4. `hierarchy` 與 `hierarchy_selection` 的循環依賴必須在遷移時消除。

## 11. 相容入口政策

1. 舊公開 import path 有已知使用者時，原模組可以暫時保留相容重匯出。
2. 相容模組只能匯入並重匯出新入口，不得保留、複製或新增業務邏輯。
3. 新架構模組不得反向匯入相容模組。
4. 每個相容入口必須有明確消費者、測試及後續移除 Task。
5. 未確認外部使用情況前，不因專案內沒有引用就移除公開符號。
6. 相容入口的移除必須在功能遷移與完整測試完成後另行執行。

## 12. 遷移期規則

1. 新目錄從建立起立即遵守本文件。
2. 舊目錄可以暫時存在，但不得新增新的逆向依賴或 Service 橫向依賴。
3. 每個遷移 Task 必須產出可匯入、可測試且行為相容的原子結果。
4. 不允許先複製邏輯再於未來清理；需要相容時只保留薄重匯出。
5. 每個垂直切片應依序完成 Model、單一功能 Service、Business Service、Controller 與測試。
6. 大型功能若無法安全原子遷移，必須在執行前拆成更小 Task，不得留下半套架構。
7. 公開 Schema ID、欄位順序、錯誤碼、檔案 bytes、fingerprint 及安全限制預設保持不變。

## 13. TASK-003 自動檢查契約

架構測試至少必須檢查下列規則：

1. `controllers/` 不得匯入 `services/`、`models/`、`foundation/`、`infrastructure/` 或舊業務目錄。
2. `controllers/` 只能匯入對應 Business Service 及 Controller 共用元件。
3. `business_services/` 不得匯入 Controller、Infrastructure、Foundation 或舊業務目錄。
4. 不同 `business_services/<business>/` 不得互相匯入。
5. `services/<feature-a>/` 不得匯入 `services/<feature-b>/`。
6. Service 不得匯入 Controller、Business Service 或舊業務目錄。
7. `models/` 不得匯入 Controller、Business Service、Service、Foundation、Infrastructure 或舊業務目錄。
8. Model module 的 top-level 定義只允許 class、import、module docstring 及 class 建立所需的靜態宣告；不得含 module-level function。
9. Foundation 不得匯入 Model、Infrastructure 或任何業務層。
10. Infrastructure 不得匯入 Controller、Business Service 或 Service。
11. 相容入口只能包含 import、明確 `__all__`、module docstring 及必要的靜態型別資訊。
12. 架構檢查必須解析相對匯入、`__init__.py` 重匯出及 package 入口。
13. 遷移期間，舊目錄以明確 allowlist 管理；allowlist 只能減少，不能無理由擴張。
14. 所有層都可以匯入 Protocol；Protocol 只能匯入自身模組或 Python 標準函式庫。

## 14. Review 檢查表

1. 這段程式碼屬於 CLI 邊界、跨功能編排、單一功能、資料 class 或技術 adapter 中的哪一種？
2. Controller 是否只呼叫 Business Service？
3. Business Service 是否只做流程聚合，而沒有重新實作單一功能？
4. Service 是否能以完整輸入獨立執行，且沒有呼叫其他 Service？
5. Model 是否只有 class，且沒有 I/O 或外部狀態？
6. Foundation 與 Infrastructure 是否不含業務決策？
7. 是否新增逆向、跳層、循環或隱藏於重匯出的依賴？
8. 相容入口是否足夠薄，且有明確移除條件？
9. 公開行為、Contract、fingerprint 與安全限制是否有相應測試？
10. 共用常數是否放在正確的全域、功能或模組私有範圍，且沒有因字串相同而誤合併？
