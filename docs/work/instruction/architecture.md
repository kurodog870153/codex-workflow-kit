# Work 指令階層架構規範

## 1. 範圍

本文件規範 `skills/work/references/instructions/` 的節點分類、指令歸屬與路徑組合方式，供新增及調整指令時遵循。

## 2. 指令歸屬

1. `plan`、`task`、`execute` 各自保留 `general/instructions.md`，存放該模式通用的規則。
2. 跨多種應用或框架適用的規則放在共同父節點；適用範圍較窄的規則放在對應子節點。
3. 程式語言放在 `programming-language/<語言>`；應用類型、框架與能力若可獨立適用，應分為可組合的路徑，不因某一種常見組合而固定為彼此的祖先與子孫。
4. 每個節點須有明確責任邊界，避免與父節點、兄弟節點或 reference 重複。
5. 僅在多條獨立路徑同時適用的規則，應放在責任所屬節點並明述適用條件；不為單一組合增設交叉節點。

## 3. 目錄結構

```text
instructions/
├── plan/
│   ├── general/instructions.md
│   ├── programming-language/
│   │   ├── instructions.md
│   │   └── <語言>/instructions.md
│   └── <領域>/...
├── task/
│   ├── general/instructions.md
│   ├── programming-language/
│   │   ├── instructions.md
│   │   └── <語言>/instructions.md
│   └── <領域>/
│       ├── instructions.md
│       ├── <應用類型>/instructions.md
│       ├── <框架>/instructions.md
│       └── <能力>/
│           ├── instructions.md
│           └── <實作方式>/instructions.md
└── execute/
    ├── general/instructions.md
    ├── programming-language/
    │   ├── instructions.md
    │   └── <語言>/instructions.md
    └── <領域>/...
```

`<領域>`、`<語言>` 等標記表示分類位置，不要求每個領域都建立所有種類的節點。每個節點及其祖先都須有 `instructions.md`。

## 4. 路徑選擇

1. 一項工作可選擇多條適用的葉節點路徑，組合其共通、程式語言、應用、框架與能力規則；應用路徑不得隱含特定語言。
2. 同一選擇不得同時指定祖先及其子節點；載入時依選定路徑展開祖先，並去除重複。
3. Plan 依其目錄解析可用的最深祖先；Task 與 Execute 所需的路徑須在各自模式具備完整祖先節點。
4. 指令須靠責任邊界避免衝突，不依賴載入順序掩蓋相互矛盾的規則。

## 5. YAML frontmatter

本節適用於 `skills/work/references/instructions/` 內所有 `.md`，包括節點的 `instructions.md` 與 `references/` 下的參考指令。

1. 每份檔案都須從第一行起以 `---` 包住有效的 YAML frontmatter，並在結束的 `---` 後撰寫正文。節點的 YAML 頂層只允許 `name`、`description`、`metadata`；`name` 須為非空的可讀名稱，`description` 須簡短說明何時適用及何時不適用。
2. `metadata` 只允許 `work-tags`。其值須為非空陣列，包含不重複的小寫 kebab-case 字串，用來標示具辨識力的技術或能力，不重複目錄階層。
3. 參考指令的 YAML 頂層除了上述欄位，還須有 `reference-name`。此欄位是全專案唯一的識別碼，格式為 `<模式>.<節點路徑>.<reference 檔名>`；模式與節點路徑由所在目錄決定，檔名不含 `.md`。
4. 節點依目錄階層選擇。參考指令依父節點的觸發條件及相對路徑載入；載入時須核對 `reference-name` 與實際路徑及所選名稱一致。正文不宣告參考名稱、適用層級或 `指令分類狀態`；指令分類是否完成由實際內容與驗證結果判定。

節點範例：

```yaml
---
name: 領域指令
description: 處理此領域的工作時使用；不涉及此領域的工作不適用。
metadata:
  work-tags:
    - domain-capability
---
```

Reference 範例：

```yaml
---
name: 領域參考指令
description: 涉及此領域的特定能力時使用；不涉及此能力的工作不適用。
reference-name: task.domain.example
metadata:
  work-tags:
    - domain-capability
---
```

## 6. 節點變更要求

1. 建立節點前，確認適用範圍、父子關係與各模式的責任。
2. 維持有效的 frontmatter、祖先節點及 reference 路由。
3. 路徑或來源名稱變更時，檢查安裝器、引用、測試及既有正式文件的階層選擇與指紋。
4. 依既有驗證流程確認目錄、選擇及來源載入結果。
