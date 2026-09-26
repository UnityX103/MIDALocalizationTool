# 翻译工作量与自动交接协议 v1

日期：2026-09-26。此文件是独立编辑器与 Unity 生产/回收端共同实现的协议。
保留数据 ZIP manifest v2 / 媒体 ZIP manifest v3、逐单元 JSON v1 和现有安全校验。
本扩展只增加 `manifest.json` 根字段，不新增 ZIP 条目，不复制台账到每个片段 JSON。
不支持本扩展的旧客户端不能用于维护工作量或自动交接。

## 计数与身份

- `countingRule: "han-v1"`：按 Unicode 标量计数，U+3007 以及闭区间
  U+3400–4DBF、U+4E00–9FFF、U+F900–FAFF、U+20000–2EBEF、
  U+2F800–2FA1F、U+30000–323AF 内每个标量计 1。其他字符计 0。
  这是固定的协议范围，不随运行时 Unicode 数据库变化。
- 按目标语言分别统计。相同原文不同 key 分别计；同一词条、同一中文原文重复确认不再计。
- 首次有效翻译、中文变更后的有效重译产生工作记录。单纯复核、不改译文确认、
  同一中文下润色不增加翻译字数；它们仍可改变交付内容。
- 已确认的既有译文计入当前完成进度，但不追溯归入当前台账的劳动量。
- `ledgerId` 是编辑器语言工作区的稳定 UUID，不是已认证的人员身份。
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
| `countingRule` | `"han-v1"` |
| `scope` | `{projectId, lineageId, language, partNames: [...]}`；精确匹配本包文本任务 |
| `progressChars` | 本范围当前原文已确认字数（合法显式空译文算确认） |
| `totalChars` | 本范围当前原文总字数 |
| `cumulativeChars` | `records[].chars` 求和，包括已作废的旧文工作 |
| `handoverChars` | 不在 `acknowledgedRecordIds` 的记录字数和 |
| `previousDeliveryDeltaChars` | `newRecordIds` 的记录字数和；相对上一交付快照，按记录 ID 集合差 |
| `sinceSourceUpdateChars` | `sourceUpdateRecordIds` 的记录字数和；按单元最近一次真正的新任务版本导入划界，同版本重复导入不重置 |
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
  "chars": 10,
  "confirmedAt": "ISO-8601 时间"
}
```

上例仅说明字段，不是可导入样例，`chars` 必须由 `sourceText` 按规则复算。
旧存档可能只保留最后一次部分导入的 manifest，其他单元无法追溯来源包。
这类启用后新工作允许 `sourcePackageId` 和 `sourceRevision` **同时为 null**，
明确表示无法追溯，不得伪造为别的单元的最近来源；`taskVersion/sourceText` 仍必填。
其他情况必须为非空来源包 ID 与正安全整数来源版本。回退后的工作使用回退快照的来源。
`kind` 为 `translation` 或 `source_revision`。记录不可原地改写，同 ID 内容冲突必须拒绝。
记录上限 100000；回执上限 10000；仍受 manifest 64 MiB、总文本 128 MiB 限制。
整数字段必须为非负安全整数（正数字段另注明）。

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
- 编辑器只确认匹配当前 ledger、已知 delivery 且该 delivery 覆盖的 recordIds。
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

## 界面

左下角显示当前语言空间的 `翻译进度：X 字 / Y 字　累计翻译：Z 字`。
逐字段悬停解释口径。问号打开对账说明，列出 `manifest.json` 中上述字段和统计范围。
界面为辅助计算；只导出部分任务时界面全空间数与 ZIP 本范围数不同。
功能启用前的历史工作不伪造补录；历史累计不足应明确说明。
