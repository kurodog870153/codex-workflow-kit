# Infrastructure fixtures

1. `cases/<domain>/<feature>/<case>/` 保存單一測試情境。`input/` 是請求與 setup 輸入；`expected/` 是唯讀預期結果；`project/` 是專案根目錄樣本，正式 Work 路徑從其 `outputs/work/` 開始。case 外層路徑不進 artifact、request 或 approval。
2. `shared/<name>/` 保存已確認由多個測試共用的唯讀素材。`task-diagnostics-project` 與 `specification-migration-project` 是明確專案根目錄；`specification-reconciliation-inputs` 保存共用 Attempt、Execution index 及 migration preview。測試明確指定資源，不跨讀其他 case 的 project。
3. `historical/` 保存歷史 bytes、舊契約拒絕證據與保留的 instruction baseline。用途包含 Delegation 的退休 plan／progress-saver、Task assembly 與 draft-sources、Reconciliation 的原始 Attempt、Handoff 的 Execute instruction baseline。負例中的舊資料也可位於 case project；分類不會解除精確 SHA 保護。
4. 測試只將必要 project 素材複製到獨立暫存專案，再於暫存專案執行寫入與 recovery。input 與 expected 分別載入，不整包複製到專案，不回寫 fixture 原件。唯讀流程可直接使用明確 project 根目錄，並驗證前後 bytes 不變。
5. `fixture_support::copy_fixture_sources` 接受實際 project 根目錄，複製完整的 Source Snapshot triplets：manifest、完成 marker 與原始 content bytes。必要 project 或 Source 缺失會失敗。Migration provenance 或 ledger-only 等明確無 Snapshot 情境不呼叫此 helper，也不虛構 Source。
6. `work-cli/tests/current_only_contract.rs` 保護精確歷史位置與完整 SHA，掃描現行 JSON 與嵌入 stdout，維持退休契約拒絕。Reconciliation 的 reviewed journal 從 case 的 `input/publication.json` 綁定 `project/` 正式路徑；未審查 journal 必須失敗，不以檔名或整個 historical 目錄放寬。
7. 新案例依既有 domain／feature 與情境命名，僅建立實際需要的 input、expected 或 project。`latest-attempts` 的預期結果由正式 project 資料衍生，沒有虛構 input。需要 Source 時保留真實 Snapshot triplets；刻意損毀 bytes 與歷史指紋不重新序列化或修復。現行指紋變更使用既有 fixture builders，完整準備後才更新一個 case。
8. Issue #79 的 225 筆原始檔案與最終位置映射、子任務及驗證紀錄見 repository 的 `docs/issue-79/task.md` 附錄 F。Task assembly 中沒有一般 loader 的伴隨證據仍保留，不能因未被直接載入而刪除。

在 `rust/` 執行完整驗證：

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
```

驗證前後應確認 fixture payload bytes 不變；歷史 digest 不以重新計算的新值替換原始證據。
