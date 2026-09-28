---
name: Java 持久層任務規劃
description: 固定 Java 關聯式資料存取技術與共通契約時使用；不涉及持久層的工作不適用。
metadata:
  work-tags:
    - persistence
    - relational-data
---

# Java 持久層任務規劃指令

指令邊界：本層只固定持久層技術選擇與共通資料契約；JPA 與 MyBatis 細節由子層負責。

## 1. Reference 路由

1. [強制] TASK 涉及關聯式資料庫、schema、migration、SQL、交易、索引或資料映射時，載入 `task.programming-language.java.persistence.relational-data`（`references/relational-data.md`）。

## 2. 技術選擇

1. [強制] 使用 JPA 或 MyBatis／MyBatis-Plus 時，依實際受影響流程載入對應子層；兩者同時適用時維持各自邊界，不得互套 API 或預設只能選一種。
2. [強制] 涉及關聯式資料時須交叉確認相依與目標流程的註解、介面、設定、查詢或映射以判定 JPA、MyBatis 或 MyBatis-Plus；既有流程沿用實際鏈路，新流程有多個同樣適用選項時展示證據並由使用者選擇，不得把候選留給 Execute。
3. [強制] 技術選擇不授權新增或升級相依、processor、外掛、設定或 migration；同一不可分割 TASK 可涵蓋多個既有技術分支，但須分別固定流程邊界與指令層級。
4. [強制] 關聯式技術判定須交叉確認建置相依與實際設定、註解、介面、實作、查詢或映射；不得只因父模組、BOM、starter、傳遞相依或未引用程式碼存在就判定。JPA 至少核對 Persistence namespace、實際適用的 Spring Data 或 Hibernate 相依及 Entity／Repository／EntityManager／查詢；MyBatis 至少核對相依及 Mapper／XML／SqlSession／BaseMapper／實際查詢。
5. [強制] 既有流程依實際資料存取鏈路選擇，不能用 repository 其他位置的技術覆蓋；新流程有多個同樣適用選項、兩者皆未使用或證據衝突時，須展示證據並由使用者指定。正式 TASK 分別固定每個受影響流程的技術、模組、證據、實際版本、namespace／provider 與資料架構，不得留給 Execute。
