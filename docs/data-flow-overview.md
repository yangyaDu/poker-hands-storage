# Proto V3 数据流概览

更新日期：2026-07-16

## 离线导出

```text
source range.db
  -> discover dimensions
  -> load drill + abstract/concrete mappings
  -> source concrete id 按升序重编号为 V3 id 1..N
  -> encode paged metadata + fixed-width mmap indexes
  -> load every strategy cell（包括 NULL EV）
  -> encode HandStrategy payloads + direct locator index
  -> write manifest last
  -> standalone read-back
  -> SQLite cross verify
```

导出在 sibling `.building` 目录完成，manifest 最后发布。源 SQLite 只用于离线导出、验证和 benchmark。

## 在线查询

```text
HTTP / native SDK
  -> V3Facade dimension handle pool
  -> V3QueryService
  -> metadata hash index/page cache 或 strategy direct index/decode cache
  -> 业务 QueryResult / ConcreteLineRow / hand list
```

每个 handle 共享只读 mmap。metadata page 与 decoded strategy cache 均有 byte budget；handle pool 也有
容量上限。线上路径不创建 SQLite connection，不读取 `meta.db`/`lines.db`，不探测 V2 格式。

NULL EV 使用 `frequency_x10000=20000, hand_ev_x10000=0` 存盘，并在业务 API 解码为
`frequency=0, hand_ev=null`。
