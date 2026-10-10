# Issue #81：Task 粒度、跨 Task 檔案相依與多檔交易

## 1. 來源、交付範圍與狀態

1. 來源：[Issue #81](https://github.com/kurodog870153/codex-workflow-kit/issues/81)，盤點日期：2026-10-08；讀取時為 open，沒有留言。
2. 本次交付：相關檔案盤點、已確認決策、依序可執行的最小任務與驗收條件；使用者已授權依序完成 T1～T8，執行期間範圍調整可自行決定。未授權提交或推送。
3. 任務狀態依下列實際驗證紀錄更新；未驗證條件保持未完成。
4. 不新增依賴、不建立平行 Task／Attempt／Recovery 架構、不修改不可變的 Source Snapshot 或歷史核准證據。
5. 維持正式 Task Collection、Execution Index 與既有 Attempt 的資料格式相容；新增審查使用 Discussion，檔案交易使用既有 runtime／derivation／ports 架構及新增交易契約。
6. 每項任務的程式、契約登錄、必要指紋處理、對應測試與相關指令，構成同一完整變更集。執行前重新讀取全部相關內容，再一次套用；本文件中的新模組名稱為預定位置，不代表模組已存在。

## 2. 已逐項確認的決策

1. D1：採可復原的交易完整性。先保存完整前後內容，再發布；中斷時允許暫時存在部分發布，但保留證據並阻擋 Task 完成及後續執行，不宣稱外部程序看不到中間狀態。
2. D2：新增結構化粒度審查，沿用 `PlanningReview` 與失效機制。記錄單一成果、拆分判斷、不可分割理由及交易可行性；缺漏或過期時阻擋正式化。既有 Discussion 在再次正式化前須補審查。
3. D3：無相依順序的共用檔案操作，須有明確的獨立性審查證據，綁定 Task 配對、共用路徑及相關規劃內容；變更後失效。不自行建立相依、不將文字理由視為程式已證明修改區域互不重疊。
4. D4：不可分割的多檔修改由 Work CLI 統一發布。AI 在暫存區準備完整結果，CLI 驗證、保存復原證據並發布；Execute 完成判定須核對交易結果。
5. D5：提交失敗的專案檔案交易，經新的復原授權後恢復交易前原始狀態；若實際內容有額外修改，停止並保留證據。原有執行紀錄 Recovery 的向前復原語意維持不變。

## 3. 唯讀盤點

| 責任 | 主要既有檔案 | 現況與缺口 |
| --- | --- | --- |
| Discussion 審查型別 | `rust/crates/work-model/src/discussion/{mod,operation,contracts}.rs` | `PlanningReview` 有 context revision、decision versions、文字證據與 needs_review；沒有結構化粒度審查。 |
| 審查驗證、失效與組裝 | `rust/crates/work-operations/src/discussion/{validation,update,assembly}.rs`；`rust/crates/work-feature/src/discussion/{assembly,mod,repository}.rs` | 正式化要求有效 review；Task／上游變動已有審查失效機制。 |
| Task 檔案與輸入契約 | `rust/crates/work-model/src/task/{item,candidate,index,projection,discussion_trace}.rs` | 檔案操作只有 create／modify／move；inputs 有 task_output 等種類。正式集合有固定 Discussion revision 的 trace，不能改綁後續 Session。 |
| DAG 與 Task 語意 | `rust/crates/work-operations/src/task/{mod,item,semantic,collection,ordering}.rs` | 已檢查循環、自我與冗餘相依；已有依排序模擬檔案存在狀態，缺少完整跨 Task 讀寫衝突分析。 |
| Task 功能及正式化 | `rust/crates/work-feature/src/task/{mod,create}.rs`；`rust/crates/work-infrastructure/src/task/{storage,create_storage}.rs` | 全集合驗證及正式化已有入口，可接入審查與衝突規則。 |
| Execute 前置與完成 | `rust/crates/work-operations/src/execution/{preflight,attempt_prepare,attempt_close,acceptance}.rs`；`rust/crates/work-feature/src/execution/{mod,recovery}.rs` | 已阻擋 direct dependency 未完成；沒有專案檔案交易完成證據的完整 gate。 |
| Execute 工作區與命令 | `rust/crates/work-operations/src/execution/{worktree,command_run}.rs`；`rust/crates/work-feature/src/execution/command_publication.rs` | 有 Git 狀態快照與授權命令流程；狀態快照不能代替完整檔案 bytes 的前置與復原證據。 |
| Runtime 型別與衍生規則 | `rust/crates/work-model/src/runtime.rs`；`rust/crates/work-operations/src/derivation/{transaction,publication,identity,fingerprint,graph}.rs` | 可沿用身分、staging、指紋與完整 inventory；既有規則有受限交易種類，不能直接套到任意專案檔案。 |
| 儲存與復原 | `rust/crates/work-infrastructure/src/{transaction_storage,recovery,files,writer_lock}.rs`；`rust/crates/work-infrastructure/src/execution/storage.rs` | 主要保護執行紀錄與正式產物，採可復原逐步發布；不保證任意多檔案的瞬間原子切換。 |
| 公開入口與效果分類 | `rust/crates/work-flow/src/{discussion,task,execution}.rs`；`rust/crates/work-cli/src/{runtime,parser/mod}.rs`；`rust/crates/work-cli/src/parser/commands.json`；`rust/crates/work-operations/src/operation_effects.json` | 新入口必須走 CLI → Flow → Feature，並登錄唯讀／寫入效果，不繞過 ports。 |
| 契約登錄與測試 | `rust/crates/work-model/src/{schema,contract_data}.rs`；各相關模組內測試；`rust/crates/work-cli/tests/{current_only_contract,architecture_dependencies,derivation_architecture}.rs`；`rust/crates/work-feature/tests/execution_worktree_flow.rs` | 契約、canonical bytes、producer／validator 與 fixture 必須一起更新，不能只加資料欄位。 |
| 指令與架構文件 | `skills/work/references/workflows/task/`、`skills/work/references/workflows/execute/`、`skills/work/references/instructions/{task,execute}/general/references/`、`docs/work/rust/architecture.md` | 正式產物與執行紀錄復原規則目前偏向完成原核准發布；必須明確區分新的專案檔案還原交易。 |

## 4. 共通執行與驗證規則

1. 執行順序為 T1 → T2 → T3 → T4 → T5 → T6 → T7 → T8。每項直接依賴前項即可，避免另加傳遞冗餘相依。T2～T7 會共用契約、入口或指令檔案，執行時必須讀取前項完成後的實際內容。
2. T1 是單一設計文件成果；T2～T7 各為單一可驗收能力。T5 的提交、失敗辨識、持久化與還原不能拆成無復原能力的部分 writer；T6 的公開發布入口與完成 gate 同時交付。
3. 所有任務共同維護本文件的對應狀態與驗證紀錄。通過後才勾選完成條件；任務全部條件通過後才勾選任務本身。
4. 實作前檢查 `git status --short`；發現使用者修改即停止確認。每個原子變更先讀取全部檔案，檢查 patch 的路徑相依，不能依賴同一 patch 前段建立或移動的新路徑。
5. 下列 Cargo 命令的工作目錄均為 `rust/`。每項執行 `cargo fmt --all -- --check` 與 `git diff --check`，並執行列明的 crate 測試及對應 Clippy。格式檢查不等於授權全專案格式重寫。
6. Cargo 驗證會建立 `rust/target/`；Infrastructure 測試可建立測試專用暫存目錄、檔案、Git repository 與故障注入資料，不操作真實外部服務或使用者專案。
7. 中斷測試應重用既有故障注入慣例。單次等待逾時只繼續等待原程序；實際失敗先分析根因與是否需要決策，再搜尋同根因位置，完整修復後重跑相關驗證。
8. 新 schema／命令名稱以 T1 的邊界文件為準；公開前同步 model、registry、canonical ordering、scaffold、parser、effect catalog 與 producer／validator。沒有可證明的交易能力即拒絕寫入。
9. 審查只能強制「完成語意審查且證據有效」，不能聲稱程式能從自然語言自動證明成果不可拆分。檔案路徑衝突、DAG、bytes drift 與交易狀態則由確定性規則強制驗證。

## 5. 依序執行的 Task

### T1：固定審查、交易與相容性邊界

1. [x] 任務完成。
2. 依賴：無。
3. 成果：一份涵蓋 D1～D5 的可實作邊界文件，避免後續 Task 各自定義不同交易或審查語意。
4. 邊界文件現行位置：[Rust 架構的執行產物與復原邊界](../work/rust/architecture.md#執行產物與復原邊界)；T1 原先建立的獨立文件已整併至此。此任務不改程式。
5. 步驟：定義粒度與獨立性審查欄位、內容綁定與失效；定義檔案操作衝突矩陣與祖先關係；定義完整變更集、預覽核准、持久化狀態、提交點、還原及完成 gate；列出新契約與命令及其 effects。
6. 相容性：正式 Task／Execution／Attempt 不新增必填欄位；審查證據使用既有 Discussion authority 與不可變 revision trace。新審查缺漏不阻止唯讀盤點，但禁止新正式化；既有正式集合驗證不得改寫歷史。
7. 保證範圍：首批支援宣告的普通檔案 create／modify／move；不支援的 link／alias、跨 filesystem、特殊檔案或無法保存的 metadata，在正式 targets 寫入前拒絕。還原範圍為交易實際承諾保存的 bytes、存在狀態、路徑與必要 metadata。
8. 邊界：不能把直接專案寫入、任意 CMD／shell、資料庫或外部 API 宣稱為同一檔案交易。CLI 與 instruction 的拒絕、隔離及效果驗證要具體列出；不宣稱能防止其他程序自行修改檔案。
9. 命令／驗證：唯讀比對現有契約與入口，`git diff --check`；人工逐項對照 Issue 與 D1～D5，不執行 Cargo 或外部操作。
10. 風險：設計需要擴大既有決策、引入新依賴或改正式契約時，先逐項確認，不能在文件中直接決定。
11. [x] 完成條件：欄位、命令、交易階段及失敗還原方向有唯一明確定義。
12. [x] 完成條件：完整列出兼容、既有集合缺證據的處理、指紋綁定、外部效果與拒絕支援範圍。
13. [x] 完成條件：D1～D5 與 Issue 最終驗收都有對應規則，文件差異檢查通過。

### T2：強制結構化粒度審查與正式化 gate

1. [x] 任務完成。
2. 依賴：T1。
3. 成果：缺漏、過期、要求拆分或交易不可行的 Task 不能正式化；不可分割多檔 Task 可以通過。
4. 原子變更集：`work-model/src/discussion/{mod,operation,contracts}.rs`；必要的 `work-model/src/{schema,contract_data}.rs`；`work-operations/src/discussion/{validation,update,assembly}.rs`；`work-feature/src/discussion/assembly.rs`；相應 Session／Task 正式化測試及 fixtures；`skills/work/references/workflows/task/{initialize-and-save,formalization-boundary}.md` 與 task-records 指令。
5. 步驟：新增型別化審查內容；保留既有 review authority；按成果與不可分割性審查而非數量門檻；將審查綁定完整規劃內容；Task、自身決策、上游 inputs、context 或相關規劃變動時失效；正式化 gate 驗證有效審查。
6. 命令／驗證：`cargo test -p work-model -p work-operations -p work-feature -p work-infrastructure`；`cargo clippy -p work-model -p work-operations -p work-feature -p work-infrastructure --all-targets -- -D warnings`；共通格式檢查。
7. 風險：Discussion 格式延伸與歷史 canonical hash。既有保存證據保持原 bytes；不能為補欄位回寫 Session history。修正現行合成 fixtures 須由既有 derivation API 產生指紋。
8. [x] 完成條件：單一成果、小 Task、不可分割多檔可通過；多個獨立成果、未定拆分及不可安全完成者拒絕正式化。
9. [x] 完成條件：缺證據、needs_review、內容／context／decision drift 均拒絕；補審查後才恢復可正式化。
10. [x] 完成條件：不存在以檔案數、行數、步驟數硬切的 gate；既有不可變歷史未被改寫。
11. [x] 完成條件：對應契約、正常與失效測試、Clippy 及格式檢查通過。

### T3：驗證跨 Task 檔案衝突與有效獨立性證據

1. [x] 任務完成。
2. 依賴：T2。
3. 成果：Validator 能區分有序操作、無序衝突與已確認獨立操作，並拒絕無法判定的集合。
4. 原子變更集：`work-model/src/discussion/{mod,contracts}.rs`；`work-operations/src/discussion/{validation,update,assembly}.rs`；`work-operations/src/task/{mod,item,semantic}.rs`；`work-feature/src/task/mod.rs`、`work-feature/src/discussion/assembly.rs`；必要的 Task read ports／`work-infrastructure/src/task/storage.rs`；契約與相關 fixtures；formal-task-validation、formalization-boundary 與 task-records 指令。可新增 `work-operations/src/task/file_dependencies.rs`，由既有 Task validator 呼叫。
5. 步驟：使用既有 portable path identity 統一路徑；同時分析 create／modify／move 的 source／destination、inputs 與 DAG 的完整祖先關係；解析 task_output 的 producer 與檔案；產生具 Task 配對及路徑的拒絕診斷。
6. 證據：保存明確 Task 配對、路徑、操作、獨立理由、確認結果及規劃內容綁定；相關 Task／操作／input 變更時失效。正式 Validator 使用固定 trace 指向的原審查證據，不以最新 Session 覆蓋歷史；無有效證據的無序共用寫入拒絕。
7. 規則：相同路徑不自動建立 edge；有序也不能使 create-existing、move-missing 等不合法生命週期變合法；使用傳遞祖先關係，不強制新增冗餘 direct edge。只讀共用輸入不因同路徑被誤判為寫入衝突。
8. 命令／驗證：`cargo test -p work-model -p work-operations -p work-feature -p work-infrastructure -p work-cli`；相同五 crate 的 `cargo clippy ... --all-targets -- -D warnings`；共通格式檢查。
9. 風險：Revise／Migration 或沒有 Discussion provenance 的集合不能偽造獨立性證據；有序集合維持現行契約，無序且不能證明安全者要求確認及既有修訂流程。獨立性證據不免除執行時重新讀取內容與 drift 檢查。
10. [x] 完成條件：create→modify、modify→modify、move 前後路徑、producer→consumer、傳遞相依與路徑 alias 情境均有測試。
11. [x] 完成條件：缺必要順序、無序衝突、失效／錯綁證據拒絕；有效獨立性證據不被強制加 edge。
12. [x] 完成條件：循環、自我、冗餘相依及非法生命周期仍被拒絕，既有 eligibility gate 未退化。
13. [x] 完成條件：正式化與獨立 Validator 共用規則，相關驗證全部通過。

### T4：提供完整檔案變更集的唯讀準備與預覽

1. [x] 任務完成。
2. 依賴：T3。
3. 成果：從暫存內容形成可審查、可核准的完整交易候選，任何不具復原條件的變更在 target 寫入前被拒絕。
4. 原子變更集：`work-model/src/execution/{request,response}.rs` 及新 `file_transaction.rs`；`work-model/src/{schema,contract_data}.rs`；`work-operations/src/derivation/{transaction,publication,identity,fingerprint}.rs`；新 `work-operations/src/execution/file_transaction.rs`、新 `work-feature/src/execution/file_transaction.rs` 與 mod 登錄；`work-infrastructure/src/{files,execution/storage}.rs` 的唯讀 ports；`work-flow/src/execution.rs`、`work-cli/src/runtime.rs`、parser commands、operation_effects；對應測試及預覽指令。
5. 步驟：讀取宣告的全部 target／move paths 與 staging final bytes；核對實體路徑、存在狀態、完整內容、metadata 與完整範圍；以 derivation 計算交易 identity、前後證據及核准指紋，綁定 Task Collection、Task、Attempt、授權與 staging inventory。
6. 行為：prepare／preview 為唯讀；使用已授權 workspace 中準備好的內容，不建立 target、鎖或 runtime transaction。使用者看到完整 create／modify／move 最終結果、復原範圍與已知限制；此階段不提供公開發布入口。
7. 命令／驗證：`cargo test -p work-model -p work-operations -p work-feature -p work-flow -p work-infrastructure -p work-cli`；相同六 crate Clippy；共通格式檢查。新 CLI 名稱依 T1，不能聲稱當前已有這些命令。
8. 風險：Git status 無法代表檔案內容不變；必須保存原始 bytes 與尺寸／hash，保護大小寫／Unicode alias、link、hard-link 與 reparse 邊界。保留證據含機密或無法完整保存時拒絕並交由使用者處理。
9. [x] 完成條件：完整 create／modify／move 候選可產生，預覽沒有任何正式 target／lock／runtime 寫入。
10. [x] 完成條件：遺漏 target、越界、alias、unsupported filesystem／metadata、錯誤前置與 stale staging 均拒絕。
11. [x] 完成條件：任一 target／staging／授權／正式來源內容變動使原核准失效。
12. [x] 完成條件：model、registry、parser、effects 與各層測試一起通過。

### T5：建立可提交且可還原的檔案交易儲存能力

1. [x] 任務完成。
2. 依賴：T4。
3. 成果：僅供內部呼叫的完整儲存能力；保證核准集合提交驗證或經授權還原至原始狀態，具備中斷證據。
4. 原子變更集：`work-model/src/runtime.rs` 及 T4 交易型別；`work-operations/src/derivation/{transaction,publication,identity}.rs` 與 file_transaction 純規則；`work-feature/src/execution/file_transaction.rs` ports；`work-infrastructure/src/{transaction_storage,files,writer_lock}.rs` 與新 `work-infrastructure/src/execution/file_transaction_storage.rs`；相關 mod 登錄與儲存故障注入測試。
5. 步驟：沿用 requirement writer ownership 與唯一 runtime staging；完整持久化前後 bytes、inventory、來源綁定與復原所需 metadata；驗證所有復原資料可讀且一致後才開始 target 寫入；每個發布階段更新可辨識進度；成功後回讀完整 target 集合。
6. 還原：另行授權的恢復操作先唯讀檢查整組實際狀態及 preserved evidence，不能只憑進度計數推定寫入結果。恢復原 bytes、存在狀態及 move 路徑；對本交易建立且未被額外修改的檔案，依核准還原計畫恢復原本不存在的狀態。不得修改其他使用者檔案。
7. 中斷：提交或還原過程中斷都保留完整證據與阻擋狀態；還原可重入，但不能重新執行 CMD／OP，也不能把 restored 記成 published 或 Task completed。清理不得先移除唯一復原 manifest；永久紀錄依原有 history policy 保存。
8. 命令／驗證：`cargo test -p work-model -p work-operations -p work-feature -p work-infrastructure`；對應 Clippy；共通格式檢查。故障注入僅使用測試 temporary project。
9. 風險：fsync／rename／權限／磁碟不足與程序終止；必須測實際支援平台。內部 API 尚未接公開 apply，不使使用者可啟動尚未具備完成 gate 的交易。
10. [x] 完成條件：提交前任何準備／證據驗證失敗都未修改 targets；正常提交全部回讀符合核准內容。
11. [x] 完成條件：每個發布邊界中斷均可經授權完整還原；還原中斷後可安全重入。
12. [x] 完成條件：額外修改、混合交易、錯 root／owner／identity／inventory／hash 均停止且保留全部證據。
13. [x] 完成條件：move、原本不存在檔案與必要 metadata 的還原測試通過；未影響既有執行紀錄向前 Recovery。

### T6：公開發布／還原入口並串接 Execute 完成 gate

1. [x] 任務完成。
2. 依賴：T5。
3. 成果：核准後可發布／復原，只有完整且驗證過的成果可完成；不能從其他 Execute 入口繞過交易狀態。
4. 原子變更集：`work-model/src/execution/{request,response,recovery}.rs` 及新交易契約；schema／contract_data；`work-operations/src/execution/{preflight,attempt_close,acceptance}.rs` 與 file_transaction；`work-feature/src/execution/{mod,recovery,file_transaction}.rs`；`work-infrastructure/src/execution/{storage,file_transaction_storage}.rs`；`work-infrastructure/src/recovery.rs` 的 inventory 整合；`work-flow/src/execution.rs`、`work-cli/src/runtime.rs`、parser commands、operation_effects；對應流程與 CLI 測試；Execute 交易及 closure／recovery 指令。
5. 步驟：公開 apply 與 recovery prepare／restore；寫入前在 owner 邊界重驗 exact approval、原始集合及所有前置內容；將檔案交易綁定目前 Task／Attempt，不改寫原授權歷史；公開具完整證據的成功或 recovery-required 結果。
6. Gate：Attempt 啟動、後續 record／command、completed closure 及 dependency eligibility 必須檢查未完成交易；部分發布、unknown、待還原與 restored 都不能被當成已完成成果。完整發布仍須原有 VAL／acceptance 通過，不能只靠 transaction status 完成 Task。
7. 舊集合：保持 Task／Execution／Attempt 正式格式不變，交易證據可由既有 runtime／永久紀錄綁定辨識；沒有審查或不能證明原子需求者要求補審查，不以缺欄位默認交易已完成。不將其他正式產物交易誤判成專案檔案交易。
8. 命令／驗證：六 crate 測試與 Clippy；`cargo test -p work-cli --test current_only_contract`；共通格式檢查。
9. 風險：新公開命令與 completion gate 必須同一次更新；不能先開 writer 再補 gate。requirement lock 不能防止所有外部 writer，觀察到 drift 時須停止。
10. [x] 完成條件：未核准、核准過期、錯 Task／Attempt／scope 的 apply 在寫 targets 前拒絕。
11. [x] 完成條件：完整發布加既有驗收可完成；中斷／部分／未知／已還原交易不能完成或啟動下游。
12. [x] 完成條件：復原需新授權，按 D5 還原；既有紀錄 recover、Attempt closure 與歷史證據無回歸。
13. [x] 完成條件：所有公開入口、reason codes、效果分類及 gate 的相關測試通過。

### T7：封住檔案交易外的執行與副作用邊界

1. [x] 任務完成。
2. 依賴：T6。
3. 成果：Execute 能明確拒絕不可納入交易或未定義失敗處理的操作，指令不再引導 AI 直接多檔寫入正式 targets。
4. 原子變更集：`work-operations/src/execution/{authorization,command_run,preflight}.rs`；`work-feature/src/execution/{mod,command_publication,file_transaction}.rs`；`work-infrastructure/src/execution/storage.rs` 的命令能力邊界；必要的效果分類與契約；`skills/work/references/workflows/execute/{apply-one-attempt-authorization-boundary,execute-one-authorized-argv-cmd,record-one-execution-result,close-one-attempt,recover-one-execution-transaction}.md`；`skills/work/references/instructions/{task,execute}/general/references/` 中相關交易／external-operations 指令；相應測試。
5. 步驟：生成／修改在 staging 完成；可能直接修改正式 targets 的 CMD／shell，須有可驗證隔離或被拒絕，不以操作前後快照宣稱已保證原子性。驗證命令、local-state／external-state OP 各自有明確效果、授權與失敗處理邊界。
6. 外部操作：不重跑結果未知的外部 CMD／OP；沒有可證明交易、補償或失敗處理的副作用阻擋執行並逐項確認。保留既有 OP／VAL references，不能用無效驗收掩蓋副作用失敗。
7. 命令／驗證：`cargo test -p work-operations -p work-feature -p work-infrastructure -p work-cli`；對應 Clippy；共通格式檢查。外部效果以 fake ports 測試，不連線真實 API／資料庫。
8. 風險：一般程序與任意 shell 的能力不能靠 argv 字串完全推斷；沒有強制隔離或可信能力證據時拒絕。不得新增 sandbox 依賴或宣稱 Work CLI 控制了所有其他工具。
9. [x] 完成條件：直接多檔 target 寫入與未知命令能力不能繞過發布 gate；staging 準備及安全驗證流程可用。
10. [x] 完成條件：外部效果與檔案交易分別驗證，缺失敗處理或結果未知時阻擋而不重跑。
11. [x] 完成條件：Task／Execute 指令明確說明保證、限制、授權及還原方向，相關測試通過。

### T8：驗證整條流程、相容性與平台限制

1. [x] 任務完成。
2. 依賴：T7。
3. 成果：Issue 全部驗收具備可追溯證據；新增一條從 planning 到下游 Task 的端到端回歸案例，覆蓋整合邊界而非重複單元測試。
4. 原子變更集：新增 `rust/crates/work-cli/tests/task_execution_safety.rs` 或沿用現有適合的 CLI integration suite；必要的 current fixtures／helpers；`rust/crates/work-feature/tests/execution_worktree_flow.rs`；`docs/work/rust/architecture.md` 與 T1 邊界文件的實際能力說明；本文件驗收與執行紀錄。
5. 步驟：核對 Source／Session／TASK／Execution／新交易的現行 bindings；以 derivation API 產生測試證據，不改歷史核准。串測審查、衝突、預覽、授權、發布、驗收、下游 eligibility，以及中斷→拒絕完成→授權還原→重新規劃／執行。
6. 命令／驗證：`cargo test --workspace`；`cargo clippy --workspace --all-targets -- -D warnings`；`cargo fmt --all -- --check`；`cargo test -p work-cli --test architecture_dependencies --test derivation_architecture --test current_only_contract`；`git diff --check`。
7. 平台：至少記錄當前 Windows 的實際測試結果；macOS／其他平台若沒有實測，保持未驗證，不能把 Windows 結果當作跨平台證明。原子替換、路徑 alias 與故障復原能力需符合實測支援範圍。
8. 風險：全 workspace 測試發現非同根因問題不得順便修正；平台不足需列出限制，不能把缺測項目勾選通過。
9. [x] 完成條件：完整正常流程及中斷／還原流程的 integration 測試通過，合法 DAG 下游依完成狀態正確執行。
10. [x] 完成條件：契約、指紋、架構方向、既有 Task／Execute／Specification／Recovery 回歸檢查通過。
11. [x] 完成條件：workspace tests、Clippy、格式及差異檢查全部通過，平台結果及剩餘限制如實記錄。
12. [x] 完成條件：第 6 節全部驗收有實際證據，本文件狀態與執行紀錄一致。

## 6. Issue 最終驗收追蹤

| 狀態 | Issue 驗收 | 主要 Task | 預期證據 |
| --- | --- | --- | --- |
| [x] | Task 產生能檢查粒度並要求合理拆分。 | T2 | 獨立成果、split-required、審查過期與正式化拒絕測試。 |
| [x] | 不可分割多檔可以保留同一 Task。 | T2、T4 | 不按數量切分、多檔可行審查與完整候選測試。 |
| [x] | Validator 偵測跨 Task 共用檔案相依與寫入衝突。 | T3 | 操作矩陣、inputs、alias 與獨立性證據測試。 |
| [x] | 缺必要相依或非法順序拒絕正式化。 | T3 | DAG、無序衝突與非法生命周期測試。 |
| [x] | 合法 DAG 依前置完成狀態執行。 | T6、T8 | eligibility 及下游端到端測試。 |
| [x] | 不可分割多檔具完整提交或失敗還原。 | T5、T6 | 全集合回讀、每階段故障與原始狀態還原測試。 |
| [x] | 無法保證原子性時寫入前阻擋。 | T4、T7 | unsupported／unknown capability／外部效果拒絕測試。 |
| [x] | 中斷、部分失敗及 Recovery 不誤標完成。 | T5、T6、T8 | 每階段中斷、restore 重入、closure 與下游 gate 測試。 |
| [x] | 不能自動判定者逐項確認，不自行推測。 | T2、T3、T7 | needs-confirmation 診斷、無自動 DAG edge 與失效審查測試。 |
| [x] | 新測試全部通過且既有 Task／Execute 無回歸。 | T8 | workspace／Clippy／架構／契約及平台驗證紀錄。 |

## 7. 執行紀錄

| 日期 | Task | 實作結果 | 驗證命令及結果 | 未完成條件／風險 |
| --- | --- | --- | --- | --- |
| 2026-10-08 | T1～T8 完成 | 交付結構化粒度審查、跨 Task 相依／獨立性、唯讀完整候選、保存前後 bytes 與支援 metadata 的發布／還原、CLI 入口與完成 gate；結案復原重驗檔案成果；Discussion 來源的空 file list 不豁免能力檢查；核對目前上游與 Source context，不沿用過期 trace 證據。 | cargo test --workspace -- --test-threads=8：772 項通過、1 項既有 process worker 由子程序驗證；包含 Windows installer、process boundary 48 項、CLI 3 項、Infrastructure 253 項、25 組發布×還原中斷與結案復原 drift。其後上游／Source 綁定正負例、CLI 3 項、四核心 crate 381 項與 workspace doctest 補充回歸通過；最新 Clippy、fmt 與 git diff --check 通過。 | 實測 Windows GNU／目前 filesystem，需具備支援 metadata 的維護權限；權限不足會在 target 寫入前拒絕。macOS／Linux 未實測；逐檔發布可暫時可見，不保證全組瞬間切換或控制外部程序。timestamps／audit SACL 排除於保存範圍；無 Discussion provenance 的既有無檔案宣告執行保留原流程，不具新交易保障。未提交或推送。 |
| 2026-10-08 | T4～T8 整合驗證 | 公開 CLI 覆蓋完整發布、staging drift、中斷→新授權還原→舊 Attempt 不可完成→新 Attempt 發布與驗收→下游啟用；保存 Windows owner/group/DACL 並拒絕 named streams。 | process boundary 48 項、檔案交易 CLI 3 項、原生 25 組發布×還原故障、named-stream 拒絕測試通過；Model／Operations／Feature／Flow 回歸與 Clippy 通過。Windows installer 權限失敗的同項沙箱外重跑通過。 | 當時仍待完整回歸；已由最終驗證列結案。歷史測試使用隔離指令 baseline，新流程使用目前指令；其他平台未實測。 |
| 2026-10-08 | T2～T3 進度 | 結構化粒度 gate、完整規劃指紋、跨 Task 存取與不可變 trace 審查已實作；更新 current fixtures 的審查準備，不改歷史。 | 粒度及存取單元測試、Discussion assembly 8 項測試、current-only／架構／derivation 18 項檢查通過。 | 當時尚待獨立性正例與完整回歸；已由最終驗證列結案。 |
| 2026-10-08 | T4～T7 進度 | 新增四個檔案交易命令、沿用 RuntimeManifest／staging／writer、完成 gate 與無隔離效果拒絕；依授權增加 Windows owner/group/DACL 保存與 named-stream 拒絕能力。 | 原始 25 組發布×還原中斷及正常／drift／alias 測試通過；Windows 精確 security readback 單項測試通過；workspace Clippy 通過。 | 當時尚待權限補強後故障與 CLI 回歸；已由最終驗證列結案。 |
| 2026-10-08 | T1 | 新增 `docs/work/task-execution-safety.md`，固定審查欄位、指紋、命令、完整交易與還原邊界；已取得 T1～T8 整體及範圍調整授權。 | 對照 D1～D5、十項驗收與既有架構；`git diff --check` 通過。 | 當時仍待實作與平台驗證；已由最終驗證列結案。 |
| 2026-10-08 | 規劃準備 | 讀取 Issue、唯讀盤點、逐項確認 D1～D5；建立本 task 文件。 | 唯讀腳本確認 T1～T8 順序、48 個未完成標記、80 個既有或明確標示新增的路徑，以及 UTF-8／行尾檢查通過；`git diff --check` 通過。尚未執行實作測試。 | T1～T8 均未開始；本次沒有實作授權。 |

## 8. 安全規格文件整併

1. [x] 任務完成：將安全規格整併至 Rust 架構文件，移除獨立文件。
2. 原子變更集：`docs/work/rust/architecture.md`、原 `docs/work/task-execution-safety.md` 與本文件。
3. 步驟：完整保留審查、發布、還原與支援限制；驗收對照放入測試章節；更新現行引用，保留歷史紀錄。
4. 驗證：逐項比較原條文、檢查現行引用與 Markdown 路徑，執行 `git diff --check`。僅文件整併，不需重算 artifact 指紋或重跑 Cargo。
5. 風險：不得遺漏安全條文；原文件路徑只保留於歷史紀錄與整併說明。
6. [x] 完成條件：原安全文件的全部 20 項條文完整保留，四個主題均有對應章節。
7. [x] 完成條件：獨立文件已移除、現行引用有效，歷史執行紀錄未改寫。
8. [x] 完成條件：文件差異檢查通過。
9. 驗證紀錄：2026-10-08，逐字比對原 20 項條文全部保留；全專案搜尋確認原路徑僅剩歷史紀錄與整併說明；現行文件及章節存在，`git diff --check` 通過。未修改程式或指令來源，未重跑 Cargo。

## 9. 架構說明精簡

1. [x] 任務完成：依使用者要求，將本次整併內容精簡為架構說明。
2. 原子變更集：Rust 架構文件與本文件；移除本次新增的詳細欄位、命令程序、平台實測敘述與驗收對照，保留職責、資料流及保障邊界。
3. 驗證：檢查架構內容、現行引用及 `git diff --check`；僅文件調整，不重跑 Cargo。風險為過度刪減責任邊界，須核對保留內容。
4. [x] 完成條件：保留審查、相依、交易、還原與完成判定的責任分工，不再逐項重述安全規格。
5. [x] 完成條件：現行引用有效、差異檢查通過；第 8 節記錄的是精簡前已完成的歷史整併結果。
6. 驗證紀錄：2026-10-08，確認五項責任與交易資料流均保留，詳細安全規格及新增驗收對照已移除；現行引用章節存在，`git diff --check` 通過。未修改程式或指令來源。

## 10. CI 與審核修復

1. [x] R1：修復 Clippy 布林式。檔案：`work-operations/src/execution/file_transaction.rs`。原等價簡化已通過檢查，R6 最終改為能力判定並移除該布林式。完成條件：2026-10-08 最新 `cargo clippy --workspace --all-targets -- -D warnings` 通過。
2. [x] R2：綁定 Task Instruction Selection。檔案：Task 相依純規則、Discussion 衍生與相關測試。以 selected_paths、references、instructions_sha256 綁定，正式解析細節仍由現有 instruction validator 核對。2026-10-08：自身與上游 Revise 選擇／指紋變更反例及未變正例共 3 項單元測試、Discussion assembly 8 項回歸、相關 Clippy 通過。風險：規劃與正式指令形狀不同，不能直接比較整份 JSON。
3. [x] R3：授權及控制資料的中斷復原。檔案：交易儲存、新 control_commit adapter 與故障測試。步驟：完整暫存、同步、不可覆蓋的原子提交；新暫存及舊部分授權均不授予權限，復原重新核准。完成條件：完整 Manifest 下的 Candidate／Authorization／狀態更新中斷可恢復；損壞 Manifest 在 target 寫入前停止並保留原資料。風險：缺少完整 before evidence 時不能推測還原。
4. [x] R4：交易資源可行性。檔案：Feature 交易準備、Infrastructure 儲存／資源／記憶體 adapter、借用序列化與 Cargo.toml。步驟：以 bytes、路徑及 metadata 估算記憶體／暫存／復原需求；準備、發布及復原前重驗 frozen before／after、限制控制 JSON 讀取、避免完整 bytes 的 Value 複製與多候選累積。完成條件：資源不足在 target 寫入前拒絕、容量邊界及原生大型檔案反例通過。風險：容量可被其他程序消耗；既有 fs4 由測試依賴提升為執行依賴，沒有新增套件或版本。
5. [x] R5：成果、驗收與修改範圍的語意一致性。檔案：Discussion 模型／驗證／契約及 fixtures。使用者已選定結構化對應；步驟：驗收、檔案、scope 完整覆蓋，多個獨立成果拒絕 single_outcome；不可分割須綁定成果及具體分離後果，不確定要求確認。完成條件：錯誤宣告、遺漏／錯誤對應反例拒絕、合法多檔正例通過。風險：對應證據不能證明所有自然語言敘述的真實性。
6. [x] R6：可驗證執行能力。檔案：Execution 純規則、Model 內部能力、Feature repository ports、CLI／fake-port 與原生儲存測試及精簡架構說明。步驟：唯讀／隔離能力由可信 executor 提供，核對偏差與命令修正後的實際 invocation，Task JSON 不可自授；不再依 Discussion trace 或 files 是否為空判定。完成條件：可信能力正例、未知能力拒絕、手動驗收與歷史紀錄流程回歸通過。風險：目前原生 executor 沒有隔離能力，因此未知本機命令保持拒絕；不新增隔離服務。
7. [x] 範圍調整：依使用者最新指示，移除 GitHub CI 證據核對功能、兩個新增檔案、模型／契約／呼叫整合及其專屬測試。專案測試仍可由既有 GitHub CI 執行，Work 不連線核對、不提交、推送或觸發 workflow。全專案搜尋未留下本次功能的型別／呼叫。
8. [x] 共通驗證：每項對應 crate／integration 測試、workspace Clippy、`cargo fmt --all -- --check`、`git diff --check` 均通過；測試只使用既有 fake ports 與測試暫存專案。架構文件僅保留職責、資料流及限制。
9. 最終驗證：workspace 回歸 780 項通過、1 個既有 worker 由子程序驗證；包含 CLI 48 項、檔案交易端到端 3 項、Windows installer 9 項、Infrastructure 259 項、Model 86 項、Operations 209 項及 Feature 回歸。首次失敗是舊測試預期未知原生命令可執行，已依新需求改為拒絕且保存狀態，單獨及完整重跑均通過。完整測試啟動後補上 R4 對 frozen before／after 的資源重驗；最後版本另跑交易 11 項（282.92 秒）及 CLI 3 項（33.52 秒）全部通過，含大型保存內容拒絕、部分授權、Candidate／Manifest 保留與 25 組發布×還原故障；最新 Clippy、格式、差異與 GitHub 功能殘留檢查通過。R1～R6 全部完成。
10. 能力及平台紀錄：weighted before／after bytes、路徑及 metadata 預算上限 8 MiB，控制檔讀取上限 64 MiB；Windows security snapshot 上限 4 KiB。資源需求使用保守序列化／複製預算並核對目前可用記憶體及空間，不保證其他程序不消耗容量。實測 Windows GNU；macOS／Linux 的原生能力未實測，未將本機結果宣稱為遠端 CI 結果。未提交、推送或觸發 workflow。

## 11. macOS metadata 完整性修復

1. [ ] R7：保存 macOS mode、owner/group，拒絕無法保證還原的 ACL、extended attributes、檔案 flags 與查詢失敗；Linux 及其他 Unix 不支援原生檔案交易。
2. 原子變更集：FileMetadata 模型、候選序列化測試、原生交易儲存、新 macOS metadata adapter、macOS 測試、精簡架構說明及本文件。
3. 步驟：ownership 證據綁入完整候選；讀取時拒絕未支援 metadata；暫存檔先還原 owner/group 再還原 mode 並回讀；發布與 Recovery 在整組 target 寫入前預檢。
4. [ ] 完成條件：舊或不完整 ownership 證據拒絕，未支援 metadata 不得造成部分 target 寫入；新增反向與還原測試。
5. [ ] 完成條件：Windows 交易／CLI 回歸、Model／Operations 測試、Clippy、fmt 及差異檢查通過。
6. [ ] 完成條件：macOS 原生 owner/group、ACL、xattr、flags 及發布／還原預檢測試實測通過。
7. 風險：macOS 權限或 metadata 查詢不足時拒絕；不保存 timestamps 或 inode 身分，另拒絕獨立 UUID ownership。舊交易保留原始證據，不回寫或推測 ownership。
8. 本機驗證紀錄：Model 86 項、Operations 209 項、ownership 證據反向測試 1 項及 CLI 端到端 3 項通過；Windows 交易 10 項通過、1 項大型資源案例失敗，實際為 `file_transaction_memory_insufficient`，測試原預期 `file_transaction_resource_limit`。控制 JSON 讀取的記憶體檢查先拒絕，未降低 assertion 或修改資源 gate。Windows workspace Clippy、fmt 及差異檢查通過；macOS adapter 與三項原生測試尚未編譯／實測。
9. [x] CI 驗證接線確認：依使用者要求，後續檢查由既有 `.github/workflows/work-rust-macos.yml` 與 `work-rust-windows.yml` 執行。macOS job 的 `cargo check --workspace --all-targets --locked`、Clippy 與 `cargo test --workspace --locked` 自動涵蓋新 adapter 及原生 metadata 測試，無須新增重複 workflow。未提交、推送或觸發 CI；R7 及待驗證完成條件保留未完成，取得對應 commit 的 CI 結果後才能結案。

## 12. 跨平台交易與正式命令隔離修復

1. 授權：2026-10-08，使用者授權以下六項整體計畫。隔離邊界為專案唯讀、僅本次 Attempt staging／暫存可寫、禁止網路及區外寫入，無法強制時拒絕。不新增套件，不提交、推送或觸發 CI。
2. [ ] F1：修復 restore metadata Inventory。檔案：Infrastructure `execution/file_transaction_storage.rs`。步驟：綁定合法名稱、target index 與 before bytes，保留並拒絕損壞及外來暫存。完成條件：[ ] 中斷還原可重入；[ ] 偽造、損壞及外來檔案拒絕且保留。
3. [ ] F2：修復 macOS Recovery 測試。檔案：同上。步驟：由真正發布中斷建立還原案例。完成條件：[ ] ownership／預檢測試通過；[ ] PublishedVerified 仍拒絕 Recovery。
4. [x] F3：修復 macOS UUID 判斷。檔案：`execution/project_file_metadata_macos.rs`。步驟：驗證 UUID 與數值 ownership 對應，查詢失敗或不一致拒絕。完成條件：[x] 零 UUID 與精確數值映射規則通過；[x] 不一致、查詢失敗與舊 metadata 證據拒絕。驗證：metadata 純規則 3 項通過；完整原生 metadata 仍受下述 xattr 環境限制。
5. [ ] F4：接入正式 CLI 原生隔離。檔案：process、新增平台 sandbox adapters、execution storage、必要 Feature／Model 契約及 CLI 整合測試。步驟：以可信 executor 綁定實際 invocation／隔離政策，macOS Seatbelt、Windows AppContainer／Job Object，檢查子程序、越界寫入、網路、逾時與不可用時拒絕。完成條件：[x] macOS 正式 CLI 可執行隔離命令；[x] macOS 專案／區外寫入與網路拒絕；[x] macOS 一般子程序、逾時及 detached capture 測試通過；[ ] Windows 原生驗證通過。風險：macOS API 棄用、平台權限與工具相容性；detached 子程序仍受 Seatbelt 限制，但不宣稱程序群組能終止已另建 session 的所有子程序。
6. [ ] F5：修復 Windows metadata 與資源契約。檔案：`execution/project_file_metadata.rs`、resources、file transaction 測試。步驟：精確保存安全設定並驗證回讀，分開固定容量超限與主機記憶體不足。完成條件：[x] security 表示差異與真實權限差異有反例；[x] 資源拒絕順序可確定驗證；[ ] Windows 交易及 CLI 原生回歸通過。驗證：security 純規則及資源 2 項通過；超大 frozen evidence 與控制 JSON 記憶體 gate 分別驗證，不放寬 reason code assertion。
7. [ ] F6：原生整合與中斷復原驗證。檔案：相關 CLI 測試、`docs/work/rust/architecture.md` 與本文件。步驟：macOS metadata、發布×還原中斷及 CLI 回歸；`cargo +1.85.0 test --workspace --locked`、`check --workspace --all-targets --locked`、`clippy --workspace --all-targets --all-features --locked -- -D warnings`、`fmt --all -- --check`、`git diff --check`。完成條件：[ ] macOS 原生／中斷／CLI 實測通過；[ ] workspace、Clippy、check、fmt、diff 通過；[ ] Windows 原生驗證通過；[x] 紀錄區分本機實測與待驗證平台。
8. 驗證範圍：測試只操作隔離暫存專案。Windows 原生結果未取得前維持未完成，不將 macOS 結果當作 Windows 證據。
9. 2026-10-08 原生驗證進度：macOS 隔離 3 項沙箱外通過，含 staging 可寫、原始檔案／區外／symlink／子程序寫入拒絕、loopback 網路拒絕、逾時終止子程序及 bounded capture；平台沙箱內 `sandbox_apply` 被拒絕，沒有啟動使用者命令。metadata 測試在沙箱內及外均因本機為新檔案附加 `com.apple.provenance` 而在 xattr gate 拒絕；維持正式拒絕規則，沒有移除屬性或跳過測試。
10. 範圍補充：依使用者要求，將 `src/process.rs` 移至 `src/process/mod.rs`，平台 adapter 位於 `src/process/isolation/{macos,windows}.rs`，維持原有模組 API。
11. 2026-10-08 正式 CLI 隔離整合 1 項沙箱外通過：預覽不建立 sandbox、錯誤 SHA 不啟動、合法 staging 產出、越界拒絕、完整 started／finished receipts 及禁止重跑。workspace `check --all-targets --locked`、Clippy `--all-targets --all-features -- -D warnings` 通過；後續 capture／Windows 測試補強後需重驗。
12. 補強驗證：macOS 隔離 6 項沙箱外通過（含 1 個由子程序實際驗證的 worker）：核准後 argv／cwd／timeout 漂移拒絕、kernel 限制、一般子程序逾時及 detached inherited pipe 的 bounded capture。首次 workspace 回歸中 CLI unit 60、架構／契約 18、installer 20 通過；process boundary 48 通過、1 個新調整的 shell fixture 因替代命令未保留模式而失敗。已搜尋同模式修改位置，修復 fixture，待重驗；未修改正式 mode 契約。
13. 後續回歸：shell fixture 單項重跑通過；workspace `--no-fail-fast` 完整執行後僅 `task_execution_safety` 與 Infrastructure lib 失敗。後者以 terse 重跑確認 275 通過、14 失敗、1 個既有 worker ignored 且由子程序驗證；13 個失敗皆為同一 xattr gate，1 個為舊 Task storage 測試仍預期 native argv 不可隔離。依既有新需求修正該整合測試，保留沒有可信 executor 的拒絕，驗證 native 預覽唯讀及錯誤核准拒絕；不放寬正式規則。CLI 檔案交易 3 項重跑皆為 xattr gate，沒有降低 assertion、刪除或跳過案例。
14. 最後驗證：Task storage 整合測試修復後單項通過，沒有原生 backend 的平台仍預期拒絕。隔離重跑發現測試時間戳目錄碰撞，為同一 fixture 加入程序內原子序號後，macOS 隔離 6 項及正式 CLI 隔離 1 項全部重跑通過；metadata 4 項、資源 2 項、temporary Inventory 純規則 1 項通過。最新 workspace check、Clippy、fmt 與差異檢查通過。完整交易驗證仍未通過，需不自動附加 provenance 的 macOS 環境；Windows adapter 及原生 security／CLI 尚未在 Windows 編譯與執行，不以 macOS 靜態檢查代替。F1、F2、F4、F5、F6 保持未完成；未提交、推送或觸發 CI。
