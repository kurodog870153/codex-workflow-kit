# Issue 82 CI 基準量測

來源是 GitHub Actions 成功 Run 的 step 時間及 `cargo test` log。下表包含不同 commit 的觀測及同 commit `081667b` 的重複量測。不同 commit 只能用來找瓶頸；同 commit 結果可呈現 runner 波動，尚不是優化前後比較。

| Run／commit | 平台 | `Test Rust workspace` step | Cargo `test` profile 編譯 | `work-infrastructure` unit tests |
| --- | --- | ---: | ---: | ---: |
| [39018043067](https://github.com/kurodog870153/codex-workflow-kit/actions/runs/39018043067)／`d989b67` | macOS | 15m 30s | 1m 35s | 509.92s（325 passed，1 ignored） |
| [39018043066](https://github.com/kurodog870153/codex-workflow-kit/actions/runs/39018043066)／`d989b67` | Windows MSVC | 32m 22s | 2m 30s | 1085.11s（309 passed，1 ignored） |
| [39018043066](https://github.com/kurodog870153/codex-workflow-kit/actions/runs/39018043066)／`d989b67` | Windows GNU | 16m 14s | 1m 24s | 547.29s（309 passed，1 ignored） |
| [39020533263](https://github.com/kurodog870153/codex-workflow-kit/actions/runs/39020533263)／`081667b` | macOS | 14m 38s | 1m 38s | 455.45s（325 passed，1 ignored） |
| [39020533233](https://github.com/kurodog870153/codex-workflow-kit/actions/runs/39020533233)／`081667b` | Windows MSVC | 32m 07s | 2m 30s | 1070.93s（309 passed，1 ignored） |
| [39020533233](https://github.com/kurodog870153/codex-workflow-kit/actions/runs/39020533233)／`081667b` | Windows GNU | 31m 12s | 2m 22s | 1042.95s（309 passed，1 ignored） |
| [39044849975](https://github.com/kurodog870153/codex-workflow-kit/actions/runs/39044849975)／`081667b` | macOS | 16m 37s | 1m 18s | 529.61s（325 passed，1 ignored） |
| [39044857578](https://github.com/kurodog870153/codex-workflow-kit/actions/runs/39044857578)／`081667b` | Windows MSVC | 16m 13s | 1m 21s | 520.21s（309 passed，1 ignored） |
| [39044857578](https://github.com/kurodog870153/codex-workflow-kit/actions/runs/39044857578)／`081667b` | Windows GNU | 16m 43s | 1m 33s | 539.59s（309 passed，1 ignored） |

`Test Rust workspace` step 包含編譯、所有 unit／integration tests、doc tests 與 Cargo 啟動成本，不能直接當純測試時間。首輪 log 顯示 macOS 的 `installer`、`process_boundary`、`task_execution_safety` integration binary 各約 144.54s、110.59s、61.92s；MSVC 各約 248.80s、241.38s、203.31s；GNU 各約 134.95s、110.22s、93.23s。MSVC 的 `work-infrastructure` unit binary 耗時最大，包含多個超過 60 秒的復原／交易邊界測試。libtest 目前沒有輸出逐測試精確耗時；`has been running for over 60 seconds` 只能當下限，不能據此精確排名。

上述 CI 皆使用 workflow 固定的 Rust 1.85.0、`cargo test --workspace --locked`；macOS 以 runner 原生 target 執行，Windows 明確指定 `x86_64-pc-windows-msvc` 與 `x86_64-pc-windows-gnu`。Cargo 的 `test` profile 完成訊息是編譯階段，從其後第一個 `Running` 到最後一個 doc test 結果是測試執行階段；表中的主 unit binary 時間是 libtest 自行回報，不等於全部測試時間。

本機在 `2de26e3`、macOS arm64、Rust 1.85.0 執行 `cargo test --workspace --locked --no-fail-fast` 成功，總耗時 356.58s；`work-infrastructure` unit binary 257.51s。此結果不是 GitHub-hosted runner 基準，不能與表內數值直接相比。

同一工作樹以官方預編譯 `cargo-nextest 0.9.148` 執行 `cargo nextest run --workspace --locked --no-fail-fast`，926 項測試全部通過、1 項 ignored，耗時 312.88s，較上述 Cargo 本機執行少 43.70s（約 12.3%）。原 `cargo test` 另執行 1 項 doc test，nextest 不涵蓋；若採用，必須獨立執行 `cargo test --workspace --locked --doc`。此比較不包含 nextest 下載／安裝時間，亦未證明 Windows 上有淨效益。

本機 nextest 逐測試排名前三：`specification::reconciliation_storage::tests::migration_recovery_preserves_one_approved_set_at_each_write_boundary` 222.609s、`work-cli::process_boundary` 的 `reconciliation_public_commands_complete_both_approved_paths_and_recover` 95.177s、`execution::file_transaction::storage::tests::every_publication_boundary_restores_and_every_restore_boundary_reenters` 62.126s。這些測試遍歷多個發布／復原邊界，主要瓶頸屬反覆檔案與程序操作；在三平台 CI 尚無逐測試精確耗時，不能把本機排名直接套用到 Windows。

同一 commit 的兩次 CI Run，MSVC 主測試 binary 約 1070.93s 與 520.21s；GNU 約 1042.95s 與 539.59s，差距接近兩倍。後續比較必須保存 runner 映像、工具鏈、target、測試集合與 CI 設定；`d989b67` 與 `081667b` 的程式及測試範圍不同，不能歸因於單一實作。

## 本機修改後驗證

工作樹在上述三項長測試拆分後，仍以 macOS arm64、Rust 1.85.0 驗證。所有原有邊界組合保留：遷移寫入邊界拆成 5 項、發布／還原故障組合拆成 5 項、六種 journal 類型拆成 6 項。每項仍建立自己的 fixture；這讓 nextest 能並行執行原本單一測試內的獨立案例。遷移原單項 222.609s；拆分後 5 項各約 61–65s，但五項會並行，不能將個別耗時相加當作測試階段耗時。

| 本機命令／階段 | 修改前 | 修改後 | 差異 |
| --- | ---: | ---: | ---: |
| `cargo test --workspace --locked --no-fail-fast` 全程 | 356.58s | 237.90s | -118.68s（-33.3%） |
| `cargo nextest run --workspace --locked --no-fail-fast` 測試階段 | 312.88s | 165.99s | -146.89s（-46.9%） |
| 修改後 `cargo test --workspace --locked --doc` | — | 6.223s | 1 項 doc test 通過 |

修改後完整 Cargo 測試與 nextest 均通過；nextest 有 939 項通過、1 項原有 ignored，另有 1 項 doc test 通過。報告程式讀取本機 JUnit 與三階段退出碼，產生 940 筆（含 ignored）逐測試資料；`test_execution_seconds` 為 172.380s，狀態 `passed`。`cargo check --workspace --all-targets --locked`、`cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`、`cargo fmt --all -- --check` 均通過。

nextest 固定為 0.9.148，CI 安裝步驟使用預編譯工具並另跑 doc tests。這次本機比較僅有各一筆，不含下載／安裝成本；官方發行檔約 16.7 MB，GitHub-hosted runner 的安裝時間及 Windows 淨效益尚待新 workflow 的三平台 Run 驗證。`cargo test --no-run` 與 nextest 在同一次 Run 共用 Cargo test profile 產物；既有 fmt、check、clippy、release build、smoke test 均留在 workflow 中。三平台 CI 的最終效益不能用本機單次數值推論。

## 首次推送的 Windows CI 診斷

[Run 39067405714](https://github.com/kurodog870153/codex-workflow-kit/actions/runs/39067405714)（`b0d94bb`）的 MSVC 與 GNU 均完成 fmt、check、clippy 和測試編譯；nextest 開始後，量測腳本將 Unicode 輸出轉送至 cp1252 主控台時拋出 `UnicodeEncodeError`，未寫入 test 階段紀錄與 JUnit。MSVC 的 doc tests 通過 1 項；GNU 的 doc 階段亦因相同輸出編碼問題而缺少結果。兩個 job 最終由 enforce 步驟判為失敗，release build 和 smoke test 未執行。Artifact 上傳也略過隱藏的 `.ci-perf` 目錄。修正加入 cp1252 回歸測試、讓主控台不支援的字元以 escape 形式顯示，並明確上傳隱藏的量測檔；三平台遠端結果仍須下一次 Run 驗證。
