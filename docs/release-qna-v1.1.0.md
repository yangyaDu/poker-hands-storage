# poker-hands-storage v1.1.0 汇报 Q&A

## 这份文档怎么用

这份 Q&A 覆盖了汇报时听众最可能提出的问题，按主题分类。每个问题都给出简洁回答和深入解释。

---

## 一、项目整体

### Q: 这个项目到底是做什么的？

**简短回答**：我们把德州扑克策略数据从 SQLite 迁移到了自定义二进制格式，查询更快、占盘更小。

**深入解释**：原始数据是一个 1.45GB 的 SQLite 数据库，包含约 2380 万条 poker hand strategy 记录。每次查询都需要打开 SQLite 文件、解析 SQL、遍历 B-tree。我们把这些数据转换成了 `.bin` + `.idx` 二进制格式，查询时直接用 mmap 内存映射 + 密集索引 O(1) 定位，省去了 SQL 解析和 B-tree 遍历的开销。

### Q: 为什么要自己做二进制格式，不用 SQLite 优化一下？

**简短回答**：SQLite 的 B-tree 遍历和 SQL 解析对这种固定结构的只读数据来说是浪费。

**深入解释**：我们的数据是只读的、结构固定的（每行都是 concrete_line_id + hole_cards + action + frequency + hand_ev）。SQLite 的 B-tree 索引、事务日志、SQL 解析器这些特性对我们都是不必要的开销。自定义二进制格式可以针对我们的数据结构做极致优化：固定大小的 record、连续索引、零拷贝 mmap。

### Q: 这个项目的核心创新点是什么？

**简短回答**：密集索引 + 零拷贝 mmap + 位掩码过滤的组合设计。

**深入解释**：
- **密集索引**：`.idx` 文件要求 `concrete_line_id` 是连续的，这样查找不需要二分搜索，直接 `index = id - first_id` 下标访问
- **零拷贝 mmap**：`.bin` 和 `.idx` 文件整个映射到内存，OS 按需加载页，没有 `read()` 系统调用
- **位掩码过滤**：hands-by-actions 查询把 169 手牌的动作信息编码为位向量，一次位运算筛选全部手牌

---

## 二、二进制编码

### Q: 你们的二进制格式长什么样？

**简短回答**：`.bin` 是数据包串联，`.idx` 是指向每个包位置的索引。

**深入解释**：

`.bin` 文件结构：
```
[16字节 PFSP 头][Pack 1][Pack 2][Pack 3]...
```

`.idx` 文件结构：
```
[16字节 PFXI 头][22字节记录1][22字节记录2]...
```

每条 idx 记录包含：concrete_line_id、action_schema_id、hand_count、在 .bin 中的偏移量、包长度、CRC32C 校验码。

### Q: 169 手牌字典是什么？

**简短回答**：德州扑克 169 种等价手牌的整数编码。

**深入解释**：标准德州扑克有 1326 种不同的两张牌组合，但由于对称性（AKs = KAs = sAK），可以压缩到 169 种。我们用 13x13 矩阵编码：对角线是排列型（AA, KK...），上三角是同花不同花（AKs），下三角是不同花（AKo）。每种手牌用一个 `u8` (0-168) 表示，节省存储空间。

### Q: Pack 的格式为什么是 `hand_count * (5 + action_count * 8)`？

**简短回答**：每个手牌固定占用 5 + action_count*8 字节。

**深入解释**：
- 1 字节：hand_id (0-168)
- 4 字节：action_mask (u32 位掩码，第 N 位为 1 表示第 N 个动作存在)
- 8 字节 × action_count：每个动作的 frequency(f32) + hand_ev(f32)

这种固定大小的设计让我们可以直接通过算术偏移计算任意手牌的位置，不需要遍历或解析变长字段。

### Q: 为什么用 CRC32C 而不是 CRC32？

**简短回答**：CRC32C (Castagnoli 多项式) 比传统 CRC32 对大数据块有更好的检错能力。

**深入解释**：CRC32C 是 iSCSI 标准采用的多项式，在硬件层面有 SSE4.2 和 ARM CRC 指令加速。我们的 `crc32c` crate 会自动选择最优实现：有 SSE4.2 用 SSE4.2，有 ARM CRC 用 ARM CRC，否则回退到纯软件实现。

---

## 三、Benchmark

### Q: Hot 和 Cold benchmark 的区别是什么？

**简短回答**：Hot 测的是服务正常运行时的查询性能，Cold 测的是首次启动的性能。

**深入解释**：
- **Hot**：服务已打开、维度文件已 mmap、schema 已加载。测的是纯粹的数据查询和编解码性能。
- **Cold**：全新进程启动，OS page cache 中没有数据。测的是文件打开、mmap 建立、第一次数据解码的完整开销。

### Q: 为什么 Cold benchmark 要用子进程？

**简短回答**：确保每次测量都是全新的进程状态。

**深入解释**：如果在同一个进程中多次测量 cold start，之前的 mmap 文件描述符、OS page cache、Rust 内存分配器缓存都会影响结果。通过 spawn 子进程，每次都是从零开始，测出来的时间才是真实的冷启动时间。

### Q: Workload 是怎么生成的？为什么强调确定性？

**简短回答**：从 SQLite 源数据中按种子随机采样，确定性保证每次生成的 workload 一致。

**深入解释**：我们实现了 `SeededRandom`，给定相同的 seed 总是产生相同的随机序列。这样：
1. 不同版本的 benchmark 结果可比较（用同样的 workload）
2. workload 可以序列化到 JSON，跨次复用
3. 回归测试时可以精确复现

### Q: 为什么 binary 比 SQLite 快这么多？底层原理是什么？

**简短回答**：省掉了 SQL 解析、B-tree 遍历、文本解码三个主要开销。

**深入解释**：

| 步骤 | SQLite | Binary |
|------|--------|--------|
| 定位数据 | SQL 解析 -> 准备语句 -> B-tree 遍历 | 密集索引 O(1) 下标访问 |
| 读取数据 | read() 系统调用 -> 页缓存 -> 行解析 | mmap 零拷贝 -> 直接取切片 |
| 解码数据 | 文本字段解析 -> 类型转换 | 固定偏移位运算 |
| 动作名称 | SQL JOIN meta.db | HashMap 查找 |

### Q: hands-by-actions 查询为什么 binary 比 SQLite 快 9.45 倍？

**简短回答**：SQLite 用 OR 条件过滤，binary 用位向量运算。

**深入解释**：SQLite 的查询是 `WHERE (action = 'call' OR action = 'fold') AND frequency > 0.005`，需要对每行做字符串比较和逻辑运算。binary 的方式是：先把整个 pack 解码到位掩码数组 `hand_masks[169]`，然后 `hand_mask & filter_mask != 0` 一次位运算判断所有手牌。

---

## 四、验证体系

### Q: Standalone 和 Cross 验证有什么区别？

**简短回答**：Standalone 检查文件自己内部是否一致，Cross 检查二进制和 SQLite 源数据是否一致。

**深入解释**：
- **Standalone**：假设 manifest.json 是正确的，验证 .idx 指向的 .bin 区域是否合法、CRC32C 是否匹配、pack 格式是否正确
- **Cross**：从 SQLite 源数据中取样本，逐单元格和二进制解码结果对比，验证数据转换过程没有出错

### Q: Float32 bit-exact 是什么意思？为什么不用容差比较？

**简短回答**：要求 f64->f32->f64 截断后的 bit 模式完全一致。

**深入解释**：源 SQLite 存的是 f64 (8 字节)，我们的二进制存的是 f32 (4 字节)。转换过程是 `f64_value as f32 as f64`。bit-exact 意味着我们存储的 f32 的 IEEE 754 bit 模式，和 f64 截断到 f32 后的 bit 模式完全一样。如果用容差（比如 1e-6），就无法区分是精度损失还是真正的编码错误。

### Q: Cross verify 的采样策略是什么？

**简短回答**：按维度大小比例分配采样配额，使用确定性哈希排序。

**深入解释**：如果总共 2380 万条记录要采样 10000 条，那么每个维度的采样数 = `(该维度行数 / 总行数) * 10000`。SQL 排序用 `(concrete_line_id * 1103515245 + id * 12345) & 0x7FFFFFFF` 作为伪随机排序键，保证每次采样结果一致。

### Q: 2380 万条记录全量验证通过了吗？

**简短回答**：是的，sample_size = 0 时全量验证通过，失败数为 0。

**深入解释**：这是 v1.0.0 阶段的成果。全量验证覆盖了 9 个维度（不同策略、玩家数、深度组合），逐单元格对比了 frequency 和 hand_ev，float32 bit-exact 匹配。

---

## 五、断点续跑

### Q: build-state.json 记录了什么？

**简短回答**：记录了每个维度的构建状态和源数据校验和。

**深入解释**：
- `source_checksum`：源数据库的校验和，防止用错数据源重建
- 每个维度的 `status`：pending / in_progress / completed / failed
- 每个维度的文件路径和统计数据（concrete_line_count、pack_count）
- 全局参数（max_concrete_lines）

### Q: Resume 时怎么保证已完成维度没损坏？

**简短回答**：重新校验 .idx/.bin 文件的大小和 CRC32C。

**深入解释**：Resume 流程会对每个 completed 维度：
1. 检查文件是否存在
2. 检查文件大小是否与 build-state.json 中记录的一致
3. 如果启用了 checksum 验证，重新计算 CRC32C 对比

### Q: 为什么 --resume 和 --overwrite 不能一起用？

**简短回答**：语义冲突。

**深入解释**：`--resume` 的意思是"接着上次中断的地方继续"，`--overwrite` 的意思是"从头开始全部重建"。这两个意图矛盾，不允许同时使用是为了防止误操作丢失已有的构建成果。

---

## 六、Native SDK

### Q: Native SDK 和 HTTP Service 有什么区别？

**简短回答**：SDK 是直接嵌入到 Node.js 进程中的原生模块，HTTP Service 是独立的网络服务。

**深入解释**：
- **Native SDK**：通过 napi-rs 编译成 `.node` 原生模块，导入到 JS 中直接调用，零网络开销，适合 Bun/Node.js 应用内集成
- **HTTP Service**：独立的 axum HTTP 服务，通过 REST API 访问，适合多语言客户端、分布式部署

两者底层用的是同一套 `range-store-core` 查询逻辑。

### Q: napi-rs 是什么？

**简短回答**：Rust 到 JavaScript/N-API 的绑定框架。

**深入解释**：napi-rs 让 Rust 函数可以直接暴露给 Node.js/Bun 调用。我们在 `range-store-native/src/lib.rs` 中用 `#[napi]` 注解标记导出的函数，编译后生成 `index.node` 原生模块。JS 层的 `index.js` 包装了一层，把原生返回值转换为业务信封格式（code/data/message）。

### Q: SDK 的 error envelope 是什么格式？

**简短回答**：`{ code, data, message }`。

**深入解释**：
```typescript
{
  code: 0,           // 0=成功, 非0=错误码
  data: { ... } | null,  // 成功时返回数据，失败时为 null
  message: null      // 失败时包含错误描述
}
```
这种格式和 HTTP Service 的响应保持一致，方便上层统一处理。

---

## 七、架构与设计

### Q: 为什么分成 4 个 crate？

**简短回答**：按职责分离，每个 crate 有明确的边界。

**深入解释**：
- `range-store-core`：纯 Rust 库，不依赖 HTTP 或 CLI，可以被任何消费者引用（service、native SDK、独立工具）
- `service`：依赖 core，提供 HTTP API
- `range-store-native`：依赖 core，提供 JS 绑定
- `storage-tools`：依赖 core，提供构建/验证/benchmark CLI

core 不依赖任何其他项目 crate，这是关键的设计约束。

### Q: LRU Handle Pool 的作用是什么？

**简短回答**：缓存已打开的维度 reader，避免重复打开文件。

**深入解释**：每个维度对应一对 `.idx` + `.bin` 文件，打开时需要 mmap。频繁打开/关闭会影响性能。Handle Pool 维护一个最多 N 个 dimension reader 的缓存（默认 2，可通过 `PHS_MAX_OPEN_HANDLES` 配置），使用 LRU 策略淘汰不常用的维度。

### Q: mmap 的安全风险是什么？

**简短回答**：如果文件在运行时被外部修改，可能导致 SIGBUS。

**深入解释**：mmap 的契约是文件在映射期间保持不变。我们的部署策略是通过版本化目录（带时间戳）+ 原子 swap 来解决这个问题：先写入新版本到临时目录，验证通过后原子重命名，旧版本同时移除。
