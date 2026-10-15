# 翻译工作量与自动交接协议 v1

更新日期：2026-10-15。此文件是独立编辑器与 Unity 生产/回收端共同实现的协议。
保留数据 ZIP manifest v2 / 媒体 ZIP manifest v3、逐单元 JSON v1 和现有安全校验。
本扩展只增加 `manifest.json` 根字段，不新增 ZIP 条目，不复制台账到每个片段 JSON。
不支持本扩展的旧客户端不能用于维护工作量或自动交接。

## 计数与身份

- 新交付使用 `countingRule: "mida-gmx-fullwidth-v2"`，单位是**计量词**。
  唯一项目扩展是全角字符逐个计词，数字仍按完整数值计数；其余词数规则严格依据 GMX-V 2.0。
  这是项目计量协议，不声明整个 ZIP 协议是 GMX-V XML 格式，
  全角扩展覆盖的字符不再折算。继续依据中文原文计量，各语言空间分别统计。
- 非全角文字遵循 §4.2.13：中文 /2.8、日文 /3、韩文 /3.3、泰文 /6。
  数字仍按完整数值计词，不纳入语言字符折算。六个泰文字母 `กกกกกก` 为 1 词，
  六个半角日文字母 `ｶｶｶｶｶｶ` 为 2 词；单个 `ก` 为 1/6 词。
  老挝语、高棉语、缅甸语没有规范认可的词数系数，相关非全角非标点字素仅累加到
  记录的 `characterCounts: {lao, khmer, myanmar}`，不得忽略字符量或自造词数。
- 新规则使用精确整数分子：`countingScale: 462`，`chars` 和所有 `*Chars` 的值为
  **1/462 词单位**。例如 6 词保存为 `2772`，1/6 词保存为 `77`。462 是上述折算
  分母的最小公倍数，属于无损表示方式，不是新的计数规则。明细与汇总均不提前取整，
  界面显示实际整数、有限小数或最简分数，合计六次 1/6 仍严格等于 1。
- 词界固定依据 GMX-V 2.0 的 Unicode 4.1.0 UAX #29：英文缩写及连字符复合词
  `can't`、`well-known`、`out-of-the-box` 各计 1；字母与数字相邻不拆词，`GPT4` 计 1；
  `3.14`、`1,000.00` 各计 1，`2026-10-15` 按连字符词界计 1。
  保留 GMX-V 的撇号后接元音拆分规则，例如 `l'objectif` 计 2。
- 先识别完整数值，再应用全角扩展：数字序列不逐位计数，包括 U+FF10–FF19 全角数字。
  数值内部的全角逗号、句点、分号按对应 GMX-V 数值分隔符识别；只有两侧均为数字时
  才合入数值，末尾标点仍按全角字符计词。`123`、`１２３`、`１２３．４５`、`１，０００．００`
  各计 1；`１２３，` 计 2。数字宽度识别仅用于计量，不修改原文。
- 全角扩展按 Unicode 17.0.0 East_Asian_Width 为 `W` 或 `F` 判定：除上述完整数字单元外，
  每个非空白全角字素计 1，包括汉字、全角标点、日文假名及全角字母。
  `你好，Hello 123。` 计 6；`ＡＢＣ` 计 3；`ＡＢＣ１２３，。` 计 6。
  不把非 `W/F` 的中点、引号、省略号、破折号另行加词；它们严格按 GMX-V 词界处理，
  如 `can’t` 计 1，`“Hello”` 计 1，独立 `……` 计 0。
  词界使用固定 Unicode 4.1.0 的规范分解、组合类排序与字素表，组合附加符不单独加词。
  不依赖系统 Unicode 版本，`l'ā` 与 `l'a\u0304` 都计 2；
  Hangul 组合字素与其预组合写法同数；不修改原文或其哈希。
- 计量空白依据 GMX-V §4.2.7：Unicode 4.1 空格/行/段分隔符（排除 U+00A0、U+2007、
  U+202F）、U+0009–000D、U+001C–001F、U+200B。不以“译文非空”的业务空白集合替代。
  U+00A0 保留 UAX #29 的 `ALetter` 词界属性，U+FEFF 按 `Format` 处理；
  `hello\u00a0world`、`hello\ufeffworld` 各计 1，不另加词界。
  独立半角标点不计词，正负号及百分号不额外加词，`-20`、`10%` 各计 1。
  稳定分类数据为仓库根 `workload-counting.json`，三端不依赖运行时的 Unicode
  版本或系统分词器；来源 URL、校验哈希和 Unicode 数据许可随仓库保留。
- 兼容读取的旧 `countingRule: "han-v1"`：按 Unicode 标量计数，U+3007 以及闭区间
  U+3400–4DBF、U+4E00–9FFF、U+F900–FAFF、U+20000–2EBEF、
  U+2F800–2FA1F、U+30000–323AF 内每个标量计 1。其他字符计 0。
  仅用于验证历史原件，不能标成新规则后继续使用旧汇总。
- 按目标语言分别统计。相同原文不同 key 分别计；同一词条、同一中文原文重复确认不再计。
- 首次有效翻译、中文变更后的有效重译产生工作记录。单纯复核、不改译文确认、
  同一中文下润色不增加翻译字数；它们仍可改变交付内容。
- 已确认的既有译文计入当前完成进度，但不追溯归入当前台账的劳动量。
- `ledgerId` 是一份台账生命周期内的稳定 UUID；用户明确归档后，新台账使用新的 UUID，旧台账的身份和记录保持不变。它不是已认证的人员身份。
  本版本不证明真实操作者；多人接力时不得把来源工作重新归为接收者贡献。
- 来源变更或删除不能删除旧工作记录。回退历史不回退累计台账与交接记录。
- 译文非空使用固定空白集合：U+0009–000D、U+001C–0020、U+0085、U+00A0、
  U+1680、U+2000–200A、U+2028、U+2029、U+202F、U+205F、U+3000、U+FEFF。
  只含以上标量的译文为空；原协议合法显式空译文例外不变。JS、Python、Rust、
  Unity 的进度复算与 ready 校验使用同一规则，不依赖运行时默认 trim/strip。

## manifest.json 根字段

`delivery`（编辑器交付包必带，Unity 来源包可不带）：

| 字段 | 类型 / 语义 |
| --- | --- |
| `version` | 数字 `1` |
| `id` | 等于该交付包 `packageId`，稳定非空标识 |
| `ledgerId` | 工作区台账 UUID |
| `revision` | 独立递增的交付序号，正安全整数 |
| `previousId` | 上一交付快照 ID；首份为 `null` |

`workload`（有 delivery 时必带）：

| 字段 | 类型 / 语义 |
| --- | --- |
| `version` | 数字 `1` |
| `countingRule` | 新交付为 `"mida-gmx-fullwidth-v2"`；兼容读取 `"han-v1"` 与修正前的 `"mida-gmx-fullwidth-v1"` 原件 |
| `countingScale` | 新规则固定为 `462`；`chars` / `*Chars` 的共同分母，必须验证 |
| `scope` | `{projectId, lineageId, language, partNames: [...]}`；精确匹配本包文本任务 |
| `progressChars` | 本范围当前原文已确认计量分子（合法显式空译文算确认） |
| `totalChars` | 本范围当前原文总计量分子 |
| `progressCharacterCounts` / `totalCharacterCounts` | 当前已确认 / 全部任务中仅计字符的 `{lao, khmer, myanmar}` 数量；新规则必填，由任务复算 |
| `cumulativeChars` | `records[].chars` 求和，包括已作废的旧文工作 |
| `handoverChars` | 不在 `acknowledgedRecordIds` 的记录计量分子之和 |
| `previousDeliveryDeltaChars` | `newRecordIds` 的记录计量分子之和；相对上一交付快照，按记录 ID 集合差 |
| `sinceSourceUpdateChars` | `sourceUpdateRecordIds` 的记录计量分子之和；按单元最近一次真正的新任务版本导入划界，同版本重复导入不重置 |
| `records` | 本范围累计工作记录，包括已从当前词条集合移出的记录 |
| `acknowledgedRecordIds` | 当前导出时已收到有效回执的记录 ID，records 的子集 |
| `newRecordIds` | records 的子集；上一快照没有的记录 |
| `sourceUpdateRecordIds` | records 的子集；本轮来源更新后完成的记录 |

每条 `records[]`：

```json
{
  "id": "稳定的工作记录 UUID",
  "partName": "来源单元",
  "language": "en",
  "key": "来源词条 key",
  "sourceText": "确认时的中文原文",
  "translation": "确认时的目标译文",
  "sourcePackageId": "该单元的来源包 ID",
  "sourceRevision": 1,
  "taskVersion": 1,
  "kind": "translation",
  "chars": 4620,
  "characterCounts": {"lao": 0, "khmer": 0, "myanmar": 0},
  "confirmedAt": "ISO-8601 时间"
}
```

上例仅说明字段，不是可导入样例，`chars` 必须由 `sourceText` 按本包 `countingRule` 复算。
为兼容字段名，`chars` 和各 `*Chars` 字段保留原名，新规则下的值均为精确计量分子，除以 `countingScale` 后才是词数。
新规则每条记录必须包含可从 `sourceText` 复算的 `characterCounts`，旧规则保持原始字段。
旧存档可能只保留最后一次部分导入的 manifest，其他单元无法追溯来源包。
这类启用后新工作允许 `sourcePackageId` 和 `sourceRevision` **同时为 null**，
明确表示无法追溯，不得伪造为别的单元的最近来源；`taskVersion/sourceText` 仍必填。
其他情况必须为非空来源包 ID 与正安全整数来源版本。回退后的工作使用回退快照的来源。
`kind` 为 `translation` 或 `source_revision`。记录的身份、原文、译文、来源及时间不可原地改写，
同 ID 的这些业务内容冲突必须拒绝。仅派生的 `chars` 与 `characterCounts` 可随明确声明的计数规则重算，
对比新旧规则记录前分别验证原件，再归一到新规则；不能忽略未经校验的数量差异。
记录上限 100000；回执上限 10000；仍受 manifest 64 MiB、总文本 128 MiB 限制。
整数字段必须为非负安全整数（正数字段另注明）。`characterCounts` 的值为实际字符数，不除以计量分母。

## Unity 回执

Unity 成功接收交付后持久保存以下信息，在后续来源包 `manifest.json` 根携带
`workReceipts: [...]`。不要求用户另导入/确认验收文件。

```json
{
  "version": 1,
  "id": "稳定回执 UUID",
  "projectId": "项目",
  "lineageId": "来源谱系",
  "language": "en",
  "ledgerId": "原交付 delivery.ledgerId",
  "deliveryId": "原交付 delivery.id",
  "recordIds": ["实际接收的工作记录 ID"],
  "receivedAt": "ISO-8601 时间"
}
```

- Unity 必须校验台账汇总与明细，按 `(ledgerId, recordId)` 去重；不能累加每包的
  `handoverChars`。同交付 ID 再次接收内容冲突必须拒绝，重复接收不重复计量。
- 不能只记录一个全局“最后版本号”：按项目、谱系、语言、ledgerId 保存接收身份与记录。
- 接收包括已作废旧文的工作量明细，不要求这些旧文重新写入本地化表。
- 来源/译文冲突与资源写入授权仍按既有协议处理，工作量清单不能绕过这些检查。
- 导入失败不能发出成功回执。若支持部分接收，recordIds 只包含确认的范围；
  不支持部分接收时保持全有或全无。
- 回执随下一次匹配的项目/谱系/语言来源包携带，必须重发持久保存的历史回执，
  不能仅发最后一份，否则跳过某次来源包会丢确认。
- 编辑器只确认匹配当前或本空间已归档 ledger、已知 delivery 且该 delivery 覆盖的 recordIds。
  归档台账的回执只更新该归档的确认摘要，不重写原始记录或增加当前台账的劳动量。
  同项目其他 ledger 的回执保留但不作用于当前台账。未知 delivery 不推测验收。
- 回执校验、任务合并、台账确认在一次工作区提交内完成。取消或保存失败不推进。
- 缺少 workReceipts 的旧包仍可导入；不据此推断验收。重复回执幂等，旧回执不回退状态。
- 自动确认仅表示交接接收，不表示付款；离线哈希/UUID 不是签名与人员认证。

## 导出与持久化

导出前冻结并保存交付身份、明细及汇总。相同内容、范围、来源身份和确认基线的重复导出
复用同一 packageId / delivery revision / exportedAt / 汇总；不同内容才产生新快照。
未成功输出的快照可在重试时复用，不能因此扣减工作量。只有有效回执推进确认。
导出快照保存失败时不输出 ZIP，避免产生无法识别其回执的包。
导出选择不同范围属于不同内容；上一交付比较取同 ledger 的上一快照并按记录集合差计算。
交付历史保留 ID、来源、范围、明细 ID 与汇总；不依赖最近 10 份工作区恢复备份。

## 旧数据重计与验收兼容

- 没有 `countingRule` 的本地台账按 `han-v1` 先校验；修正前的 `mida-gmx-fullwidth-v1`
  按冻结的旧算法验证。读取后均由保存的 `sourceText`
  重计并标记新规则。当前任务进度、活动与归档累计、待交接量都使用新口径；
  不补造未记录的历史劳动，不改变工作 ID、确认状态和已验收记录集合。
- 历史交付摘要保留自己的规则、汇总、指纹和身份。验证这些历史摘要时，按其规则
  从同一原文重建对应记录数量，不把新数量塞回旧交付。旧备份和归档原件不覆盖。
- 新规则下再次导出产生新的交付身份；重复导出同一新快照仍幂等。
  `newRecordIds` 仍按工作 ID 集合差计算，重计自身不产生新增劳动。
  旧交付回执仍可确认原工作 ID，后续待交接量按新规则求和。
- 旧归档的 ZIP 重导出也使用新规则。新交付摘要与原交付 ID 的关联保存在活动工作区的
  归档索引 `recountedDeliveries`，在输出 ZIP **之前**原子保存，重启后重建同一新快照；
  原冻结 ZIP 不重写。修正前 v1 的归档重计摘要继续保留，新 v2 摘要另建；
  每份归档最多保留 10 份这两版重计摘要，旧、新归档交付回执都只能确认实际交付的记录集合。
- 浏览器、Python 与 Rust 都按包声明的已知规则复算明细和汇总；未知规则、冒充新口径
  的旧数量以及数量篡改必须拒绝。尚不支持修正版规则及精确分母的外部接收端必须同步升级后，才能
  接收新交付；本仓库不修改或启动外部 Unity 工程。

## 显式归档与续卷

活动台账保留 100000 条记录和 10000 次交付的上限。用户可以在达到上限前或之后，
通过「工作量归档与继续生产」冻结完整台账、原有交付历史和当前片段的固定交付快照，
持久化归档成功后再切换到新台账。保存失败不推进当前台账身份。

归档不等于接收，不清空旧台账待交接量；后续来源 ZIP 的匹配回执仍更新归档确认摘要。
记录按原始身份和内容去重，同一词条、同一原文的既有工作不能再次归入新台账。
工作区文件版本在续卷后继续递增；新台账的 `delivery.revision` 从自己的首份交付开始。
ZIP 仍采用 workload v1 / manifest v2，不把本地归档存储字段加入交付协议。

固定 ZIP 每包最多 200 个当前片段；归档允许另外保存至多 5 份冻结交付快照，
不截断原有 10000 次交付历史。完整台账 JSON 保留已移出片段的工作明细；
已移出的片段数据应核对当时交付 ZIP，不能从仅含工作量明细的台账重建整个片段。

## 界面

左下角显示当前语言空间的 `翻译进度：X 词 / Y 词　累计翻译：Z 词`。
逐字段悬停解释口径。问号打开对账说明，列出 `manifest.json` 中上述字段和统计范围。
界面为辅助计算；只导出部分任务时界面全空间数与 ZIP 本范围数不同。
归档后界面累计词数包括活动台账和本空间归档；每个 ZIP 的累计和待交接量仍只代表该包的台账及片段范围，
不能把不同台账的重复交付包总量直接相加，应按 `(ledgerId, recordId)` 去重。
功能启用前的历史工作不伪造补录；历史累计不足应明确说明。

依据：[GMX-V 2.0（ETSI GS LIS 004）](https://www.etsi.org/deliver/etsi_gs/lis/001_099/004/02.00.00_60/gs_lis004v020000p.pdf)，
[Unicode 4.1.0 词界](https://www.unicode.org/reports/tr29/tr29-9.html)。全角逐个计词为本项目扩展。

## 计数验收

- 离线共享示例覆盖全角、数字、词界、语言折算、仅字符计量及规范等价形式，三端使用相同输入与精确预期分子。
- 官方 Unicode 4.1 参考语料采用 [NormalizationTest.txt](https://www.unicode.org/Public/4.1.0/ucd/NormalizationTest.txt)
  与 [WordBreakTest.txt](https://www.unicode.org/Public/4.1.0/ucd/auxiliary/WordBreakTest.txt)。
  下载到独立目录后可运行 `python3 scripts/workload-acceptance-check.py --unicode-dir <目录>`；
  普通词界核对排除 GMX-V 明确指定的差异、语言折算及全角扩展，相关情况由项目示例核对。
- JS 与 Rust 可通过 `UNICODE_NORMALIZATION_TEST` 指向官方正规化语料执行同一等价性验收。
  Rust 的该项为显式 opt-in，常规检查不下载资料或访问真实工作区。
