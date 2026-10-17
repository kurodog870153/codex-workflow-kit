# Issue 82：Rust 測試與 CI 效能任務

來源：[Issue 82](https://github.com/kurodog870153/codex-workflow-kit/issues/82)。本文件依相依順序安排工作；所有任務與完成條件在實際驗證前維持未完成。每項任務執行時，須記錄量測環境、命令、結果及對應的 PR／CI Run。效能改善不設固定門檻，但不得減少測試或既有品質檢查。

## 已盤點的範圍

1. `.github/workflows/work-rust-macos.yml`：macOS 的格式、check、clippy、workspace test、release build 與 smoke test；目前 `push`、`pull_request`、`workflow_dispatch` 均觸發。
2. `.github/workflows/work-rust-windows.yml`：Windows MSVC／GNU matrix，與 macOS 同類檢查；目前每個 target 分別執行 `cargo test --workspace --locked --target ...`。
3. `rust/Cargo.toml`、`rust/Cargo.lock`、六個 `rust/crates/*/Cargo.toml`：workspace、工具鏈版本與依賴；若評估新增測試工具，須核對鎖檔及安裝成本。
4. `rust/crates/*/src/**/*.rs`、`rust/crates/*/tests/*.rs`：unit 與 integration tests。優先調查 `work-cli/tests/process_boundary.rs`、`task_execution_safety.rs`、`current_only_contract.rs`，以及 `work-infrastructure/src/fixture_support.rs` 和大量檔案／程序操作的測試；這只是靜態盤點候選，不能當作已確認瓶頸。
5. `rust/crates/work-infrastructure/fixtures/`、`rust/crates/work-infrastructure/fixtures/README.md`：fixture 建立與使用規則；效能修正須保留隔離、清理及跨平台行為。
6. `docs/work/rust/architecture.md`：六 crate 責任與依賴界線；若量測證實需要改善正式程式可測試性，須依此維持架構及公開行為。

## 依序執行的任務

### 1. [x] 建立可比較的三平台測試基準

預計檔案：`docs/issue-82/` 下的量測紀錄，以及必要的唯讀 CI 資料；暫不修改正式程式。執行：檢查 Issue 所列成功 Run，記錄 runner、commit、工具鏈、target、workflow 設定；用同一口徑區分測試編譯、unit／integration／doc test 執行時間，蒐集最慢測試與重複初始化、I/O、等待、fixture／mock、重複編譯證據。三平台以可比較的多次觀測呈現波動，先分析 Windows MSVC。命令／外部操作：唯讀 `gh run view`、`gh api`；獲授權後於適用環境執行分階段 Cargo 量測。驗證：原始 log 與紀錄中的數值可互相核對。風險：GitHub-hosted runner 波動、各平台可用環境不同。

- [x] 三平台各有工具鏈、commit、測試範圍與分階段時間紀錄，明確區分編譯與測試執行（證據：`baseline.md` 與所列 CI Run）。
- [x] unit、integration、doc tests 與最慢測試有可追溯證據；列出已確認瓶頸及未能量測的部分（證據：`baseline.md` 的 CI log 與本機 nextest 量測）。
- [x] 比較方法與樣本波動已記錄，不以單次 Run 推論穩定改善（證據：`baseline.md` 的同 commit 重複量測）。

### 2. [x] 定義效能報告資料契約與擷取方式

依賴：任務 1。預計檔案：`.github/workflows/work-rust-*.yml`、必要時新增 `.github/scripts/` 下共用報告程式及其測試。執行：定義版本化 JSON 欄位（Run／commit、平台／target、工具鏈、測試範圍、各階段耗時、逐測試耗時、狀態與資料缺漏原因），決定從 Cargo／測試輸出可靠擷取的方式；保留原始測試退出碼。命令：報告程式的離線 fixture／單元測試與格式檢查。驗證：成功、測試失敗及輸出不完整樣本都產生合法報告或明確缺漏狀態。風險：測試輸出格式及平台差異。

- [x] JSON schema／範例與擷取程式可表示三平台、編譯／測試時間、逐測試時間及失敗／缺漏狀態（證據：`report-schema.md`、`ci_perf.py`）。
- [x] 報告測試涵蓋成功、測試失敗與不完整輸出；不會把解析失敗誤報為測試成功（證據：`test_ci_perf.py` 的 7 項測試通過）。

### 3. [ ] 在每次 CI 產生 Summary 與 Artifact

依賴：任務 2。預計檔案：兩個 Rust workflow、任務 2 的報告程式。執行：在 macOS、Windows MSVC／GNU 的 PR 與 main 執行中，擷取時間及最慢測試排名，寫入 Job Summary，將 JSON 以 `actions/upload-artifact` 上傳並設 `retention-days: 90`；測試失敗時仍盡可能保留診斷與報告，最後維持原始失敗狀態。命令／外部操作：workflow 語法檢查、報告測試；獲授權後觸發或觀察三平台 Actions Run。驗證：三平台成功／失敗路徑的 Summary、Artifact 與 Job 結果。風險：artifact 上傳或報告失敗不可掩蓋測試失敗。

- [ ] 三平台每次 PR／main Run 均有分階段時間、最慢測試與結果的 Job Summary，以及可讀 JSON Artifact，保留 90 天。
- [ ] 測試失敗時盡可能留下報告；原測試、編譯及品質檢查失敗仍使 Job 失敗。
- [x] Artifact 不提交至 repository、不當 cache，也不參與測試通過判定（證據：`.gitignore`、workflow 與 `test_enforcement_uses_test_evidence_not_uploaded_report`；遠端上傳仍待驗證）。

### 4. [ ] 加入安全的 main 歷史中位數比較

依賴：任務 2、3。預計檔案：兩個 Rust workflow、報告／比較程式及其測試。執行：PR 按平台取得 main 最近 5 次**成功且有效**的 CI JSON，核對 schema、測試範圍、工具鏈及 CI 設定相容性後計算測試時間中位數；本次時間嚴格大於中位數 130% 才顯示警告和差異百分比。少於 5 筆顯示「歷史資料不足」，不相容則標示不可比較。使用最小 `actions: read` 權限，隔離不可信 PR 程式碼與較高權限 token／secrets。命令／外部操作：比較程式離線測試、workflow 權限審查；獲授權後以 PR Actions Run 驗證。風險：fork PR 權限、過期 Artifact、API 限額及歷史資料缺口。

- [ ] 各平台只採用最近 5 筆相容且成功的 main 紀錄，計算正確中位數及嚴格 `> 130%` 判定。
- [ ] 資料不足、不相容、下載或統計失敗時顯示明確狀態，且不阻擋 PR。
- [x] 歷史讀取權限及執行路徑經審查，不讓不可信 PR 程式碼使用高權限 token／secrets（證據：workflow 僅 `contents: read`、`actions: read`，checkout 不保存憑證，使用一般 PR token，跨 host Artifact 轉址移除 Authorization；遠端權限仍待驗證）。

### 5. [ ] 依量測結果優化 CI 重複工作

依賴：任務 1、3。預計檔案：兩個 Rust workflow；僅在實測需要時調整 `rust/Cargo.toml` 或相關設定。執行：檢查 check、clippy、test、release build 的順序、並行與同一次 Run 的編譯產物重用；每一項變更保留格式、lint、完整 workspace unit／integration／doc tests、release build 與 smoke test。比較變更前後三平台分階段時間及變異。命令／外部操作：Cargo 檢查與完整測試；獲授權後觀察多次 Actions Run。風險：Cargo profile／target 不同可能不能安全共用產物。

- [ ] 每項採用的 CI 調整均有同條件前後量測與原因，無效調整不保留。
- [ ] PR 與 main 每次在三平台完整執行既有測試及品質檢查；沒有跨次 CI cache 或付費 runner。

### 6. [ ] 依已確認熱點優化測試實作

依賴：任務 1。預計檔案：由量測定位的 `rust/crates/*/src/**/*.rs`、`rust/crates/*/tests/*.rs` 及相關 fixture；寫入前逐一確認實際範圍。執行：針對已證實的等待、I/O、重複初始化或 fixture 成本做最小修正；如需正式程式可測試性調整，維持六 crate 架構與外部行為。命令：受影響測試、完整 `cargo test --workspace --locked`、fmt／clippy／check，以及三平台 CI。驗證：測試數量、語意、隔離與失敗偵測保持一致，時間改善有可比證據。風險：平行化或共用 fixture 可能引入競態及跨平台差異。

- [x] 僅修改量測證實的熱點，並記錄每項改善前後耗時（證據：`baseline.md` 的單項、完整 Cargo 與 nextest 前後量測）。
- [ ] 原測試目的、失敗偵測、fixture 隔離、公開行為及跨平台結果維持正確。

### 7. [ ] 評估額外測試工具並決定是否採用

依賴：任務 1；若採用，須先於任務 5、6 的最終驗證前完成整合。預計檔案：評估紀錄；採用時才修改兩個 workflow、必要的工具版本設定與相關報告程式。執行：以相同範圍實測 `cargo-nextest` 等成熟工具相對 `cargo test` 的時間及維護成本，確認 doc tests、輸出擷取、失敗語意與三平台支援；僅在有實測淨效益時導入並補齊未涵蓋的 doc tests。命令／外部操作：獲授權後安裝／執行候選工具及三平台 CI。驗證：比較紀錄與完整測試清單。風險：新增安裝時間、版本維護與輸出格式變動。

- [ ] 有同條件效能、安裝時間、覆蓋率與維護成本比較，明確記錄採用或不採用理由。
- [ ] 若採用，三平台完整 unit／integration／doc tests 及原有失敗語意均通過；若無淨效益，不增加工具依賴。

### 8. [ ] 完成端到端驗收與成果紀錄

依賴：任務 3 至 7。預計檔案：`docs/issue-82/` 的結果紀錄及本 task 狀態；如有驗收發現，限修正前述任務範圍內的檔案。執行：核對 PR 與 main 的三平台完整測試、既有檢查、Summary／90 天 JSON Artifact、歷史比較的正常／不足／失敗情境；彙整前後分階段耗時、波動、主要瓶頸與實際改善。命令／外部操作：本機相關檢查及獲授權的 Actions Run／唯讀查核。驗證：逐項對照 Issue 82 驗收標準與 Run／Artifact 證據。風險：main 五次成功紀錄可能尚未累積，須將未實證條件保持未完成。

- [ ] PR 與 main 在 macOS、Windows MSVC、Windows GNU 的完整測試與既有檢查均有可追溯成功證據。
- [ ] 三平台 Summary、JSON Artifact、90 天保留及歷史中位數警告／安全降級均有實際驗證證據。
- [x] 成果紀錄包含可比較的優化前後分階段時間、樣本波動、主要瓶頸與改善作法；未驗證項目仍標示未完成（證據：`baseline.md`；三平台修改後 CI 尚待執行）。
