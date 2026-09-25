---
name: Java Web 後端需求規劃
description: Java 或 Spring 平台限制影響 Web 後端成果、部署或既有消費者時使用；其他後端語言不適用。
metadata:
  work-tags:
    - java
    - spring
---

# Java Web 後端需求規劃指令

指令分類狀態：已完成
指令邊界：本層只記錄 Java 平台造成的需求限制，不固定版本、依賴、框架 API、建置或測試做法。

1. [強制] 只有 Java 相容性、部署平台或既有消費者結果屬於需求限制時，才能寫入 Plan；實際 JDK、框架、依賴與語言寫法由 Task 依專案證據確定。
2. [強制] 不得在 Plan 預設四層架構、JPA、MyBatis、Lombok、Swagger 實作、特定建置工具或資料庫產品。
3. [強制] 已確認且會影響成果的 Java／Spring 模組、相容性或架構限制可寫入 Plan；不得因此指定 Maven、Gradle、JUnit、測試類型、測試檔案、命令或實作方式。
4. [強制] Java Plan 的提問、選項、摘要與草案只能確認 Java 平台造成的需求限制；不得提出或固定 Controller、Service、Repository、Mapper、Entity、DTO、Java 型別、方法簽章、註解或套件結構，相關決策由 Task 依專案證據確定。
