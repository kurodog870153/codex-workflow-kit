---
name: Java 持久層任務執行
description: 執行 TASK 已固定的 Java 持久層契約時使用；不涉及持久層的工作不適用。
metadata:
  work-tags:
    - persistence
    - relational-data
---

# Java 持久層任務執行指令

指令邊界：本層只核對受影響資料技術與 TASK 一致，實作細節由適用子層負責。

1. [強制] 執行前核對實際資料存取流程、技術分支、schema 來源與 TASK 一致；差異須記錄並交回 Task，不得自行更換持久層技術。
