---
name: Java Web 後端任務執行
description: 依已確認的 Java 或 Spring 技術基準執行 Web 後端修改與驗證時使用；其他後端語言不適用。
metadata:
  work-tags:
    - java
    - spring
---

# Java Web 後端任務執行指令

指令分類狀態：已完成
指令邊界：本層只在已確認 Java 技術基準上執行 TASK 的修改與 VAL，不選擇版本、架構、型別、建置或測試策略。

1. [強制] 執行前以既有入口核對實際 JDK、建置工具、框架、依賴與目標模組仍符合 TASK；不一致時記錄證據並以規格缺陷停止。
2. [強制] 依 TASK 的步驟與既有風格修改，保持未列程式碼不變；需要新依賴、外掛、模組、產生器或工具時停止重新授權。
3. [強制] 依 TASK 執行 Red／Green、編譯、測試、靜態分析與其他適用 VAL；是否 clean 只由 TASK 與本次驗證目的決定，不得固定追加。
4. [強制] TASK 適用 Swagger reference 時，須實際產生或載入 OpenAPI 契約並執行完整性 VAL；Task 階段已確認的 Endpoint 契約不重複詢問，只有實際產物與核准內容不同時才停止並重新確認。TASK 不適用 Swagger 時，不得新增相關依賴、設定或工具，也不得因此阻擋 Controller 變更。
5. [強制] 產生來源與產生檔不一致時只修改 TASK 明列的正式來源並重新產生，不得手動編輯產生檔掩蓋差異。
6. [強制] 測試 VAL 須記錄目標模組、指定測試與實際測試數，零測試、只編譯或未到達目標測試不得記為通過。
7. [強制] 執行前須核對 TASK 的實際 Task instruction hierarchy 與 references 已完整固定建置入口、版本證據、架構、集合、日期時間、Lombok、資料技術及適用驗證；缺漏或與現況不符時以規格缺陷停止，不得由 Execute 補決策。
8. [強制] 每個 CMD 使用 TASK 固定環境與精確參數；TASK 固定執行檔、`JAVA_HOME` 或 toolchain 時必須使用該精確值，只有 TASK 明確指定 PATH 解析時才能使用 PATH。不得切換建置系統、改用其他 Wrapper／執行檔、安裝工具、替換版本、追加 clean 或改寫測試篩選；等價命令修正只依 Execute 通用授權流程處理。測試須確認指定 module／project、task／goal 與測試實際執行且數量大於零。
