---
name: Java Spring Boot 任務規劃
description: 固定 Spring Boot 或 Spring MVC 專屬契約時使用；未使用該框架的 Java 工作不適用。
metadata:
  work-tags:
    - spring
    - spring-boot
---

# Java Spring Boot 任務規劃指令

指令邊界：本層只固定 Spring Boot 與 Spring MVC 專屬契約，沿用 Java 共通規則。

## 1. Spring Web 契約

1. [預設] Spring MVC `@RequestParam` 多值參數使用 `Collection<T>`；需要順序或索引時使用 `List<T>`，需要唯一性時使用 `Set<T>`，不得以 `Iterable<T>` 取代框架已確認的綁定契約。Request Body／Response 等序列化邊界須依實際框架版本與往返證據決定。
2. [強制] TASK 須確認實際 Spring Boot、Spring MVC 與相關模組版本及受影響入口；未使用 Spring MVC 時不套用 Web 綁定規則。

## 2. 已確認四層架構

1. [強制] 只有專案已採用四層架構，或使用者明確核准架構調整時，才套用 `Controller → 業務 Service → Persistence Service → 資料庫存取層`；無法確認既有架構時先提出方案並逐項確認，未觸及程式不得主動重構。
2. [強制] Controller 只處理傳輸協定、輸入驗證、目前使用者資訊、請求／回應轉換及呼叫業務 Service，不得包含業務規則或直接呼叫其他層；目前使用者資訊依既有風格取得，預設只傳用例所需使用者 ID，業務確實需要時才傳既有使用者物件。業務 Service 不得存取 Controller、HTTP Session 或安全框架上下文。
3. [強制] 業務 Service 負責業務規則、用例流程、跨 Persistence Service 協調及交易邊界，不得直接查詢或持久化資料；Persistence Service 只封裝資料操作並呼叫資料庫存取層，不得包含業務規則、用例流程或交易邊界；資料庫存取層只執行查詢與持久化，不得包含資料操作編排或業務邏輯。
4. [預設] 四層架構的命名優先沿用專案；沒有明確慣例時，Controller 使用 `Controller`，業務 Service 使用 `Service`，但專案的 `Service` 已代表資料操作層時改用 `BusinessService`，Persistence Service 使用 `PersistenceService`，JPA 使用 `Repository`，MyBatis／MyBatis-Plus 使用 `Mapper`，介面不加 `I`，實作使用 `Impl`。所有預設須先納入方案並取得確認；其他架構不得套用。
5. [預設] 套件與目錄優先沿用專案；四層架構且沒有明確慣例時，方案使用 `controller/<功能>`、`service/business/<功能>`、`service/persistence/<功能>`，以及 JPA 的 `repository/<功能>` 或 MyBatis／MyBatis-Plus 的 `mapper/<功能>`，並在核准前確認；其他架構依既有結構固定。
