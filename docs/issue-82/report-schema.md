# CI 效能報告契約

`.github/scripts/ci_perf.py` 產生 `schema_version: 1` 的 `performance.json`。每個平台 Job 各產生一份；JSON Artifact 只供診斷與歷史效能比較，不決定測試結果。

1. `run_id`、`commit`、`workflow`：GitHub Run 與來源版本。
2. `platform`、`target`、`runner_os`、`runner_arch`、`runner_image`、`toolchain`：執行環境。
3. `config_id`：測試工具、範圍與 CI 設定識別。`test_ids_sha256` 是逐測試 ID 排序後的 SHA-256；歷史比較要求兩者相同。
4. `phases.compile`、`phases.test`、`phases.doc`：命令、開始 Unix 秒數、耗時、退出碼與狀態；缺少或無效紀錄有明確狀態。
5. `tests`：nextest JUnit 中每項測試的 ID、秒數及結果；`test_count` 是清單長度。`doc_tests` 保存 Cargo doc test 的通過、失敗與 ignored 數量。
6. `test_execution_seconds`：nextest 執行與 doc test 執行秒數總和，與編譯時間分開；任一階段缺漏時為 `null`。
7. `status` 與 `diagnostics`：只有三階段成功、JUnit 可解析且 doc test 結果存在時，狀態才是 `passed`。測試失敗時保留可取得的部分資料。

範例（數值僅示意）：

```json
{
  "schema_version": 1,
  "config_id": "nextest-0.9.148-workspace-locked-ci-doc-v1",
  "run_id": "12345",
  "commit": "abcdef123456",
  "workflow": "Work Rust macOS",
  "platform": "macos",
  "target": "native",
  "runner_os": "macOS",
  "runner_arch": "ARM64",
  "runner_image": "macos15",
  "toolchain": "1.85.0",
  "phases": {
    "compile": {"phase": "compile", "command": ["cargo", "test", "--no-run"], "started_unix_seconds": 1, "duration_seconds": 90.0, "exit_code": 0, "status": "passed"},
    "test": {"phase": "test", "command": ["cargo", "nextest", "run"], "started_unix_seconds": 100, "duration_seconds": 300.0, "exit_code": 0, "status": "passed"},
    "doc": {"phase": "doc", "command": ["cargo", "test", "--doc"], "started_unix_seconds": 400, "duration_seconds": 3.0, "exit_code": 0, "status": "passed"}
  },
  "tests": [{"id": "work-model::work-model::example", "duration_seconds": 0.01, "outcome": "passed"}],
  "test_count": 1,
  "test_ids_sha256": "示意值，實際為 64 字元 SHA-256",
  "doc_tests": {"passed": 1, "failed": 0, "ignored": 0},
  "test_execution_seconds": 303.0,
  "status": "passed",
  "diagnostics": []
}
```

歷史比較向前搜尋 main 成功 Workflow Run，使用最近 5 份有效且相容的同平台 Artifact（最多檢查 90 個 Run）。少於 5 份報告顯示「歷史資料不足」；環境、設定或測試 ID 不相容則標示不可比較。效能退化以本次 `test_execution_seconds > median × 1.30` 判定，警告不影響 Job 結果。
